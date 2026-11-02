//! The shared optimizer: one table of fusion rules over the one IR, decided once for every chip.
//!
//! Each rule is legal by dataflow alone — a producer's only reader, nothing between that writes what it reads — and
//! commits only when the chip's shim runs the result whole. Rules run in a fixed order, each to a fixed point over the
//! slots in turn, and any can be held back. A rule's result is a fused symbol defined by its parts, a part read in place
//! (a strided access), or a result landing in rows or units of another tensor (a destination): the meaning never changes,
//! and `tensor_graph::eval` computes the same values from the rewritten graph as from the lifted one.
//!
//! The rules keep the names and the order of the lane's table, so a compile answers `fired("add_norm")` as it always
//! has, and each fused op sits at the slot the lane's launch sits at, so the launch order does not move.

use std::collections::{BTreeMap, BTreeSet};

use crate::tensor_recognize::qwen::{Activation, MoeAct, RowIndex};
use crate::tensor_recognize::{ElementwiseOp, Epsilon, RowReduceKind};

use super::facts::{self, Slots};
use super::front::TensorOpSpec;
use super::graph::{Access, Dst, Fused, Op, Part, Role, RowSel, Storage, Strided, Sym, TensorGraph};
use super::shim::{Linking, Position, Shim, Support};
use super::symbols::ClassId;

/// A rule that fired: its name, the class it built, and the lifted ops the result computes.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Fired {
    pub rule: &'static str,
    pub class: ClassId,
    pub covers: Vec<usize>,
}

/// A graph after the shared rules, every rule that fired on it, and the report of every decision.
#[derive(Clone, Debug)]
pub struct Optimized {
    pub graph: TensorGraph,
    pub fired: Vec<Fired>,
    pub report: Report,
}

impl Optimized {
    /// How many times `rule` fired.
    pub fn fired(&self, rule: &str) -> usize {
        self.fired.iter().filter(|f| f.rule == rule).count()
    }
}

/// What became of a place a rule could fire.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    Fired,
    /// The chip does not run the result whole, and why.
    Refused(String),
    /// The compile held the rule back.
    Held,
}

/// One place a rule could fire — the class it builds over the lifted ops it would compute — and what became of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Decision {
    pub rule: &'static str,
    pub class: ClassId,
    pub covers: Vec<usize>,
    pub outcome: Outcome,
}

/// Every decision a compile made, in the order the table made them: the fusions a model received on a chip, and every
/// one it did not with the reason.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Report {
    pub decisions: Vec<Decision>,
}

impl Report {
    /// Each decision as one line: the rule, the class, the lifted ops, and what became of it.
    pub fn lines(&self) -> Vec<String> {
        self.decisions
            .iter()
            .map(|d| {
                let outcome = match &d.outcome {
                    Outcome::Fired => "fired".to_string(),
                    Outcome::Refused(why) => format!("refused: {why}"),
                    Outcome::Held => "held".to_string(),
                };
                format!("{} {} {:?} {outcome}", d.rule, d.class, d.covers)
            })
            .collect()
    }

    /// The report's sha256, as 64 lowercase hex digits: what an artifact records beside its program's identity.
    pub fn digest(&self) -> String {
        crate::crypto_accel::sha256(self.lines().join("\n").as_bytes()).iter().map(|b| format!("{b:02x}")).collect()
    }

    /// The decisions about `rule`.
    pub fn of(&self, rule: &str) -> Vec<&Decision> {
        self.decisions.iter().filter(|d| d.rule == rule).collect()
    }
}

/// What a rule does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RuleKind {
    /// Several ops as one.
    Group,
    /// A part read in place, or a result written in place.
    Place,
}

/// A chip's own rule: its name, the classes it builds, and what it does — reading the chip's own configuration.
pub struct ChipRule<'a> {
    pub name: &'static str,
    pub builds: &'static [ClassId],
    pub apply: Box<dyn Fn(&mut Rewriter) + 'a>,
}

/// One rule of the table.
pub struct Rule {
    pub name: &'static str,
    pub kind: RuleKind,
    /// The classes it builds.
    pub builds: &'static [ClassId],
    pub apply: fn(&mut Rewriter),
}

/// The graph being rewritten: its slots, the lifted graph's tensors, the chip's shim, what has fired, the rule running
/// and every decision made.
pub struct Rewriter<'a> {
    pub s: Slots,
    pub g: &'a TensorGraph,
    pub shim: &'a dyn Shim,
    pub fired: Vec<Fired>,
    pub rule: &'static str,
    /// The class the running rule builds, for a candidate that is a placement on a lifted op.
    pub building: ClassId,
    pub report: Vec<Decision>,
}

impl Rewriter<'_> {
    /// Whether the chip runs `op` whole: the shim reads the op against the graph's tensors. A candidate it does not run is
    /// reported refused, with the chip's reason.
    pub fn allows(&mut self, op: &Op) -> bool {
        let answer = super::shim::allows(self.shim, op, self.g);
        let native = answer.is_native();
        if !native {
            let why = match answer {
                super::shim::Allow::Refuse(why) => why,
                _ => format!("the chip runs `{}` as its parts", class(op).map_or(op.kind(), |c| c.id())),
            };
            let mut covers: Vec<usize> = leaves(op.clone()).iter().flat_map(|l| l.covers.clone()).collect();
            covers.sort_unstable();
            let decision = Decision { rule: self.rule, class: class(op).unwrap_or(self.building), covers, outcome: Outcome::Refused(why) };
            if self.report.last() != Some(&decision) {
                self.report.push(decision);
            }
        }
        native
    }

    /// The tensors the graph's host reads across steps: they stay whole wherever they are computed.
    pub fn carried(&self, t: &str) -> bool {
        self.g.io.as_ref().is_some_and(|io| io.names().contains(&t))
    }

    pub fn observed(&self, t: &str) -> bool {
        self.g.tensors.get(t).is_some_and(|x| x.observed) || self.carried(t)
    }

    pub fn record(&mut self, rule: &'static str, class: ClassId, at: usize) {
        let mut covers = self.s.get(at).covers.clone();
        covers.sort_unstable();
        self.report.push(Decision { rule, class, covers: covers.clone(), outcome: Outcome::Fired });
        self.fired.push(Fired { rule, class, covers });
    }

    /// Where `apply` would fire on the graph as it stands under `shim`, without firing it: each firing reported as `then`
    /// says — held, or refused because the chip does not link the class whole — and each refusal it meets reported as it
    /// is.
    fn dry(&mut self, apply: &dyn Fn(&mut Rewriter), shim: &dyn Shim, then: &dyn Fn(ClassId) -> Outcome) {
        let mut scratch = Rewriter { s: self.s.clone(), g: self.g, shim, fired: Vec::new(), rule: self.rule, building: self.building, report: Vec::new() };
        apply(&mut scratch);
        self.report.extend(scratch.report.into_iter().map(|d| match d.outcome {
            Outcome::Fired => Decision { outcome: then(d.class), ..d },
            _ => d,
        }));
    }
}

// ── Forms: what each op is, as the rules read it ───────────────────────────────────────────────────────

/// A matrix product's weights: rows of a checkpoint tensor in a block format, or values.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weights {
    Rows(crate::model::quant::GgmlType),
    Values,
}

/// The weights of a product by `w`, when `w` is a constant whose storage says how it is read.
pub fn weights(g: &TensorGraph, w: &str) -> Option<Weights> {
    match g.tensors.get(w).map(|t| (&t.role, &t.storage)) {
        Some((Role::Constant, Storage::Rows { format, .. })) => Some(Weights::Rows(*format)),
        Some((Role::Constant, Storage::Values)) => Some(Weights::Values),
        _ => None,
    }
}

/// A fused op's class, if any.
pub fn class(op: &Op) -> Option<ClassId> {
    match &op.sym {
        Sym::Fused(f) => Some(f.class),
        _ => None,
    }
}

fn part_of(a: Access) -> Option<Part> {
    match a {
        Access::Whole => None,
        Access::Strided(s) => Some(Part { pitch: s.pitch, offset: s.offset }),
    }
}

/// A norm: the input it reads (in place as a part of a wider tensor, or whole), the operand it adds first, the norm it
/// takes of that operand, whether it writes the sum on, its weight, rows and columns, epsilon, and where it lands.
#[derive(Clone, Debug)]
pub struct NormForm {
    pub x: String,
    pub part: Option<Part>,
    pub add: Option<String>,
    pub pre: bool,
    pub sum: bool,
    pub w: Option<String>,
    pub out: String,
    pub own: bool,
    pub rows: usize,
    pub cols: usize,
    pub eps: Epsilon,
}

/// The RMS norm an op is: a lifted norm without a bias, or a norm a rule built.
pub fn norm(op: &Op) -> Option<NormForm> {
    let rms = |spec: &TensorOpSpec| match spec {
        TensorOpSpec::FusedRow(r) if r.reduce == RowReduceKind::RmsNorm && r.bias.is_none() => Some(r.clone()),
        _ => None,
    };
    match &op.sym {
        Sym::Prim(spec) => {
            let r = rms(spec)?;
            let mut k = 1;
            let w = r.weight.map(|_| {
                k += 1;
                op.ins[k - 1].clone()
            });
            let add = r.residual.map(|_| op.ins[k].clone());
            Some(NormForm {
                x: op.ins[0].clone(),
                part: part_of(op.access[0]),
                add,
                pre: false,
                sum: false,
                w,
                out: op.out.clone(),
                own: op.dst.own().is_some(),
                rows: r.rows,
                cols: r.cols,
                eps: r.eps,
            })
        }
        Sym::Fused(f) if f.class == ClassId::AddNorm || f.class == ClassId::NormThenAdd => {
            let found = leaves(op.clone());
            let last = found.last()?;
            let r = rms(last.prim()?)?;
            let w = r.weight.map(|_| last.ins[1].clone());
            let sum = !op.keeps.is_empty();
            // The norm's input and the operand it adds: an add-then-norm's two summands, a sandwich's kept side and its
            // inner norm's input.
            let (x, add) = (op.ins[0].clone(), op.ins[1].clone());
            Some(NormForm {
                x,
                part: part_of(op.access[0]),
                add: Some(add),
                pre: f.class == ClassId::NormThenAdd,
                sum,
                w,
                out: op.out.clone(),
                own: op.dst.own().is_some(),
                rows: r.rows,
                cols: r.cols,
                eps: r.eps,
            })
        }
        _ => None,
    }
}

/// The elementwise op an op is: its operator, operands, the gate folded in, the row a bias is broadcast over, its length.
#[derive(Clone, Debug)]
pub struct BinaryForm {
    pub op: ElementwiseOp,
    pub a: String,
    pub b: String,
    pub gate: Option<Activation>,
    pub row: Option<usize>,
    pub out: String,
    pub n: usize,
}

pub fn binary(op: &Op) -> Option<BinaryForm> {
    match &op.sym {
        Sym::Prim(TensorOpSpec::Elementwise(e)) if op.dst.own().is_some() => {
            Some(BinaryForm { op: e.op, a: op.ins[0].clone(), b: op.ins[1].clone(), gate: None, row: e.row, out: op.out.clone(), n: e.n })
        }
        Sym::Fused(f) if f.class == ClassId::GluActMul && op.dst.own().is_some() => {
            let found = leaves(op.clone());
            let act = activation(found.first()?)?;
            let TensorOpSpec::Elementwise(e) = found.last()?.prim()? else { return None };
            Some(BinaryForm { op: e.op, a: op.ins[0].clone(), b: op.ins[1].clone(), gate: Some(act), row: e.row, out: op.out.clone(), n: e.n })
        }
        _ => None,
    }
}

/// The activation a lifted unary op applies.
pub fn activation(op: &Op) -> Option<Activation> {
    match op.prim()? {
        TensorOpSpec::Silu(_) => Some(Activation::Silu),
        TensorOpSpec::Sigmoid(_) => Some(Activation::Sigmoid),
        TensorOpSpec::Gelu(_) => Some(Activation::Gelu),
        _ => None,
    }
}

/// A matrix product by a constant: its activation, weights and how they are stored, its bias, where it lands, and its
/// shape (`k` in, `n` out, `m` rows).
#[derive(Clone, Debug)]
pub struct GemvForm {
    pub x: String,
    pub w: String,
    pub weights: Weights,
    pub bias: Option<String>,
    pub dst: Dst,
    pub k: usize,
    pub n: usize,
    pub m: usize,
}

pub fn gemv(op: &Op, g: &TensorGraph) -> Option<GemvForm> {
    let lifted = |p: &Op| match p.prim()? {
        TensorOpSpec::Matmul(m) => Some(m.clone()),
        _ => None,
    };
    let (m, bias) = match &op.sym {
        Sym::Prim(_) => (lifted(op)?, None),
        Sym::Fused(f) if f.class == ClassId::GemmBias => (lifted(leaves(op.clone()).first()?)?, Some(op.ins[2].clone())),
        _ => return None,
    };
    let w = op.ins[1].clone();
    Some(GemvForm { x: op.ins[0].clone(), weights: weights(g, &w)?, w, bias, dst: op.dst.clone(), k: m.k, n: m.n, m: m.m })
}

/// A rotation: its input (whole or as a part of every unit of a wider tensor), the per-head norm folded in, its tables,
/// its position, where it lands, and its heads (`per_row` a row), head width and rotated half.
#[derive(Clone, Debug)]
pub struct RopeForm {
    pub x: String,
    pub part: Option<Part>,
    pub norm: Option<(String, Epsilon)>,
    pub cos: String,
    pub sin: String,
    pub pos: Option<String>,
    pub dst: Dst,
    pub heads: usize,
    pub per_row: usize,
    pub hd: usize,
    pub half: usize,
}

pub fn rope(op: &Op) -> Option<RopeForm> {
    let (spec, norm, at) = match &op.sym {
        Sym::Prim(TensorOpSpec::Rope(r)) => (r.clone(), None, 0),
        Sym::Fused(f) if f.class == ClassId::QkNormRope => {
            let found = leaves(op.clone());
            let TensorOpSpec::FusedRow(n) = found.first()?.prim()? else { return None };
            let TensorOpSpec::Rope(r) = found.last()?.prim()? else { return None };
            (r.clone(), Some((op.ins[1].clone(), n.eps)), 1)
        }
        _ => return None,
    };
    let pos = spec.from.as_ref().and_then(|(p, _)| match p {
        RowIndex::Named(_) => Some(op.ins[at + 3].clone()),
        RowIndex::Literal(_) => None,
    });
    Some(RopeForm {
        x: op.ins[0].clone(),
        part: part_of(op.access[0]),
        norm,
        cos: op.ins[at + 1].clone(),
        sin: op.ins[at + 2].clone(),
        pos,
        dst: op.dst.clone(),
        heads: spec.h,
        per_row: spec.h / spec.rows.max(1),
        hd: spec.hd,
        half: spec.half,
    })
}

/// A head slice, or a row of a table taken as a copy: `y[g·width + i] = x[g·every + from + i]` over `groups` groups.
#[derive(Clone, Debug)]
pub struct SliceForm {
    pub x: String,
    pub y: String,
    pub groups: usize,
    pub every: usize,
    pub from: usize,
    pub width: usize,
}

pub fn slice(op: &Op) -> Option<SliceForm> {
    if op.dst.own().is_none() {
        return None;
    }
    match op.prim()? {
        TensorOpSpec::HeadSlice(h) => Some(SliceForm { x: op.ins[0].clone(), y: op.out.clone(), groups: h.n / h.every, every: h.every, from: h.from, width: h.width }),
        TensorOpSpec::RowGather(r) => match r.index {
            RowIndex::Literal(row) => Some(SliceForm {
                x: op.ins[0].clone(),
                y: op.out.clone(),
                groups: 1,
                every: r.rows * r.cols,
                from: (row as usize - 1) * r.cols,
                width: r.cols,
            }),
            RowIndex::Named(_) => None,
        },
        _ => None,
    }
}

/// The input an op reads unit by unit and its units, `(x, units, width)`, for an op that can read them as a part of
/// every unit of a wider tensor: a norm (not of what it adds), a rotation, a per-head product — none reading in place
/// already.
pub fn unit_input(op: &Op) -> Option<(String, usize, usize)> {
    if let Some(n) = norm(op) {
        return (n.part.is_none() && n.add.as_deref() != Some(n.x.as_str())).then_some((n.x, n.rows, n.cols));
    }
    if let Some(r) = rope(op) {
        return r.part.is_none().then_some((r.x, r.heads, r.hd));
    }
    if let Some(TensorOpSpec::HeadMatmul(m)) = op.prim() {
        return (op.access[0] == Access::Whole).then(|| (op.ins[0].clone(), m.m * m.heads, m.a));
    }
    None
}

/// The units an op writes its result in, `(units, width)`, when it can write them as a part of every unit of a wider
/// tensor: a norm, a rotation, a per-head product.
pub fn unit_output(op: &Op) -> Option<(usize, usize)> {
    if let Some(n) = norm(op) {
        return Some((n.rows, n.cols));
    }
    if let Some(r) = rope(op) {
        return Some((r.heads, r.hd));
    }
    if let Some(TensorOpSpec::HeadMatmul(m)) = op.prim() {
        return Some((m.m * m.heads, m.b));
    }
    None
}

/// The elements a row-writing producer writes, and the row width it writes them in: a matrix product, a rotation, a
/// mixture, a norm, a join, a per-head product.
pub fn written(op: &Op, g: &TensorGraph) -> Option<(usize, usize)> {
    if let Some(p) = gemv(op, g) {
        return Some((p.n * p.m, p.n));
    }
    if let Some(r) = rope(op) {
        return Some((r.heads * r.hd, r.per_row * r.hd));
    }
    if let Some(n) = norm(op) {
        return Some((n.rows * n.cols, n.cols));
    }
    match op.prim()? {
        TensorOpSpec::Moe(m) => Some((m.d, m.d)),
        TensorOpSpec::Join(j) => Some((j.rows * (j.wa + j.wb), j.wa + j.wb)),
        TensorOpSpec::HeadMatmul(m) => Some((m.m * m.heads * m.b, m.heads * m.b)),
        _ => match class(op) {
            Some(ClassId::SharedExpertFusion) | Some(ClassId::MoeTokenSort) => {
                let t = &g.tensors[&op.out];
                Some((t.len(), t.cols))
            }
            _ => None,
        },
    }
}

// ── Placements, carried down to the parts ──────────────────────────────────────────────────────────────

/// Land `op`'s result where `dst` says — on the op, and on the part of a fused op that computes its result, so the
/// placement survives the op being fused again.
pub fn set_dst(op: &mut Op, dst: Dst) {
    if let Sym::Fused(f) = &mut op.sym {
        if let Some(part) = f.parts.iter_mut().rev().find(|p| p.out == op.out) {
            set_dst(part, dst.clone());
        }
    }
    op.dst = dst;
}

/// Read `to` with `access` wherever `op` — or any part of it — reads `from`.
pub fn read_in_place(op: &mut Op, from: &str, to: &str, access: Access) {
    if let Sym::Fused(f) = &mut op.sym {
        for part in &mut f.parts {
            read_in_place(part, from, to, access);
        }
    }
    for k in 0..op.ins.len() {
        if op.ins[k] == from {
            op.ins[k] = to.to_string();
            op.access[k] = access;
        }
    }
}

// ── Building fused ops ─────────────────────────────────────────────────────────────────────────────────

/// The lifted ops a slot computes, as leaves: each member's in turn, in the order the members run.
pub fn leaves(op: Op) -> Vec<Op> {
    match op.sym {
        Sym::Fused(f) => f.parts.into_iter().flat_map(leaves).collect(),
        _ => vec![op],
    }
}

/// Every class `op` realizes: its own and each member's; the placements its reads and its result take — a part read in
/// place (an attention's values read in its key rows), a result written into rows of a table or into units of a wider
/// tensor — and the composites they make: a rotation landing in a cache's rows appends to it (`rope_kv_append`, and
/// `qk_norm_rope_kv_append` with its per-head norm folded in), a gated delta rule norms its output gated by the SiLU of
/// its gate (`gated_norm`), a gate and an up product of one activation run as one product (`merged_gate_up`: two
/// products side by side of one width, or a gated pair), a token-sorted mixture gathers each expert's rows inside its
/// product (`moe_gather_in_operand`), and a mixture with its shared expert folded in computes the router and the shared
/// expert's gate in one op (`router_shared_gate_merge`).
pub fn realizes(op: &Op, g: &TensorGraph) -> BTreeSet<ClassId> {
    let mut out = BTreeSet::new();
    if let Sym::Fused(f) = &op.sym {
        out.insert(f.class);
        for part in &f.parts {
            out.extend(realizes(part, g));
        }
        match f.class {
            ClassId::MoeTokenSort => {
                out.insert(ClassId::MoeGatherInOperand);
            }
            ClassId::SharedExpertFusion => {
                out.insert(ClassId::RouterSharedGateMerge);
            }
            _ => {}
        }
        let widths: Vec<usize> = f.parts.iter().filter_map(|p| gemv(p, g)).map(|p| p.n).collect();
        let pair = f.class == ClassId::MergedQkv && widths.len() == 2 && widths[0] == widths[1];
        if pair || f.class == ClassId::GemvGateUpGlu {
            out.insert(ClassId::MergedGateUp);
        }
    }
    for (k, a) in op.access.iter().enumerate() {
        if let Access::Strided(_) = a {
            let values = matches!(op.prim(), Some(TensorOpSpec::Attention(_))) && k == 2;
            out.insert(if values { ClassId::StridedValues } else { ClassId::StridedRead });
        }
    }
    if op.dst.row.is_some() {
        out.insert(ClassId::RowWrite);
        if let Some(ro) = rope(op) {
            out.insert(if ro.norm.is_some() { ClassId::QkNormRopeKvAppend } else { ClassId::RopeKvAppend });
        }
    }
    if op.dst.unit.is_some() {
        out.insert(ClassId::JoinInPlace);
    }
    if let Some(TensorOpSpec::GatedDelta(_)) = op.prim() {
        out.insert(ClassId::GatedNorm);
    }
    out
}

impl Rewriter<'_> {
    /// Fold the ops of `gone` into the op at `at` as one op of `class` defined by those ops as they stand — a member a
    /// rule built keeps its own class and parts — in the order their slots run, its operands `ins` (with their accesses), landing
    /// where `dst` says, keeping every result computed inside that anything else reads; one firing of `rule`.
    pub fn fuse(&mut self, rule: &'static str, class: ClassId, at: usize, gone: &[usize], ins: Vec<(String, Access)>, out: String, dst: Dst) {
        // The members in the order their slots run.
        let mut slots: Vec<usize> = std::iter::once(at).chain(gone.iter().copied()).collect();
        slots.sort_unstable();
        let parts: Vec<Op> = slots.into_iter().map(|k| self.s.take(k).expect("a live slot")).collect();
        let mut covers: Vec<usize> = parts.iter().flat_map(|o| o.covers.clone()).collect();
        covers.sort_unstable();
        // What the parts compute that something outside them reads, or the host does: it outlives the op.
        let inside: BTreeSet<String> = parts.iter().flat_map(|p| leaves(p.clone())).map(|l| l.out).collect();
        let keeps: Vec<String> = inside
            .iter()
            .filter(|t| **t != out && (self.observed(t) || !self.s.readers(t).is_empty()))
            .cloned()
            .collect();
        let (names, access): (Vec<String>, Vec<Access>) = ins.into_iter().unzip();
        let op = Op { sym: Sym::Fused(Fused { class, parts }), ins: names, access, out, dst, keeps, covers };
        self.s.put(at, op);
        self.record(rule, class, at);
    }

    /// Fold the slots of `gone` into the op at `at`, its symbol and operands unchanged: a placement, whose meaning the
    /// op's new access or destination carries.
    pub fn absorb(&mut self, rule: &'static str, class: ClassId, at: usize, gone: &[usize]) {
        let mut covers = self.s.get(at).covers.clone();
        for &g in gone {
            covers.extend(self.s.take(g).expect("a live slot").covers);
        }
        covers.sort_unstable();
        self.s.get_mut(at).covers = covers;
        self.record(rule, class, at);
    }

    /// Whether the candidate `op` would run whole on the chip.
    pub fn admits(&mut self, op: &Op) -> bool {
        self.allows(op)
    }
}

/// A candidate op for the shim to judge: `class` over `parts` as they stand — each a lifted op or one a rule built —
/// so the shim reads every member as it is.
pub fn candidate(class: ClassId, parts: Vec<Op>, ins: Vec<String>, out: &str, dst: Dst) -> Op {
    let n = ins.len();
    Op {
        sym: Sym::Fused(Fused { class, parts }),
        ins,
        access: vec![Access::Whole; n],
        out: out.to_string(),
        dst,
        keeps: Vec::new(),
        covers: Vec::new(),
    }
}

// ── The rules ──────────────────────────────────────────────────────────────────────────────────────────

/// A residual add whose sum an RMS norm reads first runs inside the norm, which writes the sum on for its later readers.
fn add_norm(r: &mut Rewriter) {
    for i in 0..r.s.len() {
        let Some(b) = r.s.slot(i).and_then(binary) else { continue };
        if b.op != ElementwiseOp::Add || b.gate.is_some() || b.row.is_some() {
            continue;
        }
        let readers = r.s.readers(&b.out);
        let Some(&j) = readers.first() else { continue };
        let Some(n) = norm(r.s.get(j)) else { continue };
        if n.x != b.out || n.part.is_some() || n.add.is_some() || n.rows * n.cols != b.n || class(r.s.get(j)).is_some() {
            continue;
        }
        let cand = candidate(ClassId::AddNorm, vec![r.s.get(i).clone(), r.s.get(j).clone()], vec![b.a.clone(), b.b.clone()], &n.out, r.s.get(j).dst.clone());
        if !r.admits(&cand) {
            continue;
        }
        let mut ins = vec![(b.a.clone(), r.s.get(i).access[0]), (b.b.clone(), r.s.get(i).access[1])];
        ins.extend(n.w.iter().map(|w| (w.clone(), Access::Whole)));
        let (out, dst) = (n.out.clone(), r.s.get(j).dst.clone());
        r.fuse("add_norm", ClassId::AddNorm, j, &[i], ins, out, dst);
    }
}

/// A norm of a sum whose added operand is itself a plain norm's whole output, read by nothing else: the first norm runs
/// inside the second, its operand normed in the same block. And a plain norm whose whole output is one summand of an
/// add no norm follows, read first by the add: the add in the norm's row op, the sum its result, the norm's output
/// written on for any op after the add that reads it.
fn sandwich_norm(r: &mut Rewriter) {
    sandwich_norms(r);
    norm_then_adds(r);
}

/// A plain norm of the sum's shape, `t` its whole output, read by op `j` — and, where `later` allows it, by ops after `j`,
/// for which the fold writes it on: its slot.
fn plain_norm(r: &Rewriter, t: &str, j: usize, rows: usize, cols: usize, later: bool) -> Option<usize> {
    let i = r.s.producer(t)?;
    let n = norm(r.s.get(i))?;
    let fits = n.out == t && n.part.is_none() && n.add.is_none() && !n.pre && !n.sum && n.own && class(r.s.get(i)).is_none() && n.rows == rows && n.cols == cols;
    let readers = r.s.readers(t);
    let read = readers.first() == Some(&j) && (readers.len() == 1 || (later && !r.carried(t)));
    (fits && read).then_some(i)
}

fn norm_then_adds(r: &mut Rewriter) {
    for j in 0..r.s.len() {
        let Some(b) = r.s.slot(j).and_then(binary) else { continue };
        if b.op != ElementwiseOp::Add || b.gate.is_some() || b.row.is_some() || r.s.get(j).dst.own().is_none() {
            continue;
        }
        let shape = |t: &str| r.s.producer(t).and_then(|i| norm(r.s.get(i))).map(|n| (n.rows, n.cols)).filter(|(rows, cols)| rows * cols == b.n);
        let found = [(b.b.clone(), b.a.clone()), (b.a.clone(), b.b.clone())]
            .into_iter()
            .find_map(|(normed, kept)| shape(&normed).and_then(|(rows, cols)| plain_norm(r, &normed, j, rows, cols, true)).map(|i| (i, kept)));
        let Some((i, kept)) = found else { continue };
        let inner = norm(r.s.get(i)).expect("a plain norm");
        if kept == inner.out || !r.s.untouched(&inner.x, i, j, false, &[]) {
            continue;
        }
        let cand = candidate(ClassId::NormThenAdd, vec![r.s.get(i).clone(), r.s.get(j).clone()], vec![kept.clone(), inner.x.clone()], &b.out, r.s.get(j).dst.clone());
        if !r.admits(&cand) {
            continue;
        }
        let kept_access = if kept == b.a { r.s.get(j).access[0] } else { r.s.get(j).access[1] };
        let mut ins = vec![(kept, kept_access), (inner.x.clone(), r.s.get(i).access[0])];
        ins.extend(inner.w.iter().map(|w| (w.clone(), Access::Whole)));
        let (out, dst) = (b.out.clone(), r.s.get(j).dst.clone());
        r.fuse("sandwich_norm", ClassId::NormThenAdd, j, &[i], ins, out, dst);
    }
}

fn sandwich_norms(r: &mut Rewriter) {
    for j in 0..r.s.len() {
        let Some(outer) = r.s.slot(j).and_then(norm) else { continue };
        let Some(add) = outer.add.clone() else { continue };
        if outer.pre {
            continue;
        }
        let plain = |r: &Rewriter, t: &str| plain_norm(r, t, j, outer.rows, outer.cols, false);
        let (i, kept) = match (plain(r, &add), plain(r, &outer.x)) {
            (Some(i), _) => (i, outer.x.clone()),
            (None, Some(i)) if outer.part.is_none() => (i, add.clone()),
            _ => continue,
        };
        let inner = norm(r.s.get(i)).expect("a plain norm");
        if !r.s.untouched(&inner.x, i, j, false, &[]) {
            continue;
        }
        let cand = candidate(ClassId::NormThenAdd, vec![r.s.get(i).clone(), r.s.get(j).clone()], vec![kept.clone(), inner.x.clone()], &outer.out, r.s.get(j).dst.clone());
        if !r.admits(&cand) {
            continue;
        }
        // The kept side keeps its access; the inner norm's input is read whole.
        let kept_access = if kept == outer.x { r.s.get(j).access[0] } else { Access::Whole };
        let mut ins = vec![(kept, kept_access), (inner.x.clone(), r.s.get(i).access[0])];
        ins.extend(inner.w.iter().map(|w| (w.clone(), Access::Whole)));
        ins.extend(outer.w.iter().map(|w| (w.clone(), Access::Whole)));
        let (out, dst) = (outer.out.clone(), r.s.get(j).dst.clone());
        r.fuse("sandwich_norm", ClassId::NormThenAdd, j, &[i], ins, out, dst);
    }
}

/// A constant bias added to a product's every row, the add's one input, inside the product.
fn gemv_bias(r: &mut Rewriter) {
    for j in 0..r.s.len() {
        let Some(b) = r.s.slot(j).and_then(binary) else { continue };
        if b.op != ElementwiseOp::Add || b.gate.is_some() {
            continue;
        }
        let Some(i) = r.s.producer(&b.a) else { continue };
        let Some(p) = gemv(r.s.get(i), r.g) else { continue };
        let constant = weights(r.g, &b.b) == Some(Weights::Values) && r.g.tensors.get(&b.b).map(|t| t.len()) == Some(b.row.unwrap_or(b.n));
        let fits = p.bias.is_none() && p.dst.row.is_none() && b.row.unwrap_or(b.n) == p.n;
        if !(constant && fits && r.s.readers(&b.a) == [j]) {
            continue;
        }
        let cand = candidate(ClassId::GemmBias, vec![r.s.get(i).clone(), r.s.get(j).clone()], vec![p.x.clone(), p.w.clone(), b.b.clone()], &b.out, Dst::to(&b.out));
        if !r.admits(&cand) {
            continue;
        }
        let ins = vec![(p.x.clone(), r.s.get(i).access[0]), (p.w.clone(), Access::Whole), (b.b.clone(), Access::Whole)];
        let out = b.out.clone();
        r.fuse("gemv_bias", ClassId::GemmBias, i, &[j], ins, out.clone(), Dst::to(&out));
    }
}

/// A per-head RMS norm read only by a rotation runs inside the rotation.
fn head_norm_rope(r: &mut Rewriter) {
    for j in 0..r.s.len() {
        let Some(ro) = r.s.slot(j).and_then(rope) else { continue };
        if ro.norm.is_some() || class(r.s.get(j)).is_some() {
            continue;
        }
        let Some(i) = r.s.producer(&ro.x) else { continue };
        let Some(n) = norm(r.s.get(i)) else { continue };
        let fits = n.part.is_none() && n.add.is_none() && n.w.is_some() && n.own && class(r.s.get(i)).is_none() && n.rows == ro.heads && n.cols == ro.hd;
        if !(fits && r.s.readers(&ro.x) == [j]) {
            continue;
        }
        let cand = candidate(ClassId::QkNormRope, vec![r.s.get(i).clone(), r.s.get(j).clone()], vec![n.x.clone()], &ro.dst.buf, ro.dst.clone());
        if !r.admits(&cand) {
            continue;
        }
        let rope_op = r.s.get(j).clone();
        let mut ins = vec![(n.x.clone(), r.s.get(i).access[0]), (n.w.clone().expect("a weight"), Access::Whole)];
        ins.extend(rope_op.ins[1..].iter().map(|t| (t.clone(), Access::Whole)));
        r.fuse("head_norm_rope", ClassId::QkNormRope, j, &[i], ins, rope_op.out.clone(), rope_op.dst.clone());
    }
}

/// A part of every unit of a tensor read only by an op that reads its input unit by unit is read where it lies, the
/// slice no op.
fn slice_read(r: &mut Rewriter) {
    for i in 0..r.s.len() {
        let Some(sl) = r.s.slot(i).and_then(slice) else { continue };
        let readers = r.s.readers(&sl.y);
        let [j] = readers[..] else { continue };
        let untouched = r.s.untouched(&sl.x, i, j, false, &[]);
        if !(unit_input(r.s.get(j)) == Some((sl.y.clone(), sl.groups, sl.width)) && untouched) {
            continue;
        }
        let mut cand = r.s.get(j).clone();
        read_in_place(&mut cand, &sl.y, &sl.x, Access::Strided(Strided { pitch: sl.every, offset: sl.from, width: sl.width }));
        if !r.admits(&cand) {
            continue;
        }
        r.s.put(j, cand);
        r.absorb("slice_read", ClassId::StridedRead, j, &[i]);
    }
}

/// A value read only by the row write(s) that put it in a table is written straight into the table's rows by the op
/// that computes it.
fn row_write(r: &mut Rewriter) {
    for j in 0..r.s.len() {
        let Some(Sym::Prim(TensorOpSpec::SetRow(sr))) = r.s.slot(j).map(|o| &o.sym) else { continue };
        let set = r.s.get(j).clone();
        let (table, src) = (set.ins[0].clone(), set.ins[1].clone());
        let index = match sr.index {
            RowIndex::Named(_) => RowSel::Param(set.ins[2].clone()),
            RowIndex::Literal(n) => RowSel::Fixed(n as usize),
        };
        let (cols, count) = (sr.cols, sr.count);
        let Some(i) = r.s.producer(&src) else { continue };
        let writes_rows = written(r.s.get(i), r.g) == Some((count * cols, cols));
        let untouched = r.s.untouched(&table, i, j, true, &[]);
        if !(writes_rows && untouched && r.s.readers(&src) == [j]) {
            continue;
        }
        let mut cand = r.s.get(i).clone();
        set_dst(&mut cand, Dst { buf: table.clone(), row: Some((index, cols)), unit: None });
        if !r.admits(&cand) {
            continue;
        }
        r.s.put(i, cand);
        r.absorb("row_write", ClassId::RowWrite, i, &[j]);
    }
}

/// The two halves of a join, each read only by it, are written by their producers straight into their parts of every
/// unit of the joined tensor — of the table's rows, when the join itself lands in one.
fn join_in_place(r: &mut Rewriter) {
    for j in 0..r.s.len() {
        let Some(TensorOpSpec::Join(jn)) = r.s.slot(j).and_then(|o| o.prim()) else { continue };
        let join_op = r.s.get(j).clone();
        let (a, b, y) = (join_op.ins[0].clone(), join_op.ins[1].clone(), join_op.dst.clone());
        let (ha, hb) = (jn.wa / jn.heads, jn.wb / jn.heads);
        let (Some(pa), Some(pb)) = (r.s.producer(&a), r.s.producer(&b)) else { continue };
        let units = jn.rows * jn.heads;
        let len = |t: &str| r.g.tensors.get(t).map(|t| t.len());
        let fits = pa != pb
            && y.unit.is_none()
            && len(&a) == Some(units * ha)
            && len(&b) == Some(units * hb)
            && r.s.readers(&a) == [j]
            && r.s.readers(&b) == [j];
        let row = match &y.row {
            Some((RowSel::Param(p), _)) => Some(p.clone()),
            _ => None,
        };
        let lo = pa.min(pb);
        let untouched = r.s.live().filter(|(k, _)| *k >= lo && *k < j).all(|(k, o)| {
            let other = k != pa && k != pb;
            let (rd, wr) = (super::facts::reads(o), super::facts::writes(o));
            !rd.contains(&y.buf.as_str()) && (!other || !wr.contains(&y.buf.as_str())) && row.as_deref().is_none_or(|p| !wr.contains(&p))
        });
        if !(fits && untouched) {
            continue;
        }
        let (mut ca, mut cb) = (r.s.get(pa).clone(), r.s.get(pb).clone());
        set_dst(&mut ca, Dst { unit: Some(Strided { pitch: ha + hb, offset: 0, width: ha }), ..y.clone() });
        set_dst(&mut cb, Dst { unit: Some(Strided { pitch: ha + hb, offset: ha, width: hb }), ..y.clone() });
        if !(r.admits(&ca) && r.admits(&cb)) {
            continue;
        }
        r.s.put(pa, ca);
        r.s.put(pb, cb);
        r.absorb("join_in_place", ClassId::JoinInPlace, pa.max(pb), &[j]);
    }
}

/// A convolution: its input, weights, ring, output, channels, taps, rows, whether a SiLU and a per-head L2 norm follow
/// inside it.
#[derive(Clone, Debug)]
pub struct ConvForm {
    pub x: String,
    pub w: String,
    pub ring: String,
    pub y: String,
    pub chan: usize,
    pub taps: usize,
    pub rows: usize,
    pub silu: bool,
    pub l2: Option<(usize, Epsilon)>,
}

pub fn conv(op: &Op) -> Option<ConvForm> {
    let lifted = |p: &Op| match p.prim()? {
        TensorOpSpec::CausalConv(c) => Some(c.clone()),
        _ => None,
    };
    let (c, silu, l2) = match &op.sym {
        Sym::Prim(_) => (lifted(op)?, false, None),
        Sym::Fused(f) if f.class == ClassId::ConvBiasSilu => (lifted(leaves(op.clone()).first()?)?, true, None),
        Sym::Fused(f) if f.class == ClassId::ConvSiluL2norm => {
            let found = leaves(op.clone());
            let TensorOpSpec::L2Norm(l) = found.last()?.prim()? else { return None };
            (lifted(found.first()?)?, true, Some((l.cols, l.eps)))
        }
        _ => return None,
    };
    Some(ConvForm { x: op.ins[0].clone(), w: op.ins[1].clone(), ring: op.ins[2].clone(), y: op.out.clone(), chan: c.chan, taps: c.taps, rows: c.rows, silu, l2 })
}

/// A causal convolution's SiLU, read only by it, in the convolution's epilogue.
fn conv_silu(r: &mut Rewriter) {
    for i in 0..r.s.len() {
        let Some(c) = r.s.slot(i).and_then(conv) else { continue };
        if c.silu {
            continue;
        }
        let readers = r.s.readers(&c.y);
        let [j] = readers[..] else { continue };
        if activation(r.s.get(j)) != Some(Activation::Silu) {
            continue;
        }
        let out = r.s.get(j).out.clone();
        let cand = candidate(ClassId::ConvBiasSilu, vec![r.s.get(i).clone(), r.s.get(j).clone()], vec![c.x.clone(), c.w.clone(), c.ring.clone()], &out, Dst::to(&out));
        if !r.admits(&cand) {
            continue;
        }
        let ins = vec![(c.x, r.s.get(i).access[0]), (c.w, Access::Whole), (c.ring, Access::Whole)];
        r.fuse("conv_silu", ClassId::ConvBiasSilu, i, &[j], ins, out.clone(), Dst::to(&out));
    }
}

/// The per-head L2 norm after a convolution's SiLU, read only by it, in the convolution's epilogue.
fn conv_silu_l2(r: &mut Rewriter) {
    for i in 0..r.s.len() {
        let Some(c) = r.s.slot(i).and_then(conv) else { continue };
        if !c.silu || c.l2.is_some() {
            continue;
        }
        let readers = r.s.readers(&c.y);
        let [k] = readers[..] else { continue };
        let Some(TensorOpSpec::L2Norm(l)) = r.s.get(k).prim() else { continue };
        if l.rows * l.cols != c.chan * c.rows {
            continue;
        }
        let out = r.s.get(k).out.clone();
        let cand = candidate(ClassId::ConvSiluL2norm, vec![r.s.get(i).clone(), r.s.get(k).clone()], vec![c.x.clone(), c.w.clone(), c.ring.clone()], &out, Dst::to(&out));
        if !r.admits(&cand) {
            continue;
        }
        let ins = vec![(c.x, r.s.get(i).access[0]), (c.w, Access::Whole), (c.ring, Access::Whole)];
        r.fuse("conv_silu_l2", ClassId::ConvSiluL2norm, i, &[k], ins, out.clone(), Dst::to(&out));
    }
}

/// A step's causal convolutions as one op over their channels, each a convolution moved up to the first one's place only
/// where nothing between them writes what it reads or touches what it writes; the chip says which it takes together.
fn conv_parts(r: &mut Rewriter) {
    let convs: Vec<usize> = r.s.live().filter(|(_, o)| conv(o).is_some()).map(|(i, _)| i).collect();
    for (at, &i) in convs.iter().enumerate() {
        if r.s.slot(i).and_then(conv).is_none() {
            continue;
        }
        let mut group = vec![i];
        for &j in &convs[at + 1..] {
            if r.s.slot(j).and_then(conv).is_none() || r.s.crosses(i, j, &group) {
                continue;
            }
            let members: Vec<Op> = group.iter().chain([&j]).map(|&k| r.s.get(k).clone()).collect();
            let ins: Vec<String> = members.iter().flat_map(|m| m.ins.iter().cloned()).collect();
            let out = members[0].out.clone();
            if r.admits(&candidate(ClassId::ConvParts, members, ins, &out, Dst::to(&out))) {
                group.push(j);
            }
        }
        if group.len() < 2 {
            continue;
        }
        let members: Vec<Op> = group.iter().map(|&k| r.s.get(k).clone()).collect();
        let ins: Vec<(String, Access)> = members.iter().flat_map(|m| m.ins.iter().cloned().zip(m.access.iter().copied())).collect();
        let out = members[0].out.clone();
        r.fuse("conv_parts", ClassId::ConvParts, i, &group[1..], ins, out.clone(), Dst::to(&out));
    }
}

/// Gated DeltaNet's write strength and forget gate in one op, at the later gate's place.
fn gates(r: &mut Rewriter) {
    for j in 0..r.s.len() {
        let Some(TensorOpSpec::GatedDelta(_)) = r.s.slot(j).and_then(|o| o.prim()) else { continue };
        let (b, a) = (r.s.get(j).ins[4].clone(), r.s.get(j).ins[5].clone());
        let (Some(pb), Some(pa)) = (r.s.producer(&b), r.s.producer(&a)) else { continue };
        let (Some(TensorOpSpec::Sigmoid(sg)), Some(TensorOpSpec::Decay(dc))) = (r.s.get(pb).prim().cloned(), r.s.get(pa).prim().cloned()) else { continue };
        if sg.n != dc.n || r.s.readers(&b) != [j] || r.s.readers(&a) != [j] {
            continue;
        }
        let (keep, gone) = (pb.max(pa), pb.min(pa));
        let (sop, dop) = (r.s.get(pb).clone(), r.s.get(pa).clone());
        let ins: Vec<(String, Access)> = [&sop, &dop].iter().flat_map(|o| o.ins.iter().cloned().zip(o.access.iter().copied())).collect();
        let cand = candidate(ClassId::GdnGating, vec![sop, dop], ins.iter().map(|(t, _)| t.clone()).collect(), &b, Dst::to(&b));
        if !r.admits(&cand) {
            continue;
        }
        r.fuse("gates", ClassId::GdnGating, keep, &[gone], ins, b.clone(), Dst::to(&b));
    }
}

/// A step's two Gated DeltaNet gates and the two value-weight products of one activation row they read, read by nothing
/// else — no bias, each whole, one length of `k` — as one op at the gates' place. The activation must still hold there.
fn gate_projections(r: &mut Rewriter) {
    for g in 0..r.s.len() {
        let Some(op) = r.s.slot(g) else { continue };
        if class(op) != Some(ClassId::GdnGating) {
            continue;
        }
        let Sym::Fused(f) = &op.sym else { continue };
        let (Some(TensorOpSpec::Sigmoid(_)), Some(TensorOpSpec::Decay(dc))) = (f.parts[0].prim(), f.parts[1].prim()) else { continue };
        let (br, ar) = (f.parts[0].ins[0].clone(), f.parts[1].ins[0].clone());
        let heads = dc.n / dc.rows.max(1);
        if dc.n != heads || r.s.readers(&br) != [g] || r.s.readers(&ar) != [g] {
            continue;
        }
        let (Some(ib), Some(ia)) = (r.s.producer(&br), r.s.producer(&ar)) else { continue };
        let projection = |i: usize| -> Option<(String, String, usize)> {
            let p = gemv(r.s.get(i), r.g)?;
            (p.bias.is_none() && p.dst.own().is_some() && p.n == heads && p.m == 1).then(|| (p.x, p.w, p.k))
        };
        let (Some((xb, _, kb)), Some((xa, _, ka))) = (projection(ib), projection(ia)) else { continue };
        if xb != xa || kb != ka || !r.s.untouched(&xb, ib.min(ia), g, false, &[]) {
            continue;
        }
        let members = vec![r.s.get(ib).clone(), r.s.get(ia).clone(), r.s.get(g).clone()];
        let mut ins: Vec<(String, Access)> = vec![(xb.clone(), r.s.get(ib).access[0]), (r.s.get(ib).ins[1].clone(), Access::Whole), (r.s.get(ia).ins[1].clone(), Access::Whole)];
        ins.extend(f.parts[1].ins[1..].iter().map(|t| (t.clone(), Access::Whole)));
        let out = r.s.get(g).out.clone();
        let cand = candidate(ClassId::GdnGateProjections, members, ins.iter().map(|(t, _)| t.clone()).collect(), &out, Dst::to(&out));
        if !r.admits(&cand) {
            continue;
        }
        r.fuse("gate_projections", ClassId::GdnGateProjections, g, &[ib, ia], ins, out.clone(), Dst::to(&out));
    }
}

/// An activation (an output gate's sigmoid, a GeLU or SiLU gate) inside the product it gates: either factor may be the
/// activation's, since the product is the same with the operands swapped.
fn act_gate(r: &mut Rewriter) {
    for j in 0..r.s.len() {
        let Some(bf) = r.s.slot(j).and_then(binary) else { continue };
        if bf.op != ElementwiseOp::Mul || bf.gate.is_some() || bf.row.is_some() || class(r.s.get(j)).is_some() {
            continue;
        }
        let gated = [(bf.b.clone(), bf.a.clone(), 0usize), (bf.a.clone(), bf.b.clone(), 1usize)].into_iter().find_map(|(act_out, other, other_k)| {
            let i = r.s.producer(&act_out)?;
            activation(r.s.get(i))?;
            (r.s.readers(&act_out) == [j] && class(r.s.get(i)).is_none()).then_some((i, other, other_k))
        });
        let Some((i, other, other_k)) = gated else { continue };
        let x = r.s.get(i).ins[0].clone();
        let ins = vec![(other.clone(), r.s.get(j).access[other_k]), (x.clone(), r.s.get(i).access[0])];
        let cand = candidate(ClassId::GluActMul, vec![r.s.get(i).clone(), r.s.get(j).clone()], vec![other, x], &bf.out, Dst::to(&bf.out));
        if !r.admits(&cand) {
            continue;
        }
        r.fuse("act_gate", ClassId::GluActMul, j, &[i], ins, bf.out.clone(), Dst::to(&bf.out));
    }
}

/// An attention's whole result, read by nothing but its product with a gate's activation (a gated attention's output
/// gate) and unseen by the host, gated in the attention's place: the gate applied as the attention stores its output. A
/// batch's attentions — one a sequence, each result set whole into its row of one table that nothing but those sets and
/// the product touches, every row of which the product gates — are gated with their sets in the product's place: each
/// attention gating its row by the gate's row as it stores it, into its row of the product.
fn attn_output_gate(r: &mut Rewriter) {
    for j in 0..r.s.len() {
        let Some(op) = r.s.slot(j) else { continue };
        if class(op) != Some(ClassId::GluActMul) || op.access.iter().any(|a| *a != Access::Whole) {
            continue;
        }
        let Some(bf) = binary(op).filter(|b| b.op == ElementwiseOp::Mul && b.row.is_none()) else { continue };
        match r.s.producer(&bf.a) {
            Some(i) if attention(r.s.get(i)) => {
                if r.s.readers(&bf.a) == [j] && !r.observed(&bf.a) && !r.s.sinks_past(i, j, &[]) {
                    pair(r, "attn_output_gate", ClassId::AttnOutputGate, i, j, j);
                }
            }
            _ => {
                let gate_len = r.g.tensors.get(&bf.b).map(|t| t.len());
                if let Some(mut members) = attended_rows(r, &bf.a, j).filter(|_| gate_len == Some(bf.n)) {
                    members.push(j);
                    group_at_last(r, "attn_output_gate", ClassId::AttnOutputGate, members);
                }
            }
        }
    }
}

/// Whether a gated attention's parts gate a batch's rows (`attn_output_gate`): its attentions' results set into rows of
/// the table its product gates, not one attention's result gated whole.
pub fn gates_rows(parts: &[Op]) -> bool {
    parts.iter().any(|p| matches!(p.prim(), Some(TensorOpSpec::SetRow(_))))
}

/// Whether `op` is a lifted attention writing its whole result into a buffer of its own.
fn attention(op: &Op) -> bool {
    matches!(op.prim(), Some(TensorOpSpec::Attention(_))) && op.dst.own().is_some()
}

/// The attentions and row sets that fill `table` — each of its rows set whole, once, by a literal index, from the result
/// of an attention only that set reads and the host does not see — when nothing but those sets writes the table and
/// nothing but they and slot `j` reads it, and neither the host nor a later step sees it: each attention and its set, in
/// the order the sets run.
fn attended_rows(r: &Rewriter, table: &str, j: usize) -> Option<Vec<usize>> {
    let t = r.g.tensors.get(table)?;
    if t.carried || r.observed(table) {
        return None;
    }
    let sets: Vec<usize> = r.s.live().filter(|(_, op)| facts::writes(op).contains(&table)).map(|(k, _)| k).collect();
    let (mut rows, mut members) = (BTreeSet::new(), Vec::new());
    for &k in &sets {
        let set = r.s.get(k);
        let Some(TensorOpSpec::SetRow(sr)) = set.prim() else { return None };
        let RowIndex::Literal(row) = sr.index else { return None };
        if (sr.rows, sr.cols, sr.count) != (t.rows, t.cols, 1) || set.ins[0] != table || set.dst.own().is_none() {
            return None;
        }
        let src = &set.ins[1];
        let i = r.s.producer(src).filter(|&i| attention(r.s.get(i)) && r.s.readers(src) == [k] && !r.observed(src))?;
        if !rows.insert(row) {
            return None;
        }
        members.extend([i, k]);
    }
    let whole = rows.len() == t.rows && rows.iter().copied().eq(1..=t.rows as i64);
    let touched = r.s.readers(table).iter().all(|k| *k == j || sets.contains(k));
    (whole && touched).then_some(members)
}

/// A gated pair: two products of one activation and the gate between them (a SwiGLU, or a gate inside its product), as
/// one op at the gate's place. The activation must still hold there.
fn glu_pair(r: &mut Rewriter) {
    for j in 0..r.s.len() {
        let Some(op) = r.s.slot(j) else { continue };
        let (g, u, len) = match op.prim() {
            Some(TensorOpSpec::SwiGlu(sw)) if op.dst.own().is_some() => (op.ins[0].clone(), op.ins[1].clone(), sw.total),
            _ => match binary(op) {
                Some(b) if b.op == ElementwiseOp::Mul && b.gate.is_some() && b.row.is_none() => (b.b, b.a, b.n),
                _ => continue,
            },
        };
        let out = op.out.clone();
        let (Some(pg), Some(pu)) = (r.s.producer(&g), r.s.producer(&u)) else { continue };
        let (Some(fg), Some(fu)) = (gemv(r.s.get(pg), r.g), gemv(r.s.get(pu), r.g)) else { continue };
        let fits = pg != pu
            && fg.x == fu.x
            && (fg.k, fg.n, fg.m) == (fu.k, fu.n, fu.m)
            && fg.m * fg.n == len
            && fg.dst.own().is_some()
            && fu.dst.own().is_some()
            && r.s.untouched(&fg.x, pg.min(pu), j, false, &[])
            && r.s.readers(&g) == [j]
            && r.s.readers(&u) == [j];
        if !fits {
            continue;
        }
        let members = vec![r.s.get(pg).clone(), r.s.get(pu).clone(), r.s.get(j).clone()];
        let mut ins: Vec<(String, Access)> = vec![(fg.x.clone(), r.s.get(pg).access[0]), (fg.w.clone(), Access::Whole), (fu.w.clone(), Access::Whole)];
        ins.extend(fg.bias.iter().chain(fu.bias.iter()).map(|b| (b.clone(), Access::Whole)));
        let cand = candidate(ClassId::GemvGateUpGlu, members, ins.iter().map(|(t, _)| t.clone()).collect(), &out, Dst::to(&out));
        if !r.admits(&cand) {
            continue;
        }
        r.fuse("glu_pair", ClassId::GemvGateUpGlu, j, &[pg, pu], ins, out.clone(), Dst::to(&out));
    }
}

/// A mixture of experts, its shared expert folded in or not, and its rows: the lifted mixture, one a rule built, or a
/// block's mixtures as one.
#[derive(Clone, Debug)]
pub struct MoeForm {
    pub x: String,
    pub dst: Dst,
    pub d: usize,
    pub k: usize,
    pub act: MoeAct,
    pub rows: usize,
    pub shared: bool,
}

pub fn moe(op: &Op) -> Option<MoeForm> {
    let (m, shared, rows) = match &op.sym {
        Sym::Prim(TensorOpSpec::Moe(m)) => (m.clone(), false, 1),
        Sym::Fused(f) if f.class == ClassId::SharedExpertFusion || f.class == ClassId::MoeTokenSort => {
            let found = leaves(op.clone());
            let m = found.iter().find_map(|p| match p.prim()? {
                TensorOpSpec::Moe(m) => Some(m.clone()),
                _ => None,
            })?;
            match f.class {
                ClassId::SharedExpertFusion => (m, true, 1),
                _ => (m, false, found.iter().filter(|p| matches!(p.prim(), Some(TensorOpSpec::Moe(_)))).count()),
            }
        }
        _ => return None,
    };
    Some(MoeForm { x: op.ins[0].clone(), dst: op.dst.clone(), d: m.d, k: m.k, act: m.act, rows, shared })
}

/// A mixture's shared SwiGLU expert, read only by its sum with the mixture, inside the mixture: its gate and up rows
/// beside the kept experts', its down row summed after theirs.
fn shared_expert(r: &mut Rewriter) {
    for j in 0..r.s.len() {
        let Some(bf) = r.s.slot(j).and_then(binary) else { continue };
        if bf.op != ElementwiseOp::Add || bf.gate.is_some() || bf.row.is_some() || class(r.s.get(j)).is_some() {
            continue;
        }
        let parts = [(bf.a.clone(), bf.b.clone()), (bf.b.clone(), bf.a.clone())].into_iter().find_map(|(mixed, shared)| {
            let (pm, pd) = (r.s.producer(&mixed)?, r.s.producer(&shared)?);
            moe(r.s.get(pm)).filter(|m| !m.shared).map(|_| (mixed, shared, pm, pd))
        });
        let Some((mixed, shared, pm, pd)) = parts else { continue };
        let mf = moe(r.s.get(pm)).expect("a mixture");
        let Some(down) = gemv(r.s.get(pd), r.g) else { continue };
        let h = down.x.clone();
        let Some(pg) = r.s.producer(&h) else { continue };
        if class(r.s.get(pg)) != Some(ClassId::GemvGateUpGlu) {
            continue;
        }
        let glu = r.s.get(pg).clone();
        let Sym::Fused(gf) = &glu.sym else { continue };
        let first = gf.parts.iter().find_map(|p| gemv(p, r.g));
        let Some(gm) = first else { continue };
        let fits = glu.ins[0] == mf.x
            && (gm.k, gm.n, down.n, down.m * down.n) == (mf.d, down.k, mf.d, bf.n)
            && gm.m == down.m
            && mf.dst.own().is_some()
            && down.dst.own().is_some()
            && r.s.readers(&mixed) == [j]
            && r.s.readers(&shared) == [j]
            && r.s.readers(&h) == [pd]
            && r.s.untouched(&mf.x, pm.min(pg), j, false, &[]);
        if !fits {
            continue;
        }
        let members = vec![r.s.get(pm).clone(), r.s.get(pg).clone(), r.s.get(pd).clone(), r.s.get(j).clone()];
        let mut ins: Vec<(String, Access)> = r.s.get(pm).ins.iter().cloned().zip(r.s.get(pm).access.iter().copied()).collect();
        ins.extend(glu.ins[1..].iter().map(|t| (t.clone(), Access::Whole)));
        ins.push((down.w.clone(), Access::Whole));
        let cand = candidate(ClassId::SharedExpertFusion, members, ins.iter().map(|(t, _)| t.clone()).collect(), &bf.out, Dst::to(&bf.out));
        if !r.admits(&cand) {
            continue;
        }
        r.fuse("shared_expert", ClassId::SharedExpertFusion, j, &[pm, pg, pd], ins, bf.out.clone(), Dst::to(&bf.out));
    }
}

/// A block's mixtures — one a row of a stacked tensor, read through a view, writing the same row of a table — as one
/// mixture over the rows, each routed on its own, each expert's weights read once for every row that chose it.
fn moe_rows(r: &mut Rewriter) {
    let mut groups: BTreeMap<(String, String), Vec<(usize, usize, usize)>> = BTreeMap::new();
    for (i, op) in r.s.live() {
        let Some(mf) = moe(op) else { continue };
        if mf.rows != 1 || mf.shared {
            continue;
        }
        let Some((RowSel::Fixed(at), cols)) = &mf.dst.row else { continue };
        if mf.dst.unit.is_some() {
            continue;
        }
        let Some(v) = r.s.producer(&mf.x) else { continue };
        let Sym::View { offset } = r.s.get(v).sym else { continue };
        if *cols == mf.d && offset % mf.d == 0 && *at == offset / mf.d + 1 {
            groups.entry((r.s.get(v).ins[0].clone(), mf.dst.buf.clone())).or_default().push((i, v, offset / mf.d));
        }
    }
    for ((src, table), mut members) in groups {
        members.sort_by_key(|&(_, _, row)| row);
        let rows = members.len();
        // Every mixture alike but for its activation and destination.
        let params = |i: usize| {
            let op = r.s.get(i);
            (op.prim().map(|p| format!("{p:?}")).unwrap_or_default(), op.ins[1..].to_vec())
        };
        let first = params(members[0].0);
        let alike = members.iter().enumerate().all(|(n, &(i, _, row))| row == n && {
            let (p, ins) = params(i);
            let (fp, fins) = &first;
            ins == *fins && strip_symbols(&p) == strip_symbols(fp)
        });
        let lo = members.iter().map(|m| m.0.min(m.1)).min().unwrap_or(0);
        let hi = members.iter().map(|m| m.0).max().unwrap_or(0);
        let ours: Vec<usize> = members.iter().flat_map(|&(i, v, _)| [i, v]).collect();
        let untouched = r.s.untouched(&table, lo, hi, true, &ours) && r.s.untouched(&src, lo, hi, false, &ours);
        let only_theirs = members.iter().all(|&(i, v, _)| r.s.readers(&r.s.get(v).out) == [i]);
        if rows < 2 || !alike || !untouched || !only_theirs {
            continue;
        }
        let gone: Vec<usize> = ours.iter().copied().filter(|&g| g != hi).collect();
        let mut ins: Vec<(String, Access)> = vec![(src.clone(), Access::Whole)];
        ins.extend(r.s.get(hi).ins[1..].iter().map(|t| (t.clone(), Access::Whole)));
        let members_ops: Vec<Op> = ours.iter().map(|&k| r.s.get(k).clone()).collect();
        let cand = candidate(ClassId::MoeTokenSort, members_ops, ins.iter().map(|(t, _)| t.clone()).collect(), &table, Dst::to(&table));
        if !r.admits(&cand) {
            continue;
        }
        r.fuse("moe_rows", ClassId::MoeTokenSort, hi, &gone, ins, table.clone(), Dst::to(&table));
    }
}

/// A spec's debug text without the symbols it names: what two mixtures of different tensors share.
fn strip_symbols(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(i) = rest.find("Symbol(") {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.find(')').map_or(rest.len(), |e| e + 1);
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

/// A scale of gathered rows (an embedding's `√d`) inside the gather.
fn gather_scale(r: &mut Rewriter) {
    for j in 0..r.s.len() {
        let Some(TensorOpSpec::Scale(_)) = r.s.slot(j).and_then(|o| o.prim()) else { continue };
        let x = r.s.get(j).ins[0].clone();
        let Some(i) = r.s.producer(&x) else { continue };
        let gather = match r.s.get(i).prim() {
            Some(TensorOpSpec::RowGather(rg)) => matches!(rg.index, RowIndex::Named(_)) && weights(r.g, &r.s.get(i).ins[0]).is_some(),
            Some(TensorOpSpec::RowsGather(_)) => weights(r.g, &r.s.get(i).ins[0]).is_some(),
            _ => false,
        };
        if !gather || r.s.readers(&x) != [j] {
            continue;
        }
        let out = r.s.get(j).out.clone();
        let ins: Vec<(String, Access)> = r.s.get(i).ins.iter().cloned().zip(r.s.get(i).access.iter().copied()).collect();
        let cand = candidate(ClassId::EmbedScale, vec![r.s.get(i).clone(), r.s.get(j).clone()], ins.iter().map(|(t, _)| t.clone()).collect(), &out, Dst::to(&out));
        if !r.admits(&cand) {
            continue;
        }
        r.fuse("gather_scale", ClassId::EmbedScale, i, &[j], ins, out.clone(), Dst::to(&out));
    }
}

/// An attention whose values are a part of every row of a table (a latent cache's latent) reads them where they lie.
fn strided_values(r: &mut Rewriter) {
    for j in 0..r.s.len() {
        let Some(TensorOpSpec::Attention(a)) = r.s.slot(j).and_then(|o| o.prim()).cloned() else { continue };
        let vc = r.s.get(j).ins[2].clone();
        let Some(i) = r.s.producer(&vc) else { continue };
        let Some(sl) = slice(r.s.get(i)) else { continue };
        let group = a.h / a.kv.max(1);
        if sl.width != a.h / group * a.vd || r.s.readers(&vc) != [j] || !r.s.untouched(&sl.x, i, j, false, &[]) {
            continue;
        }
        let mut cand = r.s.get(j).clone();
        read_in_place(&mut cand, &vc, &sl.x, Access::Strided(Strided { pitch: sl.every, offset: sl.from, width: sl.width }));
        if !r.admits(&cand) {
            continue;
        }
        r.s.put(j, cand);
        r.absorb("strided_values", ClassId::StridedValues, j, &[i]);
    }
}

/// A step's rotations of its one row by one table at one position — normed alike or not at all, one head width — as one
/// op over their heads, at the first one's place: each later rotation moves up only where nothing between them writes
/// what it reads, or reads or writes what it writes.
fn rope_parts(r: &mut Rewriter) {
    for i in 0..r.s.len() {
        let Some(first) = r.s.slot(i).and_then(rope) else { continue };
        let mut group = vec![i];
        for j in i + 1..r.s.len() {
            let Some(ro) = r.s.slot(j).and_then(rope) else { continue };
            let fits = ro.cos == first.cos && ro.sin == first.sin && ro.pos == first.pos && ro.hd == first.hd && ro.half == first.half;
            if !fits || r.s.crosses(i, j, &[]) {
                continue;
            }
            let members: Vec<Op> = group.iter().chain([&j]).map(|&k| r.s.get(k).clone()).collect();
            let ins: Vec<String> = members.iter().flat_map(|m| m.ins.iter().cloned()).collect();
            let (out, dst) = (members[0].out.clone(), members[0].dst.clone());
            if r.admits(&candidate(ClassId::RopeQkOneLaunch, members, ins, &out, dst)) {
                group.push(j);
            }
        }
        if group.len() < 2 {
            continue;
        }
        let members: Vec<Op> = group.iter().map(|&k| r.s.get(k).clone()).collect();
        let ins: Vec<(String, Access)> = members.iter().flat_map(|m| m.ins.iter().cloned().zip(m.access.iter().copied())).collect();
        let (out, dst) = (members[0].out.clone(), members[0].dst.clone());
        let cand = candidate(ClassId::RopeQkOneLaunch, members, ins.iter().map(|(t, _)| t.clone()).collect(), &out, dst.clone());
        if !r.admits(&cand) {
            continue;
        }
        r.fuse("rope_parts", ClassId::RopeQkOneLaunch, i, &group[1..], ins, out, dst);
    }
}

/// The step's arg-max moves the token, the position and the generated run on.
fn argmax_advance(r: &mut Rewriter) {
    let Some(io) = r.g.io.clone() else { return };
    let nexts: Vec<String> = std::iter::once(io.next.clone()).chain(io.more.iter().map(|q| q.next.clone())).collect();
    for i in 0..r.s.len() {
        let Some(TensorOpSpec::ArgMax(_)) = r.s.slot(i).and_then(|o| o.prim()) else { continue };
        if !nexts.contains(&r.s.get(i).out) {
            continue;
        }
        let op = r.s.get(i).clone();
        let ins: Vec<(String, Access)> = op.ins.iter().cloned().zip(op.access.iter().copied()).collect();
        r.fuse("argmax_advance", ClassId::FusedSampling, i, &[], ins, op.out.clone(), op.dst.clone());
    }
}

/// The activation, depth and rows an op projects: a product's, or a fold's whose every product reads that one activation
/// at that one depth — a projection with its heads' epilogues, or several — first among its inputs.
fn projects(op: &Op, g: &TensorGraph) -> Option<(String, usize, usize)> {
    if let Some(p) = gemv(op, g) {
        return Some((p.x, p.k, p.m));
    }
    let Sym::Fused(_) = &op.sym else { return None };
    let products: Vec<GemvForm> = leaves(op.clone()).iter().filter_map(|l| gemv(l, g)).collect();
    let first = products.first()?;
    let one = products.iter().all(|p| (&p.x, p.k, p.m) == (&first.x, first.k, first.m));
    (one && op.ins.first() == Some(&first.x)).then(|| (first.x.clone(), first.k, first.m))
}

/// A step's products of one activation's rows by matrices — each landing in a buffer of its own or a table's rows its
/// width wide, or folded with the epilogues of its heads — as one op over their outputs side by side, at the first one's
/// place. Nothing between may write the activation, or touch what a later member writes; the chip says which products
/// it takes together.
fn merged_projections(r: &mut Rewriter) {
    for i in 0..r.s.len() {
        let Some(first) = r.s.slot(i).and_then(|o| projects(o, r.g)) else { continue };
        let x = first.0.clone();
        if !r.admits(&candidate(ClassId::MergedQkv, vec![r.s.get(i).clone()], vec![x.clone()], &r.s.get(i).out, r.s.get(i).dst.clone())) {
            continue;
        }
        let mut group = vec![i];
        for j in i + 1..r.s.len() {
            let Some(op) = r.s.slot(j) else { continue };
            if super::facts::writes(op).contains(&x.as_str()) {
                break;
            }
            if projects(op, r.g).as_ref() != Some(&first) {
                continue;
            }
            let written: Vec<&str> = super::facts::writes(op);
            let read: Vec<&str> = super::facts::reads(op);
            let crossed = (i + 1..j).filter(|b| !group.contains(b)).filter_map(|b| r.s.slot(b)).any(|between| {
                let (br, bw) = (super::facts::reads(between), super::facts::writes(between));
                written.iter().any(|t| br.contains(t) || bw.contains(t)) || read.iter().any(|t| bw.contains(t))
            });
            if crossed {
                continue;
            }
            let members: Vec<Op> = group.iter().chain([&j]).map(|&k| r.s.get(k).clone()).collect();
            if r.admits(&candidate(ClassId::MergedQkv, members, vec![x.clone()], &r.s.get(i).out, r.s.get(i).dst.clone())) {
                group.push(j);
            }
        }
        if group.len() < 2 {
            continue;
        }
        let members: Vec<Op> = group.iter().map(|&k| r.s.get(k).clone()).collect();
        let mut ins: Vec<(String, Access)> = vec![(x.clone(), r.s.get(i).access[0])];
        for m in &members {
            for (t, a) in m.ins.iter().zip(&m.access).skip(1) {
                if !ins.iter().any(|(u, _)| u == t) {
                    ins.push((t.clone(), *a));
                }
            }
        }
        let (out, dst) = (members[0].out.clone(), members[0].dst.clone());
        r.fuse("merged_projections", ClassId::MergedQkv, i, &group[1..], ins, out, dst);
    }
}

/// The element count of a pointwise op computing its result whole: a lifted elementwise op over whole operands, or a
/// lifted activation or scale.
fn pointwise(op: &Op) -> Option<usize> {
    if op.dst.own().is_none() || op.access.iter().any(|a| *a != Access::Whole) {
        return None;
    }
    match op.prim()? {
        TensorOpSpec::Elementwise(e) if e.row.is_none() => Some(e.n),
        TensorOpSpec::Silu(u) => Some(u.n),
        TensorOpSpec::Sigmoid(u) => Some(u.n),
        TensorOpSpec::Gelu(u) => Some(u.n),
        TensorOpSpec::Relu(u) => Some(u.n),
        TensorOpSpec::Scale(u) => Some(u.n),
        _ => None,
    }
}

/// A lifted scale of a whole tensor: its input, factor and length.
fn scale_of(op: &Op) -> Option<(String, usize)> {
    match op.prim()? {
        TensorOpSpec::Scale(sc) if op.access[0] == Access::Whole && op.dst.own().is_some() => Some((op.ins[0].clone(), sc.n)),
        _ => None,
    }
}

/// An op's operands with their accesses.
fn operands(op: &Op) -> Vec<(String, Access)> {
    op.ins.iter().cloned().zip(op.access.iter().copied()).collect()
}

/// The operands of `members` that none of them computes, each once, in the order the members read them.
fn external(members: &[&Op]) -> Vec<(String, Access)> {
    let made: BTreeSet<&str> = members.iter().map(|m| m.out.as_str()).collect();
    let mut ins: Vec<(String, Access)> = Vec::new();
    for m in members {
        for (t, a) in operands(m) {
            if !made.contains(t.as_str()) && !ins.iter().any(|(n, _)| *n == t) {
                ins.push((t, a));
            }
        }
    }
    ins
}

/// Fuse `producer` and `consumer` — the consumer reading the producer's result, which nothing else reads — into one op
/// of `class` at the slot `anchor` names, when the chip runs it whole.
fn pair(r: &mut Rewriter, rule: &'static str, class: ClassId, producer: usize, consumer: usize, anchor: usize) -> bool {
    let (p, c) = (r.s.get(producer).clone(), r.s.get(consumer).clone());
    let ins = external(&[&p, &c]);
    let names: Vec<String> = ins.iter().map(|(t, _)| t.clone()).collect();
    if !r.admits(&candidate(class, vec![p, c.clone()], names, &c.out, c.dst.clone())) {
        return false;
    }
    let gone = if anchor == producer { consumer } else { producer };
    r.fuse(rule, class, anchor, &[gone], ins, c.out.clone(), c.dst.clone());
    true
}

/// A scalar scale of a plain norm's whole result, which nothing else reads, in the norm's epilogue.
fn norm_scale(r: &mut Rewriter) {
    for j in 0..r.s.len() {
        let Some((x, n)) = r.s.slot(j).and_then(scale_of) else { continue };
        let Some(i) = r.s.producer(&x) else { continue };
        let Some(nf) = norm(r.s.get(i)) else { continue };
        let plain = class(r.s.get(i)).is_none() && nf.own && nf.part.is_none() && nf.add.is_none() && nf.rows * nf.cols == n;
        if plain && r.s.readers(&x) == [j] && !r.s.crosses(i, j, &[]) {
            pair(r, "norm_scale", ClassId::NormScale, i, j, i);
        }
    }
}

/// A scalar scale of a product's whole result, which nothing else reads, in the product's epilogue.
fn gemm_scale(r: &mut Rewriter) {
    for j in 0..r.s.len() {
        let Some(op) = r.s.slot(j) else { continue };
        let Some((x, n)) = scale_of(op).or_else(|| column_scale(r, op)) else { continue };
        let Some(i) = r.s.producer(&x) else { continue };
        let Some(p) = gemv(r.s.get(i), r.g) else { continue };
        if !(p.dst.own().is_some() && p.n * p.m == n && r.s.readers(&x) == [j] && !r.s.crosses(i, j, &[])) {
            continue;
        }
        if !pair(r, "gemm_scale", ClassId::GemmScale, i, j, i) {
            continue;
        }
        // A bias after the scale, a constant row added to its every row, joins the epilogue.
        let y = r.s.get(i).out.clone();
        let Some(&k) = r.s.readers(&y).first().filter(|_| r.s.readers(&y).len() == 1) else { continue };
        let Some(b) = binary(r.s.get(k)) else { continue };
        let bias = if b.a == y { &b.b } else { &b.a };
        let constant = weights(r.g, bias) == Some(Weights::Values) && r.g.tensors.get(bias).map(|t| t.len()) == Some(b.row.unwrap_or(b.n));
        if b.op == ElementwiseOp::Add && b.gate.is_none() && (b.row.is_some() || b.n == n) && b.row.unwrap_or(p.n) == p.n && constant && !r.s.crosses(i, k, &[]) {
            pair(r, "gemm_scale", ClassId::GemmScale, i, k, i);
        }
    }
}

/// A product of a whole tensor by a constant row, each column its own factor — over every row of the tensor or its one
/// row — as a scale: the scaled tensor and its length.
fn column_scale(r: &Rewriter, op: &Op) -> Option<(String, usize)> {
    let b = binary(op)?;
    if b.op != ElementwiseOp::Mul || b.gate.is_some() || op.dst.own().is_none() || op.access.iter().any(|a| *a != Access::Whole) {
        return None;
    }
    let constant = |t: &str, len: usize| weights(r.g, t) == Some(Weights::Values) && r.g.tensors.get(t).map(|t| t.len()) == Some(len);
    if constant(&b.b, b.row.unwrap_or(b.n)) {
        Some((b.a.clone(), b.n))
    } else if b.row.is_none() && constant(&b.a, b.n) {
        Some((b.b.clone(), b.n))
    } else {
        None
    }
}

/// A scalar scale of a product's activation, which nothing else reads, folded into the product's input.
fn affine_input_fold(r: &mut Rewriter) {
    for i in 0..r.s.len() {
        let Some(op) = r.s.slot(i) else { continue };
        let Some((_, n)) = scale_of(op) else { continue };
        let s = op.out.clone();
        let [j] = r.s.readers(&s)[..] else { continue };
        let Some(p) = gemv(r.s.get(j), r.g) else { continue };
        if p.x == s && r.s.get(j).access[0] == Access::Whole && p.k * p.m == n && !r.s.sinks_past(i, j, &[]) {
            pair(r, "affine_input_fold", ClassId::AffineInputFold, i, j, j);
        }
    }
}

/// A ReLU squared: the ReLU's result, read only by the product of it with itself, inside that product.
fn relu_sqr(r: &mut Rewriter) {
    for j in 0..r.s.len() {
        let Some(b) = r.s.slot(j).and_then(binary) else { continue };
        if b.op != ElementwiseOp::Mul || b.gate.is_some() || b.row.is_some() || b.a != b.b || class(r.s.get(j)).is_some() {
            continue;
        }
        let Some(i) = r.s.producer(&b.a) else { continue };
        let relu = matches!(r.s.get(i).prim(), Some(TensorOpSpec::Relu(u)) if u.n == b.n) && pointwise(r.s.get(i)).is_some();
        if relu && r.s.readers(&b.a) == [j] && !r.s.crosses(i, j, &[]) {
            pair(r, "relu_sqr", ClassId::ReluSqr, i, j, i);
        }
    }
}

/// What a region member computes: `n` values of one element type, each from its operands' values at its place — a
/// pointwise op, or an elementwise op combining every row of its first operand with one row of its second (a bias
/// over a block).
fn region_member(op: &Op, g: &TensorGraph) -> Option<(usize, super::graph::Elem)> {
    if op.dst.own().is_none() || op.access.iter().any(|a| *a != Access::Whole) || class(op).is_some() {
        return None;
    }
    let n = match op.prim()? {
        TensorOpSpec::Elementwise(e) => e.n,
        _ => pointwise(op)?,
    };
    Some((n, g.tensors.get(&op.out)?.elem))
}

/// A region of pointwise ops of one length and element type feeding one op — some op of it joining two of the
/// region's results — as one op at the last one's place: each member moved down past nothing that writes what it reads
/// or touches what it writes, a member's result read after the region written out beside it.
fn elementwise_regions(r: &mut Rewriter) {
    for y in (0..r.s.len()).rev() {
        let Some(sink) = r.s.slot(y) else { continue };
        let Some(form) = region_member(sink, r.g) else { continue };
        let mut members: Vec<usize> = vec![y];
        let mut joins = false;
        loop {
            let mut grew = false;
            for m in members.clone() {
                let mut fed = 0;
                for t in r.s.get(m).ins.clone() {
                    let Some(p) = r.s.producer(&t) else { continue };
                    if members.contains(&p) {
                        fed += 1;
                        continue;
                    }
                    let fits = region_member(r.s.get(p), r.g) == Some(form) && !r.carried(&t);
                    if fits && !r.s.sinks_past(p, y, &members) {
                        members.push(p);
                        fed += 1;
                        grew = true;
                    }
                }
                joins |= fed >= 2;
            }
            if !grew {
                break;
            }
        }
        if members.len() < 3 || !joins {
            continue;
        }
        members.sort_unstable();
        let ops: Vec<Op> = members.iter().map(|&k| r.s.get(k).clone()).collect();
        let ins = external(&ops.iter().collect::<Vec<_>>());
        let names: Vec<String> = ins.iter().map(|(t, _)| t.clone()).collect();
        let (out, dst) = (r.s.get(y).out.clone(), r.s.get(y).dst.clone());
        if !r.admits(&candidate(ClassId::ElementwiseRegions, ops, names, &out, dst.clone())) {
            continue;
        }
        let gone: Vec<usize> = members.iter().copied().filter(|&k| k != y).collect();
        r.fuse("elementwise_regions", ClassId::ElementwiseRegions, y, &gone, ins, out, dst);
    }
}

/// A run of elementwise ops of one length in consecutive slots, each carrying the one before's result as exactly one
/// operand — or a SiLU of it — that nothing else reads, as one op at the first one's place.
fn elementwise_chains(r: &mut Rewriter) {
    for i in 0..r.s.len() {
        let Some(first) = r.s.slot(i) else { continue };
        let Some(TensorOpSpec::Elementwise(e0)) = first.prim() else { continue };
        if pointwise(first).is_none() {
            continue;
        }
        let n = e0.n;
        let mut chain = vec![i];
        let mut cur = first.out.clone();
        let mut k = i;
        loop {
            let Some(next) = (k + 1..r.s.len()).find(|&b| r.s.slot(b).is_some()) else { break };
            let op = r.s.get(next);
            let carried = match op.prim() {
                Some(TensorOpSpec::Elementwise(e)) if e.n == n && pointwise(op).is_some() => (op.ins[0] == cur) ^ (op.ins[1] == cur),
                Some(TensorOpSpec::Silu(_)) if pointwise(op).is_some() => op.ins[0] == cur,
                _ => false,
            };
            if !carried || class(op).is_some() || r.carried(&cur) || r.s.readers(&cur) != [next] {
                break;
            }
            chain.push(next);
            cur = op.out.clone();
            k = next;
        }
        if chain.len() < 2 {
            continue;
        }
        let ops: Vec<Op> = chain.iter().map(|&k| r.s.get(k).clone()).collect();
        let ins = external(&ops.iter().collect::<Vec<_>>());
        let names: Vec<String> = ins.iter().map(|(t, _)| t.clone()).collect();
        let last = ops.last().expect("a chain").clone();
        if !r.admits(&candidate(ClassId::ElementwiseChains, ops, names, &last.out, last.dst.clone())) {
            continue;
        }
        r.fuse("elementwise_chains", ClassId::ElementwiseChains, i, &chain[1..], ins, last.out, last.dst);
    }
}

/// A cheap pointwise op read by several pointwise ops, recomputed inside each — its result never stored — where it is
/// not observed or carried and each reader can take it where it stands.
fn recompute_fanout(r: &mut Rewriter) {
    for i in 0..r.s.len() {
        let Some(op) = r.s.slot(i) else { continue };
        let cheap = matches!(op.prim(), Some(TensorOpSpec::Silu(_) | TensorOpSpec::Sigmoid(_) | TensorOpSpec::Gelu(_) | TensorOpSpec::Relu(_) | TensorOpSpec::Scale(_)));
        if !cheap || pointwise(op).is_none() || r.observed(&op.out) {
            continue;
        }
        let producer = op.clone();
        let readers = r.s.readers(&producer.out);
        let fits = readers.len() >= 2
            && readers.iter().all(|&k| class(r.s.get(k)).is_none() && pointwise(r.s.get(k)).is_some() && r.s.untouched(&producer.ins[0], i, k, false, &[]));
        if !fits {
            continue;
        }
        let candidates: Vec<Op> = readers
            .iter()
            .map(|&k| {
                let c = r.s.get(k).clone();
                let ins = external(&[&producer, &c]);
                candidate(ClassId::RecomputeFanout, vec![producer.clone(), c.clone()], ins.into_iter().map(|(t, _)| t).collect(), &c.out, c.dst.clone())
            })
            .collect();
        if !candidates.iter().all(|c| r.admits(c)) {
            continue;
        }
        for k in readers {
            let c = r.s.take(k).expect("a live slot");
            let ins = external(&[&producer, &c]);
            let mut covers: Vec<usize> = producer.covers.iter().chain(&c.covers).copied().collect();
            covers.sort_unstable();
            let (names, access): (Vec<String>, Vec<Access>) = ins.into_iter().unzip();
            let fused = Op {
                sym: Sym::Fused(Fused { class: ClassId::RecomputeFanout, parts: vec![producer.clone(), c.clone()] }),
                ins: names,
                access,
                out: c.out.clone(),
                dst: c.dst.clone(),
                keeps: Vec::new(),
                covers,
            };
            r.s.put(k, fused);
            r.record("recompute_fanout", ClassId::RecomputeFanout, k);
        }
        r.s.take(i);
    }
}

/// Two independent plain norms of one row shape, the later moved up to the earlier's place, as one op.
fn dual_norm(r: &mut Rewriter) {
    let plain = |op: &Op| norm(op).filter(|n| class(op).is_none() && n.own && n.part.is_none() && n.add.is_none());
    for i in 0..r.s.len() {
        let Some(a) = r.s.slot(i).and_then(plain) else { continue };
        for j in i + 1..r.s.len() {
            let Some(b) = r.s.slot(j).and_then(plain) else { continue };
            if (b.rows, b.cols) != (a.rows, a.cols) || facts::reads(r.s.get(j)).contains(&a.out.as_str()) || r.s.crosses(i, j, &[]) {
                continue;
            }
            let ops = vec![r.s.get(i).clone(), r.s.get(j).clone()];
            let ins = external(&ops.iter().collect::<Vec<_>>());
            let names: Vec<String> = ins.iter().map(|(t, _)| t.clone()).collect();
            let dst = r.s.get(i).dst.clone();
            if r.admits(&candidate(ClassId::DualNorm, ops, names, &a.out, dst.clone())) {
                r.fuse("dual_norm", ClassId::DualNorm, i, &[j], ins, a.out.clone(), dst);
            }
            break;
        }
    }
}

/// The embedding's gathered rows and the first norm of them, the norm moved up to the gather's place; the rows stay
/// written for their later readers.
fn embed_norm(r: &mut Rewriter) {
    for i in 0..r.s.len() {
        let Some(op) = r.s.slot(i) else { continue };
        let gather = match op.prim() {
            Some(TensorOpSpec::RowsGather(_)) => true,
            Some(TensorOpSpec::RowGather(rg)) => matches!(rg.index, RowIndex::Named(_)),
            _ => class(op) == Some(ClassId::EmbedScale),
        };
        let table = op.ins[0].clone();
        if !gather || op.dst.own().is_none() || weights(r.g, &table).is_none() {
            continue;
        }
        let x = op.out.clone();
        let Some(&j) = r.s.readers(&x).first() else { continue };
        let Some(n) = norm(r.s.get(j)) else { continue };
        if class(r.s.get(j)).is_some() || n.x != x || !n.own || n.part.is_some() || n.add.is_some() || r.s.crosses(i, j, &[]) {
            continue;
        }
        let ops = vec![r.s.get(i).clone(), r.s.get(j).clone()];
        let ins = external(&ops.iter().collect::<Vec<_>>());
        let names: Vec<String> = ins.iter().map(|(t, _)| t.clone()).collect();
        let dst = r.s.get(j).dst.clone();
        if r.admits(&candidate(ClassId::EmbedNorm, ops, names, &n.out, dst.clone())) {
            r.fuse("embed_norm", ClassId::EmbedNorm, i, &[j], ins, n.out.clone(), dst);
        }
    }
}

/// A product of a step's one row whose whole result is one summand of a residual add and the norm of the sum — or the
/// input of a norm whose result is that summand (a post-norm before the add, `sandwich_norm`'s) — read by nothing else,
/// run in the add-and-norm's place: the projection with its post-norm, the residual and the next norm in its epilogue.
fn gemv_norm_epilogue(r: &mut Rewriter) {
    norm_epilogues(r, "gemv_norm_epilogue", ClassId::GemvNormEpilogue, |m| m == 1);
}

/// [`gemv_norm_epilogue`] over a chunk's rows: the norms in the epilogue of a product of several.
fn prefill_norm_epilogues(r: &mut Rewriter) {
    norm_epilogues(r, "prefill_norm_epilogues", ClassId::PrefillNormEpilogues, |m| m > 1);
}

/// A product of `rows` rows read only by an add-and-norm (or a norm-then-add), folded into it as `builds`.
fn norm_epilogues(r: &mut Rewriter, rule: &'static str, builds: ClassId, rows: fn(usize) -> bool) {
    for j in 0..r.s.len() {
        let Some(op) = r.s.slot(j) else { continue };
        if !matches!(class(op), Some(ClassId::AddNorm | ClassId::NormThenAdd)) {
            continue;
        }
        let summands = [op.ins[0].clone(), op.ins[1].clone()];
        for t in summands {
            let Some(i) = r.s.producer(&t) else { continue };
            let Some(p) = gemv(r.s.get(i), r.g) else { continue };
            if !rows(p.m) || p.dst.own().is_none() || r.s.readers(&t) != [j] || r.s.sinks_past(i, j, &[]) {
                continue;
            }
            if pair(r, rule, builds, i, j, j) {
                break;
            }
        }
    }
}

/// A product whose whole result is the input of a rotation — its per-head norm folded in or none — read by nothing else,
/// run in the rotation's place: the projection with the norm, the rotation and its cache append in its epilogue.
fn qkv_head_epilogue(r: &mut Rewriter) {
    for j in 0..r.s.len() {
        let Some(op) = r.s.slot(j) else { continue };
        let Some(ro) = rope(op).filter(|ro| ro.part.is_none()) else { continue };
        let Some(i) = r.s.producer(&ro.x) else { continue };
        let Some(p) = gemv(r.s.get(i), r.g) else { continue };
        if p.dst.own().is_some() && r.s.readers(&ro.x) == [j] && !r.s.sinks_past(i, j, &[]) {
            pair(r, "qkv_head_epilogue", ClassId::QkvHeadEpilogue, i, j, j);
        }
    }
}

/// Rows written into tables at one row, each write moved up to the first one's place past nothing that touches what it
/// reads or writes, as one op: every layer's cache insert in one launch.
fn cross_layer_launch(r: &mut Rewriter) {
    let row = |op: &Op| match op.prim()? {
        TensorOpSpec::SetRow(s) if op.access.iter().all(|a| *a == Access::Whole) => Some(match &s.index {
            RowIndex::Named(_) => op.ins[2].clone(),
            RowIndex::Literal(n) => format!("#{n}"),
        }),
        _ => None,
    };
    for i in 0..r.s.len() {
        let Some(at) = r.s.slot(i).and_then(row) else { continue };
        let mut group = vec![i];
        for j in i + 1..r.s.len() {
            if r.s.slot(j).and_then(row).as_ref() == Some(&at) && !r.s.crosses(i, j, &group) {
                let ops: Vec<Op> = group.iter().chain([&j]).map(|&k| r.s.get(k).clone()).collect();
                let names: Vec<String> = external(&ops.iter().collect::<Vec<_>>()).into_iter().map(|(t, _)| t).collect();
                let (out, dst) = (ops[0].out.clone(), ops[0].dst.clone());
                if r.admits(&candidate(ClassId::CrossLayerLaunch, ops, names, &out, dst)) {
                    group.push(j);
                }
            }
        }
        if group.len() < 2 {
            continue;
        }
        let ops: Vec<Op> = group.iter().map(|&k| r.s.get(k).clone()).collect();
        let ins = external(&ops.iter().collect::<Vec<_>>());
        let (out, dst) = (ops[0].out.clone(), ops[0].dst.clone());
        r.fuse("cross_layer_launch", ClassId::CrossLayerLaunch, i, &group[1..], ins, out, dst);
    }
}

/// Whether every tensor `op` writes is read only by the slots of `within`.
fn read_only_within(r: &Rewriter, op: &Op, within: &[usize]) -> bool {
    facts::writes(op).into_iter().all(|t| !r.observed(t) && r.s.readers(t).iter().all(|k| within.contains(k)))
}

/// Fold `members` into one op of `class` at the last one's slot, its result the anchor's, when each member moves down
/// to the anchor past nothing that sees the difference and the chip runs the result whole.
fn group_at_last(r: &mut Rewriter, rule: &'static str, class: ClassId, mut members: Vec<usize>) -> bool {
    members.sort_unstable();
    members.dedup();
    let at = *members.last().expect("members");
    if members.iter().any(|&m| m != at && r.s.sinks_past(m, at, &members)) {
        return false;
    }
    let ops: Vec<Op> = members.iter().map(|&k| r.s.get(k).clone()).collect();
    let ins = external(&ops.iter().collect::<Vec<_>>());
    let names: Vec<String> = ins.iter().map(|(t, _)| t.clone()).collect();
    let (out, dst) = (r.s.get(at).out.clone(), r.s.get(at).dst.clone());
    if !r.admits(&candidate(class, ops, names, &out, dst.clone())) {
        return false;
    }
    let gone: Vec<usize> = members.iter().copied().filter(|&k| k != at).collect();
    r.fuse(rule, class, at, &gone, ins, out, dst);
    true
}

/// A mixture's whole result — or its sum with its shared expert's product, where the shared expert stays apart (a
/// chunk's token-sorted mixture) — one summand of a residual add and the norm of the sum, each read by nothing else,
/// combined in the add-and-norm's place: the combine folded into the next add and norm.
fn moe_deferred_finalize(r: &mut Rewriter) {
    let mixture = |r: &Rewriter, t: &str, reader: usize| {
        let i = r.s.producer(t)?;
        moe(r.s.get(i)).filter(|m| m.dst.own().is_some() && r.s.readers(t) == [reader]).map(|_| i)
    };
    for j in 0..r.s.len() {
        let Some(op) = r.s.slot(j) else { continue };
        if class(op) != Some(ClassId::AddNorm) {
            continue;
        }
        for t in [op.ins[0].clone(), op.ins[1].clone()] {
            if let Some(i) = mixture(r, &t, j) {
                if !r.s.sinks_past(i, j, &[]) && pair(r, "moe_deferred_finalize", ClassId::MoeDeferredFinalize, i, j, j) {
                    break;
                }
                continue;
            }
            // The mixture summed with its shared expert's product first.
            let Some(i) = r.s.producer(&t).filter(|_| r.s.readers(&t) == [j]) else { continue };
            let Some(b) = binary(r.s.get(i)) else { continue };
            if b.op != ElementwiseOp::Add || b.gate.is_some() || b.row.is_some() || class(r.s.get(i)).is_some() {
                continue;
            }
            let Some(m) = [&b.a, &b.b].into_iter().find_map(|x| mixture(r, x, i)) else { continue };
            if group_at_last(r, "moe_deferred_finalize", ClassId::MoeDeferredFinalize, vec![m, i, j]) {
                break;
            }
        }
    }
}

/// A latent attention's attended latent, read only by the product that expands it into each head's values, expanded in
/// the attention's place: the values un-absorbed in the attention's fold.
fn mla_unabsorb_in_fold(r: &mut Rewriter) {
    for i in 0..r.s.len() {
        let Some(TensorOpSpec::Attention(a)) = r.s.slot(i).and_then(|o| o.prim()).cloned() else { continue };
        if a.kv != 1 || a.vd >= a.hd || r.s.get(i).dst.own().is_none() {
            continue;
        }
        let o = r.s.get(i).out.clone();
        let [j] = r.s.readers(&o)[..] else { continue };
        let Some(TensorOpSpec::HeadMatmul(c)) = r.s.get(j).prim() else { continue };
        let fits = r.s.get(j).ins[0] == o && r.s.get(j).access[0] == Access::Whole && c.heads == a.h && c.a == a.vd && r.s.get(j).dst.own().is_some();
        if fits && !r.s.crosses(i, j, &[]) {
            pair(r, "mla_unabsorb_in_fold", ClassId::MlaUnabsorbInFold, i, j, i);
        }
    }
}

/// Everything a latent attention's query and cache row are made by in its step — the projection of the layer's
/// activation, the latent's norm, the rotations, the cache append and the query's absorption — read by nothing but
/// itself and the attention, as one op before the attention.
fn mla_prologue(r: &mut Rewriter) {
    for at in 0..r.s.len() {
        let Some(op) = r.s.slot(at) else { continue };
        let latent = leaves(op.clone()).into_iter().find_map(|l| match l.prim() {
            Some(TensorOpSpec::Attention(a)) if a.kv == 1 && a.vd < a.hd => Some((l.ins[0].clone(), l.ins[1].clone())),
            _ => None,
        });
        let Some((q, kc)) = latent else { continue };
        // What writes the query and the cache in this step.
        let mut members: Vec<usize> = r.s.live().filter(|(k, o)| *k < at && (o.dst.buf == q || o.dst.buf == kc) && o.dst.own().is_none()).map(|(k, _)| k).collect();
        if members.is_empty() {
            continue;
        }
        // Everything they read that is made only for them.
        loop {
            let mut grew = false;
            for m in members.clone() {
                for t in r.s.get(m).ins.clone() {
                    let Some(p) = r.s.maker(&t) else { continue };
                    if members.contains(&p) || p > at {
                        continue;
                    }
                    let within: Vec<usize> = members.iter().copied().chain([p, at]).collect();
                    if read_only_within(r, r.s.get(p), &within) {
                        members.push(p);
                        grew = true;
                    }
                }
            }
            if !grew {
                break;
            }
        }
        let ops: Vec<&Op> = members.iter().map(|&k| r.s.get(k)).collect();
        let projects = ops.iter().any(|o| gemv(o, r.g).is_some() || class(o) == Some(ClassId::MergedQkv));
        let absorbs = ops.iter().any(|o| leaves((*o).clone()).iter().any(|l| matches!(l.prim(), Some(TensorOpSpec::HeadMatmul(_)))));
        let rotates = ops.iter().any(|o| leaves((*o).clone()).iter().any(|l| matches!(l.prim(), Some(TensorOpSpec::Rope(_)))));
        if projects && absorbs && rotates {
            group_at_last(r, "mla_prologue", ClassId::MlaPrologue, members);
        }
    }
}

/// A gated delta rule's inputs as its step makes them, with its rows: the convolutions of its query, key and values and
/// its two gates, and — where an op makes each — the products of the layer's activation those convolutions read and its
/// output gate is.
fn delta_inputs(r: &Rewriter, j: usize) -> Option<(usize, Vec<usize>, Option<Vec<usize>>)> {
    let Some(TensorOpSpec::GatedDelta(d)) = r.s.slot(j)?.prim() else { return None };
    let op = r.s.get(j);
    let convs: BTreeSet<usize> = op.ins[0..3].iter().map(|t| r.s.maker(t)).collect::<Option<_>>()?;
    if !convs.iter().all(|&c| conv(r.s.get(c)).is_some() || class(r.s.get(c)) == Some(ClassId::ConvParts)) {
        return None;
    }
    let gating = r.s.maker(&op.ins[4])?;
    if r.s.maker(&op.ins[5]) != Some(gating) || !matches!(class(r.s.get(gating)), Some(ClassId::GdnGating | ClassId::GdnGateProjections)) {
        return None;
    }
    let mut read: Vec<String> = convs
        .iter()
        .flat_map(|&c| leaves(r.s.get(c).clone()))
        .filter(|l| matches!(l.prim(), Some(TensorOpSpec::CausalConv(_))))
        .map(|l| l.ins[0].clone())
        .collect();
    read.push(op.ins[3].clone());
    let products: Option<BTreeSet<usize>> = read.iter().map(|t| r.s.maker(t)).collect();
    Some((d.rows, convs.into_iter().chain([gating]).collect(), products.map(|p| p.into_iter().collect())))
}

/// Whether every op of `members` is read by nothing but the others and the op at `j`.
fn made_for(r: &Rewriter, members: &[usize], j: usize) -> bool {
    let within: Vec<usize> = members.iter().copied().chain([j]).collect();
    members.iter().all(|&k| read_only_within(r, r.s.get(k), &within))
}

/// A one-row gated delta rule's every input projection — query, key, values, output gate, both gates — with its
/// convolutions, SiLU, L2 norms and gates in the projections' epilogue, as one op.
fn gdn_input_epilogue(r: &mut Rewriter) {
    for j in 0..r.s.len() {
        let Some((rows, prep, Some(products))) = delta_inputs(r, j) else { continue };
        let one_activation = products.iter().filter_map(|&p| gemv(r.s.get(p), r.g)).map(|p| p.x).collect::<BTreeSet<_>>().len() == 1;
        let members: Vec<usize> = prep.into_iter().chain(products.iter().copied()).collect();
        if rows == 1 && one_activation && products.iter().all(|&p| gemv(r.s.get(p), r.g).is_some()) && made_for(r, &members, j) {
            group_at_last(r, "gdn_input_epilogue", ClassId::GdnInputEpilogue, members);
        }
    }
}

/// A chunk's gated delta inputs past their projections — the convolutions with their SiLU and L2 norms, and the gates —
/// as one op.
fn gdn_post_conv_prep(r: &mut Rewriter) {
    for j in 0..r.s.len() {
        let Some((rows, prep, _)) = delta_inputs(r, j) else { continue };
        if rows > 1 && made_for(r, &prep, j) {
            group_at_last(r, "gdn_post_conv_prep", ClassId::GdnPostConvPrep, prep);
        }
    }
}

/// A one-row gated delta rule's whole step between its projections — the convolutions of its query, key and values
/// with their SiLU and L2 norms, its two gates, the update of its memory and its gated norm — as one op.
fn gdn_decode_whole(r: &mut Rewriter) {
    for j in 0..r.s.len() {
        let Some((rows, prep, _)) = delta_inputs(r, j) else { continue };
        if rows == 1 && made_for(r, &prep, j) {
            group_at_last(r, "gdn_decode_whole", ClassId::GdnDecodeWhole, prep.into_iter().chain([j]).collect());
        }
    }
}

/// Whether an op computes each row of its result from that row of its activations alone (an attention: each query
/// row from its row and the whole cache; a token-sorted mixture: each row routed on its own, its views a row each),
/// writing no state and no rows of a table.
fn row_wise(op: &Op) -> bool {
    if op.dst.row.is_some() || !op.state_inputs().is_empty() {
        return false;
    }
    if matches!(class(op), Some(ClassId::MoeTokenSort | ClassId::MoeDeferredFinalize)) {
        return true;
    }
    leaves(op.clone()).iter().all(|l| {
        matches!(
            l.prim(),
            Some(
                TensorOpSpec::Matmul(_)
                    | TensorOpSpec::Elementwise(_)
                    | TensorOpSpec::FusedRow(_)
                    | TensorOpSpec::Rope(_)
                    | TensorOpSpec::Silu(_)
                    | TensorOpSpec::Sigmoid(_)
                    | TensorOpSpec::Gelu(_)
                    | TensorOpSpec::Relu(_)
                    | TensorOpSpec::Scale(_)
                    | TensorOpSpec::SwiGlu(_)
                    | TensorOpSpec::HeadMatmul(_)
                    | TensorOpSpec::Attention(_)
                    | TensorOpSpec::Moe(_)
                    | TensorOpSpec::Join(_)
                    | TensorOpSpec::L2Norm(_)
                    | TensorOpSpec::HeadSlice(_)
            )
        ) || matches!(l.sym, Sym::Quantize(_))
    })
}

/// A chunk's output row read from its last layer's result: every op of the layer's tail that only that row reads —
/// row by row from the attention on, each read by nothing but the tail and writing no state the step carries, so the
/// cache writes stay before it — computes with it, as one op at the row's place.
fn last_layer_rows(r: &mut Rewriter) {
    for v in 0..r.s.len() {
        let Some(view) = r.s.slot(v) else { continue };
        let Sym::View { offset } = view.sym else { continue };
        let table = view.ins[0].clone();
        let Some(t) = r.g.tensors.get(&table) else { continue };
        if t.rows < 2 || offset % t.cols != 0 {
            continue;
        }
        let rows = t.rows;
        let Some(first) = r.s.producer(&table) else { continue };
        let mut members: Vec<usize> = vec![v];
        let mut frontier = vec![first];
        while let Some(p) = frontier.pop() {
            if members.contains(&p) {
                continue;
            }
            let op = r.s.get(p);
            let same_rows = r.g.tensors.get(&op.out).is_none_or(|x| x.rows == rows);
            let uncarried = facts::writes(op).into_iter().all(|t| r.g.tensors.get(t).is_none_or(|x| !x.carried));
            let within: Vec<usize> = members.iter().copied().chain([p]).collect();
            if !(row_wise(op) && same_rows && uncarried && read_only_within(r, op, &within)) {
                continue;
            }
            members.push(p);
            for t in op.ins.clone() {
                if let Some(q) = r.s.producer(&t) {
                    frontier.push(q);
                }
            }
        }
        let attends = members.iter().any(|&m| leaves(r.s.get(m).clone()).iter().any(|l| matches!(l.prim(), Some(TensorOpSpec::Attention(_)))));
        if attends && members.len() > 2 {
            group_at_last(r, "last_layer_rows", ClassId::LastLayerRows, members);
        }
    }
}

/// The rotation leaf of an op that rotates normed heads — a per-head norm and a rotation, alone or with the product
/// before them — and its tables and position.
fn normed_rotation(op: &Op) -> Option<(String, String, Option<String>)> {
    let found = leaves(op.clone());
    let normed = found.iter().any(|l| matches!(l.prim(), Some(TensorOpSpec::FusedRow(_))));
    let ro = found.iter().find(|l| matches!(l.prim(), Some(TensorOpSpec::Rope(_))))?;
    normed.then(|| (ro.ins[1].clone(), ro.ins[2].clone(), ro.ins.get(3).cloned()))
}

/// The gate a gated attention's result is multiplied by — the activation's input in the gated product of the attention
/// `op` computes: the product folded into the attention's store, which gates its first operand, or the op's one reader.
fn output_gate(r: &Rewriter, op: &Op) -> Option<String> {
    if let Sym::Fused(f) = &op.sym {
        if f.class == ClassId::AttnOutputGate {
            return binary(f.parts.last()?).filter(|b| b.gate.is_some()).map(|b| b.b);
        }
    }
    let [gk] = r.s.readers(&op.out)[..] else { return None };
    let b = binary(r.s.get(gk)).filter(|b| b.gate.is_some())?;
    Some(if b.a == op.out { b.b } else { b.a })
}

/// A gated attention's gate — a product of the activation its query is projected from, read after the attention — in the
/// op that projects, norms and rotates the query: the split, the norms, the partial rotation and the gate as one op.
fn qk_norm_rope_gate_split(r: &mut Rewriter) {
    for at in 0..r.s.len() {
        let Some(op) = r.s.slot(at) else { continue };
        let Some(attn) = leaves(op.clone()).into_iter().find(|l| matches!(l.prim(), Some(TensorOpSpec::Attention(_)))) else { continue };
        let q = attn.ins[0].clone();
        let Some(gate) = output_gate(r, op) else { continue };
        let (Some(p), Some(qk)) = (r.s.maker(&gate), r.s.maker(&q)) else { continue };
        let Some(gp) = gemv(r.s.get(p), r.g) else { continue };
        let query_x = leaves(r.s.get(qk).clone()).into_iter().find_map(|l| match l.prim() {
            Some(TensorOpSpec::Matmul(_)) => Some(l.ins[0].clone()),
            _ => None,
        });
        if normed_rotation(r.s.get(qk)).is_none() || query_x.as_deref() != Some(gp.x.as_str()) || p > qk || r.s.sinks_past(p, qk, &[]) {
            continue;
        }
        pair(r, "qk_norm_rope_gate_split", ClassId::QkNormRopeGateSplit, p, qk, qk);
    }
}

/// A layer's normed rotations by one table at one position — its query and key branches, one inserting its row into the
/// cache — as one op over the row, the later moved up to the first one's place.
fn horizontal_norm_rope_insert(r: &mut Rewriter) {
    for i in 0..r.s.len() {
        let Some(tables) = r.s.slot(i).and_then(normed_rotation) else { continue };
        let mut group = vec![i];
        for j in i + 1..r.s.len() {
            if r.s.slot(j).and_then(normed_rotation).as_ref() != Some(&tables) || r.s.crosses(i, j, &group) {
                continue;
            }
            let ops: Vec<Op> = group.iter().chain([&j]).map(|&k| r.s.get(k).clone()).collect();
            let names: Vec<String> = external(&ops.iter().collect::<Vec<_>>()).into_iter().map(|(t, _)| t).collect();
            let (out, dst) = (ops[0].out.clone(), ops[0].dst.clone());
            if r.admits(&candidate(ClassId::HorizontalNormRopeInsert, ops, names, &out, dst)) {
                group.push(j);
            }
        }
        let inserts = group.iter().any(|&k| leaves(r.s.get(k).clone()).iter().any(|l| l.dst.row.is_some()));
        if group.len() < 2 || !inserts {
            continue;
        }
        let ops: Vec<Op> = group.iter().map(|&k| r.s.get(k).clone()).collect();
        let ins = external(&ops.iter().collect::<Vec<_>>());
        let (out, dst) = (ops[0].out.clone(), ops[0].dst.clone());
        r.fuse("horizontal_norm_rope_insert", ClassId::HorizontalNormRopeInsert, i, &group[1..], ins, out, dst);
    }
}

/// The rule table, in the order the rules apply.
pub const RULES: &[Rule] = &[
    Rule { name: "add_norm", kind: RuleKind::Group, builds: &[ClassId::AddNorm], apply: add_norm },
    Rule { name: "sandwich_norm", kind: RuleKind::Group, builds: &[ClassId::NormThenAdd], apply: sandwich_norm },
    Rule { name: "gemv_bias", kind: RuleKind::Group, builds: &[ClassId::GemmBias], apply: gemv_bias },
    Rule { name: "gemm_scale", kind: RuleKind::Group, builds: &[ClassId::GemmScale], apply: gemm_scale },
    Rule { name: "affine_input_fold", kind: RuleKind::Group, builds: &[ClassId::AffineInputFold], apply: affine_input_fold },
    Rule { name: "gemv_norm_epilogue", kind: RuleKind::Group, builds: &[ClassId::GemvNormEpilogue], apply: gemv_norm_epilogue },
    Rule { name: "prefill_norm_epilogues", kind: RuleKind::Group, builds: &[ClassId::PrefillNormEpilogues], apply: prefill_norm_epilogues },
    Rule { name: "norm_scale", kind: RuleKind::Group, builds: &[ClassId::NormScale], apply: norm_scale },
    Rule { name: "head_norm_rope", kind: RuleKind::Group, builds: &[ClassId::QkNormRope], apply: head_norm_rope },
    Rule { name: "slice_read", kind: RuleKind::Place, builds: &[ClassId::StridedRead], apply: slice_read },
    Rule { name: "row_write", kind: RuleKind::Place, builds: &[ClassId::RowWrite], apply: row_write },
    Rule { name: "qkv_head_epilogue", kind: RuleKind::Group, builds: &[ClassId::QkvHeadEpilogue], apply: qkv_head_epilogue },
    Rule { name: "join_in_place", kind: RuleKind::Place, builds: &[ClassId::JoinInPlace], apply: join_in_place },
    Rule { name: "conv_silu", kind: RuleKind::Group, builds: &[ClassId::ConvBiasSilu], apply: conv_silu },
    Rule { name: "conv_silu_l2", kind: RuleKind::Group, builds: &[ClassId::ConvSiluL2norm], apply: conv_silu_l2 },
    Rule { name: "conv_parts", kind: RuleKind::Group, builds: &[ClassId::ConvParts], apply: conv_parts },
    Rule { name: "gates", kind: RuleKind::Group, builds: &[ClassId::GdnGating], apply: gates },
    Rule { name: "gate_projections", kind: RuleKind::Group, builds: &[ClassId::GdnGateProjections], apply: gate_projections },
    Rule { name: "gdn_input_epilogue", kind: RuleKind::Group, builds: &[ClassId::GdnInputEpilogue], apply: gdn_input_epilogue },
    Rule { name: "gdn_post_conv_prep", kind: RuleKind::Group, builds: &[ClassId::GdnPostConvPrep], apply: gdn_post_conv_prep },
    Rule { name: "gdn_decode_whole", kind: RuleKind::Group, builds: &[ClassId::GdnDecodeWhole], apply: gdn_decode_whole },
    Rule { name: "act_gate", kind: RuleKind::Group, builds: &[ClassId::GluActMul], apply: act_gate },
    Rule { name: "attn_output_gate", kind: RuleKind::Group, builds: &[ClassId::AttnOutputGate], apply: attn_output_gate },
    Rule { name: "glu_pair", kind: RuleKind::Group, builds: &[ClassId::GemvGateUpGlu], apply: glu_pair },
    Rule { name: "relu_sqr", kind: RuleKind::Group, builds: &[ClassId::ReluSqr], apply: relu_sqr },
    Rule { name: "elementwise_regions", kind: RuleKind::Group, builds: &[ClassId::ElementwiseRegions], apply: elementwise_regions },
    Rule { name: "elementwise_chains", kind: RuleKind::Group, builds: &[ClassId::ElementwiseChains], apply: elementwise_chains },
    Rule { name: "recompute_fanout", kind: RuleKind::Group, builds: &[ClassId::RecomputeFanout], apply: recompute_fanout },
    Rule { name: "shared_expert", kind: RuleKind::Group, builds: &[ClassId::SharedExpertFusion], apply: shared_expert },
    Rule { name: "moe_rows", kind: RuleKind::Group, builds: &[ClassId::MoeTokenSort], apply: moe_rows },
    Rule { name: "moe_deferred_finalize", kind: RuleKind::Group, builds: &[ClassId::MoeDeferredFinalize], apply: moe_deferred_finalize },
    Rule { name: "gather_scale", kind: RuleKind::Group, builds: &[ClassId::EmbedScale], apply: gather_scale },
    Rule { name: "embed_norm", kind: RuleKind::Group, builds: &[ClassId::EmbedNorm], apply: embed_norm },
    Rule { name: "strided_values", kind: RuleKind::Place, builds: &[ClassId::StridedValues], apply: strided_values },
    Rule { name: "mla_unabsorb_in_fold", kind: RuleKind::Group, builds: &[ClassId::MlaUnabsorbInFold], apply: mla_unabsorb_in_fold },
    Rule { name: "dual_norm", kind: RuleKind::Group, builds: &[ClassId::DualNorm], apply: dual_norm },
    Rule { name: "cross_layer_launch", kind: RuleKind::Group, builds: &[ClassId::CrossLayerLaunch], apply: cross_layer_launch },
    Rule { name: "rope_parts", kind: RuleKind::Group, builds: &[ClassId::RopeQkOneLaunch], apply: rope_parts },
    Rule { name: "qk_norm_rope_gate_split", kind: RuleKind::Group, builds: &[ClassId::QkNormRopeGateSplit], apply: qk_norm_rope_gate_split },
    Rule { name: "horizontal_norm_rope_insert", kind: RuleKind::Group, builds: &[ClassId::HorizontalNormRopeInsert], apply: horizontal_norm_rope_insert },
    Rule { name: "mla_prologue", kind: RuleKind::Group, builds: &[ClassId::MlaPrologue], apply: mla_prologue },
    Rule { name: "last_layer_rows", kind: RuleKind::Group, builds: &[ClassId::LastLayerRows], apply: last_layer_rows },
];

/// The rule that always closes the table: the arg-max that picks the next token moves the step on.
pub const ADVANCE: Rule = Rule { name: "argmax_advance", kind: RuleKind::Group, builds: &[ClassId::FusedSampling], apply: argmax_advance };

/// The rules that apply once every product's activation is quantized where it will be read.
pub const LATE: &[Rule] = &[Rule { name: "merged_projections", kind: RuleKind::Group, builds: &[ClassId::MergedQkv], apply: merged_projections }];

/// The lifted ops with every row of a computed table taken at a literal index read in place — a view into the table —
/// unless the table is written in place before the row's last reader, where the row is copied where the program takes
/// it.
fn views(g: &TensorGraph) -> Vec<Op> {
    let mut last_reader: BTreeMap<&str, usize> = BTreeMap::new();
    for (k, op) in g.ops.iter().enumerate() {
        for x in &op.ins {
            last_reader.insert(x.as_str(), k);
        }
    }
    g.ops
        .iter()
        .enumerate()
        .map(|(i, op)| {
            let Some(TensorOpSpec::RowGather(rg)) = op.prim() else { return op.clone() };
            let RowIndex::Literal(row) = rg.index else { return op.clone() };
            let table = &op.ins[0];
            let computed = g.tensors.get(table).is_some_and(|t| t.role != Role::Constant);
            if !computed || row < 1 || row as usize > rg.rows {
                return op.clone();
            }
            let last = last_reader.get(op.out.as_str()).copied().unwrap_or(i);
            let rewritten = g.ops[i + 1..last.max(i + 1)].iter().any(|o| o.state_inputs().into_iter().any(|k| o.ins[k] == *table));
            if rewritten {
                return op.clone();
            }
            Op { sym: Sym::View { offset: (row as usize - 1) * rg.cols }, ..op.clone() }
        })
        .collect()
}

/// `g` with the shared rules but those `held` applied under `shim`, every firing, and the report of every decision. A
/// rule held, or one building a class the chip does not link whole, is run on a copy of the graph to report where it
/// would have fired.
pub fn optimize(g: &TensorGraph, shim: &dyn Shim, held: &[&str]) -> Optimized {
    // A row of a computed table read in place is a part read in place: only a chip that links one takes the view.
    let lifted = if shim.supports(ClassId::StridedRead) == Support::Native { views(g) } else { g.ops.clone() };
    let mut r = Rewriter { s: Slots::new(lifted), g, shim, fired: Vec::new(), rule: "", building: ADVANCE.builds[0], report: Vec::new() };
    let chip = shim.rules();
    let unlinked = |builds: &[ClassId]| {
        builds.iter().find_map(|c| match shim.supports(*c) {
            Support::Native => None,
            Support::Decompose => Some(format!("the chip runs `{c}` as its parts")),
            Support::Refuse(why) => Some(format!("{c}: {why}")),
        })
    };
    let step = |r: &mut Rewriter, name: &'static str, builds: &[ClassId], apply: &dyn Fn(&mut Rewriter)| {
        r.rule = name;
        r.building = builds[0];
        if held.contains(&name) {
            r.dry(apply, shim, &|_| Outcome::Held);
        } else if let Some(why) = unlinked(builds) {
            r.dry(apply, &Linking { base: shim, classes: builds }, &|_| Outcome::Refused(why.clone()));
        } else {
            apply(r);
        }
    };
    let at = |r: &mut Rewriter, position: Position| {
        for (_, rule) in chip.iter().filter(|(p, _)| *p == position) {
            step(r, rule.name, rule.builds, &*rule.apply);
        }
    };
    for rule in RULES {
        at(&mut r, Position::Before(rule.name));
        step(&mut r, rule.name, rule.builds, &rule.apply);
        at(&mut r, Position::After(rule.name));
    }
    r.rule = ADVANCE.name;
    r.building = ADVANCE.builds[0];
    if unlinked(ADVANCE.builds).is_none() {
        (ADVANCE.apply)(&mut r);
    }
    r.rule = "fused_quant";
    r.building = ClassId::GemvQuant;
    let fold = !held.contains(&"fused_quant");
    if !fold {
        r.dry(&|r: &mut Rewriter| super::numerics::quantize(r, true), shim, &|_| Outcome::Held);
    }
    super::numerics::quantize(&mut r, fold);
    for rule in LATE {
        at(&mut r, Position::Before(rule.name));
        step(&mut r, rule.name, rule.builds, &rule.apply);
        at(&mut r, Position::After(rule.name));
    }
    at(&mut r, Position::End);
    let ops = r.s.into_ops();
    Optimized { graph: TensorGraph { ops, ..g.clone() }, fired: r.fired, report: Report { decisions: r.report } }
}

/// Every step a compile under `shim` can hold, in the order it runs them: each shared rule with the chip's own rules
/// before and after it, the numerics fold (`fused_quant`, a producer taking the quantize of its result), the late rules
/// with theirs, then the chip's closing rules. Holding every step from one on leaves the graph as the steps before it
/// make it.
pub fn steps(shim: &dyn Shim) -> Vec<&'static str> {
    let chip = shim.rules();
    let at = |p: Position| chip.iter().filter(move |(q, _)| *q == p).map(|(_, rule)| rule.name).collect::<Vec<_>>();
    let mut out = Vec::new();
    let rules = |list: &[Rule], out: &mut Vec<&'static str>| {
        for rule in list {
            out.extend(at(Position::Before(rule.name)));
            out.push(rule.name);
            out.extend(at(Position::After(rule.name)));
        }
    };
    rules(RULES, &mut out);
    out.push("fused_quant");
    rules(LATE, &mut out);
    out.extend(at(Position::End));
    out
}

/// The names of the shared rules a compile can hold back, in the order they apply.
pub fn rule_names() -> Vec<&'static str> {
    RULES.iter().chain(LATE).map(|r| r.name).collect()
}

/// The classes the table builds, by rule.
pub fn classes_built() -> BTreeMap<&'static str, &'static [ClassId]> {
    RULES.iter().chain(LATE).chain([&ADVANCE]).map(|r| (r.name, r.builds)).collect()
}
