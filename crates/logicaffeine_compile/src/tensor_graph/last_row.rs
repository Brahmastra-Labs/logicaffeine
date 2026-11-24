//! A prefill chunk's last layer on its output row (PARITY-LAST-LAYER-ROWS).
//!
//! The head reads one row of a chunk of `T`: the last (`row T of X`). Row-level liveness from that read finds what
//! each tensor of the chunk's rows is read at — no row, its last row, or every row — walking the ops backward: an op
//! that computes each row of its result from that row of its operands alone (`fuse::row_wise`, an attention's queries
//! among them) needs its operands at the rows its result is needed at; a gather of a row needs that row; a row written
//! into a table is needed where that row of the table is; anything else needs all it reads, and every carried state is
//! needed whole. [`last_row_tail`] then drops what nothing needs — a mixture's rows before the last, computed a row at a
//! time — and runs each op needed only at the last row on that row alone: its operands from outside read through views
//! of their row `T − 1`, an attention's one query row attending as row `T − 1` of its block (`AttentionSpec::from_row`),
//! a gather of that row the pruned row itself. Past the last layer's key and value cache writes, which every row makes,
//! that is the last layer's attention, output projection, norms and FFN.

use std::collections::{BTreeMap, BTreeSet};

use super::front::TensorOpSpec;
use super::fuse::row_wise;
use super::graph::{Access, Dst, Op, Role, Sym, Tensor, TensorGraph};
use crate::tensor_recognize::qwen::{AttentionSpec, GeluSpec, JoinSpec, L2NormSpec, ReluSpec, RowIndex, ScaleSpec, SigmoidSpec, SiluSpec};
use crate::tensor_recognize::{ElementwiseSpec, FusedRowSpec, HeadMatmulSpec, MatmulSpec, SwiGluSpec};

/// The rows of a tensor of the chunk's rows something reads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
enum Need {
    None,
    Last,
    All,
}

/// `g` with its prefill chunk's rows that feed nothing dropped and its ops needed only at the last row run on that
/// row, or `None` where nothing is pruned: no head reading the last row of a chunk of at least two, or no attention
/// among the ops it prunes (a last layer that walks a recurrent memory, whose rows are each needed in order, keeps
/// them all).
pub fn last_row_tail(g: &TensorGraph) -> Option<TensorGraph> {
    // The head's row: the last op that gathers a chunk's last row (a mixture's rows are gathered one by one before it).
    let (gather, table, rows) = g.ops.iter().enumerate().rev().find_map(|(i, op)| match op.prim() {
        Some(TensorOpSpec::RowGather(rg)) if matches!(rg.index, RowIndex::Literal(r) if r as usize == rg.rows) && rg.rows >= 2 => {
            let t = g.tensors.get(&op.ins[0])?;
            (t.role == Role::Activation && t.rows == rg.rows).then(|| (i, op.ins[0].clone(), rg.rows))
        }
        _ => None,
    })?;
    // A tensor of the chunk's rows: an activation, or a table the step assembles a row at a time before reading it (a
    // mixture's rows); not a state the steps carry.
    let chunked = |x: &str| g.tensors.get(x).is_some_and(|t| matches!(t.role, Role::Activation | Role::State) && t.rows == rows && !t.carried);
    let mut need: BTreeMap<String, Need> = BTreeMap::new();
    for (name, t) in &g.tensors {
        if t.carried || t.observed || !matches!(t.role, Role::Activation | Role::State) {
            need.insert(name.clone(), Need::All);
        }
    }
    let raise = |need: &mut BTreeMap<String, Need>, x: &str, n: Need| {
        let e = need.entry(x.to_string()).or_insert(Need::None);
        *e = (*e).max(n);
    };
    let mut pruned: BTreeSet<usize> = BTreeSet::new();
    let mut dead: BTreeSet<usize> = BTreeSet::new();
    // A table needed only at its last row, that row written whole by one row write of a row: the row it writes.
    let mut written: BTreeMap<String, (usize, String)> = BTreeMap::new();
    for (i, op) in g.ops.iter().enumerate().rev() {
        let out = need.get(&op.out).copied().unwrap_or(Need::None);
        let writes_state = !op.state_inputs().is_empty() || op.dst.row.is_some() || op.dst.unit.is_some();
        if i == gather {
            raise(&mut need, &table, Need::Last);
            continue;
        }
        match op.prim() {
            // A row written into a table of the chunk's rows: live where that row of the table is needed.
            Some(TensorOpSpec::SetRow(s)) if chunked(&op.ins[0]) => {
                let span = match s.index {
                    RowIndex::Literal(r) => Some((r as usize - 1)..(r as usize - 1 + s.count)),
                    RowIndex::Named(_) => None,
                };
                let live = match need.get(&op.ins[0]).copied().unwrap_or(Need::None) {
                    Need::All => true,
                    Need::Last => span.as_ref().is_none_or(|w| w.contains(&(rows - 1))),
                    Need::None => false,
                };
                if !live {
                    dead.insert(i);
                    continue;
                }
                let last_only = need.get(&op.ins[0]) == Some(&Need::Last);
                if last_only && s.count == 1 && matches!(s.index, RowIndex::Literal(r) if r as usize == rows) && g.tensors.get(&op.ins[1]).is_some_and(|t| t.rows == 1) {
                    written.insert(op.ins[0].clone(), (i, op.ins[1].clone()));
                }
                for x in &op.ins[1..] {
                    raise(&mut need, x, Need::All);
                }
                continue;
            }
            // A gather of a row of the chunk's rows: that row.
            Some(TensorOpSpec::RowGather(rg)) if chunked(&op.ins[0]) => {
                if out == Need::None {
                    dead.insert(i);
                    continue;
                }
                let last = matches!(rg.index, RowIndex::Literal(r) if r as usize == rows);
                raise(&mut need, &op.ins[0], if last { Need::Last } else { Need::All });
                for x in &op.ins[1..] {
                    raise(&mut need, x, Need::All);
                }
                continue;
            }
            _ => {}
        }
        if out == Need::None && !writes_state {
            dead.insert(i);
            continue;
        }
        let whole_reads = op.ins.iter().enumerate().all(|(k, x)| !chunked(x) || matches!(op.access.get(k), Some(Access::Whole) | None));
        let prunable = out == Need::Last
            && !writes_state
            && row_wise(op)
            && chunked(&op.out)
            && op.dst.own().is_some()
            && whole_reads
            && op.prim().and_then(|s| one_row(s, rows)).is_some();
        if prunable {
            pruned.insert(i);
        }
        let state: BTreeSet<usize> = op.state_inputs().into_iter().collect();
        for (k, x) in op.ins.iter().enumerate() {
            let n = if prunable && chunked(x) && !state.contains(&k) { Need::Last } else { Need::All };
            raise(&mut need, x, n);
        }
    }
    if !pruned.iter().any(|&i| matches!(g.ops[i].prim(), Some(TensorOpSpec::Attention(_)))) {
        return None;
    }
    // A table only its last row of is read, where one write fills that row: its readers read the written row, and the
    // write and the table go.
    let mut row: BTreeMap<String, String> = BTreeMap::new();
    for (table, (w, source)) in &written {
        if g.ops.iter().enumerate().all(|(i, op)| i == *w || !op.ins.contains(table) || pruned.contains(&i) || dead.contains(&i)) {
            dead.insert(*w);
            row.insert(table.clone(), source.clone());
        }
    }
    let mut tensors = g.tensors.clone();
    let fresh = |tensors: &BTreeMap<String, Tensor>, base: &str| -> String {
        let mut name = format!("{base}_last");
        while tensors.contains_key(&name) {
            name.push('_');
        }
        name
    };
    // Each pruned tensor's one row, and each view of an outside operand's last row, by the tensor's name.
    let mut ops: Vec<Op> = Vec::with_capacity(g.ops.len());
    for (i, op) in g.ops.iter().enumerate() {
        if dead.contains(&i) {
            continue;
        }
        let gathers_pruned = matches!(op.prim(), Some(TensorOpSpec::RowGather(rg)) if matches!(rg.index, RowIndex::Literal(r) if r as usize == rows))
            && row.contains_key(&op.ins[0]);
        if gathers_pruned {
            // A gather of the last row of a pruned tensor: that row, in place — the head's standing for every op that
            // goes, each still lowered once.
            let mut covers = op.covers.clone();
            if i == gather {
                covers.extend(dead.iter().flat_map(|&d| g.ops[d].covers.iter().copied()));
            }
            ops.push(Op { sym: Sym::View { offset: 0 }, ins: vec![row[&op.ins[0]].clone()], access: vec![Access::Whole], out: op.out.clone(), dst: op.dst.clone(), keeps: Vec::new(), covers });
            continue;
        }
        if !pruned.contains(&i) {
            ops.push(op.clone());
            continue;
        }
        let mut ins = Vec::with_capacity(op.ins.len());
        for x in &op.ins {
            if let Some(r) = row.get(x) {
                ins.push(r.clone());
                continue;
            }
            if !chunked(x) {
                ins.push(x.clone());
                continue;
            }
            // An operand from outside: its row `T − 1`, in place.
            let t = tensors[x].clone();
            let view = fresh(&tensors, x);
            tensors.insert(view.clone(), Tensor { name: view.clone(), rows: 1, role: Role::Activation, ..t.clone() });
            ops.push(Op { sym: Sym::View { offset: (rows - 1) * t.cols }, ins: vec![x.clone()], access: vec![Access::Whole], out: view.clone(), dst: Dst::to(&view), keeps: Vec::new(), covers: Vec::new() });
            row.insert(x.clone(), view.clone());
            ins.push(view);
        }
        let t = tensors[&op.out].clone();
        let out = fresh(&tensors, &op.out);
        tensors.insert(out.clone(), Tensor { name: out.clone(), rows: 1, ..t });
        row.insert(op.out.clone(), out.clone());
        let keeps = op.keeps.iter().map(|k| row.get(k).cloned().unwrap_or_else(|| k.clone())).collect();
        let spec = one_row(op.prim().expect("a pruned op is a primitive"), rows).expect("a pruned op takes one row");
        ops.push(Op { sym: Sym::Prim(spec), ins, access: op.access.clone(), out: out.clone(), dst: Dst::to(&out), keeps, covers: op.covers.clone() });
    }
    Some(TensorGraph { ops, tensors, ..g.clone() })
}

/// `spec`, an op of a chunk's `rows` rows each from that row of its operands alone, at one row; `None` for one that does
/// not take one row as it stands (a norm weighing each row by its own row of a weight).
fn one_row(spec: &TensorOpSpec, rows: usize) -> Option<TensorOpSpec> {
    let per = |n: usize| (n % rows == 0).then_some(n / rows);
    Some(match spec {
        TensorOpSpec::Matmul(m) if m.m == rows => TensorOpSpec::Matmul(MatmulSpec { m: 1, ..m.clone() }),
        TensorOpSpec::HeadMatmul(h) if h.m == rows => TensorOpSpec::HeadMatmul(HeadMatmulSpec { m: 1, ..h.clone() }),
        TensorOpSpec::Elementwise(e) => TensorOpSpec::Elementwise(ElementwiseSpec { n: per(e.n)?, ..e.clone() }),
        TensorOpSpec::FusedRow(f) if f.rows == rows && f.weight_rows <= 1 => TensorOpSpec::FusedRow(FusedRowSpec { rows: 1, ..f.clone() }),
        TensorOpSpec::SwiGlu(s) => TensorOpSpec::SwiGlu(SwiGluSpec { total: per(s.total)?, ..s.clone() }),
        TensorOpSpec::Silu(s) => TensorOpSpec::Silu(SiluSpec { n: per(s.n)?, ..s.clone() }),
        TensorOpSpec::Sigmoid(s) => TensorOpSpec::Sigmoid(SigmoidSpec { n: per(s.n)?, ..s.clone() }),
        TensorOpSpec::Gelu(s) => TensorOpSpec::Gelu(GeluSpec { n: per(s.n)?, ..s.clone() }),
        TensorOpSpec::Relu(s) => TensorOpSpec::Relu(ReluSpec { n: per(s.n)?, ..s.clone() }),
        TensorOpSpec::Scale(s) => TensorOpSpec::Scale(ScaleSpec { n: per(s.n)?, ..s.clone() }),
        TensorOpSpec::L2Norm(l) if l.rows == rows => TensorOpSpec::L2Norm(L2NormSpec { rows: 1, ..l.clone() }),
        TensorOpSpec::Join(j) if j.rows == rows => TensorOpSpec::Join(JoinSpec { rows: 1, ..j.clone() }),
        TensorOpSpec::Attention(a) if a.rows == rows => TensorOpSpec::Attention(AttentionSpec { rows: 1, from_row: a.from_row + rows - 1, ..a.clone() }),
        _ => return None,
    })
}
