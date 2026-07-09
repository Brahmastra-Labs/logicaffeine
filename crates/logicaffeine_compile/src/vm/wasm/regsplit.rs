//! Register live-range splitting — the structural pre-pass that lets a single VM register that the
//! bytecode allocator REUSED across disjoint live ranges of *different* wasm value types become one
//! wasm local per range.
//!
//! # Why
//!
//! The VM's register allocator frees a register when its value dies and reuses the number for an
//! unrelated later value. When those two values have different wasm value types (e.g. an `Enum`
//! loop variable on `i32` whose number a post-loop `Int` length reuses on `i64`), the AOT backend's
//! per-register kind inference sees one local asked to be two value types and SOUNDLY REFUSES
//! ([`super::kind`]'s `unify_strict`). That refusal is correct but pessimistic — the two ranges are
//! disjoint, so they could each have their own local. This pass performs exactly that split, before
//! kind inference runs, so the disjoint ranges become distinct locals and inference succeeds.
//!
//! # How (def-use webs)
//!
//! A classic structural renaming, computed purely from control flow — NO kinds involved:
//!
//! 1. [`visit_regs`] — the ONE exhaustive enumeration of every op's register operands, each tagged
//!    [`Role::Def`]/[`Role::Use`]/[`Role::DefUse`], plus register *ranges* (`Call` args, closure
//!    captures, `DestructureTuple` targets) whose members must stay contiguous. Analysis, renaming,
//!    and range-pinning all go through this one match, so they can never disagree on a register's
//!    role (a disagreement would be a miscompile).
//! 2. Instruction-level **reaching definitions** over the op CFG (back-edges included, so a
//!    loop-carried value stays one web).
//! 3. **Webs** (union-find): for each use, union all of its reaching defs; a `DefUse` (`x += …`)
//!    unions its own def into its use-web so the single field renames consistently. A web is a
//!    maximal def-use connected component.
//! 4. A register with ≥2 webs and a *fully analyzed* every-use-has-a-reaching-def profile, that is
//!    NOT pinned (never a member of a contiguous range and not a function parameter), is SPLIT: its
//!    first web keeps the original number, each further web gets a fresh local appended after the
//!    register file. Everything else is left exactly as it was.
//!
//! # Safety
//!
//! The pass is the IDENTITY on any register that is single-web, pinned, or not fully analyzable —
//! which is every register in a program the backend already compiles. So it can only ever *enable*
//! a program the backend previously refused; it cannot change one it already accepted. Range
//! operands stay contiguous because their members are pinned (never renumbered), so the existing
//! lowering — which reads `args_start + i` by number — is unaffected.

use std::collections::{HashMap, HashSet};

use crate::vm::instruction::{CompiledFunction, Op, Reg};

/// How an op touches a register operand.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Role {
    /// Written (the prior value is dead).
    Def,
    /// Read.
    Use,
    /// Read AND written through the SAME single field (`x += …`) — must rename to one local.
    DefUse,
}

/// A visitor over an op's register operands. `scalar` is a single register field; `range` is a
/// `count`-wide block of consecutive registers starting at `*start` (the members past `*start` are
/// implicit — not fields — so only `*start` is handed over, but the whole block is pinned).
trait RegVisitor {
    fn scalar(&mut self, r: &mut Reg, role: Role);
    fn range(&mut self, start: &mut Reg, count: u16, role: Role);
    /// A register block that must stay CONTIGUOUS only when it has ≥2 members — a range of ONE has
    /// no positional constraint, so its lone operand is a plain renamable scalar (the op reads it
    /// through `args_start`, which the rename updates). Visiting it as a scalar lets a single-arg
    /// call/builtin operand split off a register that a wider call reuses (spectral_norm's `Sqrt`
    /// arg sharing a slot with a 4-arg call's `Seq of Float` argument).
    fn members(&mut self, start: &mut Reg, count: u16, role: Role) {
        if count == 1 {
            self.scalar(start, role);
        } else {
            self.range(start, count, role);
        }
    }
}

/// THE exhaustive enumeration of every op's register operands (compiler-enforced total match: a new
/// `Op` variant fails to build until classified here). `functions` resolves a `MakeClosure`'s
/// capture count, whose register range is not stored in the op. Takes `&mut Op` so the same routine
/// drives both read-only analysis and in-place renaming.
fn visit_regs(op: &mut Op, functions: &[CompiledFunction], v: &mut dyn RegVisitor) {
    use Role::{Def, DefUse, Use};
    match op {
        Op::LoadConst { dst, .. } => v.scalar(dst, Def),
        Op::Move { dst, src } => {
            v.scalar(src, Use);
            v.scalar(dst, Def);
        }
        Op::EnsureOwned { reg } => v.scalar(reg, Role::DefUse),
        Op::Add { dst, lhs, rhs }
        | Op::Sub { dst, lhs, rhs }
        | Op::Mul { dst, lhs, rhs }
        | Op::Pow { dst, lhs, rhs }
        | Op::Div { dst, lhs, rhs }
        | Op::ExactDiv { dst, lhs, rhs }
        | Op::FloorDiv { dst, lhs, rhs }
        | Op::Mod { dst, lhs, rhs }
        | Op::Lt { dst, lhs, rhs }
        | Op::Gt { dst, lhs, rhs }
        | Op::LtEq { dst, lhs, rhs }
        | Op::GtEq { dst, lhs, rhs }
        | Op::Eq { dst, lhs, rhs }
        | Op::NotEq { dst, lhs, rhs }
        | Op::ApproxEq { dst, lhs, rhs }
        | Op::Concat { dst, lhs, rhs }
        | Op::SeqConcat { dst, lhs, rhs }
        | Op::BitXor { dst, lhs, rhs }
        | Op::BitAnd { dst, lhs, rhs }
        | Op::BitOr { dst, lhs, rhs }
        | Op::Shl { dst, lhs, rhs }
        | Op::Shr { dst, lhs, rhs }
        | Op::UnionOp { dst, lhs, rhs }
        | Op::IntersectOp { dst, lhs, rhs } => {
            v.scalar(lhs, Use);
            v.scalar(rhs, Use);
            v.scalar(dst, Def);
        }
        Op::AddAssign { dst, src } => {
            v.scalar(src, Use);
            v.scalar(dst, DefUse);
        }
        Op::DivPow2 { dst, lhs, .. } | Op::MagicDivU { dst, lhs, .. } => {
            v.scalar(lhs, Use);
            v.scalar(dst, Def);
        }
        Op::Not { dst, src } | Op::DeepClone { dst, src } => {
            v.scalar(src, Use);
            v.scalar(dst, Def);
        }
        Op::Jump { .. } => {}
        Op::JumpIfFalse { cond, .. } | Op::JumpIfTrue { cond, .. } => {
            v.scalar(cond, Use)
        }
        Op::Call { dst, args_start, arg_count, .. } => {
            v.members(args_start, *arg_count, Use);
            v.scalar(dst, Def);
        }
        Op::CallBuiltin { dst, args_start, arg_count, .. } => {
            v.members(args_start, *arg_count, Use);
            v.scalar(dst, Def);
        }
        Op::CallValue { dst, callee, args_start, arg_count, .. } => {
            v.scalar(callee, Use);
            v.members(args_start, *arg_count, Use);
            v.scalar(dst, Def);
        }
        Op::MakeClosure { dst, func, locals_start } => {
            let cap_n = functions.get(*func as usize).map_or(0, |f| f.captures.len() as u16);
            v.members(locals_start, cap_n, Use);
            v.scalar(dst, Def);
        }
        Op::CheckPolicy { subject, object, .. } => {
            v.scalar(subject, Use);
            if *object != Reg::MAX {
                v.scalar(object, Use);
            }
        }
        Op::ListPushField { obj, src, .. } => {
            v.scalar(obj, Use);
            v.scalar(src, Use);
        }
        Op::GlobalGet { dst, .. } => v.scalar(dst, Def),
        Op::GlobalSet { src, .. } => v.scalar(src, Use),
        Op::Return { src } => v.scalar(src, Use),
        Op::ReturnNothing => {}
        Op::NewList { dst, start, count } | Op::NewTuple { dst, start, count } => {
            v.members(start, *count, Use);
            v.scalar(dst, Def);
        }
        Op::NewEmptyList { dst }
        | Op::NewEmptyListI32 { dst }
        | Op::NewEmptySet { dst }
        | Op::NewEmptyMap { dst }
        | Op::LoadToday { dst }
        | Op::LoadNow { dst }
        | Op::NewStruct { dst, .. }
        | Op::NewCrdt { dst, .. }
        | Op::Args { dst }
        | Op::ChanNew { dst, .. }
        | Op::SelectWait { dst_arm: dst } => v.scalar(dst, Def),
        Op::NewRange { dst, start, end } => {
            v.scalar(start, Use);
            v.scalar(end, Use);
            v.scalar(dst, Def);
        }
        Op::ListPush { list, value } => {
            v.scalar(list, Use);
            v.scalar(value, Use);
        }
        Op::SetAdd { set, value } => {
            v.scalar(set, Use);
            v.scalar(value, Use);
        }
        Op::RemoveFrom { collection, value } => {
            v.scalar(collection, Use);
            v.scalar(value, Use);
        }
        Op::Contains { dst, collection, value } => {
            v.scalar(collection, Use);
            v.scalar(value, Use);
            v.scalar(dst, Def);
        }
        Op::SetIndex { collection, index, value } | Op::SetIndexUnchecked { collection, index, value } => {
            v.scalar(collection, Use);
            v.scalar(index, Use);
            v.scalar(value, Use);
        }
        Op::Index { dst, collection, index } | Op::IndexUnchecked { dst, collection, index } => {
            v.scalar(collection, Use);
            v.scalar(index, Use);
            v.scalar(dst, Def);
        }
        Op::RegionBoundsGuard { array, bound, iv, .. } => {
            v.scalar(array, Use);
            v.scalar(bound, Use);
            v.scalar(iv, Use);
        }
        Op::Length { dst, collection } => {
            v.scalar(collection, Use);
            v.scalar(dst, Def);
        }
        Op::FormatValue { dst, src, .. } => {
            v.scalar(src, Use);
            v.scalar(dst, Def);
        }
        Op::SliceOp { dst, collection, start, end } => {
            v.scalar(collection, Use);
            v.scalar(start, Use);
            v.scalar(end, Use);
            v.scalar(dst, Def);
        }
        Op::StructInsert { obj, value, .. } => {
            v.scalar(obj, Use);
            v.scalar(value, Use);
        }
        Op::GetField { dst, obj, .. } => {
            v.scalar(obj, Use);
            v.scalar(dst, Def);
        }
        Op::NewInductive { dst, args_start, count, .. } => {
            v.members(args_start, *count, Use);
            v.scalar(dst, Def);
        }
        Op::TestArm { dst, target, .. } | Op::BindArm { dst, target, .. } => {
            v.scalar(target, Use);
            v.scalar(dst, Def);
        }
        Op::CrdtBump { obj, amount, .. } => {
            v.scalar(obj, Use);
            v.scalar(amount, Use);
        }
        Op::CrdtMerge { target, source } => {
            v.scalar(target, Use);
            v.scalar(source, Use);
        }
        Op::CrdtAppend { seq, value } => {
            v.scalar(seq, Use);
            v.scalar(value, Use);
        }
        Op::CrdtResolve { obj, value, .. } => {
            v.scalar(obj, Use);
            v.scalar(value, Use);
        }
        Op::IterPrepare { iterable } => v.scalar(iterable, Use),
        Op::IterNext { dst, .. } => v.scalar(dst, Def),
        Op::IterPop => {}
        Op::ListPop { list, dst } => {
            v.scalar(list, Use);
            v.scalar(dst, Def);
        }
        Op::Sleep { duration } => v.scalar(duration, Use),
        Op::DestructureTuple { src, start, count } => {
            v.scalar(src, Use);
            v.members(start, *count, Def);
        }
        Op::Show { src } => v.scalar(src, Use),
        Op::ChanSend { chan, val } => {
            v.scalar(chan, Use);
            v.scalar(val, Use);
        }
        Op::ChanRecv { dst, chan } | Op::ChanTryRecv { dst, chan } => {
            v.scalar(chan, Use);
            v.scalar(dst, Def);
        }
        Op::ChanTrySend { dst, chan, val } => {
            v.scalar(chan, Use);
            v.scalar(val, Use);
            v.scalar(dst, Def);
        }
        Op::ChanClose { chan } => v.scalar(chan, Use),
        Op::Spawn { args_start, arg_count, .. } => v.members(args_start, *arg_count, Use),
        Op::SpawnHandle { dst, args_start, arg_count, .. } => {
            v.members(args_start, *arg_count, Use);
            v.scalar(dst, Def);
        }
        Op::TaskAwait { dst, handle } => {
            v.scalar(handle, Use);
            v.scalar(dst, Def);
        }
        Op::TaskAbort { handle } => v.scalar(handle, Use),
        Op::SelectArmRecv { chan, var } => {
            v.scalar(chan, Use);
            v.scalar(var, Def);
        }
        Op::SelectArmTimeout { ticks } => v.scalar(ticks, Use),
        Op::NetConnect { url } => v.scalar(url, Use),
        Op::NetListen { topic } => v.scalar(topic, Use),
        Op::NetSend { to, msg } => {
            v.scalar(to, Use);
            v.scalar(msg, Use);
        }
        Op::NetStream { to, values } => {
            v.scalar(to, Use);
            v.scalar(values, Use);
        }
        Op::NetAwait { dst, from, .. } => {
            v.scalar(from, Use);
            v.scalar(dst, Def);
        }
        Op::NetMakePeer { dst, addr } => {
            v.scalar(addr, Use);
            v.scalar(dst, Def);
        }
        Op::NetSync { dst, topic } => {
            v.scalar(topic, Use);
            v.scalar(dst, DefUse);
        }
        Op::FailWith { .. } | Op::Halt => {}
    }
}

/// Every register an op DEFINES and every register it USES (register ranges expanded to members),
/// for analyses outside the splitter that need an op's full data-flow footprint and must not miss
/// an operand — it reuses the same exhaustive [`visit_regs`] enumeration, so a new op cannot slip
/// past. A `DefUse` operand appears in both lists.
pub(crate) fn op_def_uses(op: &Op, functions: &[CompiledFunction]) -> (Vec<Reg>, Vec<Reg>) {
    struct Collect {
        defs: Vec<Reg>,
        uses: Vec<Reg>,
    }
    impl RegVisitor for Collect {
        fn scalar(&mut self, r: &mut Reg, role: Role) {
            match role {
                Role::Def => self.defs.push(*r),
                Role::Use => self.uses.push(*r),
                Role::DefUse => {
                    self.defs.push(*r);
                    self.uses.push(*r);
                }
            }
        }
        fn range(&mut self, start: &mut Reg, count: u16, role: Role) {
            for i in 0..count {
                self.scalar(&mut start.wrapping_add(i), role);
            }
        }
    }
    let mut tmp = op.clone();
    let mut c = Collect { defs: Vec::new(), uses: Vec::new() };
    visit_regs(&mut tmp, functions, &mut c);
    (c.defs, c.uses)
}

/// Successors of `ops[pc]` in the op CFG. Every non-control-flow op simply falls through, so the
/// explicit arms cover exactly the control-flow ops (mirroring [`super::cfg::Blocks`]).
fn successors(ops: &[Op], pc: usize) -> Vec<usize> {
    let n = ops.len();
    let fallthrough = |out: &mut Vec<usize>| {
        if pc + 1 < n {
            out.push(pc + 1);
        }
    };
    let mut out = Vec::new();
    match ops[pc] {
        Op::Jump { target } => out.push(target),
        Op::JumpIfFalse { target, .. } | Op::JumpIfTrue { target, .. } => {
            out.push(target);
            fallthrough(&mut out);
        }
        Op::IterNext { exit, .. } => {
            out.push(exit);
            fallthrough(&mut out);
        }
        Op::Return { .. } | Op::ReturnNothing | Op::Halt | Op::FailWith { .. } => {}
        _ => fallthrough(&mut out),
    }
    out
}

/// Minimal union-find over `0..n`.
struct UnionFind {
    parent: Vec<u32>,
}
impl UnionFind {
    fn new(n: usize) -> Self {
        UnionFind { parent: (0..n as u32).collect() }
    }
    fn find(&mut self, x: u32) -> u32 {
        let mut r = x;
        while self.parent[r as usize] != r {
            r = self.parent[r as usize];
        }
        // Path compression.
        let mut c = x;
        while self.parent[c as usize] != r {
            let next = self.parent[c as usize];
            self.parent[c as usize] = r;
            c = next;
        }
        r
    }
    /// Union with a DETERMINISTIC root (always the smaller index), so a web's representative does
    /// not depend on union order — and hence not on the `HashSet` iteration order that produced the
    /// reaching-def list. Without this the assigned local numbers (and emitted wasm) would vary
    /// run-to-run.
    fn union(&mut self, a: u32, b: u32) {
        let (ra, rb) = (self.find(a), self.find(b));
        if ra != rb {
            let (lo, hi) = (ra.min(rb), ra.max(rb));
            self.parent[hi as usize] = lo;
        }
    }
}

/// Collect each op's register operands as `(reg, role)` pairs (analysis side of [`visit_regs`]),
/// expanding a range into its members. `pinned` accumulates every register that is a member of a
/// contiguous range (it must keep its number).
/// The `(reg, role)` operands of an op (in `visit_regs` order) plus the operand INDICES that belong
/// to a contiguous register range (`Call` args, closure captures, `DestructureTuple` targets). A
/// range operand's web must keep its register number — the lowering reads `args_start + i` by
/// position — so those indices flag the webs that cannot be renumbered.
fn op_reg_roles(op: &Op, functions: &[CompiledFunction]) -> (Vec<(Reg, Role)>, Vec<(u32, Role)>) {
    struct Collect {
        roles: Vec<(Reg, Role)>,
        range_idx: Vec<(u32, Role)>,
    }
    impl RegVisitor for Collect {
        fn scalar(&mut self, r: &mut Reg, role: Role) {
            self.roles.push((*r, role));
        }
        fn range(&mut self, start: &mut Reg, count: u16, role: Role) {
            for i in 0..count {
                let idx = self.roles.len() as u32;
                self.roles.push((start.wrapping_add(i), role));
                self.range_idx.push((idx, role));
            }
        }
    }
    // `visit_regs` needs `&mut Op`; analysis never writes, so operate on a throwaway clone.
    let mut tmp = op.clone();
    let mut c = Collect { roles: Vec::new(), range_idx: Vec::new() };
    visit_regs(&mut tmp, functions, &mut c);
    (c.roles, c.range_idx)
}

/// The contiguous USE ranges `(start, count)` an op reads by position — a multi-member argument block
/// of a `Call`/`CallBuiltin`/`CallValue`/`MakeClosure`/`NewList`/… . DEF ranges (`DestructureTuple`)
/// are excluded: materializing a value INTO a range is the general split's job, not this one's.
/// Mirrors [`visit_regs`], so a new range-bearing op cannot slip past.
fn op_use_ranges(op: &Op, functions: &[CompiledFunction]) -> Vec<(Reg, u16)> {
    struct Collect {
        ranges: Vec<(Reg, u16)>,
    }
    impl RegVisitor for Collect {
        fn scalar(&mut self, _r: &mut Reg, _role: Role) {}
        fn range(&mut self, start: &mut Reg, count: u16, role: Role) {
            if matches!(role, Role::Use | Role::DefUse) {
                self.ranges.push((*start, count));
            }
        }
    }
    let mut tmp = op.clone();
    let mut c = Collect { ranges: Vec::new() };
    visit_regs(&mut tmp, functions, &mut c);
    c.ranges
}

/// The start register of an op's first USE range (its argument block), if it has one.
fn use_range_start(op: &Op, functions: &[CompiledFunction]) -> Option<Reg> {
    op_use_ranges(op, functions).first().map(|&(s, _)| s)
}

/// Repoint an op's (single) USE range to start at `base` — moving its argument block to a relocated
/// contiguous slot. Writes through the same [`visit_regs`] enumeration the lowering reads from.
fn set_use_range_start(op: &mut Op, functions: &[CompiledFunction], base: Reg) {
    struct SetStart {
        base: Reg,
        done: bool,
    }
    impl RegVisitor for SetStart {
        fn scalar(&mut self, _r: &mut Reg, _role: Role) {}
        fn range(&mut self, start: &mut Reg, _count: u16, role: Role) {
            if !self.done && matches!(role, Role::Use | Role::DefUse) {
                *start = self.base;
                self.done = true;
            }
        }
    }
    let mut s = SetStart { base, done: false };
    visit_regs(op, functions, &mut s);
}

/// Remap an op's control-flow target(s) from old pc to new pc after instruction insertion. The
/// target-bearing ops are exactly the ones [`successors`] branches on.
fn remap_target(op: &mut Op, new_index: &[usize]) {
    match op {
        Op::Jump { target } | Op::JumpIfFalse { target, .. } | Op::JumpIfTrue { target, .. } => {
            *target = new_index[*target];
        }
        Op::IterNext { exit, .. } => *exit = new_index[*exit],
        _ => {}
    }
}

/// Copy each targeted USE range's members into a fresh dedicated contiguous block right before its
/// op, and repoint the op there (the argument-materialization step of [`split_registers`]). Returns
/// the rewritten op stream — copies inserted, every jump target remapped to the shifted positions —
/// and the grown register-file size. Targets are deduplicated by pc; one fresh block per op.
fn materialize_ranges(
    ops: &[Op],
    num_regs: u32,
    targets: &[(usize, Reg, u16)],
    functions: &[CompiledFunction],
) -> (Vec<Op>, u32) {
    // A fresh contiguous block per target pc, assigned in pc order so the output is deterministic.
    let mut sorted: Vec<(usize, Reg, u16)> = targets.to_vec();
    sorted.sort_unstable();
    let mut block_of: HashMap<usize, (Reg, u16)> = HashMap::new();
    let mut next = num_regs;
    for &(pc, _start, count) in &sorted {
        block_of.entry(pc).or_insert_with(|| {
            let base = next as Reg;
            next += u32::from(count);
            (base, count)
        });
    }

    // Rebuild the stream, inserting each materialized op's member copies before it; record where each
    // ORIGINAL op landed (at its first inserted copy, if any) so jump targets can be remapped.
    let mut new_ops: Vec<Op> = Vec::with_capacity(ops.len() + block_of.len());
    let mut new_index: Vec<usize> = vec![0; ops.len()];
    for (pc, op) in ops.iter().enumerate() {
        new_index[pc] = new_ops.len();
        if let Some(&(base, count)) = block_of.get(&pc) {
            let old_start = use_range_start(op, functions).expect("materialized op has a use range");
            for i in 0..count {
                new_ops.push(Op::Move { dst: base.wrapping_add(i), src: old_start.wrapping_add(i) });
            }
            let mut relocated = op.clone();
            set_use_range_start(&mut relocated, functions, base);
            new_ops.push(relocated);
        } else {
            new_ops.push(op.clone());
        }
    }
    for op in &mut new_ops {
        remap_target(op, &new_index);
    }
    (new_ops, next)
}

/// Split registers reused across disjoint live ranges into one local each. Returns the rewritten op
/// stream and the new register-file size (`num_regs` plus one local per extra web). The identity
/// transform (same ops, same `num_regs`) whenever nothing is safely splittable — which is every
/// program the backend already compiles.
pub(crate) fn split_registers(
    ops: &[Op],
    num_regs: u32,
    num_params: u32,
    functions: &[CompiledFunction],
) -> (Vec<Op>, u32) {
    let n = ops.len();
    if n == 0 {
        return (ops.to_vec(), num_regs);
    }

    // ---- 1. Per-pc def/use roles, the range-operand indices (webs that must keep their number),
    //         and the pinned set (function parameters — they ARE the incoming wasm locals). ----
    let pinned: HashSet<Reg> = (0..num_params as u16).collect();
    let mut range_idx_at: Vec<Vec<(u32, Role)>> = Vec::with_capacity(n);
    let roles: Vec<Vec<(Reg, Role)>> = ops
        .iter()
        .map(|op| {
            let (r, ri) = op_reg_roles(op, functions);
            range_idx_at.push(ri);
            r
        })
        .collect();

    // Def-sites: one per (pc, defined reg). A `DefUse` both reads and writes.
    let mut def_sites: Vec<(usize, Reg)> = Vec::new();
    let mut sites_at: Vec<Vec<u32>> = vec![Vec::new(); n]; // def-site indices generated at pc
    let mut defs_at: Vec<Vec<Reg>> = vec![Vec::new(); n];
    for (pc, rs) in roles.iter().enumerate() {
        for &(r, role) in rs {
            if matches!(role, Role::Def | Role::DefUse) {
                sites_at[pc].push(def_sites.len() as u32);
                def_sites.push((pc, r));
                defs_at[pc].push(r);
            }
        }
    }
    let num_sites = def_sites.len();
    if num_sites == 0 {
        return (ops.to_vec(), num_regs);
    }
    // All def-sites of a given register (for KILL).
    let mut sites_of_reg: HashMap<Reg, Vec<u32>> = HashMap::new();
    for (i, &(_, r)) in def_sites.iter().enumerate() {
        sites_of_reg.entry(r).or_default().push(i as u32);
    }

    // ---- 2. Reaching definitions (instruction-level forward dataflow over the op CFG). ----
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
    for pc in 0..n {
        for s in successors(ops, pc) {
            if s < n {
                preds[s].push(pc);
            }
        }
    }
    let mut reach_in: Vec<HashSet<u32>> = vec![HashSet::new(); n];
    let mut reach_out: Vec<HashSet<u32>> = vec![HashSet::new(); n];
    // OUT[pc] = (IN[pc] \ KILL[pc]) ∪ GEN[pc]; iterate to a fixpoint.
    let mut changed = true;
    while changed {
        changed = false;
        for pc in 0..n {
            let mut new_in: HashSet<u32> = HashSet::new();
            for &p in &preds[pc] {
                new_in.extend(&reach_out[p]);
            }
            let mut new_out = new_in.clone();
            for &r in &defs_at[pc] {
                if let Some(killed) = sites_of_reg.get(&r) {
                    for &k in killed {
                        new_out.remove(&k);
                    }
                }
            }
            new_out.extend(&sites_at[pc]);
            if new_in != reach_in[pc] || new_out != reach_out[pc] {
                reach_in[pc] = new_in;
                reach_out[pc] = new_out;
                changed = true;
            }
        }
    }

    // ---- 3. Webs: union each use's reaching defs; a DefUse also folds in its own def. ----
    let mut uf = UnionFind::new(num_sites);
    // Registers proven NOT safely splittable: a use with no reaching def (e.g. read of an
    // uninitialized/param value) — left untouched so the pass never guesses.
    let mut unsplittable: HashSet<Reg> = HashSet::new();
    // The web (a def-site representative) chosen for each use occurrence `(pc, operand-index)`.
    let mut use_web: HashMap<(usize, u32), u32> = HashMap::new();
    for (pc, rs) in roles.iter().enumerate() {
        // Map this pc's generated def-sites by register, to fold a DefUse's own def into its web.
        let mut self_site: HashMap<Reg, u32> = HashMap::new();
        for &s in &sites_at[pc] {
            self_site.insert(def_sites[s as usize].1, s);
        }
        for (idx, &(r, role)) in rs.iter().enumerate() {
            if matches!(role, Role::Use | Role::DefUse) {
                let reaching: Vec<u32> =
                    reach_in[pc].iter().copied().filter(|&s| def_sites[s as usize].1 == r).collect();
                if reaching.is_empty() {
                    if !pinned.contains(&r) {
                        unsplittable.insert(r);
                    }
                    continue;
                }
                let first = reaching[0];
                for &s in &reaching[1..] {
                    uf.union(first, s);
                }
                if let Role::DefUse = role {
                    if let Some(&own) = self_site.get(&r) {
                        uf.union(first, own);
                    }
                }
                // Store a member of the web now; canonicalize to the web ROOT below, once ALL unions
                // (including those a later use contributes) are in — `local_of_web` is keyed by root.
                use_web.insert((pc, idx as u32), first);
            }
        }
    }
    // Canonicalize each recorded use to its final web root (the key shape `local_of_web` uses). This
    // is the load-bearing step: `first` above is an arbitrary reaching-def member, not the root, so
    // without this the rename's `local_of_web` lookup would miss and silently leave the use on its
    // old register — a miscompile (and, via `HashSet` iteration order, a nondeterministic one).
    for w in use_web.values_mut() {
        *w = uf.find(*w);
    }

    // ---- 4. Per register, the distinct webs of its def-sites (first-appearance order). ----
    let mut webs_of_reg: HashMap<Reg, Vec<u32>> = HashMap::new();
    for i in 0..num_sites {
        let (_, r) = def_sites[i];
        let w = uf.find(i as u32);
        let entry = webs_of_reg.entry(r).or_default();
        if !entry.contains(&w) {
            entry.push(w);
        }
    }

    // ---- 4a. Argument materialization. A contiguous USE range (`Call`/`CallBuiltin`/… args) whose
    //          member register the allocator REUSED across ≥2 webs cannot both keep its range
    //          position (the lowering reads `args_start + i` by number) AND split its disjoint,
    //          possibly differently-typed live ranges. The two constraints collide when the same slot
    //          is, say, one call's `i64` shift-count and another call's `i32` word value. Resolve it
    //          by copying each such range's members into a FRESH dedicated contiguous block right
    //          before the op and repointing the op there: the range now reads private single-use
    //          temps, and the reused member becomes a plain scalar the free-web split (below) can
    //          separate. Then re-run on the rewritten ops — those members are no longer range members,
    //          so this fires at most once. The transform is the identity unless a reused register is
    //          also a multi-arg range member, so a program the backend already compiles is untouched. ----
    let recycled = |r: Reg| webs_of_reg.get(&r).map_or(false, |w| w.len() >= 2);
    let mut to_materialize: Vec<(usize, Reg, u16)> = Vec::new();
    for (pc, op) in ops.iter().enumerate() {
        for (start, count) in op_use_ranges(op, functions) {
            if (0..count).any(|i| recycled(start.wrapping_add(i))) {
                to_materialize.push((pc, start, count));
            }
        }
    }
    // Skip (fall through to the in-place split) if the fresh blocks would not fit the `u16` register
    // index — a clean refusal beats a truncating miscompile, and re-running would loop forever.
    let extra: u32 = to_materialize.iter().map(|&(_, _, c)| u32::from(c)).sum();
    if !to_materialize.is_empty() && num_regs + extra <= u32::from(u16::MAX) + 1 {
        let (new_ops, new_regs) = materialize_ranges(ops, num_regs, &to_materialize, functions);
        return split_registers(&new_ops, new_regs, num_params, functions);
    }

    // Webs anchored to a contiguous register range (a `Call` arg, a closure capture, a
    // `DestructureTuple` target) must keep their register's number — the lowering reads
    // `args_start + i` by position. Disjoint anchored webs of one register share its slot (taking
    // turns); free webs of that register can move to fresh locals. This is what separates a register
    // the allocator reused for BOTH a Call argument and an unrelated differently-typed scalar.
    let mut anchored_webs: HashSet<u32> = HashSet::new();
    for (pc, ris) in range_idx_at.iter().enumerate() {
        for &(idx, role) in ris {
            match role {
                Role::Use | Role::DefUse => {
                    if let Some(&w) = use_web.get(&(pc, idx)) {
                        anchored_webs.insert(w); // already canonicalized to the web root above
                    }
                }
                Role::Def => {
                    // A range DEF member (DestructureTuple): anchor the web its def-site starts.
                    let r = roles[pc][idx as usize].0;
                    if let Some(&s) = sites_at[pc].iter().find(|&&s| def_sites[s as usize].1 == r) {
                        anchored_webs.insert(uf.find(s));
                    }
                }
            }
        }
    }

    // ---- 5. Split a non-param, fully analyzed, multi-web register: its anchored webs keep the
    //         register number (disjoint, so they share it); each free web moves to a fresh appended
    //         local. A register with no movable free web is left untouched. Sorted order keeps the
    //         module deterministic; decline if the extra locals would overflow the `u16` index. ----
    let fresh_count = |r: Reg| -> u32 {
        let webs = &webs_of_reg[&r];
        let anchored = webs.iter().filter(|w| anchored_webs.contains(w)).count();
        let free = webs.len() - anchored;
        // One web keeps the original number: an anchored web if any, else the first free web.
        if anchored >= 1 { free as u32 } else { free.saturating_sub(1) as u32 }
    };
    let splittable = |r: Reg| {
        !pinned.contains(&r) && !unsplittable.contains(&r) && webs_of_reg[&r].len() >= 2 && fresh_count(r) >= 1
    };
    let mut regs: Vec<Reg> = webs_of_reg.keys().copied().filter(|&r| splittable(r)).collect();
    if regs.is_empty() {
        return (ops.to_vec(), num_regs);
    }
    regs.sort_unstable();
    let extra: u32 = regs.iter().map(|&r| fresh_count(r)).sum();
    if num_regs + extra > u16::MAX as u32 + 1 {
        return (ops.to_vec(), num_regs); // would not fit the u16 register index — safe identity
    }

    // web representative → assigned local (absent ⇒ keep the register's own number). Anchored webs
    // claim the register's own number; the first free web claims it only if no anchored web did.
    let mut local_of_web: HashMap<u32, Reg> = HashMap::new();
    let mut next_local = num_regs;
    for &r in &regs {
        let mut claimed = false;
        for &w in &webs_of_reg[&r] {
            if anchored_webs.contains(&w) {
                local_of_web.insert(w, r);
                claimed = true;
            }
        }
        for &w in &webs_of_reg[&r] {
            if !anchored_webs.contains(&w) {
                if claimed {
                    local_of_web.insert(w, next_local as Reg);
                    next_local += 1;
                } else {
                    local_of_web.insert(w, r);
                    claimed = true;
                }
            }
        }
    }

    let new_ops = rename_ops(ops, &local_of_web, &use_web, &mut uf, &def_sites, functions);
    (new_ops, next_local)
}

/// Build the renamed op stream. A `Def`/`DefUse` field renames to its def-site's web-local; a `Use`
/// renames to its use-web's local. Anything without an assigned web-local keeps its number (the
/// identity that makes pinned/single-web registers untouched). The operand-index walk mirrors
/// [`op_reg_roles`] exactly (same `visit_regs` order), so a use's `(pc, idx)` key lines up.
fn rename_ops(
    ops: &[Op],
    local_of_web: &HashMap<u32, Reg>,
    use_web: &HashMap<(usize, u32), u32>,
    uf: &mut UnionFind,
    def_sites: &[(usize, Reg)],
    functions: &[CompiledFunction],
) -> Vec<Op> {
    // Each def-site's web-local for the def side; `(pc, reg)` is unique among def-sites here.
    let mut def_local_at: HashMap<(usize, Reg), Reg> = HashMap::new();
    for (i, &(pc, r)) in def_sites.iter().enumerate() {
        let w = uf.find(i as u32);
        if let Some(&l) = local_of_web.get(&w) {
            def_local_at.insert((pc, r), l);
        }
    }

    struct Rename<'a> {
        pc: usize,
        op_idx: u32,
        local_of_web: &'a HashMap<u32, Reg>,
        use_web: &'a HashMap<(usize, u32), u32>,
        def_local_at: &'a HashMap<(usize, Reg), Reg>,
    }
    impl RegVisitor for Rename<'_> {
        fn scalar(&mut self, r: &mut Reg, role: Role) {
            match role {
                Role::Use => {
                    if let Some(&w) = self.use_web.get(&(self.pc, self.op_idx)) {
                        if let Some(&l) = self.local_of_web.get(&w) {
                            *r = l;
                        }
                    }
                }
                Role::Def | Role::DefUse => {
                    if let Some(&l) = self.def_local_at.get(&(self.pc, *r)) {
                        *r = l;
                    }
                }
            }
            self.op_idx += 1;
        }
        fn range(&mut self, _start: &mut Reg, count: u16, _role: Role) {
            // Range members are pinned, hence never renamed; only advance the operand index past
            // the per-member `(reg, role)` pairs analysis produced for this range.
            self.op_idx += u32::from(count);
        }
    }

    let mut out = Vec::with_capacity(ops.len());
    for (pc, op) in ops.iter().enumerate() {
        let mut op = op.clone();
        let mut rn = Rename { pc, op_idx: 0, local_of_web, use_web, def_local_at: &def_local_at };
        visit_regs(&mut op, functions, &mut rn);
        out.push(op);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `{:?}` of each op — `Op` is not `PartialEq`, and the renaming is entirely about register
    /// fields, which the Debug rendering captures exactly.
    fn fmt(ops: &[Op]) -> Vec<String> {
        ops.iter().map(|o| format!("{o:?}")).collect()
    }

    fn split(ops: &[Op], num_regs: u32, num_params: u32) -> (Vec<Op>, u32) {
        split_registers(ops, num_regs, num_params, &[])
    }

    #[test]
    fn identity_when_every_register_is_single_web() {
        // A straight-line program: each register is defined once and used in one connected range, so
        // there is nothing to split — the pass must be the exact identity (same ops, same count).
        let ops = vec![
            Op::LoadConst { dst: 0, idx: 0 },
            Op::LoadConst { dst: 1, idx: 1 },
            Op::Add { dst: 2, lhs: 0, rhs: 1 },
            Op::Show { src: 2 },
            Op::Halt,
        ];
        let (out, n) = split(&ops, 3, 0);
        assert_eq!(fmt(&out), fmt(&ops), "single-web program must be untouched");
        assert_eq!(n, 3, "no locals added");
    }

    #[test]
    fn disjoint_ranges_of_one_register_split_into_separate_locals() {
        // reg1 is written+read in two disjoint ranges: [pc0 def, pc1 use] then [pc2 def, pc3 use].
        // The second range must move to a fresh local (3); the first keeps the number.
        let ops = vec![
            Op::LoadConst { dst: 1, idx: 0 },
            Op::Show { src: 1 },
            Op::LoadConst { dst: 1, idx: 1 },
            Op::Show { src: 1 },
            Op::Halt,
        ];
        let (out, n) = split(&ops, 2, 0);
        assert_eq!(n, 3, "one extra local for the second web");
        let got = fmt(&out);
        assert_eq!(got[0], format!("{:?}", Op::LoadConst { dst: 1, idx: 0 }), "first web keeps reg 1");
        assert_eq!(got[1], format!("{:?}", Op::Show { src: 1 }));
        assert_eq!(got[2], format!("{:?}", Op::LoadConst { dst: 2, idx: 1 }), "second web → local 2");
        assert_eq!(got[3], format!("{:?}", Op::Show { src: 2 }), "its use follows");
    }

    #[test]
    fn loop_carried_accumulator_is_one_web_not_split() {
        // `acc` is initialized then incremented across a back-edge — the body use is reached by both
        // the init and the back-edge def, so they are one web (a single local), never split.
        let ops = vec![
            Op::LoadConst { dst: 0, idx: 0 }, // acc = 0
            Op::LoadConst { dst: 1, idx: 1 }, // bound
            Op::Lt { dst: 2, lhs: 0, rhs: 1 }, // acc < bound  (header, pc2)
            Op::JumpIfFalse { cond: 2, target: 6 },
            Op::AddAssign { dst: 0, src: 1 }, // acc += bound
            Op::Jump { target: 2 },           // back-edge
            Op::Show { src: 0 },              // pc6
            Op::Halt,
        ];
        let (out, n) = split(&ops, 3, 0);
        assert_eq!(fmt(&out), fmt(&ops), "loop-carried accumulator must not split");
        assert_eq!(n, 3);
    }

    #[test]
    fn parameters_are_pinned_and_never_split() {
        // reg0 is a parameter reused across two disjoint ranges. Parameters are pinned (they ARE the
        // function signature), so even a genuine conflict leaves them on their declared number.
        let ops = vec![
            Op::Show { src: 0 },
            Op::LoadConst { dst: 0, idx: 0 },
            Op::Show { src: 0 },
            Op::Return { src: 0 },
        ];
        let (out, n) = split(&ops, 1, 1); // num_params = 1 ⇒ reg0 pinned
        assert_eq!(fmt(&out), fmt(&ops), "a parameter register is never renamed");
        assert_eq!(n, 1);
    }

    #[test]
    fn split_is_deterministic_across_runs() {
        // A diamond merges two definitions of reg1 into one web (so the union has multiple reaching
        // defs — the exact shape whose representative once depended on `HashSet` iteration order),
        // then a disjoint third definition forms a second web that must split out. Running the pass
        // many times must yield byte-identical ops AND a stable new register count.
        let ops = vec![
            Op::LoadConst { dst: 0, idx: 0 },        // cond
            Op::JumpIfFalse { cond: 0, target: 4 },
            Op::LoadConst { dst: 1, idx: 1 },        // def reg1 (branch A) — web W
            Op::Jump { target: 5 },
            Op::LoadConst { dst: 1, idx: 2 },        // def reg1 (branch B) — web W
            Op::Move { dst: 2, src: 1 },             // use reg1: reached by BOTH A and B
            Op::LoadConst { dst: 1, idx: 3 },        // def reg1 — web W2 (disjoint)
            Op::Return { src: 1 },                   // use reg1: reached by W2 only
        ];
        let (first, first_n) = split(&ops, 3, 0);
        assert_eq!(first_n, 4, "the second web of reg1 splits to one new local");
        // The disjoint third range moved to local 3; the merged-diamond range kept reg 1.
        let got = fmt(&first);
        assert_eq!(got[6], format!("{:?}", Op::LoadConst { dst: 3, idx: 3 }), "W2 def → local 3");
        assert_eq!(got[7], format!("{:?}", Op::Return { src: 3 }), "W2 use → local 3");
        assert_eq!(got[2], format!("{:?}", Op::LoadConst { dst: 1, idx: 1 }), "W def keeps reg 1");
        assert_eq!(got[5], format!("{:?}", Op::Move { dst: 2, src: 1 }), "W use keeps reg 1");
        for _ in 0..64 {
            let (again, again_n) = split(&ops, 3, 0);
            assert_eq!(fmt(&again), got, "splitting must be deterministic run-to-run");
            assert_eq!(again_n, first_n);
        }
    }

    /// The MD5 pattern: a register the allocator reused across two disjoint live ranges where EACH
    /// range is a member of a DIFFERENT multi-argument call (once a `Rotl` shift-count on `i64`, once
    /// a `Rotl` word value on `i32`). Both anchored uses want the same slot number, so the plain split
    /// cannot separate them. Argument materialization copies each call's arguments into a fresh
    /// dedicated block, turning the reused member into a plain scalar the def-use split then splits.
    #[test]
    fn recycled_range_member_across_two_calls_is_materialized_and_split() {
        let ops = vec![
            Op::LoadConst { dst: 2, idx: 0 },                          // reg2 web A def
            Op::LoadConst { dst: 3, idx: 1 },
            Op::Call { dst: 0, func: 0, args_start: 2, arg_count: 2 }, // reads reg2 as member 0
            Op::LoadConst { dst: 1, idx: 2 },
            Op::LoadConst { dst: 2, idx: 3 },                          // reg2 web B def (reused slot)
            Op::Call { dst: 0, func: 0, args_start: 1, arg_count: 2 }, // reads reg2 as member 1
            Op::Halt,
        ];
        let (out, n) = split(&ops, 4, 0);
        // Each call's argument block was relocated to a fresh contiguous block (>= the original 4),
        // populated by `arg_count` Moves in the immediately preceding slots.
        let calls: Vec<usize> =
            out.iter().enumerate().filter(|(_, o)| matches!(o, Op::Call { .. })).map(|(i, _)| i).collect();
        assert_eq!(calls.len(), 2, "both calls survive");
        for &ci in &calls {
            let Op::Call { args_start, arg_count, .. } = out[ci] else { unreachable!() };
            assert!(args_start >= 4, "call args relocated to a fresh block, got {args_start}");
            for j in 0..arg_count {
                let mv = &out[ci - arg_count as usize + j as usize];
                assert!(
                    matches!(mv, Op::Move { dst, .. } if *dst == args_start + j),
                    "arg slot {} populated by a preceding Move, got {mv:?}", args_start + j
                );
            }
        }
        // reg2's two disjoint definitions now write DISTINCT locals — the reuse was split.
        let dsts: Vec<Reg> = out
            .iter()
            .filter_map(|o| match o {
                Op::LoadConst { dst, idx } if *idx == 0 || *idx == 3 => Some(*dst),
                _ => None,
            })
            .collect();
        assert_eq!(dsts.len(), 2);
        assert_ne!(dsts[0], dsts[1], "the reused register was split into two locals");
        assert!(n > 4, "register file grew for the fresh blocks and the split web");
    }

    /// A multi-argument call whose members are each single-web (defined once, used once) has no reuse
    /// to resolve — materialization must be the exact identity, so a program the backend already
    /// compiles is never perturbed.
    #[test]
    fn single_web_range_members_are_not_materialized() {
        let ops = vec![
            Op::LoadConst { dst: 1, idx: 0 },
            Op::LoadConst { dst: 2, idx: 1 },
            Op::Call { dst: 0, func: 0, args_start: 1, arg_count: 2 },
            Op::Halt,
        ];
        let (out, n) = split(&ops, 3, 0);
        assert_eq!(fmt(&out), fmt(&ops), "no reused range member ⇒ no materialization");
        assert_eq!(n, 3);
    }

    /// Materialization inserts copy ops, shifting every later instruction, so a jump whose target sits
    /// past an inserted block must be remapped or it lands on the wrong op. The remapped target must
    /// resolve to the argument setup of the same call it named before the transform.
    #[test]
    fn materialization_remaps_jump_targets() {
        let ops = vec![
            Op::LoadConst { dst: 2, idx: 0 },                          // reg2 web A def
            Op::LoadConst { dst: 3, idx: 1 },
            Op::Call { dst: 0, func: 0, args_start: 2, arg_count: 2 }, // materialized (reg2 reused)
            Op::LoadConst { dst: 2, idx: 2 },                          // reg2 web B def
            Op::Jump { target: 6 },                                    // → the second call
            Op::LoadConst { dst: 9, idx: 3 },                          // skipped
            Op::Call { dst: 0, func: 0, args_start: 2, arg_count: 2 }, // materialized; the jump target
            Op::Halt,
        ];
        let (out, _n) = split(&ops, 4, 0);
        let Some(Op::Jump { target }) =
            out.iter().find(|o| matches!(o, Op::Jump { .. })).copied()
        else {
            panic!("jump survived materialization");
        };
        // The target lands on the relocated call's argument setup (a Move), and a call reading a
        // materialized block (fresh args_start) follows — the same call the jump originally named.
        assert!(matches!(out[target], Op::Move { .. }), "jump lands on the call's arg setup, got {:?}", out[target]);
        let next_call = out[target..].iter().find(|o| matches!(o, Op::Call { .. }));
        assert!(
            matches!(next_call, Some(Op::Call { args_start, .. }) if *args_start >= 4),
            "the targeted call reads a materialized block, got {next_call:?}"
        );
    }

    /// Materialization + the follow-on split must be byte-identical run-to-run (fresh blocks assigned
    /// in pc order, webs canonicalized deterministically) — a nondeterministic pass would break the
    /// tier cache keyed by source.
    #[test]
    fn materialization_is_deterministic() {
        let ops = vec![
            Op::LoadConst { dst: 2, idx: 0 },
            Op::LoadConst { dst: 3, idx: 1 },
            Op::Call { dst: 0, func: 0, args_start: 2, arg_count: 2 },
            Op::LoadConst { dst: 1, idx: 2 },
            Op::LoadConst { dst: 2, idx: 3 },
            Op::Call { dst: 0, func: 0, args_start: 1, arg_count: 2 },
            Op::Halt,
        ];
        let (first, first_n) = split(&ops, 4, 0);
        let got = fmt(&first);
        for _ in 0..64 {
            let (again, again_n) = split(&ops, 4, 0);
            assert_eq!(fmt(&again), got, "materialization must be deterministic run-to-run");
            assert_eq!(again_n, first_n);
        }
    }
}
