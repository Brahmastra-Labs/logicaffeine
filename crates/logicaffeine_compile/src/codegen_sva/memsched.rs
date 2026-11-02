//! memsched — the static-memory-schedule FACT.
//!
//! The chip's keystone claim is "the memory access schedule is fully static." This module
//! turns that from prose into a measurement: every ISA op is classified by its memory
//! behavior (EXHAUSTIVE match — a new op cannot compile unclassified, the same
//! pit-of-success shape as `vm::isa::silicon_status`), index registers are classified by a
//! flow-insensitive fixpoint (`Const` / `Induction` / `Opaque`), and a program gets a
//! per-site verdict: what fraction of its datapath memory sites have a compile-time
//! schedulable address stream.
//!
//! The analysis deliberately UNDER-approximates (flow-insensitive; anything unproven is
//! `Opaque` → dynamic), so the reported fraction is a FLOOR — exactly what the ratchet in
//! `logicaffeine_tests/tests/memsched_lock.rs` wants: floors only rise.
//!
//! Vocabulary (what the DRAM scheduler in `codegen_sva::dram` consumes):
//!   • static site  — address stream known at compile time: allocation, sequential
//!     append/scan/copy (a DMA descriptor), constant-offset field/global slot, or an
//!     element access whose index register is a constant or a linear induction variable;
//!   • dynamic site — data-dependent address: gather/scatter via an unproven index, or a
//!     hash-structure access (set/map);
//!   • coprocessor site — I/O, time, channels, tasks, network, kernel builtins: off the
//!     memory datapath entirely (they live on the NoC / service units, not the DRAM edge).

use crate::vm::{CompiledProgram, Op, Reg};

/// The memory shape of one ISA op. Exhaustive over every variant — no wildcard.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SiteKind {
    /// Registers/control only — never touches collection memory.
    NoMem,
    /// Allocates a fresh collection (allocator bump + sequential init) — static.
    Alloc,
    /// Sequential append/pop/scan/copy — a (base, stride, count) stream; static.
    StaticStream,
    /// Constant-offset access: struct field, enum tag/arm, global slot, length header — static.
    StaticField,
    /// Element access through an index register — static iff the register is
    /// `Const` or `Induction` at analysis time.
    IndexedBy(Reg),
    /// Hash-structure access (set/map insert/remove/lookup/union) — dynamic.
    DynamicHash,
    /// Off-datapath service: I/O, clock, scheduler, channels, tasks, network, builtins.
    Coprocessor,
}

/// Classify one op's memory behavior. EXHAUSTIVE — adding a VM op forces a decision here.
pub fn site_kind(op: &Op) -> SiteKind {
    use SiteKind::*;
    match op {
        // Pure register / ALU / control.
        Op::LoadConst { .. }
        | Op::Move { .. }
        | Op::RevSwap { .. }
        | Op::RevXor { .. }
        | Op::RevNeg { .. }
        | Op::RevAdd { .. }
        | Op::RevSub { .. }
        | Op::Add { .. }
        | Op::BigIntWide { .. }
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
        | Op::Pow { .. }
        | Op::BitXor { .. }
        | Op::BitAnd { .. }
        | Op::BitOr { .. }
        | Op::Shl { .. }
        | Op::Shr { .. }
        | Op::Jump { .. }
        | Op::JumpIfFalse { .. }
        | Op::JumpIfTrue { .. }
        | Op::Call { .. }
        | Op::CallValue { .. }
        | Op::Return { .. }
        | Op::ReturnNothing
        | Op::RegionBoundsGuard { .. }
        | Op::IterPop
        | Op::Halt => NoMem,

        // Fresh collections: allocator stream + sequential init.
        Op::NewList { .. }
        | Op::NewEmptyList { .. }
        | Op::NewEmptyListWithCapacity { .. }
        | Op::NewEmptyListFixed { .. }
        | Op::NewEmptyListI32 { .. }
        | Op::NewEmptySet { .. }
        | Op::NewEmptyMap { .. }
        | Op::NewEmptyMapWithCapacity { .. }
        | Op::NewRange { .. }
        | Op::NewTuple { .. }
        | Op::NewStruct { .. }
        | Op::NewInductive { .. }
        | Op::NewCrdt { .. }
        | Op::Args { .. } => Alloc,

        // Sequential streams: append/pop/snapshot/scan/copy — DMA-descriptor shaped.
        Op::ListPush { .. }
        | Op::ListPop { .. }
        | Op::ListPushField { .. }
        | Op::Concat { .. }
        | Op::SeqConcat { .. }
        | Op::SliceOp { .. }
        | Op::DeepClone { .. }
        | Op::EnsureOwned { .. }
        | Op::MakeClosure { .. }
        | Op::FormatValue { .. }
        | Op::DestructureTuple { .. }
        | Op::IterPrepare { .. }
        | Op::IterNext { .. }
        | Op::CrdtAppend { .. }
        | Op::CrdtMerge { .. }
        | Op::CrdtJoin { .. } => StaticStream,

        // Constant-offset slots: fields, tags, arms, globals, length headers.
        Op::GlobalGet { .. }
        | Op::GlobalSet { .. }
        | Op::GetField { .. }
        | Op::StructInsert { .. }
        | Op::TestArm { .. }
        | Op::BindArm { .. }
        | Op::Length { .. }
        | Op::CrdtBump { .. }
        | Op::CrdtResolve { .. } => StaticField,

        // Element access through an index register.
        Op::Index { index, .. }
        | Op::IndexUnchecked { index, .. }
        | Op::SetIndex { index, .. }
        | Op::SetIndexUnchecked { index, .. } => IndexedBy(*index),

        // Hash structures.
        Op::SetAdd { .. }
        | Op::RemoveFrom { .. }
        | Op::Contains { .. }
        | Op::UnionOp { .. }
        | Op::IntersectOp { .. } => DynamicHash,

        // Off-datapath services.
        Op::CallBuiltin { .. }
        | Op::CheckPolicy { .. }
        | Op::LoadToday { .. }
        | Op::LoadNow { .. }
        | Op::Sleep { .. }
        | Op::Show { .. }
        | Op::Obs { .. }
        | Op::FailWith { .. }
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
        | Op::Serve { .. } => Coprocessor,
    }
}

/// The address class of a register within one code segment (one function body).
///
/// The hardware criterion is DESCRIPTOR-TIME COMPUTABILITY: an address is static when
/// it derives only from constants, loop arithmetic over such values, and collection
/// METADATA (length) — never from loaded payload data. That is a taint analysis:
/// payload-producing writes (element loads, iterator values, pops, call results,
/// channel receives, …) taint; pure ALU over untainted operands stays untainted.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum RegClass {
    /// Provably computed from constants / loop arithmetic / length metadata only —
    /// the address stream is enumerable when the transfer descriptor is set up.
    StaticAddr,
    /// Anything unproven: payload-derived, parameter, opaque call result. Dynamic.
    Opaque,
}

/// One function body's half-open instruction range within `CompiledProgram::code`.
fn segments(prog: &CompiledProgram) -> Vec<(usize, usize)> {
    let mut entries: Vec<usize> = prog.functions.iter().map(|c| c.entry_pc).collect();
    entries.sort_unstable();
    entries.dedup();
    let mut segs = Vec::new();
    let first = entries.first().copied().unwrap_or(prog.code.len());
    segs.push((0, first)); // Main
    for (i, &e) in entries.iter().enumerate() {
        let end = entries.get(i + 1).copied().unwrap_or(prog.code.len());
        segs.push((e, end));
    }
    segs
}

/// The non-self READ operands of the pure-ALU shapes that PRESERVE static addresses.
/// Returns `None` for any op that is not address-arithmetic (its writes taint).
fn alu_reads(op: &Op) -> Option<(Reg, Vec<Reg>)> {
    let two = |dst: &Reg, lhs: &Reg, rhs: &Reg| Some((*dst, vec![*lhs, *rhs]));
    match op {
        Op::LoadConst { dst, .. } => Some((*dst, vec![])),
        Op::Move { dst, src } => Some((*dst, vec![*src])),
        // Length is METADATA (available at descriptor-setup time), not payload.
        Op::Length { dst, .. } => Some((*dst, vec![])),
        Op::Add { dst, lhs, rhs }
        | Op::Sub { dst, lhs, rhs }
        | Op::Mul { dst, lhs, rhs }
        | Op::Div { dst, lhs, rhs }
        | Op::ExactDiv { dst, lhs, rhs }
        | Op::FloorDiv { dst, lhs, rhs }
        | Op::Mod { dst, lhs, rhs }
        | Op::Pow { dst, lhs, rhs }
        | Op::BitXor { dst, lhs, rhs }
        | Op::BitAnd { dst, lhs, rhs }
        | Op::BitOr { dst, lhs, rhs }
        | Op::Shl { dst, lhs, rhs }
        | Op::Shr { dst, lhs, rhs }
        | Op::Lt { dst, lhs, rhs }
        | Op::Gt { dst, lhs, rhs }
        | Op::LtEq { dst, lhs, rhs }
        | Op::GtEq { dst, lhs, rhs }
        | Op::Eq { dst, lhs, rhs }
        | Op::NotEq { dst, lhs, rhs } => two(dst, lhs, rhs),
        Op::AddAssign { dst, src } => Some((*dst, vec![*src])),
        Op::Not { dst, src } => Some((*dst, vec![*src])),
        Op::DivPow2 { dst, lhs, .. } => Some((*dst, vec![*lhs])),
        Op::MagicDivU { dst, lhs, .. } => Some((*dst, vec![*lhs])),
        Op::RevAdd { place, delta } | Op::RevSub { place, delta } | Op::RevXor { place, delta } => {
            Some((*place, vec![*delta]))
        }
        Op::RevNeg { place } => Some((*place, vec![])),
        _ => None,
    }
}

/// Instruction successors within one segment — the op-level CFG.
fn successors(code: &[Op], lo: usize, hi: usize, pc: usize) -> Vec<usize> {
    let in_seg = |t: usize| if t >= lo && t < hi { Some(t) } else { None };
    match &code[pc] {
        Op::Jump { target } => in_seg(*target).into_iter().collect(),
        Op::JumpIfFalse { target, .. } | Op::JumpIfTrue { target, .. } => {
            let mut s: Vec<usize> = in_seg(*target).into_iter().collect();
            if pc + 1 < hi {
                s.push(pc + 1);
            }
            s
        }
        Op::IterNext { exit, .. } => {
            let mut s: Vec<usize> = in_seg(*exit).into_iter().collect();
            if pc + 1 < hi {
                s.push(pc + 1);
            }
            s
        }
        Op::Return { .. } | Op::ReturnNothing | Op::Halt | Op::FailWith { .. } => Vec::new(),
        _ => {
            if pc + 1 < hi {
                vec![pc + 1]
            } else {
                Vec::new()
            }
        }
    }
}

/// Flow-sensitive address analysis for one segment: classic REACHING DEFINITIONS at
/// the op level (registers are heavily REUSED across loops, so a whole-segment write
/// set falsely taints constant indices — nbody's `Set item 1 …` after a `Mul` temp
/// shared the register), plus a COINDUCTIVE def-level fixpoint: every arithmetic def
/// starts optimistically static and is demoted when (a) its shape taints, (b) an
/// operand has NO reaching def (parameter / callee window — the honest floor), or
/// (c) an operand can be reached by a non-static def. Induction cycles survive
/// (their only demotion witness would be themselves); payload taint propagates.
struct SegmentFlow {
    /// def id → still considered static-address?
    def_static: Vec<bool>,
    /// per pc: bitset (u64 words over def ids) of defs reaching the ENTRY of pc.
    reaching_in: Vec<Vec<u64>>,
    defs_of_reg: Vec<Vec<u32>>, // reg → def ids (for kill sets)
    lo: usize,
}

impl SegmentFlow {
    fn build(code: &[Op], lo: usize, hi: usize) -> SegmentFlow {
        // 1. Enumerate defs: (pc, reg, shape). Arithmetic shapes carry their reads.
        let mut def_at: Vec<(usize, Reg)> = Vec::new();
        let mut def_reads: Vec<Option<Vec<Reg>>> = Vec::new(); // None = taint
        let mut max_reg = 0usize;
        for pc in lo..hi {
            let op = &code[pc];
            match (alu_reads(op), op) {
                (_, Op::RevSwap { a, b }) => {
                    for r in [*a, *b] {
                        def_at.push((pc, r));
                        def_reads.push(None);
                        max_reg = max_reg.max(r as usize + 1);
                    }
                }
                (Some((dst, reads)), _) => {
                    for &r in &reads {
                        max_reg = max_reg.max(r as usize + 1);
                    }
                    max_reg = max_reg.max(dst as usize + 1);
                    def_at.push((pc, dst));
                    def_reads.push(Some(reads));
                }
                (None, other) => {
                    other.for_each_written_reg(|r| {
                        def_at.push((pc, r));
                        def_reads.push(None);
                        max_reg = max_reg.max(r as usize + 1);
                    });
                }
            }
            if let SiteKind::IndexedBy(r) = site_kind(op) {
                max_reg = max_reg.max(r as usize + 1);
            }
        }
        let ndefs = def_at.len();
        let words = ndefs.div_ceil(64).max(1);
        let mut defs_of_reg: Vec<Vec<u32>> = vec![Vec::new(); max_reg];
        for (id, &(_, r)) in def_at.iter().enumerate() {
            defs_of_reg[r as usize].push(id as u32);
        }

        // 2. Reaching-definitions dataflow (forward, may) over the segment CFG.
        let n = hi - lo;
        let mut r_in: Vec<Vec<u64>> = vec![vec![0; words]; n];
        let mut r_out: Vec<Vec<u64>> = vec![vec![0; words]; n];
        // gen/kill per pc
        let mut gen: Vec<Vec<u32>> = vec![Vec::new(); n];
        for (id, &(pc, _)) in def_at.iter().enumerate() {
            gen[pc - lo].push(id as u32);
        }
        // Precompute predecessors once.
        let mut preds: Vec<Vec<usize>> = vec![Vec::new(); n];
        for p in lo..hi {
            for s in successors(code, lo, hi, p) {
                preds[s - lo].push(p);
            }
        }
        let mut changed = true;
        while changed {
            changed = false;
            for pc in lo..hi {
                let i = pc - lo;
                let mut newin = vec![0u64; words];
                for &p in &preds[i] {
                    for w in 0..words {
                        newin[w] |= r_out[p - lo][w];
                    }
                }
                if newin != r_in[i] {
                    r_in[i] = newin;
                    changed = true;
                }
                // OUT = gen ∪ (IN − kill(regs defined here))
                let mut newout = r_in[i].clone();
                for &id in &gen[i] {
                    let (_, reg) = def_at[id as usize];
                    for &k in &defs_of_reg[reg as usize] {
                        newout[(k / 64) as usize] &= !(1u64 << (k % 64));
                    }
                }
                for &id in &gen[i] {
                    newout[(id / 64) as usize] |= 1u64 << (id % 64);
                }
                if newout != r_out[i] {
                    r_out[i] = newout;
                    changed = true;
                }
            }
        }

        // 3. Coinductive def-static fixpoint (downward, monotone, terminates).
        let mut def_static: Vec<bool> = def_reads.iter().map(|d| d.is_some()).collect();
        let reach = |r_in: &Vec<Vec<u64>>, pc: usize, reg: Reg| -> Vec<u32> {
            defs_of_reg
                .get(reg as usize)
                .map(|ids| {
                    ids.iter()
                        .copied()
                        .filter(|&id| r_in[pc - lo][(id / 64) as usize] & (1u64 << (id % 64)) != 0)
                        .collect()
                })
                .unwrap_or_default()
        };
        let mut demoted = true;
        while demoted {
            demoted = false;
            for id in 0..ndefs {
                if !def_static[id] {
                    continue;
                }
                let (pc, _) = def_at[id];
                let Some(reads) = &def_reads[id] else {
                    unreachable!()
                };
                let ok = reads.iter().all(|&o| {
                    let rd = reach(&r_in, pc, o);
                    !rd.is_empty() && rd.iter().all(|&d| def_static[d as usize])
                });
                if !ok {
                    def_static[id] = false;
                    demoted = true;
                }
            }
        }

        SegmentFlow {
            def_static,
            reaching_in: r_in,
            defs_of_reg,
            lo,
        }
    }

    /// Is register `r`'s value at the ENTRY of `pc` a static address?
    fn static_at(&self, pc: usize, r: Reg) -> bool {
        let Some(ids) = self.defs_of_reg.get(r as usize) else {
            return false;
        };
        let row = &self.reaching_in[pc - self.lo];
        let mut any = false;
        for &id in ids {
            if row[(id / 64) as usize] & (1u64 << (id % 64)) != 0 {
                any = true;
                if !self.def_static[id as usize] {
                    return false;
                }
            }
        }
        any
    }
}

/// The measured verdict for one program.
#[derive(Clone, Debug, Default)]
pub struct MemSchedReport {
    /// Alloc + stream + const-field + provably-affine indexed sites.
    pub static_sites: usize,
    /// Unproven-index gathers/scatters + hash-structure sites.
    pub dynamic_sites: usize,
    /// Off-datapath service sites (I/O, channels, tasks, net, builtins).
    pub coprocessor_sites: usize,
    /// Ops that never touch collection memory.
    pub nomem_sites: usize,
}

impl MemSchedReport {
    pub fn datapath_sites(&self) -> usize {
        self.static_sites + self.dynamic_sites
    }
    /// Fraction of datapath memory sites with a compile-time-schedulable address
    /// stream. A program with no datapath memory at all is trivially fully static.
    pub fn static_fraction(&self) -> f64 {
        let d = self.datapath_sites();
        if d == 0 {
            1.0
        } else {
            self.static_sites as f64 / d as f64
        }
    }
}

/// Analyze a compiled program: classify every op site, resolving indexed sites
/// against the enclosing function segment's register classes.
pub fn analyze(prog: &CompiledProgram) -> MemSchedReport {
    let mut report = MemSchedReport::default();
    for (lo, hi) in segments(prog) {
        if lo >= hi {
            continue;
        }
        let flow = SegmentFlow::build(&prog.code, lo, hi);
        for (i, op) in prog.code[lo..hi].iter().enumerate() {
            match site_kind(op) {
                SiteKind::NoMem => report.nomem_sites += 1,
                SiteKind::Alloc | SiteKind::StaticStream | SiteKind::StaticField => {
                    report.static_sites += 1
                }
                SiteKind::DynamicHash => report.dynamic_sites += 1,
                SiteKind::Coprocessor => report.coprocessor_sites += 1,
                SiteKind::IndexedBy(r) => {
                    if flow.static_at(lo + i, r) {
                        report.static_sites += 1
                    } else {
                        report.dynamic_sites += 1
                    }
                }
            }
        }
    }
    report
}

/// The software-managed-cache plan for a program (GAPS G19, `MEMORY_EDGE_SPEC.md §3`):
/// the datapath memory sites partitioned into `prefetch` (compile-time-known address
/// stream — served from the static schedule, zero runtime miss, zero-jitter) and
/// `probe` (data-dependent — a tag probe that may miss and fall back to the managed
/// cache). `pc` values index `CompiledProgram::code`.
#[derive(Clone, Debug, Default)]
pub struct CachePlan {
    pub prefetch: Vec<usize>,
    pub probe: Vec<usize>,
}

impl CachePlan {
    /// Every datapath memory site is in exactly one list (no access is dropped).
    pub fn is_total_over(&self, report: &MemSchedReport) -> bool {
        self.prefetch.len() == report.static_sites
            && self.probe.len() == report.dynamic_sites
            && self.prefetch.len() + self.probe.len() == report.datapath_sites()
    }
}

/// Build the managed-cache plan: exactly the memsched static/dynamic partition, so the
/// probe list — the ONLY sites that can miss — equals the compiler's `dynamic_sites`,
/// and streaming kernels (all-static) produce an EMPTY probe list (they never touch the
/// managed-cache path, preserving their zero-jitter contract).
pub fn cache_plan(prog: &CompiledProgram) -> CachePlan {
    let mut plan = CachePlan::default();
    for (lo, hi) in segments(prog) {
        if lo >= hi {
            continue;
        }
        let flow = SegmentFlow::build(&prog.code, lo, hi);
        for (i, op) in prog.code[lo..hi].iter().enumerate() {
            let pc = lo + i;
            match site_kind(op) {
                SiteKind::Alloc | SiteKind::StaticStream | SiteKind::StaticField => {
                    plan.prefetch.push(pc)
                }
                SiteKind::DynamicHash => plan.probe.push(pc),
                SiteKind::IndexedBy(r) => {
                    if flow.static_at(pc, r) {
                        plan.prefetch.push(pc)
                    } else {
                        plan.probe.push(pc)
                    }
                }
                SiteKind::NoMem | SiteKind::Coprocessor => {}
            }
        }
    }
    plan
}

/// Human-readable diagnosis of the first `limit` DYNAMIC sites: which op, which index
/// register, and every write shape that register has in its segment — the tool for
/// understanding why a kernel's fraction is what it is (and for sharpening the floor).
pub fn explain_dynamic_sites(prog: &CompiledProgram, limit: usize) -> String {
    let mut out = String::new();
    for (lo, hi) in segments(prog) {
        if lo >= hi {
            continue;
        }
        let flow = SegmentFlow::build(&prog.code, lo, hi);
        for (pc, op) in prog.code[lo..hi].iter().enumerate() {
            let dynamic = match site_kind(op) {
                SiteKind::DynamicHash => Some(None),
                SiteKind::IndexedBy(r) if !flow.static_at(lo + pc, r) => Some(Some(r)),
                _ => None,
            };
            let Some(index_reg) = dynamic else { continue };
            out.push_str(&format!("pc {} {}", lo + pc, op.name()));
            if let Some(r) = index_reg {
                out.push_str(&format!(" index=r{r} writes=["));
                for (wpc, wop) in prog.code[lo..hi].iter().enumerate() {
                    let mut hits = false;
                    wop.for_each_written_reg(|w| hits |= w == r);
                    if hits {
                        out.push_str(&format!("{}@{} ", wop.name(), lo + wpc));
                    }
                }
                out.push(']');
            }
            out.push('\n');
            if out.lines().count() >= limit {
                return out;
            }
        }
    }
    out
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/codegen_sva/memsched/tests.rs"]
mod tests;
