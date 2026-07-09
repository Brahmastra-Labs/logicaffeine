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
            MicroOp::ArrLoad { dst, idx, checked, .. } => {
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
            MicroOp::ArrStore { src, idx, checked, .. } => {
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
        MicroOp::AddF { dst, lhs, rhs } => {
            emit_bin(asm, xmm(*dst), xmm(*lhs), xmm(*rhs), scratch, Asm::addpd_rr, true)
        }
        MicroOp::MulF { dst, lhs, rhs } => {
            emit_bin(asm, xmm(*dst), xmm(*lhs), xmm(*rhs), scratch, Asm::mulpd_rr, true)
        }
        MicroOp::SubF { dst, lhs, rhs } => {
            emit_bin(asm, xmm(*dst), xmm(*lhs), xmm(*rhs), scratch, Asm::subpd_rr, false)
        }
        MicroOp::DivF { dst, lhs, rhs } => {
            emit_bin(asm, xmm(*dst), xmm(*lhs), xmm(*rhs), scratch, Asm::divpd_rr, false)
        }
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
        Xmm::Xmm0, Xmm::Xmm1, Xmm::Xmm2, Xmm::Xmm3, Xmm::Xmm4, Xmm::Xmm5, Xmm::Xmm6,
        Xmm::Xmm7, Xmm::Xmm8, Xmm::Xmm9, Xmm::Xmm10, Xmm::Xmm11, Xmm::Xmm12, Xmm::Xmm13,
    ];
    let scratch = Xmm::Xmm15;

    // Assign each distinct array pointer slot a GP base register.
    let mut arr_reg: Vec<(Slot, Reg)> = Vec::new();
    for op in body {
        let ptr = match op {
            MicroOp::ArrLoad { ptr_slot, .. } | MicroOp::ArrStore { ptr_slot, .. } => Some(*ptr_slot),
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
    let mut add_lane = |slot: Slot, lane: &mut Vec<(Slot, Xmm)>| -> Option<()> {
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
    let exit_cond = if plan.cmp == Cmp::LtEq { Cond::Gt } else { Cond::Ge };

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
mod tests {
    use super::*;

    // `c[i] = a[i] + b[i]` — the canonical map. Slots: i=0, n=1, a=2, b=3, c=4,
    // alen=5, blen=6, clen=7, ta=8, tb=9, tc=10, one=11.
    fn load(dst: Slot, idx: Slot, ptr_slot: Slot, len_slot: Slot) -> MicroOp {
        MicroOp::ArrLoad { dst, idx, ptr_slot, len_slot, byte: false, narrow32: false, checked: false }
    }
    fn store(src: Slot, idx: Slot, ptr_slot: Slot, len_slot: Slot) -> MicroOp {
        MicroOp::ArrStore { src, idx, ptr_slot, len_slot, byte: false, narrow32: false, checked: false }
    }
    fn load_checked(dst: Slot, idx: Slot, ptr_slot: Slot, len_slot: Slot) -> MicroOp {
        MicroOp::ArrLoad { dst, idx, ptr_slot, len_slot, byte: false, narrow32: false, checked: true }
    }

    // Wrap a load/compute/store `body` in the top-tested while-loop shape the
    // recognizer expects: guard / body / LoadConst(step) / increment / back-Jump /
    // Return. Induction = slot 0, limit = slot 1, step slot = 90.
    fn loop_region(body: Vec<MicroOp>, cmp: Cmp, step_val: i64) -> Vec<MicroOp> {
        let mut ops = vec![MicroOp::Branch { cmp, lhs: 0, rhs: 1, target: 0 }];
        ops.extend(body);
        ops.push(MicroOp::LoadConst { dst: 90, value: step_val });
        ops.push(MicroOp::Add { dst: 0, lhs: 0, rhs: 90 });
        let jump_idx = ops.len();
        ops.push(MicroOp::Jump { target: 0 });
        ops.push(MicroOp::Return { src: 0 });
        if let MicroOp::Branch { target, .. } = &mut ops[0] {
            *target = jump_idx + 1; // exit -> the Return
        }
        ops
    }

    fn map_add_region() -> Vec<MicroOp> {
        loop_region(
            vec![
                load(8, 0, 2, 5),
                load(9, 0, 3, 6),
                MicroOp::AddF { dst: 10, lhs: 8, rhs: 9 },
                store(10, 0, 4, 7),
            ],
            Cmp::LtEq,
            1,
        )
    }

    #[test]
    fn recognizes_elementwise_add_map() {
        let ops = map_add_region();
        let plan = recognize_elementwise_map(&ops).expect("c[i]=a[i]+b[i] is a map");
        assert_eq!(plan.induction, 0);
        assert_eq!(plan.limit, 1);
        assert_eq!(plan.cmp, Cmp::LtEq);
        // body = the 4 load/arith/store ops at [1..5] (guard at 0, control after).
        assert_eq!((plan.body_start, plan.body_end), (1, 5));
    }

    #[test]
    fn recognizes_multiop_map() {
        // c[i] = a[i]*b[i] + a[i] — still a pure map (all reads at i, no carry).
        let ops = loop_region(
            vec![
                load(8, 0, 2, 5),
                load(9, 0, 3, 6),
                MicroOp::MulF { dst: 10, lhs: 8, rhs: 9 },
                MicroOp::AddF { dst: 10, lhs: 10, rhs: 8 },
                store(10, 0, 4, 7),
            ],
            Cmp::Lt,
            1,
        );
        assert!(recognize_elementwise_map(&ops).is_some());
    }

    #[test]
    fn rejects_reduction_loop_carried_accumulator() {
        // `sum` (slot 9) is read before being defined in the body — loop-carried,
        // NOT bit-exactly vectorizable. Must reject.
        let ops = loop_region(
            vec![
                load(8, 0, 2, 5),
                MicroOp::AddF { dst: 9, lhs: 9, rhs: 8 }, // slot 9 loop-carried
                store(9, 0, 4, 7),
            ],
            Cmp::Lt,
            1,
        );
        assert_eq!(recognize_elementwise_map(&ops), None);
    }

    #[test]
    fn rejects_cross_index_access_scan() {
        // out[i] = a[j] where j != i (a scan/stencil read at a different index).
        let ops = loop_region(
            vec![load(8, 9, 2, 5), store(8, 0, 4, 7)], // idx=9 != i(0)
            Cmp::Lt,
            1,
        );
        assert_eq!(recognize_elementwise_map(&ops), None);
    }

    #[test]
    fn rejects_body_with_load_only_no_store() {
        // A body that never stores is not a map (e.g. it feeds a reduction).
        let ops = loop_region(vec![load(8, 0, 2, 5)], Cmp::Lt, 1);
        assert_eq!(recognize_elementwise_map(&ops), None);
    }

    #[test]
    fn rejects_non_unit_stride() {
        // i += 2 — the kernel visits every index, so a stride-2 loop (a subset)
        // must be rejected (the step LoadConst is not 1).
        let ops = loop_region(vec![load(8, 0, 2, 5), store(8, 0, 4, 7)], Cmp::Lt, 2);
        assert_eq!(recognize_elementwise_map(&ops), None);
    }

    #[test]
    fn rejects_bounds_checked_access() {
        // A `checked: true` access — the Oracle did NOT prove length >= n, so the
        // kernel (which elides per-element checks) would be memory-unsafe. Reject.
        let ops = loop_region(vec![load_checked(8, 0, 2, 5), store(8, 0, 4, 7)], Cmp::Lt, 1);
        assert_eq!(recognize_elementwise_map(&ops), None);
    }

    #[test]
    fn rejects_non_back_edge_region() {
        // No back-edge `Jump { target: 0 }` -> not a recognized loop region.
        let ops = vec![
            MicroOp::Branch { cmp: Cmp::Lt, lhs: 0, rhs: 1, target: 5 },
            load(8, 0, 2, 5),
            store(8, 0, 4, 7),
            MicroOp::LoadConst { dst: 90, value: 1 },
            MicroOp::Add { dst: 0, lhs: 0, rhs: 90 },
        ];
        assert_eq!(recognize_elementwise_map(&ops), None);
    }

    #[cfg(target_arch = "x86_64")]
    fn run_frame(code: &[u8], frame: &mut [i64]) -> i64 {
        let page = crate::JitPage::new(code).unwrap();
        let f: extern "C" fn(*mut i64) -> i64 =
            unsafe { std::mem::transmute(page.as_ptr()) };
        f(frame.as_mut_ptr())
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn packed_body_lowering_matches_scalar_both_lanes() {
        use crate::x64asm::{Asm, Reg, Xmm};
        // Lane assignment for body slots: ta=xmm0, tb=xmm1, tc=xmm2.
        let lane = |s: Slot| match s {
            0 => Xmm::Xmm0,
            1 => Xmm::Xmm1,
            2 => Xmm::Xmm2,
            _ => unreachable!(),
        };
        // Body: tc = ta * tb ; tc = tc + ta  (a pure map expression, 2 ops).
        let body = [
            MicroOp::MulF { dst: 2, lhs: 0, rhs: 1 },
            MicroOp::AddF { dst: 2, lhs: 2, rhs: 0 },
        ];
        let cases: [(f64, f64, f64, f64); 3] = [
            (2.5, 0.5, 1.0, 3.0),
            (-1.0, 7.25, 4.0, -2.5),
            (0.1, 0.2, 1.0 / 3.0, 9.0),
        ];
        for (a0, a1, b0, b1) in cases {
            let mut asm = Asm::new();
            asm.movupd_rm(Xmm::Xmm0, Reg::Rdi, 0); // ta = {a0,a1}
            asm.movupd_rm(Xmm::Xmm1, Reg::Rdi, 16); // tb = {b0,b1}
            for op in &body {
                emit_packed_arith(&mut asm, op, lane, Xmm::Xmm15);
            }
            asm.movupd_mr(Reg::Rdi, 32, Xmm::Xmm2); // store tc
            asm.ret();
            let mut frame = [
                a0.to_bits() as i64, a1.to_bits() as i64,
                b0.to_bits() as i64, b1.to_bits() as i64,
                0, 0,
            ];
            run_frame(&asm.resolve(), &mut frame);
            assert_eq!(f64::from_bits(frame[4] as u64).to_bits(), (a0 * b0 + a0).to_bits(), "lane0");
            assert_eq!(f64::from_bits(frame[5] as u64).to_bits(), (a1 * b1 + a1).to_bits(), "lane1");
        }
    }

    #[cfg(target_arch = "x86_64")]
    fn run_map(body: &[MicroOp], plan: &MapPlan, a: &[f64], b: &[f64]) -> Vec<f64> {
        // 1-based loop `for i in 1..=n` (LtEq), accessing buffer[i-1] — the shape
        // the recognizer fires on. i starts at 1; n = the length.
        let code = emit_map_kernel(body, plan).expect("kernel emits");
        let mut c = vec![0.0f64; a.len()];
        let mut frame = [
            1i64,
            a.len() as i64,
            a.as_ptr() as i64,
            b.as_ptr() as i64,
            c.as_mut_ptr() as i64,
            0, 0, 0,
        ];
        run_frame(&code, &mut frame);
        // The kernel leaves i at the scalar terminal value: n+1 for `i <= n`.
        assert_eq!(frame[0], a.len() as i64 + 1, "induction terminal n+1");
        c
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn map_kernel_add_matches_scalar_all_lengths() {
        // c[i] = a[i] + b[i] over various n (even, odd, 1, 0) — exercises the pair
        // loop AND the scalar tail.
        let body = vec![
            load(5, 0, 2, 0),
            load(6, 0, 3, 0),
            MicroOp::AddF { dst: 7, lhs: 5, rhs: 6 },
            store(7, 0, 4, 0),
        ];
        let plan = MapPlan { induction: 0, limit: 1, cmp: Cmp::LtEq, body_start: 0, body_end: 4 };
        for n in [0usize, 1, 2, 3, 7, 8, 15] {
            let a: Vec<f64> = (0..n).map(|i| i as f64 * 1.5 - 3.0).collect();
            let b: Vec<f64> = (0..n).map(|i| i as f64 * -0.25 + 1.0).collect();
            let got = run_map(&body, &plan, &a, &b);
            for i in 0..n {
                assert_eq!(got[i].to_bits(), (a[i] + b[i]).to_bits(), "n={n} i={i}");
            }
        }
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn map_kernel_multiop_matches_scalar() {
        // c[i] = a[i]*b[i] + a[i] — multi-op map, pair loop + tail.
        let body = vec![
            load(5, 0, 2, 0),
            load(6, 0, 3, 0),
            MicroOp::MulF { dst: 7, lhs: 5, rhs: 6 },
            MicroOp::AddF { dst: 7, lhs: 7, rhs: 5 },
            store(7, 0, 4, 0),
        ];
        let plan = MapPlan { induction: 0, limit: 1, cmp: Cmp::LtEq, body_start: 0, body_end: 5 };
        for n in [1usize, 4, 5, 16, 17] {
            let a: Vec<f64> = (0..n).map(|i| 0.1 * i as f64 + 0.3).collect();
            let b: Vec<f64> = (0..n).map(|i| 2.0 - 0.07 * i as f64).collect();
            let got = run_map(&body, &plan, &a, &b);
            for i in 0..n {
                assert_eq!(got[i].to_bits(), (a[i] * b[i] + a[i]).to_bits(), "n={n} i={i}");
            }
        }
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn map_kernel_bails_on_unsupported_loadconst() {
        // A body with a LoadConst is not lowered by the kernel -> None (the caller
        // falls back to the scalar region, staying sound).
        let body = vec![
            load(5, 0, 2, 0),
            MicroOp::LoadConst { dst: 6, value: 2 },
            MicroOp::MulF { dst: 7, lhs: 5, rhs: 6 },
            store(7, 0, 4, 0),
        ];
        let plan = MapPlan { induction: 0, limit: 1, cmp: Cmp::LtEq, body_start: 0, body_end: 4 };
        assert!(emit_map_kernel(&body, &plan).is_none());
    }

    #[cfg(target_arch = "x86_64")]
    #[test]
    fn packed_body_lowering_non_commutative_dst_eq_rhs_uses_scratch() {
        use crate::x64asm::{Asm, Reg, Xmm};
        // SubF dst==rhs: td(xmm1) = ta(xmm0) - td(xmm1). The 2-operand subpd would
        // compute xmm1 = xmm1 - xmm0 (wrong order); emit_bin must route through the
        // scratch to preserve `lhs - rhs`.
        let lane = |s: Slot| match s {
            0 => Xmm::Xmm0,
            1 => Xmm::Xmm1,
            _ => unreachable!(),
        };
        let op = MicroOp::SubF { dst: 1, lhs: 0, rhs: 1 };
        let cases: [(f64, f64, f64, f64); 2] = [(2.5, 0.5, 1.0, 3.0), (-1.0, 3.0, 4.0, -2.5)];
        for (a0, a1, b0, b1) in cases {
            let mut asm = Asm::new();
            asm.movupd_rm(Xmm::Xmm0, Reg::Rdi, 0); // ta
            asm.movupd_rm(Xmm::Xmm1, Reg::Rdi, 16); // td (the rhs, gets overwritten)
            emit_packed_arith(&mut asm, &op, lane, Xmm::Xmm15);
            asm.movupd_mr(Reg::Rdi, 32, Xmm::Xmm1);
            asm.ret();
            let mut frame = [
                a0.to_bits() as i64, a1.to_bits() as i64,
                b0.to_bits() as i64, b1.to_bits() as i64,
                0, 0,
            ];
            run_frame(&asm.resolve(), &mut frame);
            assert_eq!(f64::from_bits(frame[4] as u64).to_bits(), (a0 - b0).to_bits(), "lane0 a-b");
            assert_eq!(f64::from_bits(frame[5] as u64).to_bits(), (a1 - b1).to_bits(), "lane1 a-b");
        }
    }
}
