//! Proven value ranges: the interval every integer tensor of a program holds over every run that meets the program's
//! preconditions — every gather's index inside its table, every attention attending at least one position.
//!
//! The analysis interprets the program's tensor ops over intervals, in program order:
//!
//! - a fill holds its own least and greatest value;
//! - a named row index holds every row its ops admit — the rows of the table a gather or a row write names, the
//!   positions of the cache an attention reads — since it is data the host writes at every step;
//! - a gather reads its table's range, whichever row its index names, so a step's range holds for every token and
//!   position the host writes;
//! - a table a row write updates holds its fill's range joined with every row ever written into it. The walk repeats
//!   until no interval widens, so a cache carried from step to step is closed over its writes;
//! - an integer op the analysis does not model writes a value bounded by nothing, the whole of i64 — never by the zero
//!   its declaration starts it at.
//!
//! Float tensors are not ranged here.
//!
//! [`program_ranges`] carries the intervals through a scheduled [`SpatialProgram`]: from each tensor's declared range —
//! which the whole-program front end sets from the intervals above — through every stage, to the interval each buffer's
//! pages hold, each tensor holds, each node of each integer body computes and each stage keeps in its accumulator and
//! folds. A fabric sizes its memories and its datapath by them.

use std::collections::{BTreeMap, BTreeSet};

use crate::spatial::ir::{
    AccPolicy, ComputeOp, IterKind, ReducePool, SpatialProgram, StageKind, TensorBody, TensorRole,
};
use crate::spatial::passes::recognize_stages::{RecognizedProgram, TensorOpSpec};
use crate::spatial::target::precision_int_bits;
use crate::tensor_recognize::qwen::RowIndex;
use crate::tensor_recognize::{ElementwiseOp, Fill};

/// `[lo, hi]`, inclusive.
pub type Interval = (i64, i64);

/// Every value an i64 holds — the bound of a value no modeled op computes.
pub const UNBOUNDED: Interval = (i64::MIN, i64::MAX);

/// The walks after which a table that still widens is bounded by nothing.
const MAX_WALKS: usize = 32;

fn join(a: Interval, b: Interval) -> Interval {
    (a.0.min(b.0), a.1.max(b.1))
}

/// `[lo, hi]` as an i64 interval, or [`UNBOUNDED`] past i64.
fn fit(lo: i128, hi: i128) -> Interval {
    if lo < i64::MIN as i128 || hi > i64::MAX as i128 {
        UNBOUNDED
    } else {
        (lo as i64, hi as i64)
    }
}

/// The interval of an integer fill over `len` elements.
pub fn fill_range(fill: &Fill, len: usize) -> Option<Interval> {
    match fill {
        Fill::Const(v) => Some((*v, *v)),
        Fill::Pattern { modulus, shift } => {
            let last = (len.max(1) as i64 - 1).min(modulus - 1);
            Some((*shift, last + shift))
        }
        // The base fill holds only where no override lands.
        Fill::Overrides { base, sets } => {
            let written: BTreeSet<usize> = sets.iter().map(|(i, _)| *i).filter(|i| *i < len).collect();
            let set = sets.iter().map(|(_, v)| (*v, *v)).reduce(join);
            if len > 0 && written.len() == len {
                return set;
            }
            let base = fill_range(base, len)?;
            Some(set.map_or(base, |s| join(base, s)))
        }
        Fill::ConstF(_) | Fill::FloatOverrides { .. } | Fill::RealPattern { .. } | Fill::Unknown => None,
    }
}

/// The products of two intervals' ends, least and greatest.
fn product(a: Interval, b: Interval) -> (i128, i128) {
    let c = [
        a.0 as i128 * b.0 as i128,
        a.0 as i128 * b.1 as i128,
        a.1 as i128 * b.0 as i128,
        a.1 as i128 * b.1 as i128,
    ];
    (*c.iter().min().unwrap(), *c.iter().max().unwrap())
}

/// The first element of a declared tensor no op writes — its fill is its content.
fn first_of(rp: &RecognizedProgram, name: &str) -> Option<i64> {
    fn first(fill: &Fill) -> Option<i64> {
        match fill {
            Fill::Const(v) => Some(*v),
            Fill::Pattern { shift, .. } => Some(*shift),
            Fill::Overrides { base, sets } => sets.iter().rev().find(|(i, _)| *i == 0).map(|(_, v)| *v).or_else(|| first(base)),
            Fill::ConstF(_) | Fill::FloatOverrides { .. } | Fill::RealPattern { .. } | Fill::Unknown => None,
        }
    }
    let written = rp.stages.iter().any(|s| {
        s.out == name || matches!(&s.op, TensorOpSpec::SetRow(w) if rp.name(w.table) == name)
    });
    if written {
        return None;
    }
    rp.declared.iter().find(|d| d.name == name).and_then(|d| first(&d.fill))
}

/// The rows each named index admits: for every op that names it, the 1-based rows of the table it gathers from or
/// writes into, or the positions of the cache it attends over — met across its ops, since a run every op accepts holds
/// a row each of them admits. A superset of each op's precondition: a block from `P` needs `P + count − 1` among the
/// rows as well.
fn index_domains(rp: &RecognizedProgram) -> BTreeMap<String, Interval> {
    let mut domains: BTreeMap<String, Interval> = BTreeMap::new();
    let mut admit = |name: &str, rows: usize| {
        let rows = (1, rows.max(1) as i64);
        let met = domains.get(name).map_or(rows, |d| (d.0.max(rows.0), d.1.min(rows.1)));
        domains.insert(name.to_string(), met);
    };
    let named = |index: Option<RowIndex>| match index {
        Some(RowIndex::Named(s)) => Some(rp.name(s).to_string()),
        _ => None,
    };
    for st in &rp.stages {
        let (index, rows) = match &st.op {
            TensorOpSpec::RowGather(g) => (named(Some(g.index)), g.rows),
            TensorOpSpec::RowsGather(g) => (Some(rp.name(g.indices).to_string()), g.rows),
            TensorOpSpec::SetRow(w) => (named(Some(w.index)), w.rows),
            TensorOpSpec::Attention(a) => (named(a.pos), a.l),
            TensorOpSpec::LutAttention(a) => (named(a.pos), a.l),
            TensorOpSpec::Rope(r) => (named(r.from.map(|(p, _)| p)), r.from.map_or(0, |(_, n)| n)),
            _ => continue,
        };
        if let Some(name) = index {
            admit(&name, rows);
        }
    }
    domains
}

/// The interval an integer op writes, from its operands' — `None` where it does not model the op.
fn op_range(op: &TensorOpSpec, rp: &RecognizedProgram, r: &BTreeMap<String, Interval>) -> Option<Interval> {
    let of = |s| r.get(rp.name(s)).copied();
    match op {
        TensorOpSpec::Matmul(m) => {
            let (lo, hi) = product(of(m.a)?, of(m.b)?);
            Some(fit(lo * m.k as i128, hi * m.k as i128))
        }
        TensorOpSpec::Elementwise(e) => {
            let (a, b) = (of(e.a)?, of(e.b)?);
            Some(match e.op {
                ElementwiseOp::Add => fit(a.0 as i128 + b.0 as i128, a.1 as i128 + b.1 as i128),
                ElementwiseOp::Mul => {
                    let (lo, hi) = product(a, b);
                    fit(lo, hi)
                }
            })
        }
        // `(x · m) >> s` is monotone in `x`: increasing for `m ≥ 0`, decreasing below.
        TensorOpSpec::IntRequant(q) => {
            let x = of(q.x)?;
            let f = |v: i64| (v as i128 * q.m as i128) >> q.s;
            let (a, b) = (f(x.0), f(x.1));
            Some(fit(a.min(b), a.max(b)))
        }
        TensorOpSpec::IntClamp(c) => {
            let x = of(c.x)?;
            let clamp = |v: i64| v.max(c.lo).min(c.hi);
            Some((clamp(x.0), clamp(x.1)))
        }
        TensorOpSpec::Relu(u) if u.int => {
            let x = of(u.x)?;
            Some((x.0.max(0), x.1.max(0)))
        }
        TensorOpSpec::Transpose(t) => of(t.x),
        TensorOpSpec::RowGather(g) => of(g.table),
        TensorOpSpec::RowsGather(g) => of(g.table),
        TensorOpSpec::ArgMax(a) => Some((1, a.n as i64)),
        // The attended position with the largest score is at distance zero, so it weighs the table's first entry; a
        // first entry of at least one and no negative entry make the divisor positive, and the context — a mean of the
        // values under weights none of which is negative, floored — lies among the values.
        TensorOpSpec::LutAttention(a) => {
            let table = of(a.table)?;
            let first = first_of(rp, rp.name(a.table))?;
            (table.0 >= 0 && first >= 1).then(|| of(a.vc)).flatten()
        }
        _ => None,
    }
}

/// The proven interval of every integer tensor `rp` declares or computes, by name. `float` names the program's float
/// tensors, which are not ranged.
pub fn value_ranges(rp: &RecognizedProgram, float: &BTreeSet<String>) -> BTreeMap<String, Interval> {
    let domains = index_domains(rp);
    let mut fills: BTreeMap<String, Interval> = BTreeMap::new();
    for d in &rp.declared {
        if let Some(r) = fill_range(&d.fill, d.len) {
            let held = domains.get(&d.name).map_or(r, |rows| join(r, *rows));
            fills.insert(d.name.clone(), held);
        }
    }
    let mut ranges = fills.clone();
    for _ in 0..MAX_WALKS {
        let before = ranges.clone();
        for st in &rp.stages {
            if let TensorOpSpec::SetRow(w) = &st.op {
                let (table, source) = (rp.name(w.table).to_string(), rp.name(w.source));
                let written = ranges.get(source).copied().unwrap_or(UNBOUNDED);
                let held = ranges.get(&table).copied().map_or(written, |t| join(t, written));
                ranges.insert(table, held);
                continue;
            }
            if float.contains(&st.out) {
                continue;
            }
            let wrote = op_range(&st.op, rp, &ranges).unwrap_or(UNBOUNDED);
            let held = ranges.get(&st.out).copied().map_or(wrote, |r| join(r, wrote));
            ranges.insert(st.out.clone(), held);
        }
        if ranges == before {
            return ranges;
        }
    }
    // A table that still widens after every walk: every value an op writes is bounded by nothing.
    for st in &rp.stages {
        if !float.contains(&st.out) {
            ranges.insert(st.out.clone(), UNBOUNDED);
        }
        if let TensorOpSpec::SetRow(w) = &st.op {
            ranges.insert(rp.name(w.table).to_string(), UNBOUNDED);
        }
    }
    ranges
}

// ── Over the choreography ─────────────────────────────────────────────────────────────────────────────────────────

/// The proven intervals of a scheduled program, each `None` where nothing is ever held: a buffer no stage fills, a
/// stage with no arithmetic of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProgramRanges {
    /// By tensor index: every element the tensor holds over the run.
    pub tensors: Vec<Option<Interval>>,
    /// By buffer index: every element of every page the buffer holds.
    pub buffers: Vec<Option<Interval>>,
    /// By stage index.
    pub stages: Vec<StageRanges>,
}

/// One stage's arithmetic.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StageRanges {
    /// The value of each node of an integer Generic body, in the order [`TensorBody::post_order`] walks it, at every
    /// point of the iteration space — both arms of a select, as hardware evaluates them; a requant's product before
    /// its shift; a bias's sum. Empty for a stage with neither.
    pub nodes: Vec<Interval>,
    /// Every value the stage keeps in its accumulator over its run: a contraction's and a fold's partial results, a
    /// matmul's partial sums, the values it writes.
    pub held: Option<Interval>,
}

/// The signed two's-complement width that holds every value of `iv`.
pub fn signed_bits(iv: Interval) -> u32 {
    let need = |v: i64| if v >= 0 { 65 - v.leading_zeros() } else { 65 - (!v).leading_zeros() };
    need(iv.0).max(need(iv.1)).clamp(1, 64)
}

/// The integers a precision holds — the whole of i64 for a float or an unbounded width.
fn precision_interval(dtype: crate::codegen_sva::fabric_target::Precision) -> Option<Interval> {
    precision_int_bits(dtype).map(|b| if b >= 64 { UNBOUNDED } else { (-(1i64 << (b - 1)), (1i64 << (b - 1)) - 1) })
}

/// `iv` after a store into `bits` bits: itself when it fits, the width's every value when it may wrap.
fn wrapped(iv: Interval, bits: u32) -> Interval {
    if bits >= 64 {
        return iv;
    }
    let (lo, hi) = (-(1i64 << (bits - 1)), (1i64 << (bits - 1)) - 1);
    if iv.0 >= lo && iv.1 <= hi {
        iv
    } else {
        (lo, hi)
    }
}

fn join_into(slot: &mut Option<Interval>, iv: Interval) -> bool {
    let next = slot.map_or(iv, |h| join(h, iv));
    let changed = *slot != Some(next);
    *slot = Some(next);
    changed
}

/// The interval of `⌊a / b⌋` over every divisor `b` holds but zero, at which the division faults and yields nothing.
/// Over each sign of the divisor the quotient is monotone in each operand, so its extremes lie at the box's corners.
fn floor_div(a: Interval, b: Interval) -> Interval {
    let mut out: Option<Interval> = None;
    for (d0, d1) in [(b.0.max(1), b.1), (b.0, b.1.min(-1))] {
        if d0 > d1 {
            continue;
        }
        for x in [a.0 as i128, a.1 as i128] {
            for d in [d0 as i128, d1 as i128] {
                let q = floor_quotient(x, d);
                let iv = fit(q, q);
                out = Some(out.map_or(iv, |o| join(o, iv)));
            }
        }
    }
    out.unwrap_or((0, 0))
}

/// `⌊x / d⌋` toward −∞ for a nonzero `d`, exactly.
fn floor_quotient(x: i128, d: i128) -> i128 {
    let q = x / d;
    if x % d != 0 && ((x < 0) != (d < 0)) {
        q - 1
    } else {
        q
    }
}

/// The interval of every node of `b` over operands each lying in `operand(i)`, pushed in post order; the body's own.
fn body_nodes(b: &TensorBody, operand: &dyn Fn(usize) -> Interval, nodes: &mut Vec<Interval>) -> Interval {
    let iv = match b {
        TensorBody::Operand(i) => operand(*i),
        TensorBody::Const(bits) => {
            let v = f64::from_bits(*bits) as i64;
            (v, v)
        }
        TensorBody::Add(x, y) => {
            let (a, c) = (body_nodes(x, operand, nodes), body_nodes(y, operand, nodes));
            fit(a.0 as i128 + c.0 as i128, a.1 as i128 + c.1 as i128)
        }
        TensorBody::Sub(x, y) => {
            let (a, c) = (body_nodes(x, operand, nodes), body_nodes(y, operand, nodes));
            fit(a.0 as i128 - c.1 as i128, a.1 as i128 - c.0 as i128)
        }
        TensorBody::Mul(x, y) => {
            let (a, c) = (body_nodes(x, operand, nodes), body_nodes(y, operand, nodes));
            let (lo, hi) = product(a, c);
            fit(lo, hi)
        }
        TensorBody::Min(x, y) => {
            let (a, c) = (body_nodes(x, operand, nodes), body_nodes(y, operand, nodes));
            (a.0.min(c.0), a.1.min(c.1))
        }
        TensorBody::Max(x, y) => {
            let (a, c) = (body_nodes(x, operand, nodes), body_nodes(y, operand, nodes));
            (a.0.max(c.0), a.1.max(c.1))
        }
        TensorBody::FloorDiv(x, y) => {
            let (a, c) = (body_nodes(x, operand, nodes), body_nodes(y, operand, nodes));
            floor_div(a, c)
        }
        TensorBody::Shr { arg, by } => {
            let a = body_nodes(arg, operand, nodes);
            (a.0 >> by, a.1 >> by)
        }
        TensorBody::Unary { arg, .. } => {
            body_nodes(arg, operand, nodes);
            UNBOUNDED
        }
        TensorBody::Select { then, otherwise, .. } => {
            let (t, o) = (body_nodes(then, operand, nodes), body_nodes(otherwise, operand, nodes));
            join(t, o)
        }
    };
    nodes.push(iv);
    iv
}

/// `(x · m) >> s`, an arithmetic shift, over `x`: the product's interval and the result's. The shift is monotone, so the
/// product's ends give the result's.
fn requant(x: Interval, m: i64, s: u32) -> (Interval, Interval) {
    let (lo, hi) = product(x, (m, m));
    let prod = fit(lo, hi);
    let out = if prod == UNBOUNDED { (i64::MIN >> s, i64::MAX >> s) } else { (prod.0 >> s, prod.1 >> s) };
    (prod, out)
}

/// `max(x, 0)` over `x`.
fn relu(x: Interval) -> Interval {
    (x.0.max(0), x.1.max(0))
}

/// The sum of `n` values each in `iv`, and every partial sum on the way: the extremes of `j · iv` for `1 ≤ j ≤ n`.
fn sums(iv: Interval, n: usize) -> (Interval, Interval) {
    let n = n.max(1) as i128;
    let total = fit(n * iv.0 as i128, n * iv.1 as i128);
    (total, join(iv, total))
}

/// A fold of `n` values in `iv` by `pool`: its result, and every value its register holds on the way.
fn fold(pool: ReducePool, iv: Interval, n: usize) -> (Interval, Interval) {
    match pool {
        ReducePool::Sum => sums(iv, n),
        ReducePool::Max => (iv, iv),
        ReducePool::ArgMax => ((0, n.max(1) as i64 - 1), iv),
    }
}

/// One stage's write and its arithmetic, from the intervals its input buffers hold — `None` where the stage writes
/// nothing the analysis models.
fn stage_effect(p: &SpatialProgram, si: usize, ins: &[Option<Interval>]) -> (Option<Interval>, StageRanges) {
    let stage = &p.stages[si];
    let t = p.tile.cols.max(1);
    let trunc32 = p.plan_of_stage(stage.id).is_some_and(|pl| pl.acc == AccPolicy::I32);
    let packed = |iv: Interval| if trunc32 { wrapped(iv, 32) } else { iv };
    let any = |iv: Option<Interval>| iv.unwrap_or(UNBOUNDED);
    let op = match &stage.kind {
        StageKind::Compute(op) => op,
        StageKind::AllReduce { combine, .. } => combine.as_ref(),
        StageKind::Scalar(_) => return (Some(UNBOUNDED), StageRanges { nodes: Vec::new(), held: Some(UNBOUNDED) }),
        _ => return (None, StageRanges::default()),
    };
    match op {
        ComputeOp::MatmulTiles { block, num_blocks, operand, .. } => {
            let bits = precision_int_bits(*operand).unwrap_or(64);
            let (a, b) = (wrapped(any(ins.first().copied().flatten()), bits), wrapped(any(ins.get(1).copied().flatten()), bits));
            let (lo, hi) = product(a, b);
            let term = fit(lo.min(0), hi.max(0));
            let k = num_blocks.max(&1) * block.ibw.max(1) * t;
            let (total, _) = sums(term, k);
            let out = packed(total);
            (Some(join(out, (0, 0))), StageRanges { nodes: Vec::new(), held: Some(join(total, (0, 0))) })
        }
        ComputeOp::Requant { m, s } => {
            let (prod, out) = requant(any(ins.first().copied().flatten()), *m, *s);
            let out = packed(out);
            (Some(out), StageRanges { nodes: vec![prod], held: Some(out) })
        }
        ComputeOp::Bias { .. } => {
            let (x, b) = (any(ins.first().copied().flatten()), any(ins.get(1).copied().flatten()));
            let sum = fit(x.0 as i128 + b.0 as i128, x.1 as i128 + b.1 as i128);
            let out = packed(sum);
            (Some(out), StageRanges { nodes: vec![sum], held: Some(out) })
        }
        ComputeOp::Relu => {
            let out = packed(relu(any(ins.first().copied().flatten())));
            (Some(out), StageRanges { nodes: Vec::new(), held: Some(out) })
        }
        ComputeOp::Scalar(_) => (Some(UNBOUNDED), StageRanges { nodes: Vec::new(), held: Some(UNBOUNDED) }),
        ComputeOp::Generic { iters, body, .. } => {
            let integral = body.integral() && stage.ins.iter().all(|b| precision_int_bits(p.buffer(*b).dtype).is_some());
            if !integral {
                return (Some(UNBOUNDED), StageRanges { nodes: Vec::new(), held: Some(UNBOUNDED) });
            }
            // An offset outside its operand's page reads zero.
            let operand = |i: usize| join(any(ins.get(i).copied().flatten()), (0, 0));
            let mut nodes = Vec::new();
            let v = body_nodes(body, &operand, &mut nodes);
            let (row, col, k) = match iters.as_slice() {
                [r, c] => (*r, *c, None),
                [r, c, k] => (*r, *c, Some(*k)),
                _ => return (Some(UNBOUNDED), StageRanges { nodes, held: Some(UNBOUNDED) }),
            };
            let (value, mut held) = match k {
                Some(k) => sums(v, k.extent),
                None => (v, v),
            };
            let pool = |kind: IterKind| match kind {
                IterKind::Reduce { pool } => Some(pool),
                IterKind::Parallel => None,
            };
            let result = match (pool(row.kind), pool(col.kind)) {
                (None, None) => value,
                (None, Some(p)) => {
                    let (r, h) = fold(p, value, col.extent);
                    held = join(held, h);
                    r
                }
                (Some(p), None) => {
                    let (r, h) = fold(p, value, row.extent);
                    held = join(held, h);
                    r
                }
                (Some(rp), Some(cp)) => {
                    let (inner, h) = fold(cp, value, col.extent);
                    let (outer, ho) = fold(rp, inner, row.extent);
                    held = join(join(held, h), join(inner, ho));
                    outer
                }
            };
            // The plane starts at zero: a point the space does not reach is packed as zero.
            let out = packed(join(result, (0, 0)));
            (Some(out), StageRanges { nodes, held: Some(join(held, out)) })
        }
    }
}

/// The intervals of `p`'s integer storage and arithmetic over every run whose tensors hold their declared ranges.
///
/// A tensor starts at its declared range where it has one; an `Input`, `Param` or `State` without one at every value
/// its precision holds, since its contents may come from outside the program; an activation or a result empty, as a
/// run starts it — a section reading it before its producer stored it faults. Every stage then carries
/// its inputs' intervals to its outputs: a load reads its tensor, a receive its sender's page, a store joins its page
/// into its tensor, and a compute stage writes what `stage_effect` proves. The walk repeats until no interval widens; a
/// buffer still widening after every walk holds every value of i64. A tensor with a declared range keeps the narrower of
/// it and what its stores prove — both hold every value it takes — and holds it as its precision stores it: an interval
/// past the precision wraps to every value the precision holds. A load whose tensor ends inside a tile holds the tile's
/// padding zeros as well.
pub fn program_ranges(p: &SpatialProgram) -> ProgramRanges {
    let declared = |i: usize| -> Option<Interval> {
        let t = &p.tensors[i];
        let width = precision_interval(t.dtype)?;
        if let Some(r) = t.range {
            return Some(r);
        }
        match t.role {
            TensorRole::Input | TensorRole::Param | TensorRole::State => Some(width),
            TensorRole::Activation | TensorRole::Output => None,
        }
    };
    let start: Vec<Option<Interval>> = (0..p.tensors.len()).map(declared).collect();
    let mut stored: Vec<Option<Interval>> = vec![None; p.tensors.len()];
    let mut buffers: Vec<Option<Interval>> = vec![None; p.buffers.len()];
    let mut stages: Vec<StageRanges> = vec![StageRanges::default(); p.stages.len()];
    let sender = |g| {
        p.stages.iter().find_map(|s| match &s.kind {
            StageKind::Load { multicast: Some(m), .. } if *m == g => s.outs.first().copied(),
            _ => None,
        })
    };
    // The interval a read of tensor `i` sees: its start joined with every store, kept within a declared range, as its
    // precision stores it — a value past the precision wraps.
    let held = |start: &[Option<Interval>], stored: &[Option<Interval>], i: usize| -> Option<Interval> {
        let both = match (start[i], stored[i]) {
            (Some(a), Some(b)) => Some(join(a, b)),
            (a, b) => a.or(b),
        };
        let kept = match (p.tensors[i].range, both) {
            (Some(r), Some(b)) if r.0.max(b.0) <= r.1.min(b.1) => Some((r.0.max(b.0), r.1.min(b.1))),
            (_, b) => b,
        };
        let bits = precision_int_bits(p.tensors[i].dtype);
        kept.map(|iv| bits.map_or(iv, |b| wrapped(iv, b)))
    };
    let walks = 64;
    for walk in 0..=walks {
        let mut changed = false;
        for (si, st) in p.stages.iter().enumerate() {
            let ins: Vec<Option<Interval>> = st.ins.iter().map(|b| buffers[b.idx()]).collect();
            let wrote: Option<Interval> = match &st.kind {
                StageKind::Load { tensor, .. } => {
                    let t = tensor.idx();
                    let td = &p.tensors[t];
                    // A tile past the tensor's edge is padded with zeros.
                    let padded = td.tiles.rows_t * p.tile.rows > td.elems.rows || td.tiles.cols_t * p.tile.cols > td.elems.cols;
                    held(&start, &stored, t).map(|iv| if padded { join(iv, (0, 0)) } else { iv })
                }
                StageKind::Recv { group } => sender(*group).and_then(|b| buffers[b.idx()]),
                StageKind::Store { tensor, .. } => {
                    if let Some(iv) = ins.first().copied().flatten() {
                        changed |= join_into(&mut stored[tensor.idx()], iv);
                    }
                    continue;
                }
                StageKind::Compute(_) | StageKind::AllReduce { .. } | StageKind::Scalar(_) => {
                    let (out, ranges) = stage_effect(p, si, &ins);
                    if stages[si] != ranges {
                        let merged = StageRanges {
                            nodes: if stages[si].nodes.len() == ranges.nodes.len() {
                                stages[si].nodes.iter().zip(&ranges.nodes).map(|(a, b)| join(*a, *b)).collect()
                            } else {
                                ranges.nodes.clone()
                            },
                            held: match (stages[si].held, ranges.held) {
                                (Some(a), Some(b)) => Some(join(a, b)),
                                (a, b) => a.or(b),
                            },
                        };
                        changed |= stages[si] != merged;
                        stages[si] = merged;
                    }
                    out
                }
            };
            if let Some(iv) = wrote {
                for b in &st.outs {
                    changed |= join_into(&mut buffers[b.idx()], iv);
                }
                if let StageKind::Compute(ComputeOp::MatmulTiles { interm: Some(b), .. }) = &st.kind {
                    changed |= join_into(&mut buffers[b.idx()], iv);
                }
            }
        }
        if !changed {
            break;
        }
        if walk == walks {
            // Still widening: every value held anywhere is bounded by nothing.
            for b in buffers.iter_mut().filter(|b| b.is_some()) {
                *b = Some(UNBOUNDED);
            }
            for t in stored.iter_mut().filter(|t| t.is_some()) {
                *t = Some(UNBOUNDED);
            }
            for s in &mut stages {
                s.nodes.iter_mut().for_each(|n| *n = UNBOUNDED);
                s.held = s.held.map(|_| UNBOUNDED);
            }
        }
    }
    let tensors = (0..p.tensors.len()).map(|i| held(&start, &stored, i)).collect();
    ProgramRanges { tensors, buffers, stages }
}

#[cfg(test)]
#[path = "../../../logicaffeine_tests/unit/logicaffeine_compile/spatial/value_range/tests.rs"]
mod tests;
