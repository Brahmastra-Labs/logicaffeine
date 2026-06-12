//! The LOGOS native tier: the bridge from the bytecode VM's tier-up seam to
//! the copy-and-patch forge JIT.
//!
//! The VM profiles calls and Main-loop back-edges; when something goes hot it
//! asks its installed [`NativeTier`] to compile. This crate is that tier:
//! [`ForgeTier`] translates VM bytecode into the forge's [`MicroOp`] subset
//! (functions AND loop regions), compiles it to a native stencil chain, and
//! hands it back. Anything outside the integer subset BAILS (`None`) and stays
//! on bytecode forever — the deopt contract.
//!
//! Soundness rests on three legs, each differentially tested in
//! `logicaffeine-tests`:
//! - the kind-inference dataflow (params are Int via the entry guard,
//!   comparisons are Bool, arithmetic requires Int operands);
//! - per-call / per-entry guards (a non-Int argument routes the call back to
//!   bytecode);
//! - the write-back contract for regions (incoming-dead scratches need no
//!   guard, writes re-box by inferred kind).
//!
//! Production binaries call [`install`] once at startup; the live VM
//! constructors pick the tier up from the global seam. WASM builds compile
//! this crate to nothing — the browser runs pure bytecode.

#![cfg(not(target_arch = "wasm32"))]

use std::sync::atomic::{AtomicU32, Ordering};

use logicaffeine_compile::vm::{
    install_native_tier, Constant, NativeFn, NativeTier, Op, RegionFn,
};
use logicaffeine_forge::buffer::JitChain;
use logicaffeine_forge::jit::{compile_straightline, MicroOp};

/// Register kinds for the adapter's sound dataflow.
#[derive(Clone, Copy, PartialEq, Debug)]
enum Kind {
    Unknown,
    Int,
    Bool,
    Mixed,
}

fn join(a: Kind, b: Kind) -> Kind {
    match (a, b) {
        (Kind::Unknown, x) | (x, Kind::Unknown) => x,
        (x, y) if x == y => x,
        _ => Kind::Mixed,
    }
}

/// Reachable pcs (slice-relative) from entry 0, over rebased control flow.
fn reachable(ops: &[Op], entry_pc: usize) -> Option<Vec<bool>> {
    let mut seen = vec![false; ops.len()];
    let mut work = vec![0usize];
    while let Some(pc) = work.pop() {
        if pc >= ops.len() || seen[pc] {
            continue;
        }
        seen[pc] = true;
        match ops[pc] {
            Op::Jump { target } => work.push(target.checked_sub(entry_pc)?),
            Op::JumpIfFalse { target, .. } | Op::JumpIfTrue { target, .. } => {
                work.push(target.checked_sub(entry_pc)?);
                work.push(pc + 1);
            }
            Op::Return { .. } | Op::ReturnNothing => {}
            _ => work.push(pc + 1),
        }
    }
    Some(seen)
}

/// Translate WHOLE-PROGRAM Main bytecode into the J2 micro-op subset.
///
/// Requirements (else `None`):
/// - ops drawn from {LoadConst(Int), Move, Add, Sub, Mul, Lt, Gt, Eq,
///   Jump, JumpIfFalse, JumpIfTrue, Show};
/// - exactly ONE Show, and it is the final op (it becomes Return);
/// - no comparison-produced value is the shown register (bool-taint).
///
/// Returns the micro program and the frame size it needs.
#[allow(clippy::type_complexity)]
pub fn adapt(
    ops: &[Op],
    constants: &[Constant],
    register_count: usize,
) -> Option<(Vec<MicroOp>, usize)> {
    // The shown register must not be a comparison result anywhere.
    let mut tainted: Vec<u16> = Vec::new();
    let mut shows = 0usize;
    for op in ops {
        match *op {
            Op::Lt { dst, .. } | Op::Gt { dst, .. } | Op::LtEq { dst, .. }
            | Op::GtEq { dst, .. } | Op::Eq { dst, .. } | Op::NotEq { dst, .. } => {
                tainted.push(dst)
            }
            Op::Show { .. } => shows += 1,
            _ => {}
        }
    }
    // Terminal shape: `Show; Halt` (or a bare trailing Show). Anything that
    // jumps PAST the Show (to the Halt) would print nothing on the VM while
    // the JIT still returns a value — bail on those.
    let n = ops.len();
    let (show_pc, halt_pc) = match (n.checked_sub(2).map(|i| &ops[i]), ops.last()) {
        (Some(Op::Show { .. }), Some(Op::Halt)) => (n - 2, Some(n - 1)),
        (_, Some(Op::Show { .. })) => (n - 1, None),
        _ => return None,
    };
    if shows != 1 {
        return None;
    }
    for op in ops {
        if let Op::Jump { target } | Op::JumpIfFalse { target, .. } | Op::JumpIfTrue { target, .. } = *op {
            if target > show_pc {
                return None;
            }
        }
    }
    let _ = halt_pc;

    // Scratch slots beyond the VM's registers for lowering LtEq/GtEq/NotEq.
    let zero_slot = u16::try_from(register_count).ok()?;
    let tmp_slot = zero_slot.checked_add(1)?;
    let frame_size = register_count + 2;

    // Pass 1: translate, recording vm-pc → micro-index. Jump targets hold VM
    // pcs until pass 2 remaps them.
    let mut micro: Vec<MicroOp> = vec![MicroOp::LoadConst { dst: zero_slot, value: 0 }];
    let mut pc_to_micro: Vec<usize> = Vec::with_capacity(ops.len());
    // (micro index, vm target) needing remap.
    let mut fixups: Vec<usize> = Vec::new();

    for op in ops {
        pc_to_micro.push(micro.len());
        match *op {
            Op::LoadConst { dst, idx } => match constants.get(idx as usize)? {
                Constant::Int(v) => micro.push(MicroOp::LoadConst { dst, value: *v }),
                _ => return None,
            },
            Op::Move { dst, src } => micro.push(MicroOp::Move { dst, src }),
            Op::Add { dst, lhs, rhs } => micro.push(MicroOp::Add { dst, lhs, rhs }),
            Op::Sub { dst, lhs, rhs } => micro.push(MicroOp::Sub { dst, lhs, rhs }),
            Op::Mul { dst, lhs, rhs } => micro.push(MicroOp::Mul { dst, lhs, rhs }),
            Op::Lt { dst, lhs, rhs } => micro.push(MicroOp::Lt { dst, lhs, rhs }),
            Op::Gt { dst, lhs, rhs } => micro.push(MicroOp::Gt { dst, lhs, rhs }),
            Op::Eq { dst, lhs, rhs } => micro.push(MicroOp::Eq { dst, lhs, rhs }),
            Op::LtEq { dst, lhs, rhs } => {
                // a <= b  ⇔  !(a > b)  ⇔  (a > b) == 0
                micro.push(MicroOp::Gt { dst: tmp_slot, lhs, rhs });
                micro.push(MicroOp::Eq { dst, lhs: tmp_slot, rhs: zero_slot });
            }
            Op::GtEq { dst, lhs, rhs } => {
                micro.push(MicroOp::Lt { dst: tmp_slot, lhs, rhs });
                micro.push(MicroOp::Eq { dst, lhs: tmp_slot, rhs: zero_slot });
            }
            Op::NotEq { dst, lhs, rhs } => {
                micro.push(MicroOp::Eq { dst: tmp_slot, lhs, rhs });
                micro.push(MicroOp::Eq { dst, lhs: tmp_slot, rhs: zero_slot });
            }
            Op::Jump { target } => {
                fixups.push(micro.len());
                micro.push(MicroOp::Jump { target });
            }
            Op::JumpIfFalse { cond, target } => {
                fixups.push(micro.len());
                micro.push(MicroOp::JumpIfFalse { cond, target });
            }
            Op::JumpIfTrue { cond, target } => {
                // tmp = (cond == 0); JumpIfFalse(tmp) fires when tmp == 0,
                // i.e. exactly when cond is TRUE.
                micro.push(MicroOp::Eq { dst: tmp_slot, lhs: cond, rhs: zero_slot });
                fixups.push(micro.len());
                micro.push(MicroOp::JumpIfFalse { cond: tmp_slot, target });
            }
            Op::Show { src } => {
                if tainted.contains(&src) {
                    return None;
                }
                micro.push(MicroOp::Return { src });
            }
            // The trailing Halt after the Show: unreachable past Return.
            Op::Halt => {}
            _ => return None,
        }
    }

    // Pass 2: remap jump targets from VM pcs to micro indices.
    for &i in &fixups {
        match &mut micro[i] {
            MicroOp::Jump { target } | MicroOp::JumpIfFalse { target, .. } => {
                *target = *pc_to_micro.get(*target)?;
            }
            _ => unreachable!(),
        }
    }
    Some((micro, frame_size))
}

/// Translate a FUNCTION body (terminal = Op::Return, absolute pcs rebased by
/// `entry_pc`) into the J2 subset.
///
/// J4 kind dataflow (sound fixpoint): params are Int (the entry guard),
/// comparisons produce Bool, arithmetic requires Int operands (a Bool
/// flowing into `+` errors on the VM — native code must NOT compute it),
/// Move propagates, conflicting writes go Mixed (bail on use). All reachable
/// returns must agree on Int or Bool; the bool flag re-boxes the result.
fn adapt_function(
    ops: &[Op],
    entry_pc: usize,
    constants: &[Constant],
    param_count: u16,
    register_count: u16,
) -> Option<(Vec<MicroOp>, usize, bool)> {
    let live = reachable(ops, entry_pc)?;
    // The trailing implicit ReturnNothing must be dead code — a reachable one
    // would return Nothing, which native code cannot represent.
    for (i, op) in ops.iter().enumerate() {
        if matches!(op, Op::ReturnNothing) && live[i] {
            return None;
        }
    }

    // Kind fixpoint over reachable ops.
    let mut kinds: Vec<Kind> = vec![Kind::Unknown; register_count as usize + 2];
    for k in 0..param_count as usize {
        kinds[k] = Kind::Int;
    }
    loop {
        let before = kinds.clone();
        for (i, op) in ops.iter().enumerate() {
            if !live[i] {
                continue;
            }
            match *op {
                Op::LoadConst { dst, .. } | Op::Add { dst, .. } | Op::Sub { dst, .. }
                | Op::Mul { dst, .. } => kinds[dst as usize] = join(kinds[dst as usize], Kind::Int),
                Op::Lt { dst, .. } | Op::Gt { dst, .. } | Op::LtEq { dst, .. }
                | Op::GtEq { dst, .. } | Op::Eq { dst, .. } | Op::NotEq { dst, .. } => {
                    kinds[dst as usize] = join(kinds[dst as usize], Kind::Bool)
                }
                Op::Move { dst, src } => {
                    kinds[dst as usize] = join(kinds[dst as usize], kinds[src as usize])
                }
                _ => {}
            }
        }
        if kinds == before {
            break;
        }
    }
    // Soundness gates on every reachable USE.
    let int_only = |r: u16, kinds: &[Kind]| kinds[r as usize] == Kind::Int;
    let mut ret_kind: Option<Kind> = None;
    for (i, op) in ops.iter().enumerate() {
        if !live[i] {
            continue;
        }
        match *op {
            Op::Add { lhs, rhs, .. } | Op::Sub { lhs, rhs, .. } | Op::Mul { lhs, rhs, .. }
            | Op::Lt { lhs, rhs, .. } | Op::Gt { lhs, rhs, .. } | Op::LtEq { lhs, rhs, .. }
            | Op::GtEq { lhs, rhs, .. } | Op::Eq { lhs, rhs, .. } | Op::NotEq { lhs, rhs, .. } => {
                if !int_only(lhs, &kinds) || !int_only(rhs, &kinds) {
                    return None;
                }
            }
            Op::Move { src, .. } => {
                if kinds[src as usize] == Kind::Mixed || kinds[src as usize] == Kind::Unknown {
                    return None;
                }
            }
            Op::JumpIfFalse { cond, .. } | Op::JumpIfTrue { cond, .. } => {
                if kinds[cond as usize] != Kind::Bool {
                    return None;
                }
            }
            Op::Return { src } => {
                let k = kinds[src as usize];
                if k != Kind::Int && k != Kind::Bool {
                    return None;
                }
                match ret_kind {
                    None => ret_kind = Some(k),
                    Some(prev) if prev == k => {}
                    _ => return None,
                }
            }
            _ => {}
        }
    }
    let returns_bool = matches!(ret_kind?, Kind::Bool);

    let zero_slot = register_count;
    let tmp_slot = zero_slot.checked_add(1)?;
    let frame_size = register_count as usize + 2;

    let mut micro: Vec<MicroOp> = vec![MicroOp::LoadConst { dst: zero_slot, value: 0 }];
    let mut pc_to_micro: Vec<usize> = Vec::with_capacity(ops.len());
    let mut fixups: Vec<usize> = Vec::new();

    for op in ops {
        pc_to_micro.push(micro.len());
        match *op {
            Op::LoadConst { dst, idx } => match constants.get(idx as usize)? {
                Constant::Int(v) => micro.push(MicroOp::LoadConst { dst, value: *v }),
                _ => return None,
            },
            Op::Move { dst, src } => micro.push(MicroOp::Move { dst, src }),
            Op::Add { dst, lhs, rhs } => micro.push(MicroOp::Add { dst, lhs, rhs }),
            Op::Sub { dst, lhs, rhs } => micro.push(MicroOp::Sub { dst, lhs, rhs }),
            Op::Mul { dst, lhs, rhs } => micro.push(MicroOp::Mul { dst, lhs, rhs }),
            Op::Lt { dst, lhs, rhs } => micro.push(MicroOp::Lt { dst, lhs, rhs }),
            Op::Gt { dst, lhs, rhs } => micro.push(MicroOp::Gt { dst, lhs, rhs }),
            Op::Eq { dst, lhs, rhs } => micro.push(MicroOp::Eq { dst, lhs, rhs }),
            Op::LtEq { dst, lhs, rhs } => {
                micro.push(MicroOp::Gt { dst: tmp_slot, lhs, rhs });
                micro.push(MicroOp::Eq { dst, lhs: tmp_slot, rhs: zero_slot });
            }
            Op::GtEq { dst, lhs, rhs } => {
                micro.push(MicroOp::Lt { dst: tmp_slot, lhs, rhs });
                micro.push(MicroOp::Eq { dst, lhs: tmp_slot, rhs: zero_slot });
            }
            Op::Jump { target } => {
                fixups.push(micro.len());
                micro.push(MicroOp::Jump { target });
            }
            Op::JumpIfFalse { cond, target } => {
                fixups.push(micro.len());
                micro.push(MicroOp::JumpIfFalse { cond, target });
            }
            Op::JumpIfTrue { cond, target } => {
                micro.push(MicroOp::Eq { dst: tmp_slot, lhs: cond, rhs: zero_slot });
                fixups.push(micro.len());
                micro.push(MicroOp::JumpIfFalse { cond: tmp_slot, target });
            }
            Op::Return { src } => micro.push(MicroOp::Return { src }),
            // Statically proven unreachable above; emit a dead terminator so
            // the structural validator stays satisfied.
            Op::ReturnNothing => micro.push(MicroOp::Return { src: 0 }),
            _ => return None,
        }
    }
    for &i in &fixups {
        match &mut micro[i] {
            MicroOp::Jump { target } | MicroOp::JumpIfFalse { target, .. } => {
                let rebased = target.checked_sub(entry_pc)?;
                *target = *pc_to_micro.get(rebased)?;
            }
            _ => unreachable!(),
        }
    }
    // Function bodies may end on a Return-less path only if the VM never
    // reaches it; the chain compiler requires structural termination.
    if !matches!(micro.last(), Some(MicroOp::Return { .. }) | Some(MicroOp::Jump { .. })) {
        return None;
    }
    Some((micro, frame_size, returns_bool))
}

/// Translate a MAIN-LOOP REGION into the J2 subset.
///
/// Region contract: `ops[0]` is the loop head; the slice ends at the
/// back-edge; every jump out targets `exit_pc` (mapped to a synthetic
/// terminal). Incoming-dead slots (comparison scratches written in the
/// straight-line prefix) need no guard; every other operand slot is
/// Int-guarded; writes are re-boxed by inferred kind.
#[allow(clippy::type_complexity)]
fn adapt_region(
    ops: &[Op],
    head_pc: usize,
    exit_pc: usize,
    constants: &[Constant],
    register_count: u16,
) -> Option<(Vec<MicroOp>, usize, Vec<u16>, Vec<u16>, Vec<(u16, bool)>)> {
    // Incoming-dead: first occurrence is a comparison dst, within the
    // straight-line prefix (no jumps yet).
    let mut free: Vec<u16> = Vec::new();
    let mut seen: Vec<u16> = Vec::new();
    for op in ops {
        match *op {
            Op::Lt { dst, lhs, rhs } | Op::Gt { dst, lhs, rhs } | Op::LtEq { dst, lhs, rhs }
            | Op::GtEq { dst, lhs, rhs } | Op::Eq { dst, lhs, rhs } | Op::NotEq { dst, lhs, rhs } => {
                if !seen.contains(&dst) && !free.contains(&dst) {
                    free.push(dst);
                }
                for r in [lhs, rhs] {
                    if !seen.contains(&r) {
                        seen.push(r);
                    }
                }
            }
            Op::Jump { .. } | Op::JumpIfFalse { .. } | Op::JumpIfTrue { .. } => break,
            Op::LoadConst { dst, .. } => {
                if !seen.contains(&dst) {
                    seen.push(dst);
                }
            }
            Op::Move { dst, src } | Op::Add { dst, lhs: src, .. } | Op::Sub { dst, lhs: src, .. }
            | Op::Mul { dst, lhs: src, .. } => {
                for r in [dst, src] {
                    if !seen.contains(&r) {
                        seen.push(r);
                    }
                }
            }
            _ => return None,
        }
    }

    // Guard set: every register the region touches, minus the free set.
    let mut guard: Vec<u16> = Vec::new();
    let mut writes: Vec<u16> = Vec::new();
    for op in ops {
        let (dsts, srcs): (Vec<u16>, Vec<u16>) = match *op {
            Op::LoadConst { dst, .. } => (vec![dst], vec![]),
            Op::Move { dst, src } => (vec![dst], vec![src]),
            Op::Add { dst, lhs, rhs } | Op::Sub { dst, lhs, rhs } | Op::Mul { dst, lhs, rhs }
            | Op::Lt { dst, lhs, rhs } | Op::Gt { dst, lhs, rhs } | Op::LtEq { dst, lhs, rhs }
            | Op::GtEq { dst, lhs, rhs } | Op::Eq { dst, lhs, rhs } | Op::NotEq { dst, lhs, rhs } => {
                (vec![dst], vec![lhs, rhs])
            }
            Op::JumpIfFalse { cond, .. } | Op::JumpIfTrue { cond, .. } => (vec![], vec![cond]),
            Op::Jump { .. } => (vec![], vec![]),
            _ => return None,
        };
        for d in dsts {
            if !writes.contains(&d) {
                writes.push(d);
            }
            if !guard.contains(&d) && !free.contains(&d) {
                guard.push(d);
            }
        }
        for r in srcs {
            if !guard.contains(&r) && !free.contains(&r) {
                guard.push(r);
            }
        }
    }

    // Kind fixpoint: guarded slots start Int; free slots start Unknown.
    let mut kinds: Vec<Kind> = vec![Kind::Unknown; register_count as usize + 2];
    for &g in &guard {
        kinds[g as usize] = Kind::Int;
    }
    loop {
        let before = kinds.clone();
        for op in ops {
            match *op {
                Op::LoadConst { dst, .. } | Op::Add { dst, .. } | Op::Sub { dst, .. }
                | Op::Mul { dst, .. } => kinds[dst as usize] = join(kinds[dst as usize], Kind::Int),
                Op::Lt { dst, .. } | Op::Gt { dst, .. } | Op::LtEq { dst, .. }
                | Op::GtEq { dst, .. } | Op::Eq { dst, .. } | Op::NotEq { dst, .. } => {
                    kinds[dst as usize] = join(kinds[dst as usize], Kind::Bool)
                }
                Op::Move { dst, src } => {
                    kinds[dst as usize] = join(kinds[dst as usize], kinds[src as usize])
                }
                _ => {}
            }
        }
        if kinds == before {
            break;
        }
    }
    // Use gates (same soundness rules as functions).
    for op in ops {
        match *op {
            Op::Add { lhs, rhs, .. } | Op::Sub { lhs, rhs, .. } | Op::Mul { lhs, rhs, .. }
            | Op::Lt { lhs, rhs, .. } | Op::Gt { lhs, rhs, .. } | Op::LtEq { lhs, rhs, .. }
            | Op::GtEq { lhs, rhs, .. } | Op::Eq { lhs, rhs, .. } | Op::NotEq { lhs, rhs, .. } => {
                if kinds[lhs as usize] != Kind::Int || kinds[rhs as usize] != Kind::Int {
                    return None;
                }
            }
            Op::Move { src, .. } => {
                if matches!(kinds[src as usize], Kind::Mixed | Kind::Unknown) {
                    return None;
                }
            }
            Op::JumpIfFalse { cond, .. } | Op::JumpIfTrue { cond, .. } => {
                if kinds[cond as usize] != Kind::Bool {
                    return None;
                }
            }
            _ => {}
        }
    }
    let write_set: Vec<(u16, bool)> = writes
        .iter()
        .map(|&r| (r, kinds[r as usize] == Kind::Bool))
        .collect();
    if write_set.iter().any(|&(r, _)| kinds[r as usize] == Kind::Mixed || kinds[r as usize] == Kind::Unknown) {
        return None;
    }

    // Emit: out-of-region jumps go to the synthetic exit terminal.
    let zero_slot = register_count;
    let tmp_slot = zero_slot + 1;
    let frame_size = register_count as usize + 2;
    let in_region = |t: usize| (head_pc..head_pc + ops.len()).contains(&t);
    let mut micro: Vec<MicroOp> = vec![MicroOp::LoadConst { dst: zero_slot, value: 0 }];
    let mut pc_to_micro: Vec<usize> = Vec::with_capacity(ops.len());
    let mut fixups: Vec<(usize, usize)> = Vec::new(); // (micro idx, vm target or EXIT sentinel)
    const EXIT: usize = usize::MAX;

    for op in ops {
        pc_to_micro.push(micro.len());
        match *op {
            Op::LoadConst { dst, idx } => match constants.get(idx as usize)? {
                Constant::Int(v) => micro.push(MicroOp::LoadConst { dst, value: *v }),
                _ => return None,
            },
            Op::Move { dst, src } => micro.push(MicroOp::Move { dst, src }),
            Op::Add { dst, lhs, rhs } => micro.push(MicroOp::Add { dst, lhs, rhs }),
            Op::Sub { dst, lhs, rhs } => micro.push(MicroOp::Sub { dst, lhs, rhs }),
            Op::Mul { dst, lhs, rhs } => micro.push(MicroOp::Mul { dst, lhs, rhs }),
            Op::Lt { dst, lhs, rhs } => micro.push(MicroOp::Lt { dst, lhs, rhs }),
            Op::Gt { dst, lhs, rhs } => micro.push(MicroOp::Gt { dst, lhs, rhs }),
            Op::Eq { dst, lhs, rhs } => micro.push(MicroOp::Eq { dst, lhs, rhs }),
            Op::LtEq { dst, lhs, rhs } => {
                micro.push(MicroOp::Gt { dst: tmp_slot, lhs, rhs });
                micro.push(MicroOp::Eq { dst, lhs: tmp_slot, rhs: zero_slot });
            }
            Op::GtEq { dst, lhs, rhs } => {
                micro.push(MicroOp::Lt { dst: tmp_slot, lhs, rhs });
                micro.push(MicroOp::Eq { dst, lhs: tmp_slot, rhs: zero_slot });
            }
            Op::NotEq { dst, lhs, rhs } => {
                micro.push(MicroOp::Eq { dst: tmp_slot, lhs, rhs });
                micro.push(MicroOp::Eq { dst, lhs: tmp_slot, rhs: zero_slot });
            }
            Op::Jump { target } => {
                fixups.push((micro.len(), if in_region(target) { target } else { EXIT }));
                micro.push(MicroOp::Jump { target: 0 });
            }
            Op::JumpIfFalse { cond, target } => {
                fixups.push((micro.len(), if in_region(target) { target } else { EXIT }));
                micro.push(MicroOp::JumpIfFalse { cond, target: 0 });
            }
            Op::JumpIfTrue { cond, target } => {
                micro.push(MicroOp::Eq { dst: tmp_slot, lhs: cond, rhs: zero_slot });
                fixups.push((micro.len(), if in_region(target) { target } else { EXIT }));
                micro.push(MicroOp::JumpIfFalse { cond: tmp_slot, target: 0 });
            }
            _ => return None,
        }
    }
    let exit_micro = micro.len();
    micro.push(MicroOp::Return { src: 0 });
    let _ = exit_pc;
    for (i, vm_target) in fixups {
        let t = if vm_target == EXIT {
            exit_micro
        } else {
            *pc_to_micro.get(vm_target.checked_sub(head_pc)?)?
        };
        match &mut micro[i] {
            MicroOp::Jump { target } | MicroOp::JumpIfFalse { target, .. } => *target = t,
            _ => unreachable!(),
        }
    }
    Some((micro, frame_size, guard, free, write_set))
}

struct ChainFn {
    chain: JitChain,
    frame_size: usize,
    returns_bool: bool,
}

impl NativeFn for ChainFn {
    fn call(&self, args: &[i64]) -> i64 {
        // Reuse one frame buffer per thread — a hot function is called
        // millions of times; per-call Vec allocation would dominate.
        thread_local! {
            static FRAME: std::cell::RefCell<Vec<i64>> = const { std::cell::RefCell::new(Vec::new()) };
        }
        FRAME.with(|f| {
            let mut frame = f.borrow_mut();
            frame.clear();
            frame.resize(self.frame_size, 0);
            frame[..args.len()].copy_from_slice(args);
            self.chain.run_with_frame(&mut frame)
        })
    }
    fn returns_bool(&self) -> bool {
        self.returns_bool
    }
}

struct RegionChain {
    chain: JitChain,
    frame_size: usize,
    guard: Vec<u16>,
    free: Vec<u16>,
    writes: Vec<(u16, bool)>,
}

impl RegionFn for RegionChain {
    fn guard_set(&self) -> &[u16] {
        &self.guard
    }
    fn free_set(&self) -> &[u16] {
        &self.free
    }
    fn write_set(&self) -> &[(u16, bool)] {
        &self.writes
    }
    fn frame_size(&self) -> usize {
        self.frame_size
    }
    fn run(&self, frame: &mut [i64]) {
        self.chain.run_with_frame(frame);
    }
}

/// The forge-backed native tier, with compile observability.
#[derive(Default)]
pub struct ForgeTier {
    compiles: AtomicU32,
    successes: AtomicU32,
    region_compiles: AtomicU32,
    region_successes: AtomicU32,
}

impl ForgeTier {
    /// A fresh tier with zeroed counters.
    pub fn new() -> Self {
        ForgeTier::default()
    }

    /// (attempts, successes) for FUNCTION compiles.
    pub fn function_counts(&self) -> (u32, u32) {
        (self.compiles.load(Ordering::SeqCst), self.successes.load(Ordering::SeqCst))
    }

    /// (attempts, successes) for Main-loop REGION compiles.
    pub fn region_counts(&self) -> (u32, u32) {
        (
            self.region_compiles.load(Ordering::SeqCst),
            self.region_successes.load(Ordering::SeqCst),
        )
    }
}

impl NativeTier for ForgeTier {
    fn compile_function(
        &self,
        code: &[Op],
        entry_pc: usize,
        constants: &[Constant],
        param_count: u16,
        register_count: u16,
    ) -> Option<Box<dyn NativeFn>> {
        self.compiles.fetch_add(1, Ordering::SeqCst);
        let (micro, frame_size, returns_bool) =
            adapt_function(code, entry_pc, constants, param_count, register_count)?;
        let chain = compile_straightline(&micro).ok()?;
        self.successes.fetch_add(1, Ordering::SeqCst);
        Some(Box::new(ChainFn { chain, frame_size, returns_bool }))
    }

    fn compile_region(
        &self,
        code: &[Op],
        head_pc: usize,
        exit_pc: usize,
        constants: &[Constant],
        register_count: u16,
    ) -> Option<Box<dyn RegionFn>> {
        self.region_compiles.fetch_add(1, Ordering::SeqCst);
        let (micro, frame_size, guard, free, writes) =
            adapt_region(code, head_pc, exit_pc, constants, register_count)?;
        let chain = compile_straightline(&micro).ok()?;
        self.region_successes.fetch_add(1, Ordering::SeqCst);
        Some(Box::new(RegionChain { chain, frame_size, guard, free, writes }))
    }
}

/// Install the forge tier as the process-wide native tier. Idempotent —
/// the first call wins; later calls return the already-installed tier.
///
/// Call once at binary startup (CLI, server). The live VM constructors in
/// `logicaffeine-compile` pick it up for every program they run.
pub fn install() -> &'static ForgeTier {
    static TIER: std::sync::OnceLock<ForgeTier> = std::sync::OnceLock::new();
    let tier = TIER.get_or_init(ForgeTier::new);
    install_native_tier(tier);
    tier
}
