//! The engine's program read off a model's step graph: the decode step [`crate::model::author::decode_step`] writes
//! for a [`ModelSpec`], lifted into the one IR and fused by the shared table under the engine's shim
//! ([`super::shim::EngineShim`]), read block by block into the [`Command`]s of [`super::program`].
//!
//! [`lower_graph`] reads the step as the blocks the engine has units for: the embedding, then each layer's norm, its
//! mixer (a Gated-DeltaNet block or an attention block), the residual add, the FFN's norm, the SwiGLU FFN and its add,
//! then the step's advance, the final norm and the head. Every op of a block is checked three ways before the block
//! is written: its kind, the tensors it reads and writes (each block reads the residual the block before left and the
//! next one reads what it wrote), and the checkpoint tensor and view each constant it reads is bound to
//! ([`crate::model::bind`]). The engine's merges are the shared table's fused symbols, each read as the unit that runs
//! it: the projections of one checkpoint matrix (`merged_qkv`) — row ranges of one tensor that follow one another from
//! its first row (a Gated-DeltaNet block's query, key and value of `attn_qkv`), or the interleaved halves of each head
//! (an attention block's queries and output gate of `attn_q`) — as one matvec; each convolution with its SiLU
//! (`conv_bias_silu`) as a channel of the conv unit; the sigmoid and the decay a delta rule reads (`gdn_gating`) as the
//! gates; and an attention's output gate (`glu_act_mul`) as the sigmoid gate. A block is written in the order the
//! engine's units take it, which is not the order the author writes its ops in. What a block does that the engine has
//! no unit for — a norm after the mixer, an FFN that is not SwiGLU, experts, latent attention, attention sinks, a
//! sliding window, a scaled embedding — is refused by name, so a program the engine runs computes what the step graph
//! says.

use crate::model::bind::{self, Binding, Source, View};
use crate::model::spec::{Act, Attention, Ffn, GatedDelta, Mixer, ModelSpec};
use crate::spatial::passes::recognize_stages::TensorOpSpec;
use crate::tensor_graph::graph::{Fused, Op, Sym, TensorGraph};
use crate::tensor_graph::symbols::ClassId;
use crate::tensor_recognize::{ElementwiseOp, RowReduceKind};

use super::model::{CacheFormat, EngineNumerics};
use super::program::{Command, Program, Slot};

/// Why a step graph is not a program the engine runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LowerError {
    /// An op other than the one the block expects next: where, what it is, and what was expected.
    Unexpected { op: usize, kind: &'static str, block: String, expected: String },
    /// The step ended inside a block.
    Ended { block: String, expected: String },
    /// An op reads a tensor other than the one the block's dataflow says it reads.
    Dataflow { op: usize, block: String, expected: String, found: String },
    /// A constant the checkpoint does not bind, or binds through a view the block does not read it by.
    Binding { constant: String, why: String },
    /// A part of the model the engine has no unit for, by name.
    Unsupported { layer: Option<usize>, what: String },
    /// Ops left after the head.
    Trailing { op: usize, kind: &'static str },
}

impl std::fmt::Display for LowerError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LowerError::Unexpected { op, kind, block, expected } => write!(f, "op {op} ({kind}) in {block}, where {expected} comes next"),
            LowerError::Ended { block, expected } => write!(f, "the step ends in {block}, before {expected}"),
            LowerError::Dataflow { op, block, expected, found } => write!(f, "op {op} in {block} reads {found}, where it reads {expected}"),
            LowerError::Binding { constant, why } => write!(f, "{constant}: {why}"),
            LowerError::Unsupported { layer: Some(l), what } => write!(f, "layer {l}: the engine has no unit for {what}"),
            LowerError::Unsupported { layer: None, what } => write!(f, "the engine has no unit for {what}"),
            LowerError::Trailing { op, kind } => write!(f, "op {op} ({kind}) after the head"),
        }
    }
}

impl std::error::Error for LowerError {}

/// An op's kind as a refusal names it: a lifted op's sugar, or a fused symbol's name.
fn kind(op: &Op) -> &'static str {
    match &op.sym {
        Sym::Prim(spec) => spec.kind(),
        Sym::Fused(f) => f.class.id(),
        Sym::View { .. } => "View",
        Sym::Quantize(_) => "Quantize",
    }
}

/// The lifted op `op` is, which a block reads as a part of a unit.
fn prim(op: &Op) -> &TensorOpSpec {
    op.prim().expect("a part a unit runs is a lifted op")
}

/// The program of `graph`, the step `spec` authors as the engine's shim leaves it, at `numerics`.
pub fn lower_graph(graph: &TensorGraph, spec: &ModelSpec, numerics: EngineNumerics) -> Result<Program, LowerError> {
    let refuse = |what: &str| Err(LowerError::Unsupported { layer: None, what: what.into() });
    if spec.embedding_scale.is_some() {
        return refuse("a scaled embedding");
    }
    if spec.final_logit_softcap.is_some() {
        return refuse("capped logits");
    }
    let mut r = Reader { graph, spec, at: 0, block: String::new(), commands: Vec::new() };
    let mut residual = r.embed()?;
    for l in 0..spec.layers.len() {
        residual = r.layer(l, &residual, numerics)?;
    }
    r.head(&residual)?;
    if let Some(op) = graph.ops.get(r.at) {
        return Err(LowerError::Trailing { op: r.at, kind: kind(op) });
    }
    Ok(Program { numerics, commands: r.commands })
}

/// A walk over the step's ops in order, writing commands block by block.
struct Reader<'a> {
    graph: &'a TensorGraph,
    spec: &'a ModelSpec,
    at: usize,
    block: String,
    commands: Vec<Command>,
}

impl<'a> Reader<'a> {
    /// The next op, which must be of `kind`.
    fn take(&mut self, kind: &'static str) -> Result<&'a Op, LowerError> {
        let Some(op) = self.graph.ops.get(self.at) else {
            return Err(LowerError::Ended { block: self.block.clone(), expected: kind.into() });
        };
        if self::kind(op) != kind {
            return Err(LowerError::Unexpected { op: self.at, kind: self::kind(op), block: self.block.clone(), expected: kind.into() });
        }
        self.at += 1;
        Ok(op)
    }

    /// The next op, which must be the fused symbol `class`, and its members.
    fn take_fused(&mut self, class: ClassId) -> Result<(&'a Op, &'a [Op]), LowerError> {
        let op = self.take(class.id())?;
        let Sym::Fused(Fused { parts, .. }) = &op.sym else { unreachable!("taken as a fused symbol") };
        Ok((op, parts))
    }

    /// Whether the next op is of `kind`.
    fn next_is(&self, kind: &str) -> bool {
        self.graph.ops.get(self.at).is_some_and(|op| self::kind(op) == kind)
    }

    /// That `op` (the one just taken) reads `expected` as its operand `k`.
    fn reads(&self, op: &Op, k: usize, expected: &str) -> Result<(), LowerError> {
        match op.ins.get(k) {
            Some(found) if found == expected => Ok(()),
            found => Err(LowerError::Dataflow {
                op: self.at - 1,
                block: self.block.clone(),
                expected: expected.into(),
                found: found.cloned().unwrap_or_else(|| "nothing".into()),
            }),
        }
    }

    /// The checkpoint tensor and view `constant` is bound to.
    fn bound(&self, constant: &str) -> Result<Binding, LowerError> {
        match bind::bind(self.spec, constant) {
            Ok(Source::Weight(b)) => Ok(b),
            Ok(Source::State) => Err(LowerError::Binding { constant: constant.into(), why: "carried state where a weight is read".into() }),
            Err(bind::Unbound(_)) => Err(LowerError::Binding { constant: constant.into(), why: "the checkpoint binds nothing to it".into() }),
        }
    }

    /// The tensor `constant` reads whole, as the matrix its projection multiplies by (its rows the outputs), in the
    /// checkpoint's own row order or reordered within heads for the author's rotation.
    fn matrix(&self, constant: &str) -> Result<String, LowerError> {
        let b = self.bound(constant)?;
        match b.view {
            View::Transposed | View::NeoxPairs { transpose: true, .. } => Ok(b.tensor),
            view => Err(LowerError::Binding { constant: constant.into(), why: format!("bound through {view:?}, not as a whole matrix") }),
        }
    }

    /// The tensor `constant` reads whole, element for element.
    fn vector(&self, constant: &str) -> Result<String, LowerError> {
        let b = self.bound(constant)?;
        match b.view {
            View::Same | View::NeoxPairs { transpose: false, .. } => Ok(b.tensor),
            view => Err(LowerError::Binding { constant: constant.into(), why: format!("bound through {view:?}, not as a whole vector") }),
        }
    }

    /// The one tensor `constants` split into row ranges, each following the one before from the tensor's first row,
    /// and the rows each takes.
    fn row_ranges(&self, constants: &[&str]) -> Result<(String, Vec<usize>), LowerError> {
        let (mut tensor, mut next, mut counts) = (None::<String>, 0usize, Vec::new());
        for c in constants {
            let b = self.bound(c)?;
            let View::RowRange { first, count, transpose: true } = b.view else {
                return Err(LowerError::Binding { constant: (*c).into(), why: format!("bound through {:?}, not as a row range", b.view) });
            };
            match &tensor {
                Some(t) if *t != b.tensor => {
                    return Err(LowerError::Binding { constant: (*c).into(), why: format!("a range of {}, not of {t}", b.tensor) });
                }
                _ => tensor = Some(b.tensor),
            }
            if first != next {
                return Err(LowerError::Binding { constant: (*c).into(), why: format!("rows from {first}, where the range before ends at {next}") });
            }
            next = first + count;
            counts.push(count);
        }
        Ok((tensor.expect("at least one constant"), counts))
    }

    /// `x` RMS-normed by the weight it reads: the norm op and its weight.
    fn norm(&mut self, x: &str) -> Result<(&'a Op, String), LowerError> {
        let op = self.take("FusedRow")?;
        match prim(op) {
            TensorOpSpec::FusedRow(r) if r.reduce == RowReduceKind::RmsNorm && r.bias.is_none() && r.residual.is_none() => {}
            other => {
                return Err(LowerError::Unexpected { op: self.at - 1, kind: other.kind(), block: self.block.clone(), expected: "an RMS norm with a weight".into() });
            }
        }
        self.reads(op, 0, x)?;
        let weight = op.ins.get(1).cloned().ok_or_else(|| LowerError::Dataflow {
            op: self.at - 1,
            block: self.block.clone(),
            expected: "a norm weight".into(),
            found: "nothing".into(),
        })?;
        Ok((op, weight))
    }

    /// A projection of `x`: the op and the constant it multiplies by.
    fn projection(&mut self, x: &str) -> Result<(&'a Op, String), LowerError> {
        let op = self.take("Matmul")?;
        self.reads(op, 0, x)?;
        Ok((op, op.ins.get(1).cloned().unwrap_or_default()))
    }

    /// `a op b` into the op's result.
    fn elementwise(&mut self, op: ElementwiseOp, a: &str, b: &str) -> Result<&'a Op, LowerError> {
        let e = self.take("Elementwise")?;
        self.product(e, op, a, b)?;
        Ok(e)
    }

    /// That `e`, the op just taken or a member of it, is `a op b`.
    fn product(&self, e: &Op, op: ElementwiseOp, a: &str, b: &str) -> Result<(), LowerError> {
        if !matches!(e.prim(), Some(TensorOpSpec::Elementwise(s)) if s.op == op && s.row.is_none()) {
            return Err(LowerError::Unexpected { op: self.at - 1, kind: kind(e), block: self.block.clone(), expected: format!("an elementwise {op:?}") });
        }
        self.reads(e, 0, a)?;
        self.reads(e, 1, b)
    }

    fn embed(&mut self) -> Result<String, LowerError> {
        self.block = "the embedding".into();
        let op = self.take("RowGather")?;
        let table = self.vector(&op.ins[0])?;
        self.commands.push(Command::Embed { table });
        Ok(op.out.clone())
    }

    /// Layer `l`, reading the residual `x`; the residual it leaves.
    fn layer(&mut self, l: usize, x: &str, numerics: EngineNumerics) -> Result<String, LowerError> {
        let spec: &'a ModelSpec = self.spec;
        let layer = &spec.layers[l];
        let refuse = |what: &str| Err(LowerError::Unsupported { layer: Some(l), what: what.into() });
        if layer.post_mixer_norm || layer.post_ffn_norm {
            return refuse("a norm after the mixer or the FFN");
        }
        self.block = format!("layer {l}'s norm");
        let (normed, weight) = self.norm(x)?;
        let normed = normed.out.clone();
        let attn_norm = self.vector(&weight)?;
        self.commands.push(Command::Norm { weight: attn_norm, from: Slot::Residual, to: Slot::Normed });
        let out = match &layer.mixer {
            Mixer::GatedDelta(gd) => self.delta(l, &normed, gd)?,
            Mixer::Attention(a) => self.attention(l, &normed, a, numerics)?,
            Mixer::Mla(_) => return refuse("multi-head latent attention"),
        };
        self.block = format!("layer {l}'s mixer add");
        let mixed = self.elementwise(ElementwiseOp::Add, x, &out)?.out.clone();
        self.commands.push(Command::Add { from: Slot::Out, to: Slot::Residual });
        match &layer.ffn {
            Ffn::Dense { act: Act::Silu, .. } => {}
            Ffn::Dense { .. } => return refuse("an FFN gated by other than SiLU"),
            Ffn::Moe(_) => return refuse("a mixture of experts"),
        }
        self.block = format!("layer {l}'s FFN");
        let (normed, weight) = self.norm(&mixed)?;
        let normed = normed.out.clone();
        let ffn_norm = self.vector(&weight)?;
        self.commands.push(Command::Norm { weight: ffn_norm, from: Slot::Residual, to: Slot::Normed });
        let (gate, wg) = self.projection(&normed)?;
        let (up, wu) = self.projection(&normed)?;
        let swiglu = self.take("SwiGlu")?;
        self.reads(swiglu, 0, &gate.out)?;
        self.reads(swiglu, 1, &up.out)?;
        let (down, wd) = self.projection(&swiglu.out)?;
        let (ffn_gate, ffn_up, ffn_down) = (self.matrix(&wg)?, self.matrix(&wu)?, self.matrix(&wd)?);
        self.commands.push(Command::Matvec { tensor: ffn_gate, from: Slot::Normed, to: Slot::FfnGate });
        self.commands.push(Command::Matvec { tensor: ffn_up, from: Slot::Normed, to: Slot::FfnUp });
        self.commands.push(Command::SwiGlu { gate: Slot::FfnGate, up: Slot::FfnUp, to: Slot::Hidden });
        self.commands.push(Command::Matvec { tensor: ffn_down, from: Slot::Hidden, to: Slot::Out });
        self.block = format!("layer {l}'s FFN add");
        let out = self.elementwise(ElementwiseOp::Add, &mixed, &down.out)?.out.clone();
        self.commands.push(Command::Add { from: Slot::Out, to: Slot::Residual });
        Ok(out)
    }

    /// A Gated-DeltaNet block over the normed `x`: the block's output.
    fn delta(&mut self, l: usize, x: &str, gd: &GatedDelta) -> Result<String, LowerError> {
        self.block = format!("layer {l}'s Gated-DeltaNet block");
        let (hd, nk, nv) = (gd.head_dim, gd.k_heads, gd.v_heads);
        let (_, projections) = self.take_fused(ClassId::MergedQkv)?;
        let [q, k, v] = projections else {
            return Err(LowerError::Unexpected { op: self.at - 1, kind: ClassId::MergedQkv.id(), block: self.block.clone(), expected: "the query, key and value as one matvec".into() });
        };
        for p in [q, k, v] {
            self.reads(p, 0, x)?;
        }
        let (wq, wk, wv) = (q.ins[1].clone(), k.ins[1].clone(), v.ins[1].clone());
        let (z, wz) = self.projection(x)?;
        let (qkv, rows) = self.row_ranges(&[&wq, &wk, &wv])?;
        if rows != [nk * hd, nk * hd, nv * hd] {
            return Err(LowerError::Binding { constant: wq.clone(), why: format!("query, key and value of {rows:?} rows, where the spec says {}, {} and {}", nk * hd, nk * hd, nv * hd) });
        }
        let gate = self.matrix(&wz)?;
        let (mut convs, mut silus) = (Vec::new(), Vec::new());
        for from in [q, k, v] {
            let (_, channel) = self.take_fused(ClassId::ConvBiasSilu)?;
            let [c, s] = channel else { unreachable!("a convolution and its SiLU") };
            self.reads(c, 0, &from.out)?;
            self.reads(s, 0, &c.out)?;
            convs.push(c);
            silus.push(s);
        }
        let kernels: Vec<&str> = convs.iter().map(|c| c.ins[1].as_str()).collect();
        let (kernel, parts) = self.row_ranges(&kernels)?;
        if parts != rows {
            return Err(LowerError::Binding { constant: kernels[0].into(), why: format!("the convolution's parts of {parts:?} rows, where its operands are {rows:?}") });
        }
        let taps = match prim(convs[0]) {
            TensorOpSpec::CausalConv(c) => c.taps,
            _ => unreachable!("a channel's convolution"),
        };
        if taps != gd.conv_taps {
            return Err(LowerError::Binding { constant: kernels[0].into(), why: format!("{taps} taps, where the spec says {}", gd.conv_taps) });
        }
        let mut norms = Vec::new();
        for s in &silus[..2] {
            let n = self.take("L2Norm")?;
            self.reads(n, 0, &s.out)?;
            norms.push(n);
        }
        let (beta, wb) = self.projection(x)?;
        let (alpha, wa) = self.projection(x)?;
        let (_, gates) = self.take_fused(ClassId::GdnGating)?;
        let [sigmoid, decay] = gates else { unreachable!("a write strength and a forget gate") };
        self.reads(sigmoid, 0, &beta.out)?;
        self.reads(decay, 0, &alpha.out)?;
        let (dt, a) = (self.vector(&decay.ins[1])?, self.vector(&decay.ins[2])?);
        let delta = self.take("GatedDelta")?;
        for (k, from) in [&norms[0].out, &norms[1].out, &silus[2].out, &z.out, &sigmoid.out, &decay.out].into_iter().enumerate() {
            self.reads(delta, k, from)?;
        }
        let ssm_norm = self.vector(&delta.ins[7])?;
        let (out, wo) = self.projection(&delta.out)?;
        let (ssm_beta, ssm_alpha, ssm_out) = (self.matrix(&wb)?, self.matrix(&wa)?, self.matrix(&wo)?);
        self.commands.push(Command::Matvec { tensor: qkv, from: Slot::Normed, to: Slot::Qkv });
        self.commands.push(Command::Matvec { tensor: gate, from: Slot::Normed, to: Slot::Z });
        self.commands.push(Command::Matvec { tensor: ssm_beta, from: Slot::Normed, to: Slot::Beta });
        self.commands.push(Command::Matvec { tensor: ssm_alpha, from: Slot::Normed, to: Slot::Alpha });
        self.commands.push(Command::Conv { layer: l, kernel, taps, from: Slot::Qkv, to: Slot::Mixed });
        self.commands.push(Command::L2Heads { from: Slot::Mixed, at: 0, heads: nk, width: hd, to: Slot::Query });
        self.commands.push(Command::L2Heads { from: Slot::Mixed, at: nk * hd, heads: nk, width: hd, to: Slot::Key });
        self.commands.push(Command::Gates {
            alpha: Slot::Alpha,
            beta: Slot::Beta,
            dt,
            a,
            log_decay: Slot::LogDecay,
            decay: Slot::Decay,
            gate: Slot::Gate,
        });
        self.commands.push(Command::Delta {
            layer: l,
            query: Slot::Query,
            key: Slot::Key,
            value: Slot::Mixed,
            value_at: 2 * nk * hd,
            log_decay: Slot::LogDecay,
            decay: Slot::Decay,
            gate: Slot::Gate,
            k_heads: nk,
            v_heads: nv,
            width: hd,
            to: Slot::Read,
        });
        self.commands.push(Command::GatedNorm { weight: ssm_norm, from: Slot::Read, z: Slot::Z, width: hd, to: Slot::Read });
        self.commands.push(Command::Matvec { tensor: ssm_out, from: Slot::Read, to: Slot::Out });
        Ok(out.out.clone())
    }

    /// An attention block over the normed `x`: the block's output.
    fn attention(&mut self, l: usize, x: &str, a: &Attention, numerics: EngineNumerics) -> Result<String, LowerError> {
        self.block = format!("layer {l}'s attention block");
        let refuse = |what: &str| Err(LowerError::Unsupported { layer: Some(l), what: what.into() });
        if a.sinks {
            return refuse("attention sinks");
        }
        if a.window.is_some() {
            return refuse("a sliding window");
        }
        if a.bias || a.output_bias {
            return refuse("biased projections");
        }
        if a.softcap.is_some() {
            return refuse("capped scores");
        }
        if a.fused_qkv {
            return refuse("one projection split into queries, keys and values");
        }
        let (hd, vd) = (a.head_dim, a.value_dim);
        let rotated = numerics.cache == CacheFormat::Int8Hadamard;
        let (query_out, gate_out, attn_q) = if a.output_gate {
            let (_, projections) = self.take_fused(ClassId::MergedQkv)?;
            let [q, g] = projections else {
                return Err(LowerError::Unexpected { op: self.at - 1, kind: ClassId::MergedQkv.id(), block: self.block.clone(), expected: "the queries and their output gate as one matvec".into() });
            };
            self.reads(q, 0, x)?;
            self.reads(g, 0, x)?;
            let (wq, wqg) = (q.ins[1].clone(), g.ins[1].clone());
            let (bq, bg) = (self.bound(&wq)?, self.bound(&wqg)?);
            let halves = matches!(
                (&bq.view, &bg.view),
                (View::HeadPart { stride: s1, offset: 0, width: w1, transpose: true }, View::HeadPart { stride: s2, offset: o2, width: w2, transpose: true })
                    if *s1 == 2 * hd && *s2 == 2 * hd && *w1 == hd && *o2 == hd && *w2 == hd
            );
            if !halves || bq.tensor != bg.tensor {
                return Err(LowerError::Binding { constant: wq, why: format!("queries {:?} of {} and gates {:?} of {}, not one head's halves of one matrix", bq.view, bq.tensor, bg.view, bg.tensor) });
            }
            (q.out.clone(), Some(g.out.clone()), bq.tensor)
        } else {
            let (q, wq) = self.projection(x)?;
            (q.out.clone(), None, self.matrix(&wq)?)
        };
        let (k, wk) = self.projection(x)?;
        let (v, wv) = self.projection(x)?;
        let (attn_k, attn_v) = (self.matrix(&wk)?, self.matrix(&wv)?);
        let (mut query, mut key) = (query_out, k.out.clone());
        let mut heads = Vec::new();
        if a.qk_norm {
            for (slot, from) in [(Slot::Query, &mut query), (Slot::Key, &mut key)] {
                let (op, weight) = self.norm(from)?;
                *from = op.out.clone();
                heads.push((slot, Some(self.vector(&weight)?)));
            }
        } else {
            heads = vec![(Slot::Query, None), (Slot::Key, None)];
        }
        let mut turned = [false, false];
        if a.rope.is_some() {
            for (i, from) in [&mut query, &mut key].into_iter().enumerate() {
                let op = self.take("Rope")?;
                self.reads(op, 0, from)?;
                *from = op.out.clone();
                turned[i] = true;
            }
        }
        let set_k = self.take("SetRow")?;
        self.reads(set_k, 1, &key)?;
        let set_v = self.take("SetRow")?;
        self.reads(set_v, 1, &v.out)?;
        let attend = self.take("Attention")?;
        self.reads(attend, 0, &query)?;
        let mut read = attend.out.clone();
        if let Some(gate) = &gate_out {
            let (gated, parts) = self.take_fused(ClassId::GluActMul)?;
            let [s, product] = parts else { unreachable!("an activation and the product it gates") };
            self.reads(s, 0, gate)?;
            self.product(product, ElementwiseOp::Mul, &read, &s.out)?;
            read = gated.out.clone();
        }
        let (out, wo) = self.projection(&read)?;
        let attn_output = self.matrix(&wo)?;
        let projected = if a.output_gate { Slot::Projected } else { Slot::Query };
        self.commands.push(Command::Matvec { tensor: attn_q, from: Slot::Normed, to: projected });
        self.commands.push(Command::Matvec { tensor: attn_k, from: Slot::Normed, to: Slot::Key });
        self.commands.push(Command::Matvec { tensor: attn_v, from: Slot::Normed, to: Slot::Value });
        if a.output_gate {
            self.commands.push(Command::Split { from: Slot::Projected, heads: a.heads, width: hd, query: Slot::Query, gate: Slot::OutGate });
        }
        for (i, (slot, weight)) in heads.into_iter().enumerate() {
            if let Some(weight) = weight {
                self.commands.push(Command::RmsHeads { weight, slot, width: hd });
            }
            if let (true, Some(rope)) = (turned[i], &a.rope) {
                self.commands.push(Command::Rotate { slot, width: hd, rope: rope.clone() });
            }
        }
        if rotated {
            self.commands.push(Command::Hadamard { slot: Slot::Key, width: hd });
            self.commands.push(Command::Hadamard { slot: Slot::Value, width: vd });
        }
        self.commands.push(Command::CacheAppend { layer: l, key: Slot::Key, value: Slot::Value, kv_heads: a.kv_heads, width: hd, value_width: vd });
        if rotated {
            self.commands.push(Command::Hadamard { slot: Slot::Query, width: hd });
        }
        self.commands.push(Command::Attend {
            layer: l,
            query: Slot::Query,
            heads: a.heads,
            kv_heads: a.kv_heads,
            width: hd,
            value_width: vd,
            scale: a.scale as f32,
            to: Slot::Read,
        });
        if a.output_gate {
            self.commands.push(Command::SigmoidGate { slot: Slot::Read, gate: Slot::OutGate });
        }
        self.commands.push(Command::Matvec { tensor: attn_output, from: Slot::Read, to: Slot::Out });
        Ok(out.out.clone())
    }

    /// The step's advance, the final norm of the residual `x` and the head; the next token's choice after it.
    fn head(&mut self, x: &str) -> Result<(), LowerError> {
        self.block = "the head".into();
        self.commands.push(Command::Advance);
        let (normed, weight) = self.norm(x)?;
        let normed = normed.out.clone();
        let output_norm = self.vector(&weight)?;
        self.commands.push(Command::Norm { weight: output_norm, from: Slot::Residual, to: Slot::Normed });
        let (logits, wout) = self.projection(&normed)?;
        let head = self.matrix(&wout)?;
        self.commands.push(Command::Matvec { tensor: head, from: Slot::Normed, to: Slot::Logits });
        if self.next_is("ArgMax") {
            let pick = self.take("ArgMax")?;
            self.reads(pick, 0, &logits.out)?;
        }
        Ok(())
    }
}

#[cfg(test)]
#[path = "../../../../logicaffeine_tests/unit/logicaffeine_compile/spatial/decode_engine/lower_graph/tests.rs"]
mod tests;
