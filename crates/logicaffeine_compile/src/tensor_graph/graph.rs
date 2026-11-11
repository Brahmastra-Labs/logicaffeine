//! The one IR: a cut of a model's one program as the tensor ops it runs and the tensors they touch.
//!
//! A graph is lifted from the front end ([`walk`](super::front::walk)): every top-level tensor statement is one op,
//! and a statement the front end cannot lift — or a write it cannot account for — is refused by name. Each tensor
//! carries the facts every backend reads instead of re-deriving them:
//!
//!   • its **role** — a *parameter* the program reads as a position or row indices (the token, a chunk's tokens, the
//!     position), *state* an op writes in place (a KV cache by `Set row`, a convolution ring, a delta-rule memory),
//!     a *constant* any other declared tensor (weights, norm vectors, rotary tables), an *activation* what an op
//!     computes;
//!   • its **element** type, Real or Int, as the program declares or the op that computes it makes it;
//!   • its **fill**, the values the program gives a declared tensor, and its **storage**, where a constant's values
//!     live (computed, values, or rows of a checkpoint tensor in a block format) — storage comes from the caller,
//!     since the IR reads no checkpoint;
//!   • whether a statement other than an op **observes** it (a `Show`, a scalar read, foreign code).
//!
//! An op is a symbol over operands: a primitive the front end lifted (`Sym::Prim`), a fused symbol whose definition is
//! its parts (`Sym::Fused`), or a view into another tensor. Each operand is read whole or as a part of every
//! unit of a wider tensor (`Access`), and each result lands in its own buffer or in rows or units of another (`Dst`).
//! `covers` names the lifted ops an op computes, so every op of the program is accounted for exactly once however the
//! graph is rewritten.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

use crate::intern::{Interner, Symbol};
use crate::model::program::{Cut, Identity};
use crate::model::spec::ModelSpec;
use crate::spatial::value_range::{Interval, UNBOUNDED};
use crate::model::quant::GgmlType;
use crate::tensor_recognize::qwen::RowIndex;
use crate::tensor_recognize::Fill;

use super::front::{RecognizedProgram, TensorOpSpec};

/// How a step treats a tensor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Role {
    Param,
    State,
    Constant,
    Activation,
}

/// A tensor's element type.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Elem {
    Float,
    Int,
}

/// Where a tensor's values live.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Storage {
    /// An op computes it, or the program fills it.
    Computed,
    /// Values a backend holds as numbers, in the layout the program declares.
    Values,
    /// Rows of a checkpoint tensor read in place in a block format; `transposed` when the checkpoint's rows are the
    /// program's columns.
    Rows { format: GgmlType, transposed: bool },
}

/// One tensor of the graph: its shape as the program gives it (`rows × cols`), and its facts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tensor {
    pub name: String,
    pub rows: usize,
    pub cols: usize,
    pub role: Role,
    pub elem: Elem,
    /// The values the program declares it with; `None` for a tensor an op computes.
    pub fill: Option<Fill>,
    pub storage: Storage,
    /// The interval an integer tensor is proven to hold over the step; `None` for a float, and for an integer no bound
    /// is proven for.
    pub range: Option<Interval>,
    /// A statement other than an op reads or writes it.
    pub observed: bool,
    /// A state the step carries to the next ([`super::front::carried_state`]): written in place and read beyond what
    /// the step writes — a cache, a ring, a memory — where a table the step assembles whole before reading is not.
    pub carried: bool,
}

impl Tensor {
    pub fn len(&self) -> usize {
        self.rows * self.cols
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// Which row of a table a write starts at: the row a parameter names (1-based), or a fixed row.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum RowSel {
    Param(String),
    Fixed(usize),
}

/// A part of every unit of a wider tensor: unit `u`'s values start `u·pitch + offset` in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Part {
    pub pitch: usize,
    pub offset: usize,
}

/// A part of every unit of a wider tensor read in place: unit `u`'s `width` values from `u·pitch + offset`, for every
/// whole unit of the tensor.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Strided {
    pub pitch: usize,
    pub offset: usize,
    pub width: usize,
}

/// How an op reads an operand: the whole of it, or a part of every unit of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Access {
    Whole,
    Strided(Strided),
}

/// Where an op writes its result: a buffer of its own, or the rows of a table of rows `pitch` wide from a starting row
/// — the whole of it, or (`unit`) a part of every unit of it, each unit's share of the result `width` values.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Dst {
    pub buf: String,
    pub row: Option<(RowSel, usize)>,
    pub unit: Option<Strided>,
}

impl Dst {
    pub fn to(buf: &str) -> Dst {
        Dst { buf: buf.to_string(), row: None, unit: None }
    }

    /// The tensor the result is the whole of, in its own buffer.
    pub fn own(&self) -> Option<&str> {
        (self.row.is_none() && self.unit.is_none()).then_some(self.buf.as_str())
    }
}

/// The names a step reads and writes across steps: its token(s), its position, its logits and its next token — its first
/// sequence's, and a batch's every other sequence's in `more`, sequence `b`'s next token written to item `b` of the
/// token tensor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StepIo {
    pub token: String,
    pub position: String,
    pub logits: String,
    pub next: String,
    pub more: Vec<SeqIo>,
}

impl StepIo {
    /// Every sequence's position, the first sequence's first.
    pub fn positions(&self) -> Vec<&str> {
        std::iter::once(self.position.as_str()).chain(self.more.iter().map(|q| q.position.as_str())).collect()
    }

    /// Every name the step reads or writes across steps: its tokens and every sequence's position, logits and next token.
    pub fn names(&self) -> Vec<&str> {
        let firsts = [self.token.as_str(), self.position.as_str(), self.logits.as_str(), self.next.as_str()];
        firsts.into_iter().chain(self.more.iter().flat_map(|q| [q.position.as_str(), q.logits.as_str(), q.next.as_str()])).collect()
    }
}

/// A batch's sequence past its first: the position it reads, the logits it computes and the next token it picks.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SeqIo {
    pub position: String,
    pub logits: String,
    pub next: String,
}

/// A fused symbol: its class and its definition, the ops it computes, in order.
#[derive(Clone, Debug)]
pub struct Fused {
    pub class: super::symbols::ClassId,
    pub parts: Vec<Op>,
}

/// How an activation is quantized for the products that read it: int8 words a 32-value block shares one scale of —
/// one word a value, or a high and a low word.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum QuantFormat {
    OneWord,
    TwoWords,
}

/// What an op computes.
#[derive(Clone, Debug)]
pub enum Sym {
    /// An op the front end lifted whole.
    Prim(TensorOpSpec),
    /// A fused symbol, defined by its parts.
    Fused(Fused),
    /// The values of its one operand from `offset` on, read in place.
    View { offset: usize },
    /// Its one operand's quantized form for the products that read it: the operand's own values, under the meaning.
    Quantize(QuantFormat),
}

/// One op of the graph: its symbol, its operands and how it reads each, its result and where that lands, the further
/// results it writes whole, and the lifted ops it computes.
#[derive(Clone, Debug)]
pub struct Op {
    pub sym: Sym,
    pub ins: Vec<String>,
    pub access: Vec<Access>,
    pub out: String,
    pub dst: Dst,
    pub keeps: Vec<String>,
    pub covers: Vec<usize>,
}

impl Op {
    /// The primitive a lifted op is.
    pub fn prim(&self) -> Option<&TensorOpSpec> {
        match &self.sym {
            Sym::Prim(spec) => Some(spec),
            _ => None,
        }
    }

    /// The op's kind: a primitive's, a fused symbol's class, `View` or `Quantize`.
    pub fn kind(&self) -> &str {
        match &self.sym {
            Sym::Prim(spec) => spec.kind(),
            Sym::Fused(f) => f.class.id(),
            Sym::View { .. } => "View",
            Sym::Quantize(_) => "Quantize",
        }
    }

    /// The tensor the op computes whole into a buffer of its own: not one it lands rows or units of another in, nor one
    /// it updates in place (a row written into a table, an activation's quantized form).
    pub fn result(&self) -> Option<&str> {
        let own = self.dst.own()?;
        let in_place = matches!(self.sym, Sym::Quantize(_)) || self.state_inputs().into_iter().any(|k| self.ins[k] == own);
        (!in_place).then_some(own)
    }

    /// The positions in `ins` of the operands the op writes in place: a primitive's state, or any part's.
    pub fn state_inputs(&self) -> Vec<usize> {
        match &self.sym {
            Sym::Prim(spec) => spec.state_inputs(),
            Sym::Fused(f) => {
                let written: BTreeSet<&str> = f.parts.iter().flat_map(|p| p.state_inputs().into_iter().map(|k| p.ins[k].as_str())).collect();
                self.ins.iter().enumerate().filter(|(_, n)| written.contains(n.as_str())).map(|(k, _)| k).collect()
            }
            Sym::View { .. } | Sym::Quantize(_) => Vec::new(),
        }
    }

    /// Every tensor the op writes: its destination's buffer, what it writes in place, what it keeps, and — for a fused
    /// op — every table or wider tensor a part lands its result in.
    pub fn writes(&self) -> Vec<&str> {
        let mut w: Vec<&str> = vec![self.dst.buf.as_str()];
        w.extend(self.state_inputs().into_iter().map(|k| self.ins[k].as_str()));
        w.extend(self.keeps.iter().map(String::as_str));
        if let Sym::Fused(f) = &self.sym {
            w.extend(f.parts.iter().flat_map(|p| p.writes()).filter(|t| p_lands_outside(f, t)));
        }
        w.sort_unstable();
        w.dedup();
        w
    }

    /// The parameters that name the rows the op's result, or any part's, lands in.
    pub fn row_params(&self) -> Vec<&str> {
        let mut r: Vec<&str> = match &self.dst.row {
            Some((RowSel::Param(p), _)) => vec![p.as_str()],
            _ => Vec::new(),
        };
        if let Sym::Fused(f) = &self.sym {
            r.extend(f.parts.iter().flat_map(|p| p.row_params()));
        }
        r
    }
}

/// Whether a tensor a part of `f` writes is one the fused op writes outside itself: a table or wider tensor a part lands
/// rows or units in, or state a part updates — not an intermediate one part computes for another.
fn p_lands_outside(f: &Fused, t: &str) -> bool {
    f.parts.iter().any(|p| p.dst.buf == t && p.dst.own().is_none()) || f.parts.iter().any(|p| p.state_inputs().iter().any(|&k| p.ins[k] == t))
}

/// The ops that touch each tensor of a graph ([`TensorGraph::touches`]).
pub struct Touches<'g> {
    graph: &'g TensorGraph,
    by: BTreeMap<&'g str, BTreeSet<usize>>,
}

impl Touches<'_> {
    /// Whether ops `writer` and `reader` are the only ones that touch `tensor`: an activation no statement but an op
    /// sees, which no other op reads or writes.
    pub fn read_only_by(&self, tensor: &str, writer: usize, reader: usize) -> bool {
        self.graph.tensors.get(tensor).is_some_and(|t| t.role == Role::Activation && !t.observed)
            && self.by.get(tensor).is_none_or(|at| at.iter().all(|&i| i == writer || i == reader))
    }
}

/// A source walked by the front end, and the graph of every statement the walk lifts.
#[derive(Clone, Debug)]
pub struct Lifted {
    pub program: RecognizedProgram,
    pub graph: TensorGraph,
}

/// A cut of a model's one program as a graph.
#[derive(Clone, Debug)]
pub struct TensorGraph {
    /// The cut it was lifted from.
    pub identity: Option<Identity>,
    /// The names it reads and writes across steps.
    pub io: Option<StepIo>,
    pub tensors: BTreeMap<String, Tensor>,
    /// The ops in program order, the schedule order every backend starts from.
    pub ops: Vec<Op>,
}

/// Why a program is not a graph, by name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum GraphError {
    /// A tensor statement the front end lifts no op from.
    Unrecognized { stmt: usize, what: String },
    /// A write to a declared tensor that no op accounts for.
    UnliftedWrite { stmt: usize, tensor: String, why: &'static str },
    /// An op reads a tensor neither declared nor computed before it.
    Undefined { op: usize, tensor: String },
    /// The program computes nothing.
    Empty,
    /// The source does not parse, or the front end's walk refuses it.
    Unwalked { why: String },
}

impl fmt::Display for GraphError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GraphError::Unrecognized { stmt, what } => write!(f, "statement {stmt} (`{what}`) is no op a lane compiles"),
            GraphError::UnliftedWrite { stmt, tensor, why } => {
                write!(f, "statement {stmt} writes `{tensor}` outside any op: {why}")
            }
            GraphError::Undefined { op, tensor } => write!(f, "op {op} reads `{tensor}` before anything defines it"),
            GraphError::Empty => write!(f, "the program computes nothing"),
            GraphError::Unwalked { why } => write!(f, "the front end does not walk the program: {why}"),
        }
    }
}

impl std::error::Error for GraphError {}

/// The names an op reads as a position or as row indices.
fn index_operands(op: &TensorOpSpec, name: &dyn Fn(crate::intern::Symbol) -> String) -> Vec<String> {
    let named = |i: &RowIndex| match i {
        RowIndex::Named(s) => Some(name(*s)),
        RowIndex::Literal(_) => None,
    };
    match op {
        TensorOpSpec::RowGather(r) => named(&r.index).into_iter().collect(),
        TensorOpSpec::RowsGather(r) => vec![name(r.indices)],
        TensorOpSpec::SetRow(r) => named(&r.index).into_iter().collect(),
        TensorOpSpec::Scatter(s) => vec![name(s.indices)],
        TensorOpSpec::Attention(a) => a.pos.as_ref().and_then(named).into_iter().collect(),
        TensorOpSpec::LutAttention(a) => a.pos.as_ref().and_then(named).into_iter().collect(),
        TensorOpSpec::Rope(r) => r.from.as_ref().and_then(|(p, _)| named(p)).into_iter().collect(),
        _ => Vec::new(),
    }
}

/// The element count of an op's result.
pub fn out_len(op: &TensorOpSpec) -> usize {
    match op {
        TensorOpSpec::Matmul(m) => m.m * m.n,
        TensorOpSpec::Elementwise(e) => e.n,
        TensorOpSpec::SwiGlu(s) => s.total,
        TensorOpSpec::FusedRow(r) => r.rows * r.cols,
        TensorOpSpec::Rope(r) => r.h * r.hd,
        TensorOpSpec::Silu(s) => s.n,
        TensorOpSpec::Sigmoid(s) => s.n,
        TensorOpSpec::Gelu(g) => g.n,
        TensorOpSpec::Scale(c) => c.n,
        TensorOpSpec::Softcap(c) => c.n,
        TensorOpSpec::Decay(d) => d.n,
        TensorOpSpec::L2Norm(l) => l.rows * l.cols,
        TensorOpSpec::Transpose(t) => t.rows * t.cols,
        TensorOpSpec::HeadSlice(s) => s.n / s.every * s.width,
        TensorOpSpec::Join(j) => j.rows * (j.wa + j.wb),
        TensorOpSpec::LaneMean(m) => m.rows * m.cols,
        TensorOpSpec::LaneSpread(p) => p.rows * p.lanes * p.cols,
        TensorOpSpec::LaneSums(s) => s.rows * s.lanes,
        TensorOpSpec::SignedRoot(s) => s.n,
        TensorOpSpec::NgramIds(g) => g.tokens * g.heads(),
        TensorOpSpec::HeadMatmul(m) => m.m * m.heads * m.b,
        TensorOpSpec::CausalConv(c) => c.rows * c.chan,
        TensorOpSpec::GatedDelta(g) => g.rows * g.hv * g.d,
        TensorOpSpec::Attention(a) => a.rows * a.h * a.vd,
        TensorOpSpec::FullAttention(a) => a.t * a.h * a.hd,
        TensorOpSpec::Moe(m) => m.d,
        TensorOpSpec::RowGather(r) => r.cols,
        TensorOpSpec::RowsGather(r) => r.count * r.cols,
        TensorOpSpec::ArgMax(_) => 1,
        TensorOpSpec::SetRow(r) => r.rows * r.cols,
        TensorOpSpec::Scatter(s) => s.rows * s.cols,
        TensorOpSpec::Map(m) => m.n(),
        TensorOpSpec::IntRequant(r) => r.n,
        TensorOpSpec::IntClamp(c) => c.n,
        TensorOpSpec::Relu(r) => r.n,
        TensorOpSpec::LutAttention(a) => a.h * a.hd,
    }
}

impl TensorGraph {
    /// The graph of a walked program: `storage` says where each constant's values live (the IR reads no checkpoint), and
    /// `identity` and `io` are the cut it was lifted from and the names it carries across steps.
    pub fn lift(
        rp: &RecognizedProgram,
        identity: Option<Identity>,
        io: Option<StepIo>,
        storage: &dyn Fn(&str) -> Storage,
    ) -> Result<TensorGraph, GraphError> {
        if let Some((stmt, what)) = rp.unrecognized.first() {
            return Err(GraphError::Unrecognized { stmt: *stmt, what: what.clone() });
        }
        if let Some((stmt, tensor, why)) = rp.unlifted_writes.first() {
            return Err(GraphError::UnliftedWrite { stmt: *stmt, tensor: tensor.clone(), why });
        }
        if rp.stages.is_empty() {
            return Err(GraphError::Empty);
        }
        TensorGraph::build(rp, identity, io, storage, false)
    }

    /// The graph with each constant's values where `storage` says and `io` the names it carries across steps, which
    /// its host reads: a graph lifted before its constants were placed, placed as [`TensorGraph::lift`] places them.
    pub fn placed(&self, storage: &dyn Fn(&str) -> Storage, io: Option<StepIo>) -> TensorGraph {
        let mut g = self.clone();
        let carried: BTreeSet<String> = io.iter().flat_map(|io| io.names()).map(String::from).collect();
        for t in g.tensors.values_mut() {
            if t.role == Role::Constant {
                t.storage = storage(&t.name);
            }
            t.observed |= carried.contains(&t.name);
        }
        g.io = io;
        g
    }

    /// The graph of the statements the front end lifted, whatever else the program holds: a backend that reports the
    /// statements it could not lift on its own lowers the rest. A tensor a statement the front end passed over computes
    /// is an activation of the shape the program gives it.
    pub fn lift_stages(rp: &RecognizedProgram, identity: Option<Identity>, io: Option<StepIo>, storage: &dyn Fn(&str) -> Storage) -> TensorGraph {
        TensorGraph::build(rp, identity, io, storage, true).expect("a lenient lift refuses nothing")
    }

    /// The front end's walk of `source` and the graph of what it lifts ([`TensorGraph::lift_stages`]): the one way a
    /// backend reaches the ops of a source.
    pub fn lift_source(source: &str, storage: &dyn Fn(&str) -> Storage) -> Result<Lifted, crate::spatial::passes::SpatialReject> {
        let program = super::front::recognize_stages(source)?;
        let graph = TensorGraph::lift_stages(&program, None, None, storage);
        Ok(Lifted { program, graph })
    }

    /// The graph of the cut `cut` of `spec`'s program, recording the cut's identity: `io` the names it carries across
    /// steps, `storage` where each constant's values live.
    pub fn of_cut(spec: &ModelSpec, cut: Cut, io: Option<StepIo>, storage: &dyn Fn(&str) -> Storage) -> Result<TensorGraph, GraphError> {
        let (authored, identity) = crate::model::program::written(spec, cut);
        let rp = super::front::recognize_stages(&authored.0).map_err(|e| GraphError::Unwalked { why: format!("{e:?}") })?;
        TensorGraph::lift(&rp, Some(identity), io, storage)
    }

    /// The graph of one op over operands the source declares, each `(symbol, length, fill)`, and its result `out`: a
    /// statement a backend recognizes on its own, outside any walk of a program. Every symbol the op's spec names is
    /// one of its operands or its result.
    pub fn of_op(op: TensorOpSpec, ins: Vec<(Symbol, usize, Fill)>, out: Symbol, interner: &Interner) -> TensorGraph {
        let name = |s: Symbol| interner.resolve(s).to_string();
        let rp = RecognizedProgram {
            stages: vec![super::front::RecognizedStage { op, stmt_index: 0, out: name(out), ins: ins.iter().map(|(s, _, _)| name(*s)).collect() }],
            declared: ins.iter().map(|(s, len, fill)| super::front::DeclaredTensor { name: name(*s), len: *len, fill: fill.clone() }).collect(),
            symbols: ins.iter().map(|(s, _, _)| *s).chain([out]).map(|s| (s, name(s))).collect(),
            ..RecognizedProgram::default()
        };
        TensorGraph::lift_stages(&rp, None, None, &|_| Storage::Values)
    }

    fn build(
        rp: &RecognizedProgram,
        identity: Option<Identity>,
        io: Option<StepIo>,
        storage: &dyn Fn(&str) -> Storage,
        lenient: bool,
    ) -> Result<TensorGraph, GraphError> {
        let name = |s| rp.name(s).to_string();
        let mut params = BTreeSet::new();
        let mut state = BTreeSet::new();
        for st in &rp.stages {
            params.extend(index_operands(&st.op, &name));
            for k in st.op.state_inputs() {
                state.insert(st.ins[k].clone());
            }
        }
        // A tensor some statement other than an op touches: a `Show`, a scalar read, foreign code.
        let stage_stmts: BTreeSet<usize> = rp.stages.iter().map(|s| s.stmt_index).collect();
        let observed = |t: &str| {
            !rp.opaque.is_empty() || rp.mentions.get(t).is_some_and(|at| at.iter().any(|i| !stage_stmts.contains(i) && !rp.quiet.contains(i)))
        };
        // Each name's element kind and, an integer's, its proven interval: the front end's own analyses.
        let floats = super::front::float_tensors(rp);
        let ranges = crate::spatial::value_range::value_ranges(rp, &floats);
        let elem = |t: &str| if floats.contains(t) { Elem::Float } else { Elem::Int };
        let range = |t: &str| (!floats.contains(t)).then(|| ranges.get(t).copied()).flatten().filter(|r| *r != UNBOUNDED);
        let carried: BTreeSet<String> = super::front::carried_state(rp).into_iter().collect();
        let mut tensors = BTreeMap::new();
        for d in &rp.declared {
            let role = if params.contains(&d.name) {
                Role::Param
            } else if state.contains(&d.name) {
                Role::State
            } else {
                Role::Constant
            };
            let (rows, cols) = rp.shape_of(&d.name, d.len);
            let storage = if role == Role::Constant { storage(&d.name) } else { Storage::Computed };
            tensors.insert(
                d.name.clone(),
                Tensor {
                    name: d.name.clone(),
                    rows,
                    cols,
                    role,
                    elem: elem(&d.name),
                    fill: Some(d.fill.clone()),
                    storage,
                    range: range(&d.name),
                    observed: false,
                    carried: role == Role::State && carried.contains(&d.name),
                },
            );
        }
        let mut ops = Vec::with_capacity(rp.stages.len());
        for (i, st) in rp.stages.iter().enumerate() {
            for x in &st.ins {
                if tensors.contains_key(x) {
                    continue;
                }
                if !lenient {
                    return Err(GraphError::Undefined { op: i, tensor: x.clone() });
                }
                let (rows, cols) = rp.shapes.get(x).copied().unwrap_or((1, 0));
                tensors.insert(
                    x.clone(),
                    Tensor {
                        name: x.clone(),
                        rows,
                        cols,
                        role: Role::Activation,
                        elem: elem(x),
                        fill: None,
                        storage: Storage::Computed,
                        range: range(x),
                        observed: false,
                        carried: false,
                    },
                );
            }
            if !tensors.contains_key(&st.out) {
                let n = out_len(&st.op);
                let (rows, cols) = rp.shape_of(&st.out, n);
                tensors.insert(
                    st.out.clone(),
                    Tensor {
                        name: st.out.clone(),
                        rows,
                        cols,
                        role: Role::Activation,
                        elem: elem(&st.out),
                        fill: None,
                        storage: Storage::Computed,
                        range: range(&st.out),
                        observed: false,
                        carried: false,
                    },
                );
            }
            ops.push(Op {
                sym: Sym::Prim(st.op.clone()),
                access: vec![Access::Whole; st.ins.len()],
                ins: st.ins.clone(),
                dst: Dst::to(&st.out),
                out: st.out.clone(),
                keeps: Vec::new(),
                covers: vec![i],
            });
        }
        // The names a step carries across steps are read by its host: they are observed.
        let carried: BTreeSet<&str> = io.iter().flat_map(|io| io.names()).collect();
        let names: Vec<String> = tensors.keys().cloned().collect();
        for n in names {
            let seen = observed(&n) || carried.contains(n.as_str());
            tensors.get_mut(&n).expect("a tensor").observed = seen;
        }
        Ok(TensorGraph { identity, io, tensors, ops })
    }

    /// The `(rows, cols)` the program gives `tensor`, where that accounts for the `n` elements an op counts — the
    /// `1 × n` row otherwise.
    pub fn shape_of(&self, tensor: &str, n: usize) -> (usize, usize) {
        match self.tensors.get(tensor) {
            Some(t) if t.len() == n => (t.rows, t.cols),
            _ => (1, n),
        }
    }

    /// Which ops touch each tensor — read it, write it, or name the rows it lands in — taken once, for questions asked
    /// of many tensors.
    pub fn touches(&self) -> Touches<'_> {
        use super::facts::{reads, writes};
        let mut by: BTreeMap<&str, BTreeSet<usize>> = BTreeMap::new();
        for (i, op) in self.ops.iter().enumerate() {
            for t in reads(op).into_iter().chain(writes(op)) {
                by.entry(t).or_default().insert(i);
            }
        }
        Touches { graph: self, by }
    }

    /// The tensors of one role, by name.
    pub fn of_role(&self, role: Role) -> impl Iterator<Item = &Tensor> {
        self.tensors.values().filter(move |t| t.role == role)
    }

    /// The ops that read `tensor`, by index.
    pub fn readers(&self, tensor: &str) -> Vec<usize> {
        self.ops.iter().enumerate().filter(|(_, o)| o.ins.iter().any(|x| x == tensor)).map(|(i, _)| i).collect()
    }

    /// The op that computes `tensor` into its own buffer.
    pub fn producer(&self, tensor: &str) -> Option<usize> {
        self.ops.iter().position(|o| o.result() == Some(tensor) || o.keeps.iter().any(|k| k == tensor))
    }

    /// The ops of the graph and every member of each fused one, at every depth: each op as it computes.
    fn members(&self) -> Vec<&Op> {
        fn walk<'a>(op: &'a Op, out: &mut Vec<&'a Op>) {
            out.push(op);
            if let Sym::Fused(f) = &op.sym {
                f.parts.iter().for_each(|p| walk(p, out));
            }
        }
        let mut out = Vec::new();
        self.ops.iter().for_each(|op| walk(op, &mut out));
        out
    }

    /// Every tensor an op writes in place, however deep in a fused op it is written: what it updates as state, and every
    /// table or wider tensor a destination lands rows or units in.
    pub fn writes_in_place(&self) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for op in self.members() {
            for k in op.state_inputs() {
                out.insert(op.ins[k].clone());
            }
            if op.dst.row.is_some() || op.dst.unit.is_some() {
                out.insert(op.dst.buf.clone());
            }
        }
        out
    }

    /// The attention caches: every state tensor an attention reads its keys or values from, itself or through the
    /// head slices cut from it (a latent cache's values are its rows' first part).
    pub fn caches(&self) -> BTreeSet<String> {
        let members = self.members();
        let producer: BTreeMap<&str, &Op> = members.iter().map(|o| (o.out.as_str(), *o)).collect();
        let mut out = BTreeSet::new();
        for op in members.iter().filter(|o| matches!(o.sym, Sym::Prim(TensorOpSpec::Attention(_)))) {
            for read in &op.ins[1..3] {
                let mut t = read.as_str();
                loop {
                    if self.tensors.get(t).is_some_and(|x| x.role == Role::State) {
                        out.insert(t.to_string());
                        break;
                    }
                    match producer.get(t) {
                        Some(p) if matches!(p.sym, Sym::Prim(TensorOpSpec::HeadSlice(_))) => t = &p.ins[0],
                        _ => break,
                    }
                }
            }
        }
        out
    }
}
