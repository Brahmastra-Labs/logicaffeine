//! Which integers of a region of VM bytecode are held exact — the shared decision ([`crate::analysis::exactness`]),
//! asked of the bytecode for every backend that lowers it (the C walker, WebAssembly).
//!
//! A backend keeps an `Int` in a machine word wherever nothing makes it outgrow one and holds it exact — a tagged
//! word, small or boxed as it needs — wherever something can. The backend answers what its own typing knows
//! ([`ExactnessHost`]); this module turns the region into the decision's graph: a definition per op that writes one
//! integer register, per parameter, and per register an op that writes several leaves an integer in (a call's
//! `mutable` arguments, a swap's two sides); what each is computed from by reaching definitions; and a web of the
//! definitions that reach one use.
//!
//! An exact web is *speculative*: until a value in it outgrows i64 it is the machine word, so a backend that versions
//! the region runs it as one, checking each operation that can overflow and each exact value that arrives (a result,
//! a parameter, a global), and moves into the region held exact at the first that does not fit. Only a web the exact
//! tower itself computes (a function value's body) is exact from the start.

use std::collections::{BTreeSet, HashMap, HashSet};

use super::{CompiledProgram, Constant, Op};
use crate::analysis::exactness::{decide, Flow, Graph, Operation};

/// The integer form a backend gives a register: a machine word, or held exact.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IntForm {
    Word,
    Exact,
}

/// What the analysis asks of the backend lowering a region: its typing on machine words, where only what an exact
/// tower computes is exact already.
pub(crate) trait ExactnessHost {
    fn program(&self) -> &CompiledProgram;
    /// The op at `pc` of the region: the program's own code, unless the backend lowers a rebased copy of it.
    fn op(&self, pc: usize) -> &Op {
        &self.program().code[pc]
    }
    /// The integer form of `reg` after the op at `pc`; `None` when it holds no integer.
    fn int_after(&self, pc: usize, reg: u16) -> Option<IntForm>;
    /// Whether the op at `pc` hands its destination a value exact where it comes from: a function's exact result,
    /// an exact global.
    fn arrives_exact(&self, pc: usize) -> bool;
    /// The pcs control passes to from the op at `pc`, within the region and around its holes.
    fn successors(&self, pc: usize) -> Vec<usize>;
}

/// One region of bytecode: `[start, end)` less its `holes` (the bodies of the closures defined in it), the body of
/// `func` (or Main), whose registers hold `seed` on entry.
pub(crate) struct Region<'r> {
    pub(crate) start: usize,
    pub(crate) end: usize,
    pub(crate) holes: &'r [(usize, usize)],
    pub(crate) func: Option<usize>,
    pub(crate) seed: &'r [Option<IntForm>],
}

/// What exactness decides for one region.
#[derive(Debug, Default)]
pub(crate) struct Exactness {
    /// The ops (by pc) whose integer result is held exact.
    pub(crate) defs: BTreeSet<usize>,
    /// Those of them the region runs on machine words until a value outgrows one — every exact op but those computed
    /// on the exact tower itself (a function value's body): the region checks each one that can overflow, and each
    /// exact value that arrives, and moves into the region held exact at the first that does not fit.
    pub(crate) speculative: BTreeSet<usize>,
    /// The parameters an exact variable starts from: machine words on entry, exact in the region held exact.
    pub(crate) entry_speculative: BTreeSet<u16>,
    /// The registers an op that writes several leaves an exact integer in, by pc: a call's argument register a
    /// `mutable` parameter hands an exact value back through, a side of a swap that takes one.
    pub(crate) outs: BTreeSet<(usize, u16)>,
}

/// The `mutable` parameters of function `fi` (registers below its parameter count): the argument registers the VM
/// keeps on return, for the caller to copy back.
pub(crate) fn inout_regs(prog: &CompiledProgram, fi: usize) -> Vec<u16> {
    prog.functions.get(fi).map_or_else(Vec::new, |f| {
        f.mutable_param_regs
            .iter()
            .copied()
            .filter(|p| *p < f.param_count)
            .collect()
    })
}

/// The registers `op` writes: its own, and for a call each argument register a `mutable` parameter leaves its final
/// value in.
pub(crate) fn for_each_written(prog: &CompiledProgram, op: &Op, mut f: impl FnMut(u16)) {
    op.for_each_written_reg(&mut f);
    if let Op::Call {
        func, args_start, ..
    } = op
    {
        for p in inout_regs(prog, *func as usize) {
            f(args_start + p);
        }
    }
}

fn in_hole(pc: usize, holes: &[(usize, usize)]) -> bool {
    holes.iter().any(|&(s, e)| pc >= s && pc < e)
}

/// Where a definition's value is made: on entry to the region (a parameter), by the op at a pc, or in one of the
/// registers an op that writes several leaves a value in — a call's argument register a `mutable` parameter hands
/// its final value back through, either side of a swap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Site {
    Entry(u16),
    Op(usize),
    Out(usize, u16),
}

/// How the op at `site` makes its value, as the shared decision reads it.
fn operation<H: ExactnessHost + ?Sized>(host: &H, site: Site) -> Operation {
    let Site::Op(pc) = site else {
        return Operation::Opaque;
    };
    match *host.op(pc) {
        Op::LoadConst { idx, .. } => match host.program().constants[idx as usize] {
            Constant::Int(v) | Constant::Duration(v) => Operation::Constant(v as i128),
            _ => Operation::Opaque,
        },
        Op::Move { .. } => Operation::Copy,
        Op::Add { lhs, rhs, .. } => Operation::Sum {
            doubles: lhs == rhs,
        },
        Op::Sub { .. } => Operation::Sum { doubles: false },
        Op::AddAssign { dst, src } => Operation::Sum {
            doubles: dst == src,
        },
        Op::Mul { .. } => Operation::Product,
        Op::Pow { .. } => Operation::Power,
        Op::Div { .. }
        | Op::FloorDiv { .. }
        | Op::DivPow2 { .. }
        | Op::MagicDivU { mul_back: 0, .. } => Operation::Quotient,
        Op::Mod { .. } => Operation::Remainder,
        Op::MagicDivU { divisor, .. } => Operation::Bounded(divisor as i128),
        Op::BitXor { .. }
        | Op::BitOr { .. }
        | Op::BitAnd { .. }
        | Op::Shl { .. }
        | Op::Shr { .. } => Operation::Bitwise,
        _ => Operation::Opaque,
    }
}

/// Decide which integer definitions of `region` are exact, from the host's typing on machine words (only what an
/// exact tower computes is exact in it) and the region's entry forms (a parameter an exact value arrives in is exact):
/// the region's definitions and what each is computed from, asked of the shared decision
/// ([`crate::analysis::exactness`]).
pub(crate) fn exactness<H: ExactnessHost + ?Sized>(host: &H, region: &Region) -> Exactness {
    let prog = host.program();
    let (start, end, holes, func, seed) = (
        region.start,
        region.end,
        region.holes,
        region.func,
        region.seed,
    );

    let mut sites: Vec<Site> = Vec::new();
    // What the exact tower computes is exact from the start; what arrives exact is exact until it is checked.
    let mut computed_exact: Vec<bool> = Vec::new();
    let mut arrives_exact: Vec<bool> = Vec::new();
    let mut entry_index: HashMap<u16, usize> = HashMap::new();
    let mut op_index: HashMap<usize, usize> = HashMap::new();
    let mut out_index: HashMap<usize, Vec<(u16, usize)>> = HashMap::new();
    for (reg, form) in seed.iter().enumerate() {
        if let Some(form) = form {
            entry_index.insert(reg as u16, sites.len());
            sites.push(Site::Entry(reg as u16));
            computed_exact.push(false);
            arrives_exact.push(*form == IntForm::Exact);
        }
    }
    for pc in start..end {
        if in_hole(pc, holes) {
            continue;
        }
        let mut written = Vec::new();
        host.op(pc).for_each_written_reg(|w| written.push(w));
        if let [w] = written[..] {
            if let Some(form) = host.int_after(pc, w) {
                op_index.insert(pc, sites.len());
                sites.push(Site::Op(pc));
                computed_exact.push(form == IntForm::Exact);
                arrives_exact.push(host.arrives_exact(pc));
            }
        }
        // What a callee hands back through a `mutable` parameter arrives in its argument register.
        if let Op::Call {
            func: callee,
            args_start,
            ..
        } = *host.op(pc)
        {
            for p in inout_regs(prog, callee as usize) {
                let reg = args_start + p;
                if let Some(form) = host.int_after(pc, reg) {
                    out_index.entry(pc).or_default().push((reg, sites.len()));
                    sites.push(Site::Out(pc, reg));
                    computed_exact.push(form == IntForm::Exact);
                    arrives_exact.push(form == IntForm::Exact);
                }
            }
        }
        // A swap leaves each side the other's value.
        if let Op::RevSwap { a, b } = *host.op(pc) {
            for reg in [a, b] {
                if let Some(form) = host.int_after(pc, reg) {
                    out_index.entry(pc).or_default().push((reg, sites.len()));
                    sites.push(Site::Out(pc, reg));
                    computed_exact.push(form == IntForm::Exact);
                    arrives_exact.push(false);
                }
            }
        }
    }
    if sites.is_empty() {
        return Exactness::default();
    }
    let tracked: HashSet<u16> = sites
        .iter()
        .map(|s| match s {
            Site::Entry(reg) | Site::Out(_, reg) => *reg,
            Site::Op(pc) => {
                let mut w = 0;
                host.op(*pc).for_each_written_reg(|r| w = r);
                w
            }
        })
        .collect();

    // Reaching definitions: for each pc, the definitions of each tracked register that reach it.
    let n = end - start;
    let mut reach_in: Vec<HashMap<u16, BTreeSet<usize>>> = vec![HashMap::new(); n];
    for (reg, d) in &entry_index {
        reach_in[0].entry(*reg).or_default().insert(*d);
    }
    let mut work: Vec<usize> = (start..end)
        .rev()
        .filter(|pc| !in_hole(*pc, holes))
        .collect();
    let mut queued: HashSet<usize> = work.iter().copied().collect();
    while let Some(pc) = work.pop() {
        queued.remove(&pc);
        let mut out = reach_in[pc - start].clone();
        for_each_written(prog, host.op(pc), |w| {
            out.remove(&w);
        });
        if let Some(&d) = op_index.get(&pc) {
            let mut w = 0;
            host.op(pc).for_each_written_reg(|r| w = r);
            out.insert(w, BTreeSet::from([d]));
        }
        for &(reg, d) in out_index.get(&pc).into_iter().flatten() {
            out.insert(reg, BTreeSet::from([d]));
        }
        for s in host.successors(pc) {
            let into = &mut reach_in[s - start];
            let mut changed = false;
            for (reg, ds) in &out {
                let slot = into.entry(*reg).or_default();
                for d in ds {
                    changed |= slot.insert(*d);
                }
            }
            if changed && queued.insert(s) {
                work.push(s);
            }
        }
    }
    let reaching = |pc: usize, reg: u16| -> Vec<usize> {
        reach_in[pc - start]
            .get(&reg)
            .map(|ds| ds.iter().copied().collect())
            .unwrap_or_default()
    };

    let operations: Vec<Operation> = sites.iter().map(|site| operation(host, *site)).collect();
    let mut graph = Graph::default();
    for (d, operation) in operations.iter().enumerate() {
        graph.def(*operation, computed_exact[d], arrives_exact[d]);
    }

    // Webs: the definitions reaching one use are one variable.
    for pc in start..end {
        if in_hole(pc, holes) {
            continue;
        }
        let mut read = Vec::new();
        host.op(pc).for_each_read_reg(
            |f| prog.functions[f as usize].captures.len(),
            |r| read.push(r),
        );
        for r in read.into_iter().filter(|r| tracked.contains(r)) {
            let ds = reaching(pc, r);
            for pair in ds.windows(2) {
                graph.join(pair[0], pair[1]);
            }
        }
    }

    // What each definition is computed from.
    let returned: Vec<usize> = (start..end)
        .filter(|pc| !in_hole(*pc, holes))
        .filter_map(|pc| match *host.op(pc) {
            Op::Return { src } => Some(reaching(pc, src)),
            _ => None,
        })
        .flatten()
        .collect();
    // The integer constant the only definition of `reg` reaching here loads, if that is what it is.
    let constant_at = |pc: usize, reg: u16| -> Option<i128> {
        match reaching(pc, reg)[..] {
            [d] => match operations[d] {
                Operation::Constant(v) => Some(v),
                _ => None,
            },
            _ => None,
        }
    };
    let mut masks: Vec<(usize, i128)> = Vec::new();
    for (d, site) in sites.iter().enumerate() {
        let constant_reaching = |r: u16| match *site {
            Site::Op(pc) => constant_at(pc, r),
            _ => None,
        };
        if let Site::Out(pc, reg) = *site {
            if let Op::RevSwap { a, b } = *host.op(pc) {
                let other = if reg == a { b } else { a };
                for from in reaching(pc, other) {
                    graph.input(d, 0, from, Flow::Copy);
                }
            }
            continue;
        }
        let Site::Op(pc) = *site else { continue };
        let operands: Vec<(u16, Flow)> = match *host.op(pc) {
            Op::Mul { lhs, rhs, .. } | Op::Pow { lhs, rhs, .. } => {
                vec![(lhs, Flow::Product), (rhs, Flow::Product)]
            }
            Op::Add { lhs, rhs, .. } | Op::Sub { lhs, rhs, .. } => {
                vec![(lhs, Flow::Sum), (rhs, Flow::Sum)]
            }
            Op::AddAssign { dst, src } => vec![(dst, Flow::Sum), (src, Flow::Sum)],
            Op::Move { src, .. } => vec![(src, Flow::Copy)],
            Op::Div { lhs, .. }
            | Op::FloorDiv { lhs, .. }
            | Op::DivPow2 { lhs, .. }
            | Op::MagicDivU {
                lhs, mul_back: 0, ..
            } => {
                vec![(lhs, Flow::Quotient)]
            }
            Op::Mod { rhs, .. } => vec![(rhs, Flow::Remainder)],
            // A mask by a constant confines its value; any other bitwise combination carries its operands'.
            Op::BitAnd { lhs, rhs, .. } => {
                match [lhs, rhs].into_iter().find_map(|r| constant_reaching(r)) {
                    Some(mask) if mask >= 0 => {
                        masks.push((d, mask));
                        Vec::new()
                    }
                    _ => vec![(lhs, Flow::Copy), (rhs, Flow::Copy)],
                }
            }
            Op::BitXor { lhs, rhs, .. }
            | Op::BitOr { lhs, rhs, .. }
            | Op::Shl { lhs, rhs, .. }
            | Op::Shr { lhs, rhs, .. } => {
                vec![(lhs, Flow::Copy), (rhs, Flow::Copy)]
            }
            Op::Call { func: callee, .. } if Some(callee as usize) == func => {
                for &from in &returned {
                    graph.input(d, 0, from, Flow::Returned);
                }
                Vec::new()
            }
            // A global's read is any store to it in the region.
            Op::GlobalGet { idx, .. } => {
                for store in (start..end).filter(|pc| !in_hole(*pc, holes)) {
                    if let Op::GlobalSet { idx: stored, src } = *host.op(store) {
                        if stored == idx {
                            for from in reaching(store, src) {
                                graph.input(d, 0, from, Flow::Stored);
                            }
                        }
                    }
                }
                Vec::new()
            }
            _ => Vec::new(),
        };
        for (slot, (reg, flow)) in operands.into_iter().enumerate() {
            for from in reaching(pc, reg) {
                graph.input(d, slot as u8, from, flow);
            }
        }
    }
    // A `mutable` parameter's value at each return is the parameter on the caller's next lend.
    if let Some(fi) = func {
        for p in inout_regs(prog, fi) {
            let Some(&entry) = entry_index.get(&p) else {
                continue;
            };
            for pc in (start..end).filter(|pc| !in_hole(*pc, holes)) {
                if matches!(host.op(pc), Op::Return { .. } | Op::ReturnNothing) {
                    for from in reaching(pc, p) {
                        graph.input(entry, 0, from, Flow::LentBack);
                    }
                }
            }
        }
    }

    for (d, mask) in masks {
        graph.defs[d].operation = Operation::Bounded(mask);
    }
    let decision = decide(&graph);
    let mut out = Exactness::default();
    for (d, site) in sites.iter().enumerate() {
        if !decision.exact[d] {
            continue;
        }
        match *site {
            Site::Entry(reg) => {
                out.entry_speculative.insert(reg);
            }
            Site::Out(pc, reg) => {
                out.outs.insert((pc, reg));
            }
            Site::Op(pc) => {
                out.defs.insert(pc);
                if !decision.computed[d] {
                    out.speculative.insert(pc);
                }
            }
        }
    }
    out
}
