//! What a graph computes, answered without a chip: the evaluator every backend is held to.
//!
//! Each primitive computes what the LOGOS statements its sugar desugars to compute, in the same loop order, with the same
//! operations: Reals in `f64`, `e^x` as `E ** x` (`powf`), `max` and `min` as the language's (`maximumNumber`,
//! `minimumNumber`), Ints exactly (a sum past `i64` is refused by name, where the VM would widen it), integer division
//! truncating and `//` flooring. So a lifted graph evaluates to exactly what the VM prints for its program, bit for bit.
//!
//! A rewritten graph means what its ops say: an operand read [`Strided`](super::graph::Strided) is the part of every
//! unit of the wider tensor it names, a result with a row or unit destination lands in rows or units of the tensor it
//! names, a view is its operand's values from its offset on, a quantize is its operand's own values (the meaning is
//! exact; a chip's quantized arithmetic is its numerics, held to a declared bound), and a fused symbol is its parts, run
//! in order, of which only its result, what it keeps and the state it writes outlive it.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use logicaffeine_base::numeric::{float_max, float_min};

use crate::tensor_recognize::qwen::{GateAct, MoeAct, MoeGroups, MoeScore, RowIndex};
use crate::tensor_recognize::{ElementwiseOp, Fill, RowReduceKind};

use super::front::TensorOpSpec;
use super::graph::{Access, Dst, Elem, Op, RowSel, Sym, TensorGraph};

const E: f64 = std::f64::consts::E;

/// A tensor's values.
#[derive(Clone, Debug, PartialEq)]
pub enum Data {
    Real(Vec<f64>),
    Int(Vec<i64>),
}

impl Data {
    pub fn len(&self) -> usize {
        match self {
            Data::Real(v) => v.len(),
            Data::Int(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The values as Reals: an Int widens as the VM widens it when a Real meets it.
    pub fn reals(&self) -> Vec<f64> {
        match self {
            Data::Real(v) => v.clone(),
            Data::Int(v) => v.iter().map(|&i| i as f64).collect(),
        }
    }

    fn slice(&self, from: usize, to: usize) -> Data {
        match self {
            Data::Real(v) => Data::Real(v[from..to].to_vec()),
            Data::Int(v) => Data::Int(v[from..to].to_vec()),
        }
    }
}

/// Why a graph has no value, by name.
#[derive(Clone, Debug, PartialEq)]
pub enum EvalError {
    /// A tensor an op reads that nothing gives a value: no input, no fill, no op before it.
    Missing { op: usize, tensor: String },
    /// A row, position or index outside what the program declares — the check the program's statements make.
    OutOfRange { op: usize, what: String },
    /// An integer past `i64`, where the VM would widen to an arbitrary-precision Int the graph's tensors do not hold.
    Overflow { op: usize, what: String },
    /// An operand whose element type or length is not the one the op reads.
    Operand { op: usize, tensor: String, why: String },
    /// An integer division by zero.
    DivideByZero { op: usize },
}

impl fmt::Display for EvalError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EvalError::Missing { op, tensor } => write!(f, "op {op} reads `{tensor}`, which nothing gives a value"),
            EvalError::OutOfRange { op, what } => write!(f, "op {op}: {what}"),
            EvalError::Overflow { op, what } => write!(f, "op {op}: {what} is past a 64-bit Int"),
            EvalError::Operand { op, tensor, why } => write!(f, "op {op} reads `{tensor}`: {why}"),
            EvalError::DivideByZero { op } => write!(f, "op {op}: an integer division by zero"),
        }
    }
}

impl std::error::Error for EvalError {}

/// The values a fill gives a tensor of `len` elements, or `None` for one the program leaves to its caller.
pub fn filled(fill: &Fill, len: usize) -> Option<Data> {
    match fill {
        Fill::Const(c) => Some(Data::Int(vec![*c; len])),
        Fill::Pattern { modulus, shift } => Some(Data::Int((0..len as i64).map(|p| p % modulus + shift).collect())),
        Fill::Overrides { base, sets } => {
            let Some(Data::Int(mut v)) = filled(base, len) else { return None };
            for &(i, x) in sets {
                if i < len {
                    v[i] = x;
                }
            }
            Some(Data::Int(v))
        }
        Fill::ConstF(b) => Some(Data::Real(vec![f64::from_bits(*b); len])),
        Fill::FloatOverrides { base, sets } => {
            let mut v = vec![f64::from_bits(*base); len];
            for &(i, x) in sets {
                if i < len {
                    v[i] = f64::from_bits(x);
                }
            }
            Some(Data::Real(v))
        }
        Fill::RealPattern { modulus, shift, divisor } => {
            Some(Data::Real((0..len as i64).map(|p| (p % modulus + shift) as f64 / f64::from_bits(*divisor)).collect()))
        }
        Fill::Unknown => None,
    }
}

/// The values every tensor of `g` holds after its step: each declared tensor from `inputs` where given, else from the
/// fill the program declares it with, then every op in order.
pub fn eval(g: &TensorGraph, inputs: &BTreeMap<String, Data>) -> Result<BTreeMap<String, Data>, EvalError> {
    let mut env: BTreeMap<String, Data> = BTreeMap::new();
    for t in g.tensors.values() {
        if let Some(v) = inputs.get(&t.name) {
            env.insert(t.name.clone(), v.clone());
        } else if let Some(v) = t.fill.as_ref().and_then(|f| filled(f, t.len())) {
            env.insert(t.name.clone(), v);
        }
    }
    for (i, op) in g.ops.iter().enumerate() {
        eval_op(i, op, g, &mut env)?;
    }
    Ok(env)
}

/// Run a fused op's parts in order against `env`, a member a rule built running its own parts in turn.
fn run_parts(at: usize, op: &Op, g: &TensorGraph, env: &mut BTreeMap<String, Data>) -> Result<(), EvalError> {
    let Sym::Fused(f) = &op.sym else { return eval_op(at, op, g, env) };
    for part in &f.parts {
        match part.sym {
            Sym::Fused(_) => run_parts(at, part, g, env)?,
            _ => eval_op(at, part, g, env)?,
        }
    }
    Ok(())
}

/// Run one op against `env`.
fn eval_op(at: usize, op: &Op, g: &TensorGraph, env: &mut BTreeMap<String, Data>) -> Result<(), EvalError> {
    match &op.sym {
        Sym::Fused(_) => {
            let before: BTreeSet<String> = env.keys().cloned().collect();
            run_parts(at, op, g, env)?;
            // Each part lands its result where it says (the part computing the op's result carries the op's placement),
            // and a member a rule built gives its siblings every value it computes; only what the op writes outside
            // itself — its result, what it keeps, the tables and state its parts write — outlives it.
            let outlive: BTreeSet<&str> = op.writes().into_iter().collect();
            let fresh: Vec<String> = env.keys().filter(|k| !before.contains(*k) && !outlive.contains(k.as_str())).cloned().collect();
            for k in fresh {
                env.remove(&k);
            }
            Ok(())
        }
        Sym::View { offset } => {
            let src = read(at, op, 0, env)?;
            let len = g.tensors.get(&op.out).map(|t| t.len()).ok_or_else(|| EvalError::Missing { op: at, tensor: op.out.clone() })?;
            if offset + len > src.len() {
                return Err(EvalError::OutOfRange { op: at, what: format!("a view of {len} values from {offset} into {} values", src.len()) });
            }
            land(at, &op.dst, src.slice(*offset, offset + len), g, env)
        }
        Sym::Quantize(_) => {
            let x = read(at, op, 0, env)?;
            land(at, &op.dst, x, g, env)
        }
        Sym::Prim(spec) => {
            let ins: Vec<Data> = (0..op.ins.len()).map(|k| read(at, op, k, env)).collect::<Result<_, _>>()?;
            let (result, state) = prim(at, spec, &op.ins, &ins, env)?;
            for (k, v) in state {
                env.insert(op.ins[k].clone(), v);
            }
            if let Some(result) = result {
                land(at, &op.dst, result, g, env)?;
            }
            Ok(())
        }
    }
}

/// Operand `k` of `op`, read as its access says.
fn read(at: usize, op: &Op, k: usize, env: &BTreeMap<String, Data>) -> Result<Data, EvalError> {
    let name = &op.ins[k];
    let whole = env.get(name).ok_or_else(|| EvalError::Missing { op: at, tensor: name.clone() })?;
    match op.access.get(k).copied().unwrap_or(Access::Whole) {
        Access::Whole => Ok(whole.clone()),
        Access::Strided(s) => {
            if s.pitch == 0 || s.offset + s.width > s.pitch || whole.len() % s.pitch != 0 {
                return Err(EvalError::Operand {
                    op: at,
                    tensor: name.clone(),
                    why: format!("{} values are no whole units of {} to read {} from {} of", whole.len(), s.pitch, s.width, s.offset),
                });
            }
            let units = whole.len() / s.pitch;
            let pick = |u: usize, i: usize| u * s.pitch + s.offset + i;
            Ok(match whole {
                Data::Real(v) => Data::Real((0..units).flat_map(|u| (0..s.width).map(move |i| v[pick(u, i)])).collect()),
                Data::Int(v) => Data::Int((0..units).flat_map(|u| (0..s.width).map(move |i| v[pick(u, i)])).collect()),
            })
        }
    }
}

/// Store an op's result where its destination says. A destination of rows or units in a tensor nothing has written yet
/// starts from that tensor's zeros: its buffer exists from the first op that lands a part of it.
fn land(at: usize, dst: &Dst, result: Data, g: &TensorGraph, env: &mut BTreeMap<String, Data>) -> Result<(), EvalError> {
    if let Some(name) = dst.own() {
        env.insert(name.to_string(), result);
        return Ok(());
    }
    let table = match env.get(&dst.buf) {
        Some(t) => t.clone(),
        None => match g.tensors.get(&dst.buf) {
            Some(t) if t.elem == Elem::Int => Data::Int(vec![0; t.len()]),
            Some(t) => Data::Real(vec![0.0; t.len()]),
            None => return Err(EvalError::Missing { op: at, tensor: dst.buf.clone() }),
        },
    };
    let n = result.len();
    // Each result value's place in the table.
    let row_of = |sel: &RowSel, env: &BTreeMap<String, Data>| -> Result<i64, EvalError> {
        match sel {
            RowSel::Fixed(r) => Ok(*r as i64),
            RowSel::Param(p) => scalar(at, p, env),
        }
    };
    // Where the result starts in the table: its first row's start, or the table's.
    let base = match &dst.row {
        Some((sel, pitch)) => {
            let row = row_of(sel, env)?;
            if row < 1 {
                return Err(EvalError::OutOfRange { op: at, what: format!("row {row} of `{}`", dst.buf) });
            }
            (row - 1) as usize * pitch
        }
        None => 0,
    };
    // Each result value's place in the table: one after another, or each unit's share in its part of the unit.
    let places: Vec<usize> = match &dst.unit {
        None => (0..n).map(|i| base + i).collect(),
        Some(u) => {
            if u.width == 0 || n % u.width != 0 || u.offset + u.width > u.pitch {
                return Err(EvalError::OutOfRange { op: at, what: format!("{n} values in shares of {} at {} of units of {}", u.width, u.offset, u.pitch) });
            }
            (0..n).map(|i| base + (i / u.width) * u.pitch + u.offset + i % u.width).collect()
        }
    };
    if places.last().is_some_and(|&last| last >= table.len()) {
        return Err(EvalError::OutOfRange { op: at, what: format!("{n} values past the {} of `{}`", table.len(), dst.buf) });
    }
    let merged = match (table, result) {
        (Data::Real(mut t), Data::Real(r)) => {
            for (i, v) in places.into_iter().zip(r) {
                t[i] = v;
            }
            Data::Real(t)
        }
        (Data::Int(mut t), Data::Int(r)) => {
            for (i, v) in places.into_iter().zip(r) {
                t[i] = v;
            }
            Data::Int(t)
        }
        (Data::Int(_), Data::Real(_)) | (Data::Real(_), Data::Int(_)) => {
            return Err(EvalError::Operand { op: at, tensor: dst.buf.clone(), why: "a result of one element type landing in a tensor of another".into() })
        }
    };
    env.insert(dst.buf.clone(), merged);
    Ok(())
}

/// The value of the one-element Int tensor `name`.
fn scalar(at: usize, name: &str, env: &BTreeMap<String, Data>) -> Result<i64, EvalError> {
    match env.get(name) {
        Some(Data::Int(v)) if v.len() == 1 => Ok(v[0]),
        Some(Data::Real(v)) if v.len() == 1 && v[0].fract() == 0.0 => Ok(v[0] as i64),
        Some(_) => Err(EvalError::Operand { op: at, tensor: name.into(), why: "an index that is not one Int".into() }),
        None => Err(EvalError::Missing { op: at, tensor: name.into() }),
    }
}

/// A row index's value: a literal, or the scalar named at operand `k`.
fn index(at: usize, i: &RowIndex, names: &[String], k: usize, env: &BTreeMap<String, Data>) -> Result<i64, EvalError> {
    match i {
        RowIndex::Literal(n) => Ok(*n),
        RowIndex::Named(_) => scalar(at, &names[k], env),
    }
}

fn ints(at: usize, name: &str, d: &Data) -> Result<Vec<i64>, EvalError> {
    match d {
        Data::Int(v) => Ok(v.clone()),
        Data::Real(_) => Err(EvalError::Operand { op: at, tensor: name.into(), why: "Reals where the op reads Ints".into() }),
    }
}

fn need(at: usize, name: &str, d: &Data, len: usize) -> Result<(), EvalError> {
    if d.len() < len {
        return Err(EvalError::Operand { op: at, tensor: name.into(), why: format!("{} values where the op reads {len}", d.len()) });
    }
    Ok(())
}

fn checked(at: usize, what: &str, v: Option<i64>) -> Result<i64, EvalError> {
    v.ok_or_else(|| EvalError::Overflow { op: at, what: what.into() })
}

/// `x / (1 + e^(0 − x))`, as the sugar writes it.
fn silu(x: f64) -> f64 {
    x / (1.0 + E.powf(0.0 - x))
}

/// The result an op computes, and the operands it writes in place (by position) with their new values.
type Computed = (Option<Data>, Vec<(usize, Data)>);

#[allow(clippy::too_many_lines)]
fn prim(at: usize, spec: &TensorOpSpec, names: &[String], ins: &[Data], env: &BTreeMap<String, Data>) -> Result<Computed, EvalError> {
    let real = |k: usize, len: usize| -> Result<Vec<f64>, EvalError> {
        need(at, &names[k], &ins[k], len)?;
        Ok(ins[k].reals())
    };
    let out = |v: Vec<f64>| Ok((Some(Data::Real(v)), Vec::new()));
    match spec {
        TensorOpSpec::Matmul(m) => {
            need(at, &names[0], &ins[0], m.m * m.k)?;
            need(at, &names[1], &ins[1], m.k * m.n)?;
            if let (Data::Int(a), Data::Int(b)) = (&ins[0], &ins[1]) {
                let mut c = vec![0i64; m.m * m.n];
                for i in 0..m.m {
                    for kk in 0..m.k {
                        for j in 0..m.n {
                            let p = checked(at, "a product", a[i * m.k + kk].checked_mul(b[kk * m.n + j]))?;
                            c[i * m.n + j] = checked(at, "a sum", c[i * m.n + j].checked_add(p))?;
                        }
                    }
                }
                return Ok((Some(Data::Int(c)), Vec::new()));
            }
            let (a, b) = (ins[0].reals(), ins[1].reals());
            let mut c = vec![0.0f64; m.m * m.n];
            for i in 0..m.m {
                for kk in 0..m.k {
                    for j in 0..m.n {
                        c[i * m.n + j] = c[i * m.n + j] + a[i * m.k + kk] * b[kk * m.n + j];
                    }
                }
            }
            out(c)
        }
        TensorOpSpec::HeadMatmul(h) => {
            let (x, w) = (real(0, h.m * h.heads * h.a)?, real(1, h.heads * h.a * h.b)?);
            let mut y = vec![0.0f64; h.m * h.heads * h.b];
            for t in 0..h.m {
                for hh in 0..h.heads {
                    for e in 0..h.a {
                        for r in 0..h.b {
                            let yi = t * h.heads * h.b + hh * h.b + r;
                            y[yi] = y[yi] + x[t * h.heads * h.a + hh * h.a + e] * w[(hh * h.a + e) * h.b + r];
                        }
                    }
                }
            }
            out(y)
        }
        TensorOpSpec::Elementwise(e) => {
            need(at, &names[0], &ins[0], e.n)?;
            need(at, &names[1], &ins[1], e.row.unwrap_or(e.n))?;
            let b_at = |p: usize| e.row.map_or(p, |w| p % w);
            if let (Data::Int(a), Data::Int(b)) = (&ins[0], &ins[1]) {
                let c = (0..e.n)
                    .map(|p| match e.op {
                        ElementwiseOp::Add => checked(at, "a sum", a[p].checked_add(b[b_at(p)])),
                        ElementwiseOp::Mul => checked(at, "a product", a[p].checked_mul(b[b_at(p)])),
                    })
                    .collect::<Result<Vec<i64>, _>>()?;
                return Ok((Some(Data::Int(c)), Vec::new()));
            }
            let (a, b) = (ins[0].reals(), ins[1].reals());
            out((0..e.n)
                .map(|p| match e.op {
                    ElementwiseOp::Add => a[p] + b[b_at(p)],
                    ElementwiseOp::Mul => a[p] * b[b_at(p)],
                })
                .collect())
        }
        TensorOpSpec::Scale(s) => {
            let f = s.factor.get();
            out(real(0, s.n)?.into_iter().take(s.n).map(|x| x * f).collect())
        }
        TensorOpSpec::Softcap(s) => {
            let c = s.cap.get();
            out(real(0, s.n)?.into_iter().take(s.n).map(|x| capped(x, c)).collect())
        }
        TensorOpSpec::Silu(s) => out(real(0, s.n)?.into_iter().take(s.n).map(silu).collect()),
        TensorOpSpec::Sigmoid(s) => out(real(0, s.n)?.into_iter().take(s.n).map(|x| 1.0 / (1.0 + E.powf(0.0 - x))).collect()),
        TensorOpSpec::Gelu(g) => {
            const SQRT_2_OVER_PI: f64 = 0.797_884_560_802_865_4;
            const K: f64 = 0.044_715;
            out(real(0, g.n)?
                .into_iter()
                .take(g.n)
                .map(|x| {
                    let x3 = x * (x * x);
                    let z = SQRT_2_OVER_PI * (x + K * x3);
                    x / (1.0 + E.powf(0.0 - 2.0 * z))
                })
                .collect())
        }
        TensorOpSpec::SwiGlu(s) => {
            let (g, u) = (real(0, s.total)?, real(1, s.total)?);
            out((0..s.total).map(|i| silu(g[i]) * u[i]).collect())
        }
        TensorOpSpec::Decay(d) => {
            let heads = d.n / d.rows.max(1);
            let (ar, dt, al) = (real(0, d.n)?, real(1, heads)?, real(2, heads)?);
            let mut y = Vec::with_capacity(d.n);
            for r in 0..d.rows.max(1) {
                for i in 0..heads {
                    let base = 1.0 + E.powf(ar[r * heads + i] + dt[i]);
                    y.push(base.powf(al[i]));
                }
            }
            out(y)
        }
        TensorOpSpec::L2Norm(l) => {
            let x = real(0, l.rows * l.cols)?;
            let eps = l.eps.get();
            let mut y = Vec::with_capacity(l.rows * l.cols);
            for r in 0..l.rows {
                let base = r * l.cols;
                let mut ss = 0.0f64;
                for p in 0..l.cols {
                    ss = ss + x[base + p] * x[base + p];
                }
                let denom = (ss + eps).sqrt();
                for q in 0..l.cols {
                    y.push(x[base + q] / denom);
                }
            }
            out(y)
        }
        TensorOpSpec::FusedRow(r) => fused_row(at, r, names, ins),
        TensorOpSpec::Rope(r) => {
            let x = real(0, r.h * r.hd)?;
            let (rc, rs) = (ins[1].reals(), ins[2].reals());
            let per_row = r.h / r.rows.max(1);
            // The tables' first row: the position's, behind the program's check that the block's rows are the cache's.
            let first = match &r.from {
                Some((p, _)) => {
                    let p = index(at, p, names, 3, env)?;
                    let cache = rc.len() / r.half.max(1);
                    if p < 1 || p as usize + r.rows - 1 > cache {
                        return Err(EvalError::OutOfRange { op: at, what: format!("rotation from position {p} over {} rows of a {cache}-row table", r.rows) });
                    }
                    (p - 1) as usize
                }
                None => 0,
            };
            let mut y = Vec::with_capacity(r.h * r.hd);
            for rh in 0..r.h {
                let row = first + if r.rows > 1 || r.from.is_some() { rh / per_row } else { 0 };
                let t = |ri: usize| row * r.half + ri;
                if t(r.half - 1) >= rc.len().min(rs.len()) {
                    return Err(EvalError::OutOfRange { op: at, what: format!("table row {} past the rotary tables", row + 1) });
                }
                for ri in 0..r.half {
                    y.push(x[rh * r.hd + ri] * rc[t(ri)] - x[rh * r.hd + ri + r.half] * rs[t(ri)]);
                }
                for ri in 0..r.half {
                    y.push(x[rh * r.hd + ri] * rs[t(ri)] + x[rh * r.hd + ri + r.half] * rc[t(ri)]);
                }
                for rp in 2 * r.half..r.hd {
                    y.push(x[rh * r.hd + rp]);
                }
            }
            out(y)
        }
        TensorOpSpec::Transpose(t) => {
            need(at, &names[0], &ins[0], t.rows * t.cols)?;
            let pick: Vec<usize> = (0..t.cols).flat_map(|j| (0..t.rows).map(move |i| i * t.cols + j)).collect();
            Ok((Some(gather(&ins[0], &pick)), Vec::new()))
        }
        TensorOpSpec::HeadSlice(s) => {
            need(at, &names[0], &ins[0], s.n)?;
            let pick: Vec<usize> = (0..s.n / s.every).flat_map(|g| (0..s.width).map(move |i| g * s.every + s.from + i)).collect();
            Ok((Some(gather(&ins[0], &pick)), Vec::new()))
        }
        TensorOpSpec::Join(j) => {
            let (ha, hb) = (j.wa / j.heads, j.wb / j.heads);
            need(at, &names[0], &ins[0], j.rows * j.wa)?;
            need(at, &names[1], &ins[1], j.rows * j.wb)?;
            let both = |a: &Data, b: &Data| -> Option<Data> {
                match (a, b) {
                    (Data::Int(a), Data::Int(b)) => Some(Data::Int(join(a, b, j.rows, j.heads, ha, hb))),
                    _ => None,
                }
            };
            let joined = both(&ins[0], &ins[1]).unwrap_or_else(|| Data::Real(join(&ins[0].reals(), &ins[1].reals(), j.rows, j.heads, ha, hb)));
            Ok((Some(joined), Vec::new()))
        }
        TensorOpSpec::CausalConv(c) => {
            let x = real(0, c.rows * c.chan)?;
            let wc = real(1, c.taps * c.chan)?;
            let mut cs = real(2, c.taps * c.chan)?;
            let mut y = vec![0.0f64; c.rows * c.chan];
            for r in 0..c.rows {
                for t1 in 1..c.taps {
                    for ch in 0..c.chan {
                        cs[(t1 - 1) * c.chan + ch] = cs[t1 * c.chan + ch];
                    }
                }
                for ch in 0..c.chan {
                    cs[(c.taps - 1) * c.chan + ch] = x[r * c.chan + ch];
                }
                for ch in 0..c.chan {
                    let mut acc = 0.0f64;
                    for t in 0..c.taps {
                        acc = acc + cs[t * c.chan + ch] * wc[t * c.chan + ch];
                    }
                    y[r * c.chan + ch] = acc;
                }
            }
            Ok((Some(Data::Real(y)), vec![(2, Data::Real(cs))]))
        }
        TensorOpSpec::GatedDelta(g) => {
            let (hv, hk, d, rows) = (g.hv, g.hk, g.d, g.rows.max(1));
            let q = real(0, rows * hk * d)?;
            let k = real(1, rows * hk * d)?;
            let v = real(2, rows * hv * d)?;
            let z = real(3, rows * hv * d)?;
            let b = real(4, rows * hv)?;
            let a = real(5, rows * hv)?;
            let mut st = real(6, hv * d * d)?;
            let gn = real(7, d)?;
            let eps = g.eps.get();
            let gate = |x: f64| match g.gate {
                GateAct::Silu => silu(x),
                GateAct::Sigmoid => 1.0 / (1.0 + E.powf(0.0 - x)),
            };
            let read_scale = 1.0 / (d as f64).sqrt();
            let mut o_all = vec![0.0f64; hv * d * rows];
            for r in 0..rows {
                let (rq, rv, rh) = (r * hk * d, r * hv * d, r * hv);
                for head in 0..hv {
                    let kb = rq + (head % hk) * d;
                    let vb = rv + head * d;
                    let sb = head * d * d;
                    let (alpha, beta) = (a[rh + head], b[rh + head]);
                    let mut u = Vec::with_capacity(d);
                    for ua in 0..d {
                        let mut acc = 0.0f64;
                        for ub in 0..d {
                            acc = acc + st[sb + ua * d + ub] * k[kb + ub];
                        }
                        u.push(acc);
                    }
                    for sa in 0..d {
                        let coef = beta * (v[vb + sa] - alpha * u[sa]);
                        for sbi in 0..d {
                            st[sb + sa * d + sbi] = alpha * st[sb + sa * d + sbi] + coef * k[kb + sbi];
                        }
                    }
                    let mut o = Vec::with_capacity(d);
                    for oa in 0..d {
                        let mut acc = 0.0f64;
                        for ob in 0..d {
                            acc = acc + st[sb + oa * d + ob] * q[kb + ob];
                        }
                        o.push(acc * read_scale);
                    }
                    let mut ssq = 0.0f64;
                    for x in &o {
                        ssq = ssq + x * x;
                    }
                    let den = (ssq / d as f64 + eps).sqrt();
                    for na in 0..d {
                        o_all[vb + na] = gn[na] * (o[na] / den) * gate(z[vb + na]);
                    }
                }
            }
            Ok((Some(Data::Real(o_all)), vec![(6, Data::Real(st))]))
        }
        TensorOpSpec::Attention(a) => attention(at, a, names, ins, env),
        TensorOpSpec::FullAttention(f) => {
            let width = f.h * f.hd;
            let (q, k, v) = (real(0, f.t * width)?, real(1, f.t * width)?, real(2, f.t * width)?);
            let scale = 1.0f64 / (f.hd as f64).sqrt();
            let mut c = vec![0.0f64; f.t * width];
            for row in 0..f.t {
                for head in 0..f.h {
                    let mut scores = Vec::with_capacity(f.t);
                    for j in 0..f.t {
                        let mut acc = 0.0f64;
                        for e in 0..f.hd {
                            acc = acc + q[row * width + head * f.hd + e] * k[j * width + head * f.hd + e];
                        }
                        scores.push(acc * scale);
                    }
                    let mut mx = scores[0];
                    for s in &scores[1..] {
                        mx = float_max(mx, *s);
                    }
                    let mut den = 0.0f64;
                    for s in &scores {
                        den = den + E.powf(s - mx);
                    }
                    for e in 0..f.hd {
                        let mut acc = 0.0f64;
                        for j in 0..f.t {
                            acc = acc + E.powf(scores[j] - mx) / den * v[j * width + head * f.hd + e];
                        }
                        c[row * width + head * f.hd + e] = acc;
                    }
                }
            }
            out(c)
        }
        TensorOpSpec::Moe(m) => moe(at, m, names, ins),
        TensorOpSpec::RowGather(r) => {
            let row = index(at, &r.index, names, 1, env)?;
            if row < 1 || row as usize > r.rows {
                return Err(EvalError::OutOfRange { op: at, what: format!("row {row} of `{}`'s {} rows", names[0], r.rows) });
            }
            need(at, &names[0], &ins[0], r.rows * r.cols)?;
            let from = (row - 1) as usize * r.cols;
            Ok((Some(ins[0].slice(from, from + r.cols)), Vec::new()))
        }
        TensorOpSpec::RowsGather(r) => {
            let idx = ints(at, &names[1], &ins[1])?;
            need(at, &names[0], &ins[0], r.rows * r.cols)?;
            let mut pick = Vec::with_capacity(r.count * r.cols);
            for &row in idx.iter().take(r.count) {
                if row < 1 || row as usize > r.rows {
                    return Err(EvalError::OutOfRange { op: at, what: format!("row {row} of `{}`'s {} rows", names[0], r.rows) });
                }
                pick.extend((0..r.cols).map(|j| (row - 1) as usize * r.cols + j));
            }
            Ok((Some(gather(&ins[0], &pick)), Vec::new()))
        }
        TensorOpSpec::ArgMax(a) => {
            need(at, &names[0], &ins[0], a.n)?;
            let mut best_at = 0usize;
            match &ins[0] {
                Data::Real(x) => {
                    for i in 1..a.n {
                        if x[i] > x[best_at] {
                            best_at = i;
                        }
                    }
                }
                Data::Int(x) => {
                    for i in 1..a.n {
                        if x[i] > x[best_at] {
                            best_at = i;
                        }
                    }
                }
            }
            Ok((Some(Data::Int(vec![best_at as i64 + 1])), Vec::new()))
        }
        TensorOpSpec::SetRow(r) => {
            let row = index(at, &r.index, names, 2, env)?;
            let last = row + r.count as i64 - 1;
            if row < 1 || last as usize > r.rows {
                return Err(EvalError::OutOfRange { op: at, what: format!("rows {row}..={last} of `{}`'s {} rows", names[0], r.rows) });
            }
            need(at, &names[0], &ins[0], r.rows * r.cols)?;
            need(at, &names[1], &ins[1], r.cols * r.count)?;
            let from = (row - 1) as usize * r.cols;
            let table = match (&ins[0], &ins[1]) {
                (Data::Real(t), src) => {
                    let mut t = t.clone();
                    let s = src.reals();
                    t[from..from + r.cols * r.count].copy_from_slice(&s[..r.cols * r.count]);
                    Data::Real(t)
                }
                (Data::Int(t), Data::Int(s)) => {
                    let mut t = t.clone();
                    t[from..from + r.cols * r.count].copy_from_slice(&s[..r.cols * r.count]);
                    Data::Int(t)
                }
                (Data::Int(_), Data::Real(_)) => {
                    return Err(EvalError::Operand { op: at, tensor: names[1].clone(), why: "Reals written into a table of Ints".into() })
                }
            };
            Ok((None, vec![(0, table)]))
        }
        TensorOpSpec::IntRequant(r) => {
            let x = ints(at, &names[0], &ins[0])?;
            let y = x
                .iter()
                .take(r.n)
                .map(|&v| checked(at, "a requant's product", v.checked_mul(r.m)).map(|p| p.wrapping_shr(r.s)))
                .collect::<Result<Vec<i64>, _>>()?;
            Ok((Some(Data::Int(y)), Vec::new()))
        }
        TensorOpSpec::IntClamp(c) => {
            let x = ints(at, &names[0], &ins[0])?;
            Ok((Some(Data::Int(x.iter().take(c.n).map(|&v| v.max(c.lo).min(c.hi)).collect())), Vec::new()))
        }
        TensorOpSpec::Relu(r) => {
            need(at, &names[0], &ins[0], r.n)?;
            let y = match &ins[0] {
                Data::Int(x) => Data::Int(x.iter().take(r.n).map(|&v| v.max(0)).collect()),
                Data::Real(x) => Data::Real(x.iter().take(r.n).map(|&v| float_max(v, 0.0)).collect()),
            };
            Ok((Some(y), Vec::new()))
        }
        TensorOpSpec::LutAttention(l) => lut_attention(at, l, names, ins, env),
    }
}

fn gather(d: &Data, pick: &[usize]) -> Data {
    match d {
        Data::Real(v) => Data::Real(pick.iter().map(|&i| v[i]).collect()),
        Data::Int(v) => Data::Int(pick.iter().map(|&i| v[i]).collect()),
    }
}

fn join<T: Copy>(a: &[T], b: &[T], rows: usize, heads: usize, ha: usize, hb: usize) -> Vec<T> {
    let mut out = Vec::with_capacity(rows * heads * (ha + hb));
    for r in 0..rows {
        for h in 0..heads {
            out.extend((0..ha).map(|i| a[r * heads * ha + h * ha + i]));
            out.extend((0..hb).map(|i| b[r * heads * hb + h * hb + i]));
        }
    }
    out
}

fn fused_row(at: usize, r: &crate::tensor_recognize::FusedRowSpec, names: &[String], ins: &[Data]) -> Result<Computed, EvalError> {
    let n = r.rows * r.cols;
    need(at, &names[0], &ins[0], n)?;
    let x = ins[0].reals();
    // The operands after `x`, in the spec's order: the weight, the residual, the bias — those present.
    let mut k = 1;
    let mut next = |present: bool| -> Option<Vec<f64>> {
        present.then(|| {
            let v = ins[k].reals();
            k += 1;
            v
        })
    };
    let (w, res, bias) = (next(r.weight.is_some()), next(r.residual.is_some()), next(r.bias.is_some()));
    let eps = r.eps.get();
    let cols_f = r.cols as f64;
    let mut y = Vec::with_capacity(n);
    match r.reduce {
        RowReduceKind::RmsNorm => match &w {
            None => {
                let mut ss = 0.0f64;
                for p in 0..n {
                    ss = ss + x[p] * x[p];
                }
                let denom = (ss / n as f64 + eps).sqrt();
                for q in 0..n {
                    y.push(x[q] / denom);
                }
            }
            Some(w) => {
                for row in 0..r.rows {
                    let elem = |i: usize| match &res {
                        Some(res) => x[row * r.cols + i] + res[row * r.cols + i],
                        None => x[row * r.cols + i],
                    };
                    let mut ss = 0.0f64;
                    for i in 0..r.cols {
                        ss = ss + elem(i) * elem(i);
                    }
                    let denom = (ss / cols_f + eps).sqrt();
                    for j in 0..r.cols {
                        y.push(elem(j) / denom * w[j]);
                    }
                }
            }
        },
        RowReduceKind::Softmax => {
            for row in 0..r.rows {
                let xr = &x[row * r.cols..(row + 1) * r.cols];
                let mut m = xr[0];
                for v in &xr[1..] {
                    m = float_max(m, *v);
                }
                let mut s = 0.0f64;
                for v in xr {
                    s = s + E.powf(v - m);
                }
                for v in xr {
                    y.push(E.powf(v - m) / s);
                }
            }
        }
        RowReduceKind::LayerNorm => {
            let (Some(w), Some(b)) = (&w, &bias) else {
                return Err(EvalError::Operand { op: at, tensor: names[0].clone(), why: "a layer norm without its weight and bias".into() });
            };
            for row in 0..r.rows {
                let xr = &x[row * r.cols..(row + 1) * r.cols];
                let mut sm = 0.0f64;
                for v in xr {
                    sm = sm + v;
                }
                let mean = sm / cols_f;
                let mut vs = 0.0f64;
                for v in xr {
                    vs = vs + (v - mean) * (v - mean);
                }
                let denom = (vs / cols_f + eps).sqrt();
                for (j, v) in xr.iter().enumerate() {
                    y.push((v - mean) / denom * w[j] + b[j]);
                }
            }
        }
    }
    Ok((Some(Data::Real(y)), Vec::new()))
}

/// `x` soft-capped at `c` as the language writes it: `c·(1 − 2/(e^(x·(2/c)) + 1))`.
fn capped(x: f64, c: f64) -> f64 {
    c * (1.0 - 2.0 / (E.powf(x * (2.0 / c)) + 1.0))
}

fn attention(
    at: usize,
    a: &crate::tensor_recognize::qwen::AttentionSpec,
    names: &[String],
    ins: &[Data],
    env: &BTreeMap<String, Data>,
) -> Result<Computed, EvalError> {
    let (h, kv, hd, vd, l, rows) = (a.h, a.kv, a.hd, a.vd, a.l, a.rows.max(1));
    let group = if kv > 0 { h / kv } else { 1 };
    let scale = a.scale.map_or(1.0f64 / (hd as f64).sqrt(), |s| s.get());
    need(at, &names[0], &ins[0], rows * h * hd)?;
    need(at, &names[1], &ins[1], l * kv * hd)?;
    need(at, &names[2], &ins[2], l * kv * vd)?;
    let (q, kc, vc) = (ins[0].reals(), ins[1].reals(), ins[2].reals());
    // The operands after the caches: the position, then the sinks — those present.
    let mut k = 3;
    let pos = match &a.pos {
        Some(p) => {
            let v = index(at, p, names, k, env)?;
            if matches!(p, RowIndex::Named(_)) {
                k += 1;
            }
            Some(v)
        }
        None => None,
    };
    let sinks = a.sinks.map(|_| ins[k].reals());
    if let Some(p) = pos {
        if p < 1 || p as usize + rows - 1 > l {
            return Err(EvalError::OutOfRange {
                op: at,
                what: format!("attention length out of range: `{}` has {l} positions, and {rows} rows from {p} need {p} ≥ 1 and {p} + {} ≤ {l}", names[1], rows - 1),
            });
        }
    }
    let mut c = vec![0.0f64; rows * h * vd];
    for r in 0..rows {
        // The positions this row attends: up to its own, or every cached one.
        let n = pos.map_or(l, |p| p as usize + r);
        for head in 0..h {
            let kvh = head / group;
            let lo = a.window.map_or(1, |w| w.first(n));
            let mut scores = Vec::with_capacity(n + 1 - lo);
            for j in lo..=n {
                let mut acc = 0.0f64;
                for e in 0..hd {
                    acc = acc + q[r * h * hd + head * hd + e] * kc[(j - 1) * kv * hd + kvh * hd + e];
                }
                let score = acc * scale;
                scores.push(a.softcap.map_or(score, |c| capped(score, c.get())));
            }
            let mut mx = match &sinks {
                Some(s) => float_max(scores[0], s[head]),
                None => scores[0],
            };
            for s in &scores[1..] {
                mx = float_max(mx, *s);
            }
            let mut den = match &sinks {
                Some(s) => E.powf(s[head] - mx),
                None => 0.0,
            };
            for s in &scores {
                den = den + E.powf(s - mx);
            }
            for e in 0..vd {
                let mut acc = 0.0f64;
                for j in lo..=n {
                    acc = acc + E.powf(scores[j - lo] - mx) / den * vc[(j - 1) * kv * vd + kvh * vd + e];
                }
                c[r * h * vd + head * vd + e] = acc;
            }
        }
    }
    Ok((Some(Data::Real(c)), Vec::new()))
}

fn moe(at: usize, m: &crate::tensor_recognize::qwen::MoeSpec, names: &[String], ins: &[Data]) -> Result<Computed, EvalError> {
    let (d, dff, ne, k) = (m.d, m.dff, m.ne, m.k);
    let x = ins[0].reals();
    need(at, &names[0], &ins[0], d)?;
    need(at, &names[1], &ins[1], d * ne)?;
    need(at, &names[2], &ins[2], ne * dff * d)?;
    need(at, &names[3], &ins[3], ne * dff * d)?;
    need(at, &names[4], &ins[4], ne * d * dff)?;
    let (wr, wg, wu, wd) = (ins[1].reals(), ins[2].reals(), ins[3].reals(), ins[4].reals());
    // The operands after the four matrices: the router's bias, then the gate's, up's and down's, then the selection
    // bias — those present.
    let mut at_k = 5;
    let mut next = |present: bool| -> Option<Vec<f64>> {
        present.then(|| {
            let v = ins[at_k].reals();
            at_k += 1;
            v
        })
    };
    let rb = next(m.router_bias.is_some());
    let [bg, bu, bd] = [next(m.biases[0].is_some()), next(m.biases[1].is_some()), next(m.biases[2].is_some())];
    let bs = next(m.selection_bias.is_some());
    let mut logits = Vec::with_capacity(ne);
    for e in 0..ne {
        let mut acc = 0.0f64;
        for t in 0..d {
            acc = acc + x[t] * wr[t * ne + e];
        }
        logits.push(match &rb {
            Some(b) => acc + b[e],
            None => acc,
        });
    }
    let p: Vec<f64> = match m.score {
        MoeScore::Softmax => {
            let mut mx = logits[0];
            for l in &logits[1..] {
                mx = float_max(mx, *l);
            }
            let mut sum = 0.0f64;
            for l in &logits {
                sum = sum + E.powf(l - mx);
            }
            logits.iter().map(|l| E.powf(l - mx) / sum).collect()
        }
        // σ(L[e]) = 1 / (1 + e^(0 − L[e])), the desugar's arrangement.
        MoeScore::Sigmoid => logits.iter().map(|l| 1.0 / (1.0 + E.powf(0.0 - l))).collect(),
    };
    // The selection scores: each score plus its selection bias; the weights stay the scores.
    let mut pm: Vec<f64> = match &bs {
        Some(b) => p.iter().zip(b).map(|(v, b)| v + b).collect(),
        None => p.clone(),
    };
    // The group limit: each group scored by its two best selection scores, the best group left chosen in turn (and
    // marked below every score), only the chosen groups' experts keeping their selection scores.
    if let Some(MoeGroups { count, kept }) = m.groups {
        let per = ne / count;
        let mut scores: Vec<f64> = (0..count)
            .map(|g| {
                let (mut t1, mut t2) = (-1.0e30f64, -1.0e30f64);
                for v in &pm[g * per..(g + 1) * per] {
                    if *v > t1 {
                        t2 = t1;
                        t1 = *v;
                    } else if *v > t2 {
                        t2 = *v;
                    }
                }
                if per > 1 {
                    t1 + t2
                } else {
                    t1
                }
            })
            .collect();
        let mut chosen = vec![-1.0e30f64; ne];
        for _ in 0..kept {
            let (mut bv, mut bi) = (-1.0e30f64, 0usize);
            for (g, v) in scores.iter().enumerate() {
                if *v > bv {
                    bv = *v;
                    bi = g;
                }
            }
            chosen[bi * per..(bi + 1) * per].copy_from_slice(&pm[bi * per..(bi + 1) * per]);
            scores[bi] = -2.0e30;
        }
        pm = chosen;
    }
    let mut w = vec![0.0f64; ne];
    // The kept experts, one each, when the weights scale the inputs: their outputs join the sum unweighted.
    let mut sel = vec![0.0f64; ne];
    let mut wsum = 0.0f64;
    for _ in 0..k {
        let (mut bv, mut bi) = (-1.0e30f64, 0usize);
        for (sci, v) in pm.iter().enumerate() {
            if *v > bv {
                bv = *v;
                bi = sci;
            }
        }
        wsum = wsum + p[bi];
        w[bi] = p[bi];
        sel[bi] = 1.0;
        pm[bi] = -1.0e30;
    }
    if m.renormalize {
        // The sum clamped at 2⁻¹⁴, as llama.cpp's `build_moe_ffn` clamps it.
        let wsum = float_max(wsum, 6.103515625e-5);
        for v in &mut w {
            *v = *v / wsum;
        }
    }
    if m.scale.is_some() {
        for v in &mut w {
            *v = *v * m.scale();
        }
    }
    let mut out = vec![0.0f64; d];
    for ee in 0..ne {
        let project = |wm: &[f64], bias: &Option<Vec<f64>>| -> Vec<f64> {
            (0..dff)
                .map(|o| {
                    let mut acc = 0.0f64;
                    for t in 0..d {
                        acc = acc + x[t] * wm[(ee * dff + o) * d + t];
                    }
                    let acc = if m.weigh_inputs { w[ee] * acc } else { acc };
                    match bias {
                        Some(b) => acc + b[ee * dff + o],
                        None => acc,
                    }
                })
                .collect()
        };
        let (gate, up) = (project(&wg, &bg), project(&wu, &bu));
        let hidden = |t: usize| match m.act {
            MoeAct::Swiglu => silu(gate[t]) * up[t],
            MoeAct::ClampedSwiglu { limit, slope } => {
                let (limit, slope) = (f64::from_bits(limit), f64::from_bits(slope));
                let gc = float_min(gate[t], limit);
                let uc = float_max(float_min(up[t], limit), -limit);
                gc / (1.0 + E.powf(0.0 - slope * gc)) * (uc + 1.0)
            }
        };
        for oo in 0..d {
            let mut acc = 0.0f64;
            for t in 0..dff {
                acc = acc + hidden(t) * wd[(ee * d + oo) * dff + t];
            }
            let down = match &bd {
                Some(b) => acc + b[ee * d + oo],
                None => acc,
            };
            out[oo] = out[oo] + (if m.weigh_inputs { sel[ee] } else { w[ee] }) * down;
        }
    }
    Ok((Some(Data::Real(out)), Vec::new()))
}

/// `a // b`: the quotient rounded toward negative infinity, exactly, as the VM floors the rational quotient.
fn floor_div(a: i64, b: i64) -> i128 {
    let (a, b) = (a as i128, b as i128);
    let q = a / b;
    if a % b != 0 && ((a < 0) != (b < 0)) {
        q - 1
    } else {
        q
    }
}

fn lut_attention(
    at: usize,
    a: &crate::tensor_recognize::qwen::LutAttentionSpec,
    names: &[String],
    ins: &[Data],
    env: &BTreeMap<String, Data>,
) -> Result<Computed, EvalError> {
    let (h, kv, hd, l, n) = (a.h, a.kv, a.hd, a.l, a.table_len);
    let group = h / kv.max(1);
    let (q, kc, vc, table) = (ints(at, &names[0], &ins[0])?, ints(at, &names[1], &ins[1])?, ints(at, &names[2], &ins[2])?, ints(at, &names[3], &ins[3])?);
    need(at, &names[0], &ins[0], h * hd)?;
    need(at, &names[1], &ins[1], l * kv * hd)?;
    need(at, &names[2], &ins[2], l * kv * hd)?;
    need(at, &names[3], &ins[3], n)?;
    let len = match &a.pos {
        Some(p) => {
            let p = index(at, p, names, 4, env)?;
            if p < 1 || p as usize > l {
                return Err(EvalError::OutOfRange { op: at, what: format!("attention length out of range: `{}` has {l} positions, not {p}", names[1]) });
            }
            p as usize
        }
        None => l,
    };
    let mut c = vec![0i64; h * hd];
    for head in 0..h {
        let kvh = head / group;
        let mut scores = Vec::with_capacity(len);
        for j in 0..len {
            let mut acc = 0i64;
            for e in 0..hd {
                let p = checked(at, "a score's product", q[head * hd + e].checked_mul(kc[j * kv * hd + kvh * hd + e]))?;
                acc = checked(at, "a score", acc.checked_add(p))?;
            }
            scores.push(acc);
        }
        let mut mx = scores[0];
        for s in &scores[1..] {
            mx = mx.max(*s);
        }
        let mut wts = Vec::with_capacity(len);
        let mut den = 0i64;
        for s in &scores {
            let d = checked(at, "a score's distance", mx.checked_sub(*s))?;
            let scaled = checked(at, "a scaled distance", d.checked_mul(a.m))?.wrapping_shr(a.s);
            let w = table[scaled.max(0).min(n as i64 - 1) as usize];
            wts.push(w);
            den = checked(at, "a weight sum", den.checked_add(w))?;
        }
        for e in 0..hd {
            let mut num = 0i64;
            for j in 0..len {
                let p = checked(at, "a weighted value", wts[j].checked_mul(vc[j * kv * hd + kvh * hd + e]))?;
                num = checked(at, "a weighted sum", num.checked_add(p))?;
            }
            if den == 0 {
                return Err(EvalError::DivideByZero { op: at });
            }
            c[head * hd + e] = checked(at, "a quotient", i64::try_from(floor_div(num, den)).ok())?;
        }
    }
    Ok((Some(Data::Int(c)), Vec::new()))
}
