//! Element-wise map vectorization — the general, sound SIMD lever.
//!
//! Recognizes a region that is a PURE ELEMENT-WISE MAP — a loop `for i in 1..=n`
//! (1-based, the Logos array convention) whose body reads one or more arrays at
//! index `i`, computes a straight-line float expression, and writes the result
//! to an array at index `i`, with the
//! induction variable `i` as the ONLY loop-carried value. Such a loop is
//! bit-exactly vectorizable: lane `i` computes exactly `f(a[i], b[i], ...)`
//! whether scalar or packed, so 2-wide (or wider) SIMD changes nothing about the
//! per-element result — unlike a REDUCTION (`sum += a[i]`), whose 2-lane form
//! reassociates the float adds and is therefore NOT bit-identical (and is
//! rejected here).
//!
//! This is the recognizer half (a pure analysis). The codegen half lowers each
//! body op to its packed form (`AddF`->`addpd`, …) over 2 lanes with a scalar
//! tail; the packed primitives live in [`crate::x64asm`].

use crate::jit::{Cmp, MicroOp, Slot};
use std::collections::HashSet;

/// A recognized element-wise map loop. The straight-line load/compute/store body
/// is `ops[body_start..body_end]`; the guard, increment, and back-edge surround it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MapPlan {
    /// The induction variable slot `i` (a 1-based array index).
    pub induction: Slot,
    /// The loop-limit slot `n` (the guard `Branch` right operand).
    pub limit: Slot,
    /// The guard comparison: `Lt` (`i < n`) or `LtEq` (`i <= n`). Decides how many
    /// elements a full SIMD pair needs and where the scalar tail begins.
    pub cmp: Cmp,
    /// The straight-line load/compute/store body is `ops[body_start..body_end]`.
    pub body_start: usize,
    pub body_end: usize,
}

/// Returns `Some(MapPlan)` iff `ops` is a pure element-wise map region in the
/// top-tested `while` shape that `adapt_region` produces:
///
/// ```text
///   [0]          Branch { cmp(Lt|LtEq), lhs:i, rhs:n, .. }   // loop guard (exit when cmp FALSE)
///   [1..j-2]     body: ArrLoad{idx==i,checked:false} / float arith / ArrStore{idx==i,checked:false}
///   [j-2]        LoadConst { dst:step, value:1 }
///   [j-1]        Add { dst:i, lhs:i, rhs:step }              // increment
///   [j]          Jump { target:0 }                           // back-edge to head
///   [j+1..]      exit (Return)
/// ```
///
/// Any reduction (loop-carried operand), cross-index access, bounds-checked
/// access, non-unit stride, call, or other op makes it `None` — the
/// conservative, sound side.
pub fn recognize_elementwise_map(ops: &[MicroOp]) -> Option<MapPlan> {
    // guard + >=1 body op + step LoadConst + increment + back-Jump.
    if ops.len() < 5 {
        return None;
    }

    // [0]: the loop guard — `Branch` jumps to its target when the comparison is
    // FALSE, i.e. it falls through (continues) while `i </<= n`.
    let (induction, limit, cmp) = match &ops[0] {
        MicroOp::Branch {
            cmp: c @ (Cmp::Lt | Cmp::LtEq),
            lhs,
            rhs,
            ..
        } => (*lhs, *rhs, *c),
        _ => return None,
    };

    // The back-edge `Jump { target: 0 }`.
    let j = ops
        .iter()
        .position(|o| matches!(o, MicroOp::Jump { target: 0 }))?;
    if j < 4 {
        return None; // need [0]guard [1..]body [j-2]LoadConst [j-1]increment
    }

    // [j-1]: the induction increment `i = i + step`.
    let step = match &ops[j - 1] {
        MicroOp::Add { dst, lhs, rhs } if *dst == induction && *lhs == induction => *rhs,
        _ => return None,
    };
    // [j-2]: the stride-1 constant feeding it. The kernel processes EVERY index,
    // so a non-unit / unknown stride (a loop over a subset) must be rejected.
    match &ops[j - 2] {
        MicroOp::LoadConst { dst, value } if *dst == step && *value == 1 => {}
        _ => return None,
    }

    // The straight-line body is everything between the guard and the increment
    // machinery: `ops[1..j-2]` (load/compute/store only — no LoadConst/Add here).
    let (body_start, body_end) = (1usize, j - 2);
    let body = &ops[body_start..body_end];
    if body.is_empty() {
        return None;
    }

    // The body must be straight-line: every slot an op reads is either the
    // induction variable, a constant, or a value DEFINED EARLIER IN THE BODY.
    // A read of a slot not yet defined in the body is a loop-carried value
    // (e.g. a reduction accumulator) — NOT bit-exactly vectorizable -> reject.
    let mut defined: HashSet<Slot> = HashSet::new();
    defined.insert(induction);
    let mut wrote_an_array = false;

    let known = |s: &Slot, defined: &HashSet<Slot>| defined.contains(s);

    for op in body {
        match op {
            MicroOp::LoadConst { dst, .. } => {
                defined.insert(*dst);
            }
            MicroOp::ArrLoad {
                dst, idx, checked, ..
            } => {
                if *idx != induction || *checked {
                    // cross-index read, OR a bounds-checked access whose length the
                    // kernel can't prove >= n (the kernel elides per-element checks,
                    // so it is sound ONLY when the Oracle already proved in-bounds,
                    // i.e. `checked == false`).
                    return None;
                }
                defined.insert(*dst);
            }
            MicroOp::AddF { dst, lhs, rhs }
            | MicroOp::SubF { dst, lhs, rhs }
            | MicroOp::MulF { dst, lhs, rhs }
            | MicroOp::DivF { dst, lhs, rhs } => {
                if !known(lhs, &defined) || !known(rhs, &defined) {
                    return None; // loop-carried operand (reduction) — unsound
                }
                defined.insert(*dst);
            }
            MicroOp::SqrtF { dst, src } => {
                if !known(src, &defined) {
                    return None;
                }
                defined.insert(*dst);
            }
            MicroOp::ArrStore {
                src, idx, checked, ..
            } => {
                if *idx != induction || *checked || !known(src, &defined) {
                    return None;
                }
                wrote_an_array = true;
            }
            // Any other op (Call, Move, integer arithmetic, comparisons,
            // list/map/byte ops, a second induction) means this is not a pure
            // element-wise float map — reject conservatively.
            _ => return None,
        }
    }

    // A map must actually STORE a result; a body that only loads is not a map.
    if !wrote_an_array {
        return None;
    }

    Some(MapPlan {
        induction,
        limit,
        cmp,
        body_start,
        body_end,
    })
}

/// Lower one straight-line float-arithmetic body op to its 2-wide PACKED form,
/// computing `dst = lhs OP rhs` over both lanes. `xmm(slot)` maps a body slot to
/// its assigned lane register; `scratch` is a free XMM for the awkward
/// `dst == rhs` non-commutative case. SSE packed ops are 2-operand (`x OP= y`),
/// so this materializes the 3-operand `dst = lhs OP rhs` form, never clobbering
/// an operand it still needs. Loads/stores/LoadConst are handled by the loop
/// driver, not here.
#[cfg(target_arch = "x86_64")]
pub fn emit_packed_arith(
    asm: &mut crate::x64asm::Asm,
    op: &MicroOp,
    xmm: impl Fn(Slot) -> crate::x64asm::Xmm,
    scratch: crate::x64asm::Xmm,
) {
    use crate::x64asm::Asm;
    match op {
        MicroOp::AddF { dst, lhs, rhs } => emit_bin(
            asm,
            xmm(*dst),
            xmm(*lhs),
            xmm(*rhs),
            scratch,
            Asm::addpd_rr,
            true,
        ),
        MicroOp::MulF { dst, lhs, rhs } => emit_bin(
            asm,
            xmm(*dst),
            xmm(*lhs),
            xmm(*rhs),
            scratch,
            Asm::mulpd_rr,
            true,
        ),
        MicroOp::SubF { dst, lhs, rhs } => emit_bin(
            asm,
            xmm(*dst),
            xmm(*lhs),
            xmm(*rhs),
            scratch,
            Asm::subpd_rr,
            false,
        ),
        MicroOp::DivF { dst, lhs, rhs } => emit_bin(
            asm,
            xmm(*dst),
            xmm(*lhs),
            xmm(*rhs),
            scratch,
            Asm::divpd_rr,
            false,
        ),
        MicroOp::SqrtF { dst, src } => asm.sqrtpd_rr(xmm(*dst), xmm(*src)),
        _ => {}
    }
}

/// Materialize `d = l OP r` from a 2-operand packed `op(x, y): x OP= y`, without
/// destroying an operand before it is consumed.
#[cfg(target_arch = "x86_64")]
fn emit_bin(
    asm: &mut crate::x64asm::Asm,
    d: crate::x64asm::Xmm,
    l: crate::x64asm::Xmm,
    r: crate::x64asm::Xmm,
    scratch: crate::x64asm::Xmm,
    op: fn(&mut crate::x64asm::Asm, crate::x64asm::Xmm, crate::x64asm::Xmm),
    commutative: bool,
) {
    if d == l {
        op(asm, d, r); // d already holds l
    } else if d == r {
        if commutative {
            op(asm, d, l); // d = r OP l == l OP r
        } else {
            asm.movupd_rr(scratch, l);
            op(asm, scratch, r); // scratch = l OP r (r untouched until read)
            asm.movupd_rr(d, scratch);
        }
    } else {
        asm.movupd_rr(d, l);
        op(asm, d, r);
    }
}

/// Emit a complete vectorized loop kernel for a recognized element-wise map,
/// frame-ABI `extern "C" fn(*mut i64) -> i64`: it reads the induction `i`, limit
/// `n`, and each array base pointer from their frame slots (`[rdi + slot*8]`),
/// runs a 2-wide packed loop (`i += 2`) over full pairs, then a single-element
/// scalar tail that REUSES the packed body (load one element with `movsd` so lane
/// 1 is zero, run the same packed ops, store lane 0 with `movsd` — the junk lane
/// is never stored and masked SSE faults are harmless), writes `i = n` back, and
/// returns 0. Returns `None` (caller falls back to the scalar region) if the body
/// needs more than 4 distinct arrays or 14 lane temps, or uses an op the kernel
/// does not lower (e.g. `LoadConst`).
///
/// Register plan (all SysV caller-saved, no callee-save needed): rdi=frame,
/// rsi=i, rdx=n, rcx=i*8 offset, rax=address scratch, r8..r11=array bases,
/// xmm0..xmm13=lane temps, xmm15=arith scratch.
#[cfg(target_arch = "x86_64")]
pub fn emit_map_kernel(body: &[MicroOp], plan: &MapPlan) -> Option<Vec<u8>> {
    use crate::x64asm::{Asm, Cond, Reg, Xmm};
    const ARRAY_REGS: [Reg; 4] = [Reg::R8, Reg::R9, Reg::R10, Reg::R11];
    const LANES: [Xmm; 14] = [
        Xmm::Xmm0,
        Xmm::Xmm1,
        Xmm::Xmm2,
        Xmm::Xmm3,
        Xmm::Xmm4,
        Xmm::Xmm5,
        Xmm::Xmm6,
        Xmm::Xmm7,
        Xmm::Xmm8,
        Xmm::Xmm9,
        Xmm::Xmm10,
        Xmm::Xmm11,
        Xmm::Xmm12,
        Xmm::Xmm13,
    ];
    let scratch = Xmm::Xmm15;

    // Assign each distinct array pointer slot a GP base register.
    let mut arr_reg: Vec<(Slot, Reg)> = Vec::new();
    for op in body {
        let ptr = match op {
            MicroOp::ArrLoad { ptr_slot, .. } | MicroOp::ArrStore { ptr_slot, .. } => {
                Some(*ptr_slot)
            }
            _ => None,
        };
        if let Some(p) = ptr {
            if !arr_reg.iter().any(|(s, _)| *s == p) {
                if arr_reg.len() >= ARRAY_REGS.len() {
                    return None;
                }
                arr_reg.push((p, ARRAY_REGS[arr_reg.len()]));
            }
        }
    }
    let reg_of = |slot: Slot| arr_reg.iter().find(|(s, _)| *s == slot).map(|(_, r)| *r);

    // Assign each load/arith destination slot a lane XMM register.
    let mut lane: Vec<(Slot, Xmm)> = Vec::new();
    let add_lane = |slot: Slot, lane: &mut Vec<(Slot, Xmm)>| -> Option<()> {
        if !lane.iter().any(|(s, _)| *s == slot) {
            if lane.len() >= LANES.len() {
                return None;
            }
            lane.push((slot, LANES[lane.len()]));
        }
        Some(())
    };
    for op in body {
        match op {
            MicroOp::ArrLoad { dst, .. }
            | MicroOp::AddF { dst, .. }
            | MicroOp::SubF { dst, .. }
            | MicroOp::MulF { dst, .. }
            | MicroOp::DivF { dst, .. }
            | MicroOp::SqrtF { dst, .. } => add_lane(*dst, &mut lane)?,
            MicroOp::ArrStore { .. } => {}
            _ => return None, // unsupported op (e.g. LoadConst) — bail to scalar
        }
    }
    let xmm_of = |slot: Slot| lane.iter().find(|(s, _)| *s == slot).map(|(_, x)| *x);

    let off = |slot: Slot| (slot as i32) * 8;
    let mut a = Asm::new();
    // Prologue: load `i`, `n`, and point each array register at `&buffer[i-1]`
    // (the 1-based element `i`). The inner loop then advances these pointers
    // instead of recomputing `base + (i-1)*8` every iteration.
    a.mov_rm(Reg::Rsi, Reg::Rdi, off(plan.induction)); // i
    a.mov_rm(Reg::Rdx, Reg::Rdi, off(plan.limit)); // n
    a.mov_rr(Reg::Rcx, Reg::Rsi);
    a.sub_ri(Reg::Rcx, 1);
    a.shl_ri(Reg::Rcx, 3); // off0 = (i-1)*8
    for (slot, reg) in &arr_reg {
        a.mov_rm(*reg, Reg::Rdi, off(*slot)); // base
        a.add_rr(*reg, Reg::Rcx); // base + off0 = &buffer[i-1]
    }

    // Emit the load/arith/store body at a given element width (`packed`). The
    // array pointers already hold `&buffer[i-1]`, so each access is a direct
    // `movupd [ptr]` / `movsd [ptr]` — no per-iteration address arithmetic.
    let emit_body = |a: &mut Asm, packed: bool| {
        for op in body {
            match op {
                MicroOp::ArrLoad { dst, ptr_slot, .. } => {
                    let p = reg_of(*ptr_slot).unwrap();
                    let x = xmm_of(*dst).unwrap();
                    if packed {
                        a.movupd_rm(x, p, 0);
                    } else {
                        a.movsd_rm(x, p, 0);
                    }
                }
                MicroOp::ArrStore { src, ptr_slot, .. } => {
                    let p = reg_of(*ptr_slot).unwrap();
                    let x = xmm_of(*src).unwrap();
                    if packed {
                        a.movupd_mr(p, 0, x);
                    } else {
                        a.movsd_mr(p, 0, x);
                    }
                }
                _ => emit_packed_arith(a, op, |s| xmm_of(s).unwrap(), scratch),
            }
        }
    };

    // Exit the loop when the (1-based) index exceeds the valid range: for
    // `i <= n` (LtEq) that is `index > n`; for `i < n` (Lt) it is `index >= n`.
    let exit_cond = if plan.cmp == Cmp::LtEq {
        Cond::Gt
    } else {
        Cond::Ge
    };

    // Pair loop: process elements i and i+1 while BOTH are in range (i+1 valid).
    let pair_top = a.new_label();
    let tail = a.new_label();
    let done = a.new_label();
    a.bind(pair_top);
    a.mov_rr(Reg::Rax, Reg::Rsi);
    a.add_ri(Reg::Rax, 1); // i+1
    a.cmp_rr(Reg::Rax, Reg::Rdx);
    a.jcc(exit_cond, tail); // i+1 out of range -> no full pair left
    emit_body(&mut a, true);
    for (_, reg) in &arr_reg {
        a.add_ri(*reg, 16); // advance each pointer by 2 f64 elements
    }
    a.add_ri(Reg::Rsi, 2);
    a.jmp(pair_top);

    // Scalar tail: at most one element (i still in range, i+1 not).
    a.bind(tail);
    a.cmp_rr(Reg::Rsi, Reg::Rdx);
    a.jcc(exit_cond, done);
    emit_body(&mut a, false);

    a.bind(done);
    // Leave the induction at its scalar terminal value: n+1 for `i <= n`, n for
    // `i < n` (the first value that fails the loop test), so post-loop bytecode
    // that reads `i` sees exactly what the scalar loop would have left.
    a.mov_rr(Reg::Rax, Reg::Rdx);
    if plan.cmp == Cmp::LtEq {
        a.add_ri(Reg::Rax, 1);
    }
    a.mov_mr(Reg::Rdi, off(plan.induction), Reg::Rax);
    a.xor_rr(Reg::Rax, Reg::Rax);
    a.ret();
    Some(a.resolve())
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_forge/vectorize/tests.rs"]
mod tests;
