//! ★ THE IR PEEPHOLE ★ — a semantics-preserving `CompiledProgram → CompiledProgram`
//! transform: block-local forward **copy-propagation** followed by **dead-move
//! elimination**. It exists for the backends that lower the bytecode straight to a
//! final target with no downstream optimizing assembler — RISC-V asm, WASM, the
//! spatial fabric. gcc already performs exactly this coalescing for the C tier
//! (register-to-register moves fold away, dead copies vanish), so C does not need
//! it; the flat backends ship the VM compiler's move cascades verbatim without it.
//!
//! The measured target is the swap idiom. `Set a to b. Set b to t.` lowers to a
//! five-move cascade through fresh temps every loop iteration:
//! ```text
//!   Move R10 = R8   Move R11 = R3   Move R1 = R11   Move R12 = R10   Move R3 = R12
//! ```
//! Copy-propagation rewrites the temp reads to their roots and the now-dead temp
//! moves fall to DCE, leaving the optimal `Move R1 = R3. Move R3 = R8.` — three
//! fewer ops in the hottest loop, per iteration.
//!
//! ## Soundness — the asymmetry that makes it safe
//!
//! A wrong coalesce miscompiles every backend, so the register model is deliberately
//! one-sided:
//! - `reads` is a **superset** of the true reads (over-approximate). Liveness can
//!   then never *under*-count a use, so a still-needed value is never judged dead.
//! - `kills` is a **subset** of the registers an op FULLY overwrites
//!   (under-approximate). Liveness can then never *over*-count a def, so a live range
//!   is never cut short.
//! - `clobbers` = `kills ∪ mutates` invalidates a copy on BOTH its key and its
//!   value side. This is what keeps copy-propagation sound under LOGOS's
//!   **value-semantics collections**: after `Move x = list`, mutating OR reassigning
//!   `list` must kill the `x = list` fact, because a copy-on-write push makes `x` and
//!   `list` diverge.
//! - The in-place-mutated collection/struct register is **never rewritable** — pushing
//!   to `x` (a copy of `list`) is not the same as pushing to `list` once value
//!   semantics clone on write, so a read that is really an in-place mutation site is
//!   left untouched.
//! - Only `Move` ops are ever deleted (their semantics are total and side-effect free);
//!   and only when the destination is provably dead or the move is a self-copy.
//!
//! Every register classifier below is EXHAUSTIVE on `Op` (no `_` arm): a newly added
//! instruction fails to compile until it is classified, so no op can silently escape
//! the model. `peephole_soundness_lock` proves `program_outcome(peephole(P)) ==
//! program_outcome(P)` over the corpus + a fuzzer, and that the op count strictly drops
//! on the swap-idiom programs.

use super::instruction::{CompiledProgram, Op, Reg};
use std::collections::{HashMap, HashSet};

/// Run the peephole to a fixpoint, in place. Copy-propagate once (a single forward
/// pass fully resolves within-block chains, because the copy map stores roots), then
/// iterate liveness + dead-move elimination until no move is removed.
pub fn peephole(program: &mut CompiledProgram) {
    copy_propagate(program);
    loop {
        let live_out = liveness(program);
        let dead = dead_moves(program, &live_out);
        if dead.is_empty() {
            break;
        }
        remove_ops(program, &dead);
    }
}

// ───────────────────────── control-flow model ─────────────────────────

/// The absolute jump/branch targets `op` can transfer control to (NOT the fallthrough).
fn branch_targets(op: &Op) -> Vec<usize> {
    match op {
        Op::Jump { target } => vec![*target],
        Op::JumpIfFalse { target, .. } | Op::JumpIfTrue { target, .. } => vec![*target],
        Op::IterNext { exit, .. } => vec![*exit],
        _ => vec![],
    }
}

/// A terminator ends its basic block — control does not fall through to `pc + 1`.
fn is_hard_terminator(op: &Op) -> bool {
    matches!(
        op,
        Op::Jump { .. } | Op::Return { .. } | Op::ReturnNothing | Op::Halt | Op::FailWith { .. }
    )
}

/// Whether `op` is a branch (conditional or iterator step) — it has BOTH a jump target
/// and a fallthrough successor, and starts a new block after it.
fn is_branch(op: &Op) -> bool {
    matches!(
        op,
        Op::JumpIfFalse { .. } | Op::JumpIfTrue { .. } | Op::IterNext { .. }
    )
}

/// The successors of the op at `pc` (fallthrough + branch targets), respecting function
/// region walls: a fallthrough into a function `entry_pc` is not a real edge (Main ends
/// in `Halt` before the bodies, each body ends in a `Return`), so it is dropped.
pub(crate) fn successors(pc: usize, code: &[Op], entries: &HashSet<usize>) -> Vec<usize> {
    let op = &code[pc];
    let mut succ = branch_targets(op);
    if !is_hard_terminator(op) {
        let next = pc + 1;
        if next < code.len() && !entries.contains(&next) {
            succ.push(next);
        }
    }
    succ
}

/// The leaders (basic-block heads) of the flat program: pc 0, every function entry, every
/// branch/jump target, and every op immediately after a branch or terminator. The copy map
/// resets at each leader (a join may be entered from predecessors with different copies).
fn leaders(program: &CompiledProgram) -> HashSet<usize> {
    let code = &program.code;
    let mut ls: HashSet<usize> = HashSet::new();
    ls.insert(0);
    for f in &program.functions {
        ls.insert(f.entry_pc);
    }
    for (pc, op) in code.iter().enumerate() {
        for t in branch_targets(op) {
            ls.insert(t);
        }
        if (is_hard_terminator(op) || is_branch(op)) && pc + 1 < code.len() {
            ls.insert(pc + 1);
        }
    }
    ls
}

fn function_entries(program: &CompiledProgram) -> HashSet<usize> {
    program.functions.iter().map(|f| f.entry_pc).collect()
}

// ───────────────────────── register effects (exhaustive) ─────────────────────────

/// Push a `[start, start + count)` register range.
fn push_range(out: &mut Vec<Reg>, start: Reg, count: u16) {
    for i in 0..count {
        out.push(start.wrapping_add(i));
    }
}

/// A SUPERSET of the registers `op` reads — the ISA's exhaustive read set, with a closure's capture
/// window taken from the program.
fn reads(op: &Op, program: &CompiledProgram, out: &mut Vec<Reg>) {
    op.for_each_read_reg(
        |func| {
            program
                .functions
                .get(func as usize)
                .map_or(0, |f| f.captures.len())
        },
        |r| out.push(r),
    );
}

/// The registers `op` FULLY overwrites — a SUBSET of true defs (under-approximate: an
/// in-place *mutation* is NOT a kill, it is a read that keeps the value live). Exhaustive.
fn kills(op: &Op, out: &mut Vec<Reg>) {
    match op {
        Op::LoadConst { dst, .. }
        | Op::Move { dst, .. }
        | Op::Add { dst, .. }
        | Op::Sub { dst, .. }
        | Op::Mul { dst, .. }
        | Op::Div { dst, .. }
        | Op::ExactDiv { dst, .. }
        | Op::FloorDiv { dst, .. }
        | Op::Mod { dst, .. }
        | Op::DivPow2 { dst, .. }
        | Op::MagicDivU { dst, .. }
        | Op::Lt { dst, .. }
        | Op::Gt { dst, .. }
        | Op::LtEq { dst, .. }
        | Op::GtEq { dst, .. }
        | Op::Eq { dst, .. }
        | Op::ApproxEq { dst, .. }
        | Op::NotEq { dst, .. }
        | Op::Not { dst, .. }
        | Op::Concat { dst, .. }
        | Op::SeqConcat { dst, .. }
        | Op::Pow { dst, .. }
        | Op::BitXor { dst, .. }
        | Op::BitAnd { dst, .. }
        | Op::BitOr { dst, .. }
        | Op::Shl { dst, .. }
        | Op::Shr { dst, .. }
        | Op::Index { dst, .. }
        | Op::IndexUnchecked { dst, .. }
        | Op::Length { dst, .. }
        | Op::Contains { dst, .. }
        | Op::SliceOp { dst, .. }
        | Op::DeepClone { dst, .. }
        | Op::FormatValue { dst, .. }
        | Op::UnionOp { dst, .. }
        | Op::IntersectOp { dst, .. }
        | Op::GetField { dst, .. }
        | Op::TestArm { dst, .. }
        | Op::BindArm { dst, .. }
        | Op::NewList { dst, .. }
        | Op::NewEmptyList { dst, .. }
        | Op::NewEmptyListWithCapacity { dst, .. }
        | Op::NewEmptyListFixed { dst, .. }
        | Op::NewEmptyListI32 { dst, .. }
        | Op::NewEmptySet { dst, .. }
        | Op::NewEmptyMap { dst, .. }
        | Op::NewEmptyMapWithCapacity { dst, .. }
        | Op::NewRange { dst, .. }
        | Op::NewTuple { dst, .. }
        | Op::NewStruct { dst, .. }
        | Op::NewInductive { dst, .. }
        | Op::NewCrdt { dst, .. }
        | Op::MakeClosure { dst, .. }
        | Op::Args { dst, .. }
        | Op::LoadToday { dst, .. }
        | Op::LoadNow { dst, .. }
        | Op::GlobalGet { dst, .. }
        | Op::Call { dst, .. }
        | Op::CallBuiltin { dst, .. }
        | Op::CallValue { dst, .. }
        | Op::ListPop { dst, .. }
        | Op::ChanNew { dst, .. }
        | Op::ChanRecv { dst, .. }
        | Op::ChanTrySend { dst, .. }
        | Op::ChanTryRecv { dst, .. }
        | Op::TaskAwait { dst, .. }
        | Op::SelectWait { dst_arm: dst }
        | Op::NetMakePeer { dst, .. }
        | Op::NetAwait { dst, .. }
        | Op::BigIntWide { dst, .. }
        | Op::Tcp { dst, .. }
        | Op::SpawnHandle { dst, .. } => out.push(*dst),
        // dst = R[dst] + R[src] — dst is written (also read, tracked in `reads`)
        Op::AddAssign { dst, .. } => out.push(*dst),
        // the reversible core rewrites its place / swaps a,b (also reads them)
        Op::RevAdd { place, .. }
        | Op::RevSub { place, .. }
        | Op::RevXor { place, .. }
        | Op::RevNeg { place } => out.push(*place),
        Op::RevSwap { a, b } => {
            out.push(*a);
            out.push(*b);
        }
        // a select recv arm's received value lands in `var`
        Op::SelectArmRecv { var, .. } => out.push(*var),
        // tuple destructure writes a whole window
        Op::DestructureTuple { start, count, .. } => push_range(out, *start, *count),
        // NetSync writes the merged counter back to dst (also reads it)
        Op::NetSync { dst, .. } => out.push(*dst),
        // ops that overwrite nothing (mutate in place, effect only, or pure control)
        Op::RemoveFrom { .. }
        | Op::SetIndex { .. }
        | Op::SetIndexUnchecked { .. }
        | Op::ListPush { .. }
        | Op::ListPushField { .. }
        | Op::SetAdd { .. }
        | Op::StructInsert { .. }
        | Op::CrdtBump { .. }
        | Op::CrdtMerge { .. }
        | Op::CrdtJoin { .. }
        | Op::CrdtAppend { .. }
        | Op::CrdtResolve { .. }
        | Op::EnsureOwned { .. }
        | Op::GlobalSet { .. }
        | Op::Return { .. }
        | Op::ReturnNothing
        | Op::Show { .. }
        | Op::Obs { .. }
        | Op::Sleep { .. }
        | Op::IterPrepare { .. }
        | Op::IterNext { .. }
        | Op::IterPop
        | Op::RegionBoundsGuard { .. }
        | Op::CheckPolicy { .. }
        | Op::ChanSend { .. }
        | Op::ChanClose { .. }
        | Op::Spawn { .. }
        | Op::TaskAbort { .. }
        | Op::SelectArmTimeout { .. }
        | Op::NetConnect { .. }
        | Op::NetListen { .. }
        | Op::NetSend { .. }
        | Op::NetStream { .. }
        | Op::Serve { .. }
        | Op::Jump { .. }
        | Op::JumpIfFalse { .. }
        | Op::JumpIfTrue { .. }
        | Op::Halt
        | Op::FailWith { .. } => {}
    }
}

/// The registers an op mutates IN PLACE (read + kept, not a kill). A copy touching such a
/// register must be invalidated (value semantics: the mutated buffer diverges from a copy).
pub(crate) fn mutates(op: &Op, out: &mut Vec<Reg>) {
    match op {
        Op::ListPush { list, .. } => out.push(*list),
        Op::ListPushField { obj, .. } => out.push(*obj),
        Op::SetAdd { set, .. } => out.push(*set),
        Op::RemoveFrom { collection, .. } => out.push(*collection),
        Op::SetIndex { collection, .. } | Op::SetIndexUnchecked { collection, .. } => {
            out.push(*collection)
        }
        Op::StructInsert { obj, .. } => out.push(*obj),
        Op::CrdtBump { obj, .. } => out.push(*obj),
        Op::CrdtMerge { target, .. } | Op::CrdtJoin { target, .. } => out.push(*target),
        Op::CrdtAppend { seq, .. } => out.push(*seq),
        Op::CrdtResolve { obj, .. } => out.push(*obj),
        Op::ListPop { list, .. } => out.push(*list),
        // a copy-on-write barrier may re-seat the register's buffer identity
        Op::EnsureOwned { reg } => out.push(*reg),
        // EXHAUSTIVE (no `_`): every op that mutates NO register in place. A future op that mutates
        // a register must be moved above, or its copies would not be invalidated — a miscompile.
        Op::LoadConst { .. }
        | Op::Move { .. }
        | Op::Add { .. }
        | Op::AddAssign { .. }
        | Op::Sub { .. }
        | Op::Mul { .. }
        | Op::Div { .. }
        | Op::ExactDiv { .. }
        | Op::FloorDiv { .. }
        | Op::Mod { .. }
        | Op::DivPow2 { .. }
        | Op::MagicDivU { .. }
        | Op::Lt { .. }
        | Op::Gt { .. }
        | Op::LtEq { .. }
        | Op::GtEq { .. }
        | Op::Eq { .. }
        | Op::ApproxEq { .. }
        | Op::NotEq { .. }
        | Op::Not { .. }
        | Op::Concat { .. }
        | Op::SeqConcat { .. }
        | Op::Pow { .. }
        | Op::BitXor { .. }
        | Op::BitAnd { .. }
        | Op::BitOr { .. }
        | Op::Shl { .. }
        | Op::Shr { .. }
        | Op::RevAdd { .. }
        | Op::RevSub { .. }
        | Op::RevXor { .. }
        | Op::RevNeg { .. }
        | Op::RevSwap { .. }
        | Op::Index { .. }
        | Op::IndexUnchecked { .. }
        | Op::Length { .. }
        | Op::Contains { .. }
        | Op::SliceOp { .. }
        | Op::DeepClone { .. }
        | Op::FormatValue { .. }
        | Op::UnionOp { .. }
        | Op::IntersectOp { .. }
        | Op::GetField { .. }
        | Op::TestArm { .. }
        | Op::BindArm { .. }
        | Op::NewRange { .. }
        | Op::NewList { .. }
        | Op::NewEmptyList { .. }
        | Op::NewEmptyListWithCapacity { .. }
        | Op::NewEmptyListFixed { .. }
        | Op::NewEmptyListI32 { .. }
        | Op::NewEmptySet { .. }
        | Op::NewEmptyMap { .. }
        | Op::NewEmptyMapWithCapacity { .. }
        | Op::NewTuple { .. }
        | Op::NewStruct { .. }
        | Op::NewInductive { .. }
        | Op::NewCrdt { .. }
        | Op::MakeClosure { .. }
        | Op::Args { .. }
        | Op::LoadToday { .. }
        | Op::LoadNow { .. }
        | Op::GlobalGet { .. }
        | Op::GlobalSet { .. }
        | Op::Call { .. }
        | Op::CallBuiltin { .. }
        | Op::CallValue { .. }
        | Op::Return { .. }
        | Op::ReturnNothing
        | Op::Show { .. }
        | Op::Obs { .. }
        | Op::Sleep { .. }
        | Op::IterPrepare { .. }
        | Op::IterNext { .. }
        | Op::IterPop
        | Op::DestructureTuple { .. }
        | Op::RegionBoundsGuard { .. }
        | Op::CheckPolicy { .. }
        | Op::BigIntWide { .. }
        | Op::ChanNew { .. }
        | Op::ChanSend { .. }
        | Op::ChanRecv { .. }
        | Op::ChanTrySend { .. }
        | Op::ChanTryRecv { .. }
        | Op::ChanClose { .. }
        | Op::Spawn { .. }
        | Op::SpawnHandle { .. }
        | Op::TaskAwait { .. }
        | Op::TaskAbort { .. }
        | Op::SelectArmRecv { .. }
        | Op::SelectArmTimeout { .. }
        | Op::SelectWait { .. }
        | Op::NetConnect { .. }
        | Op::NetListen { .. }
        | Op::NetSend { .. }
        | Op::NetStream { .. }
        | Op::NetAwait { .. }
        | Op::NetMakePeer { .. }
        | Op::NetSync { .. }
        | Op::Tcp { .. }
        | Op::Serve { .. }
        | Op::Jump { .. }
        | Op::JumpIfFalse { .. }
        | Op::JumpIfTrue { .. }
        | Op::Halt
        | Op::FailWith { .. } => {}
    }
}

/// The registers whose current value this op invalidates as a copy source/target:
/// everything it fully overwrites plus everything it mutates in place.
fn clobbers(op: &Op, out: &mut Vec<Reg>) {
    kills(op, out);
    mutates(op, out);
}

/// Visit every REWRITABLE read field of `op` by mutable reference — the pure scalar value
/// reads copy-propagation may substitute. Deliberately EXCLUDES: range operands (a call's
/// arg window must stay contiguous), in-place-mutated collection/struct registers (value
/// semantics would diverge), the read-modify-write `dst` of `AddAssign`/reversible ops, and
/// speculative/effectful operands left untouched out of caution. Everything omitted here is
/// still counted by [`reads`] for liveness — it simply is not a substitution site.
fn for_each_rewritable_read(op: &mut Op, f: &mut impl FnMut(&mut Reg)) {
    match op {
        Op::Add { lhs, rhs, .. }
        | Op::Sub { lhs, rhs, .. }
        | Op::Mul { lhs, rhs, .. }
        | Op::Div { lhs, rhs, .. }
        | Op::ExactDiv { lhs, rhs, .. }
        | Op::FloorDiv { lhs, rhs, .. }
        | Op::Mod { lhs, rhs, .. }
        | Op::Lt { lhs, rhs, .. }
        | Op::Gt { lhs, rhs, .. }
        | Op::LtEq { lhs, rhs, .. }
        | Op::GtEq { lhs, rhs, .. }
        | Op::Eq { lhs, rhs, .. }
        | Op::ApproxEq { lhs, rhs, .. }
        | Op::NotEq { lhs, rhs, .. }
        | Op::Concat { lhs, rhs, .. }
        | Op::SeqConcat { lhs, rhs, .. }
        | Op::Pow { lhs, rhs, .. }
        | Op::BitXor { lhs, rhs, .. }
        | Op::BitAnd { lhs, rhs, .. }
        | Op::BitOr { lhs, rhs, .. }
        | Op::Shl { lhs, rhs, .. }
        | Op::Shr { lhs, rhs, .. }
        | Op::UnionOp { lhs, rhs, .. }
        | Op::IntersectOp { lhs, rhs, .. } => {
            f(lhs);
            f(rhs);
        }
        Op::DivPow2 { lhs, .. } | Op::MagicDivU { lhs, .. } => f(lhs),
        Op::Move { src, .. }
        | Op::Not { src, .. }
        | Op::DeepClone { src, .. }
        | Op::FormatValue { src, .. } => f(src),
        Op::AddAssign { src, .. } => f(src),
        Op::Index {
            collection, index, ..
        }
        | Op::IndexUnchecked {
            collection, index, ..
        } => {
            f(collection);
            f(index);
        }
        Op::Length { collection, .. } => f(collection),
        Op::Contains {
            collection, value, ..
        } => {
            f(collection);
            f(value);
        }
        Op::SliceOp {
            collection,
            start,
            end,
            ..
        } => {
            f(collection);
            f(start);
            f(end);
        }
        Op::GetField { obj, .. } => f(obj),
        Op::NewRange { start, end, .. } => {
            f(start);
            f(end);
        }
        Op::NewEmptyListWithCapacity { capacity, .. }
        | Op::NewEmptyMapWithCapacity { capacity, .. } => f(capacity),
        Op::CallValue { callee, .. } => f(callee),
        Op::CrdtMerge { source, .. } | Op::CrdtJoin { source, .. } => f(source),
        Op::JumpIfFalse { cond, .. } | Op::JumpIfTrue { cond, .. } => f(cond),
        Op::GlobalSet { src, .. } => f(src),
        Op::Return { src } => f(src),
        Op::Show { src } => f(src),
        Op::DestructureTuple { src, .. } => f(src),
        // NOT rewritten: range windows, in-place mutation sites, rmw dst, reversible core,
        // speculative guards, effectful/concurrency/net operands, no-read ops.
        //
        // TestArm / BindArm targets are also LEFT UNTOUCHED: they are the SCRUTINEE of a pattern
        // match, a typed anchor the flat backends' static kind-inference positionally couples (the
        // riscv/wasm emitters key a `BindArm`'s payload type on the immediately preceding `TestArm`
        // having the SAME target register). Rewriting one but not the other — copy-prop resets at the
        // arm-block leader, so only the in-condition-block TestArm is rewritten while the arm-body
        // BindArm keeps the original register — breaks that coupling and pushes the (still ≡VM)
        // program out of the backend's enum fragment. The coalescing win is on scalar loop temps, not
        // enum scrutinees, so excluding these costs nothing. (peephole_soundness_lock/enum_inspect +
        // riscv_aot_lock pin it.)
        Op::TestArm { .. }
        | Op::BindArm { .. }
        | Op::LoadConst { .. }
        | Op::RevSwap { .. }
        | Op::RevXor { .. }
        | Op::RevNeg { .. }
        | Op::RevAdd { .. }
        | Op::RevSub { .. }
        | Op::EnsureOwned { .. }
        | Op::Call { .. }
        | Op::CallBuiltin { .. }
        | Op::MakeClosure { .. }
        | Op::CheckPolicy { .. }
        | Op::ListPushField { .. }
        | Op::GlobalGet { .. }
        | Op::ReturnNothing
        | Op::NewList { .. }
        | Op::NewEmptyList { .. }
        | Op::NewEmptyListFixed { .. }
        | Op::NewEmptyListI32 { .. }
        | Op::NewEmptySet { .. }
        | Op::NewEmptyMap { .. }
        | Op::NewTuple { .. }
        | Op::ListPush { .. }
        | Op::SetAdd { .. }
        | Op::RemoveFrom { .. }
        | Op::SetIndex { .. }
        | Op::SetIndexUnchecked { .. }
        | Op::RegionBoundsGuard { .. }
        | Op::NewStruct { .. }
        | Op::StructInsert { .. }
        | Op::NewInductive { .. }
        | Op::CrdtBump { .. }
        | Op::BigIntWide { .. }
        | Op::NewCrdt { .. }
        | Op::CrdtAppend { .. }
        | Op::CrdtResolve { .. }
        | Op::IterPrepare { .. }
        | Op::IterNext { .. }
        | Op::IterPop
        | Op::ListPop { .. }
        | Op::Sleep { .. } => {}
        // remaining effectful / concurrency / net / temporal — operands left untouched
        Op::LoadToday { .. }
        | Op::LoadNow { .. }
        | Op::Args { .. }
        | Op::Obs { .. }
        | Op::ChanNew { .. }
        | Op::ChanSend { .. }
        | Op::ChanRecv { .. }
        | Op::ChanTrySend { .. }
        | Op::ChanTryRecv { .. }
        | Op::ChanClose { .. }
        | Op::Spawn { .. }
        | Op::SpawnHandle { .. }
        | Op::TaskAwait { .. }
        | Op::TaskAbort { .. }
        | Op::SelectArmRecv { .. }
        | Op::SelectArmTimeout { .. }
        | Op::SelectWait { .. }
        | Op::NetConnect { .. }
        | Op::NetListen { .. }
        | Op::NetSend { .. }
        | Op::NetStream { .. }
        | Op::NetAwait { .. }
        | Op::NetMakePeer { .. }
        | Op::NetSync { .. }
        | Op::Tcp { .. }
        | Op::Serve { .. }
        | Op::Jump { .. }
        | Op::Halt
        | Op::FailWith { .. } => {}
    }
}

// ───────────────────────── copy-propagation ─────────────────────────

/// Block-local forward copy-propagation: within each basic block, track `copy[d] = s`
/// meaning "register `d` currently holds a copy of register `s`", and rewrite each
/// rewritable read of `d` to `s`. The map stores ROOTS (a Move's source is itself
/// substituted before insertion), so a chain `d←c←b` resolves to `b` in one pass. Resets
/// at every block leader; invalidates on every clobber (key and value side).
fn copy_propagate(program: &mut CompiledProgram) {
    let ls = leaders(program);
    let mut copy: HashMap<Reg, Reg> = HashMap::new();
    let mut scratch: Vec<Reg> = Vec::new();
    for pc in 0..program.code.len() {
        if ls.contains(&pc) {
            copy.clear();
        }
        let op = &mut program.code[pc];
        // 1. substitute rewritable reads through the current copies
        for_each_rewritable_read(op, &mut |r| {
            if let Some(&s) = copy.get(r) {
                *r = s;
            }
        });
        // capture the (now-substituted) Move shape before we reborrow immutably
        let mov = if let Op::Move { dst, src } = *op {
            Some((dst, src))
        } else {
            None
        };
        // 2. invalidate every copy the op clobbers (key or value side); a call also writes each argument register
        //    a `mutable` parameter lends, with the parameter's final value
        scratch.clear();
        clobbers(&program.code[pc], &mut scratch);
        if let Op::Call {
            func, args_start, ..
        } = program.code[pc]
        {
            if let Some(f) = program.functions.get(func as usize) {
                scratch.extend(
                    f.mutable_param_regs
                        .iter()
                        .map(|p| args_start.wrapping_add(*p)),
                );
            }
        }
        if !scratch.is_empty() {
            copy.retain(|k, v| !scratch.contains(k) && !scratch.contains(v));
        }
        // 3. a real Move records a fresh copy fact (self-moves record nothing)
        if let Some((dst, src)) = mov {
            if dst != src {
                copy.insert(dst, src);
            }
        }
    }
}

// ───────────────────────── liveness ─────────────────────────

/// Iterative backward liveness over the whole code vector. `live_out[pc]` is the set of
/// registers live on exit from `pc`. Function bodies are disjoint CFG islands (Main ends in
/// `Halt`, bodies in `Return`, and `Call` does not edge into a body), so per-frame registers
/// never interfere.
fn liveness(program: &CompiledProgram) -> Vec<HashSet<Reg>> {
    let code = &program.code;
    let n = code.len();
    let entries = function_entries(program);
    let succ: Vec<Vec<usize>> = (0..n).map(|pc| successors(pc, code, &entries)).collect();

    let mut read_sets: Vec<Vec<Reg>> = Vec::with_capacity(n);
    let mut kill_sets: Vec<Vec<Reg>> = Vec::with_capacity(n);
    for op in code {
        let mut r = Vec::new();
        reads(op, program, &mut r);
        read_sets.push(r);
        let mut k = Vec::new();
        kills(op, &mut k);
        kill_sets.push(k);
    }
    // A function's exit reads its `mutable` parameters: the caller takes each one's final value back out of the
    // argument register it lent (`CompiledFunction::mutable_param_regs`), so the last write to one is never dead.
    for f in program
        .functions
        .iter()
        .filter(|f| !f.mutable_param_regs.is_empty())
    {
        let mut seen = HashSet::new();
        let mut stack = vec![f.entry_pc];
        while let Some(pc) = stack.pop() {
            if pc >= n || !seen.insert(pc) {
                continue;
            }
            if matches!(code[pc], Op::Return { .. } | Op::ReturnNothing) {
                read_sets[pc].extend(f.mutable_param_regs.iter().copied());
            }
            stack.extend(succ[pc].iter().copied());
        }
    }

    let mut live_in: Vec<HashSet<Reg>> = vec![HashSet::new(); n];
    let mut live_out: Vec<HashSet<Reg>> = vec![HashSet::new(); n];
    let mut changed = true;
    while changed {
        changed = false;
        for pc in (0..n).rev() {
            let mut out = HashSet::new();
            for &s in &succ[pc] {
                for &r in &live_in[s] {
                    out.insert(r);
                }
            }
            // live_in = reads ∪ (live_out − kills)
            let mut new_in = out.clone();
            for k in &kill_sets[pc] {
                new_in.remove(k);
            }
            for &r in &read_sets[pc] {
                new_in.insert(r);
            }
            if out != live_out[pc] {
                live_out[pc] = out;
                changed = true;
            }
            if new_in != live_in[pc] {
                live_in[pc] = new_in;
                changed = true;
            }
        }
    }
    live_out
}

// ───────────────────────── dead-move elimination ─────────────────────────

/// The indices of `Move` ops that are removable: a self-copy (`dst == src`, a no-op), or a
/// move whose destination is not live on exit (a dead assignment). Only `Move` — total and
/// side-effect free — is ever a candidate.
fn dead_moves(program: &CompiledProgram, live_out: &[HashSet<Reg>]) -> Vec<usize> {
    let mut dead = Vec::new();
    for (pc, op) in program.code.iter().enumerate() {
        if let Op::Move { dst, src } = op {
            if dst == src || !live_out[pc].contains(dst) {
                dead.push(pc);
            }
        }
    }
    dead
}

/// Delete the ops at `dead` (sorted, unique) and renumber every absolute code position that
/// refers to a pc: branch/jump targets, iterator exits, function entry points, and the
/// pc-keyed `loop_locals` map. A target that pointed at a deleted op moves to the next kept op.
fn remove_ops(program: &mut CompiledProgram, dead: &[usize]) {
    if dead.is_empty() {
        return;
    }
    let dead_set: HashSet<usize> = dead.iter().copied().collect();
    let n = program.code.len();
    // new_pc[old] = number of kept ops strictly before `old` = new index of the next kept op
    let mut new_pc = vec![0usize; n + 1];
    let mut kept = 0usize;
    for old in 0..n {
        new_pc[old] = kept;
        if !dead_set.contains(&old) {
            kept += 1;
        }
    }
    new_pc[n] = kept;

    // rebuild the code vector, remapping the surviving branch/exit targets
    let mut code: Vec<Op> = Vec::with_capacity(kept);
    for (old, mut op) in program.code.drain(..).enumerate() {
        if dead_set.contains(&old) {
            continue;
        }
        match &mut op {
            Op::Jump { target }
            | Op::JumpIfFalse { target, .. }
            | Op::JumpIfTrue { target, .. } => {
                *target = new_pc[*target];
            }
            Op::IterNext { exit, .. } => {
                *exit = new_pc[*exit];
            }
            _ => {}
        }
        code.push(op);
    }
    program.code = code;

    for f in &mut program.functions {
        f.entry_pc = new_pc[f.entry_pc];
    }
    if !program.loop_locals.is_empty() {
        program.loop_locals = program
            .loop_locals
            .drain()
            .map(|(pc, v)| (new_pc[pc], v))
            .collect();
    }
    // A site mark moves to the next surviving op, so the range it opens starts where it did.
    for (pc, _) in &mut program.site_at {
        *pc = new_pc[*pc as usize] as u32;
    }
    // A declaration names the op that DEFINES its value; one whose op is gone names nothing.
    program.declared_types = std::mem::take(&mut program.declared_types)
        .into_iter()
        .filter(|(pc, _)| !dead_set.contains(&(*pc as usize)))
        .map(|(pc, bt)| (new_pc[pc as usize] as u32, bt))
        .collect();
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/vm/peephole/tests.rs"]
mod tests;
