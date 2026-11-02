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

use crate::vm::instruction::{
    op_successors, visit_regs, CompiledFunction, Op, Reg, RegVisitor, Role,
};

/// Minimal union-find over `0..n`.
struct UnionFind {
    parent: Vec<u32>,
}
impl UnionFind {
    fn new(n: usize) -> Self {
        UnionFind {
            parent: (0..n as u32).collect(),
        }
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
    let mut c = Collect {
        roles: Vec::new(),
        range_idx: Vec::new(),
    };
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
) -> (Vec<Op>, u32, Vec<usize>) {
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
    let mut placed: Vec<usize> = vec![0; ops.len()];
    for (pc, op) in ops.iter().enumerate() {
        new_index[pc] = new_ops.len();
        if let Some(&(base, count)) = block_of.get(&pc) {
            let old_start =
                use_range_start(op, functions).expect("materialized op has a use range");
            for i in 0..count {
                new_ops.push(Op::Move {
                    dst: base.wrapping_add(i),
                    src: old_start.wrapping_add(i),
                });
            }
            let mut relocated = op.clone();
            set_use_range_start(&mut relocated, functions, base);
            placed[pc] = new_ops.len();
            new_ops.push(relocated);
            // A call leaves each `mutable` parameter's final value in its argument register, which the code after
            // it reads at the ORIGINAL position: copy those members back out of the fresh block.
            if let Op::Call { func, .. } = op {
                if let Some(f) = functions.get(*func as usize) {
                    for &p in f.mutable_param_regs.iter().filter(|p| **p < count) {
                        new_ops.push(Op::Move {
                            dst: old_start.wrapping_add(p),
                            src: base.wrapping_add(p),
                        });
                    }
                }
            }
        } else {
            placed[pc] = new_ops.len();
            new_ops.push(op.clone());
        }
    }
    for op in &mut new_ops {
        remap_target(op, &new_index);
    }
    (new_ops, next, placed)
}

/// Split registers reused across disjoint live ranges into one local each. Returns the rewritten op
/// stream, the new register-file size (`num_regs` plus one local per extra web), and where each
/// original op landed in the rewritten stream (materializing a register range inserts copies ahead
/// of the op that reads it; anything keyed by op position is read through this). The identity
/// transform (same ops, same `num_regs`) whenever nothing is safely splittable — which is every
/// program the backend already compiles.
pub(crate) fn split_registers(
    ops: &[Op],
    num_regs: u32,
    num_params: u32,
    functions: &[CompiledFunction],
) -> (Vec<Op>, u32, Vec<usize>) {
    let n = ops.len();
    let identity = || (0..n).collect::<Vec<usize>>();
    if n == 0 {
        return (ops.to_vec(), num_regs, identity());
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
        return (ops.to_vec(), num_regs, identity());
    }
    // All def-sites of a given register (for KILL).
    let mut sites_of_reg: HashMap<Reg, Vec<u32>> = HashMap::new();
    for (i, &(_, r)) in def_sites.iter().enumerate() {
        sites_of_reg.entry(r).or_default().push(i as u32);
    }

    // ---- 2. Reaching definitions (instruction-level forward dataflow over the op CFG). ----
    let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
    for pc in 0..n {
        for s in op_successors(&ops[pc], pc, ops.len()) {
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
                let reaching: Vec<u32> = reach_in[pc]
                    .iter()
                    .copied()
                    .filter(|&s| def_sites[s as usize].1 == r)
                    .collect();
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

    // ---- 4a. Argument materialization. A contiguous USE range (`Call`/`CallBuiltin`/… args) whose
    //          member register holds TWO anchored webs — the slot is, say, one call's `i64`
    //          shift-count and another call's `i32` word value — cannot keep both at its position
    //          (the lowering reads `args_start + i` by number) and still give them different local
    //          types. Resolve it by copying each such range's members into a FRESH dedicated
    //          contiguous block right before the op and repointing the op there: the range now reads
    //          private single-use temps, and the member becomes a plain scalar the free-web split
    //          (below) can separate. Then re-run on the rewritten ops — those members are no longer
    //          range members, so this fires at most once. A member whose OTHER webs are all free
    //          needs no copy: its anchored web keeps the number and the free webs move (step 5), so a
    //          list literal pays no per-element `Move` for a slot an earlier scalar also used. ----
    let conflicting = |r: Reg| {
        webs_of_reg.get(&r).map_or(false, |ws| {
            ws.iter().filter(|w| anchored_webs.contains(w)).count() >= 2
        })
    };
    let mut to_materialize: Vec<(usize, Reg, u16)> = Vec::new();
    for (pc, op) in ops.iter().enumerate() {
        for (start, count) in op_use_ranges(op, functions) {
            if (0..count).any(|i| conflicting(start.wrapping_add(i))) {
                to_materialize.push((pc, start, count));
            }
        }
    }
    // Skip (fall through to the in-place split) if the fresh blocks would not fit the `u16` register
    // index — a clean refusal beats a truncating miscompile, and re-running would loop forever.
    let extra: u32 = to_materialize.iter().map(|&(_, _, c)| u32::from(c)).sum();
    if !to_materialize.is_empty() && num_regs + extra <= u32::from(u16::MAX) + 1 {
        let (new_ops, new_regs, materialized) =
            materialize_ranges(ops, num_regs, &to_materialize, functions);
        let (split, split_regs, placed) =
            split_registers(&new_ops, new_regs, num_params, functions);
        return (
            split,
            split_regs,
            materialized.iter().map(|&at| placed[at]).collect(),
        );
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
        if anchored >= 1 {
            free as u32
        } else {
            free.saturating_sub(1) as u32
        }
    };
    let splittable = |r: Reg| {
        !pinned.contains(&r)
            && !unsplittable.contains(&r)
            && webs_of_reg[&r].len() >= 2
            && fresh_count(r) >= 1
    };
    let mut regs: Vec<Reg> = webs_of_reg
        .keys()
        .copied()
        .filter(|&r| splittable(r))
        .collect();
    if regs.is_empty() {
        return (ops.to_vec(), num_regs, identity());
    }
    regs.sort_unstable();
    let extra: u32 = regs.iter().map(|&r| fresh_count(r)).sum();
    if num_regs + extra > u16::MAX as u32 + 1 {
        return (ops.to_vec(), num_regs, identity()); // would not fit the u16 register index — safe identity
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
    (new_ops, next_local, identity())
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
        let mut rn = Rename {
            pc,
            op_idx: 0,
            local_of_web,
            use_web,
            def_local_at: &def_local_at,
        };
        visit_regs(&mut op, functions, &mut rn);
        out.push(op);
    }
    out
}

/// Nothing written only to relinquish a register's old value does not change
/// that local's payload type. Prove the new absence is unread along every CFG
/// path before a replacement definition. In-out results are implicit reads at
/// function exits; calls' argument reads/writes come from the shared operand map.
pub(crate) fn unobserved_nothing_writes(
    ops: &[Op],
    constants: &[crate::vm::instruction::Constant],
    functions: &[CompiledFunction],
    inout: &[Reg],
) -> HashSet<usize> {
    use crate::vm::instruction::{op_def_uses, Constant};
    let candidates: Vec<_> = ops
        .iter()
        .enumerate()
        .filter_map(|(pc, op)| match op {
            Op::LoadConst { dst, idx }
                if matches!(constants.get(*idx as usize), Some(Constant::Nothing)) =>
            {
                Some((pc, *dst))
            }
            _ => None,
        })
        .collect();
    if candidates.is_empty() {
        return HashSet::new();
    }
    let mut registers: Vec<_> = candidates.iter().map(|&(_, r)| r).collect();
    registers.sort_unstable();
    registers.dedup();
    let index: HashMap<_, _> = registers.iter().enumerate().map(|(i, &r)| (r, i)).collect();
    let words = registers.len().div_ceil(64);
    let n = ops.len();
    let mut defs = vec![0_u64; n * words];
    let mut uses = vec![0_u64; n * words];
    let mut exit = vec![0_u64; words];
    for r in inout {
        if let Some(&i) = index.get(r) {
            exit[i / 64] |= 1 << (i % 64);
        }
    }
    let mut successors = Vec::with_capacity(n);
    for (pc, op) in ops.iter().enumerate() {
        let (d, u) = op_def_uses(op, functions);
        for (bits, regs) in [(&mut defs, d), (&mut uses, u)] {
            for r in regs {
                if let Some(&i) = index.get(&r) {
                    bits[pc * words + i / 64] |= 1 << (i % 64);
                }
            }
        }
        successors.push(op_successors(op, pc, n));
    }
    let mut live = vec![0_u64; n * words];
    loop {
        let mut changed = false;
        for pc in (0..n).rev() {
            for w in 0..words {
                let out = (if successors[pc].is_empty() || pc + 1 == n {
                    exit[w]
                } else {
                    0
                }) | {
                    successors[pc].iter().fold(0, |bits, &next| {
                        bits | if next < n {
                            live[next * words + w]
                        } else {
                            u64::MAX
                        }
                    })
                };
                let value = uses[pc * words + w] | (out & !defs[pc * words + w]);
                if live[pc * words + w] != value {
                    live[pc * words + w] = value;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    candidates
        .into_iter()
        .filter_map(|(pc, r)| {
            let i = index[&r];
            let mask = 1_u64 << (i % 64);
            let read = ((successors[pc].is_empty() || pc + 1 == n) && exit[i / 64] & mask != 0)
                || {
                    successors[pc]
                        .iter()
                        .any(|&next| next >= n || live[next * words + i / 64] & mask != 0)
                };
            (!read).then_some(pc)
        })
        .collect()
}

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_compile/vm/wasm/regsplit/tests.rs"]
mod tests;
