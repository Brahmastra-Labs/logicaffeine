//! The decode engine's program: one step of a checkpoint as the commands its units run, in order.
//!
//! A step holds its token-rows — every row of every sequence the step advances — in the token-row store, each row a
//! vector in every [`Slot`]. A command is one unit's job over every row of the step, in the step's row order, finished
//! before the next command starts: [`Command::Matvec`] streams one matrix through the array and the block-scale lanes
//! once for all the rows; the norms, the gates and the products are the vector unit's micro-programs;
//! [`Command::Conv`] and [`Command::Delta`] carry a Gated-DeltaNet block's ring and memory, and
//! [`Command::CacheAppend`] and [`Command::Attend`] an attention block's cache, each over every row's own sequence. A
//! command that carries a sequence's state takes its rows in order, so a sequence's rows see one another as a prompt
//! run a token at a time does.
//!
//! [`lower`] reads the step off the model's one program — the decode step [`author`](crate::model::author) writes,
//! lifted into the one IR, fused by the shared table under the engine's shim and read block by block
//! ([`super::lower_graph`]) — and [`super::model::EngineModel`] runs exactly that list: the program is the model, so
//! what the sequencer issues and the cycle twin prices is what the model computes. [`hand_lowering`] writes the same
//! commands from the spec directly; it is kept only as the oracle the step-graph reading is held to.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, Mutex, OnceLock};

use crate::model::author::NotYet;
use crate::model::program::{Cut, Identity};
use crate::model::spec::{Attention, GatedDelta, Mixer, ModelSpec, Rope};
use crate::tensor_graph::fuse::optimize;
use crate::tensor_graph::graph::{GraphError, Storage, TensorGraph};

use super::lower_graph::{lower_graph, LowerError};
use super::shim::EngineShim;
use super::model::{CacheFormat, EngineNumerics};

/// A vector each token-row of a step carries, one of the token-row store's slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Slot {
    /// The residual stream.
    Residual,
    /// The residual normalized, a block's input.
    Normed,
    /// A block's output, before it joins the residual.
    Out,
    /// A Gated-DeltaNet block's fused query, key and value projection.
    Qkv,
    /// The projection through the causal convolution and SiLU.
    Mixed,
    /// The block's output gate, before its SiLU.
    Z,
    /// The projections the decay and the update gate are made from, one element a value head.
    Alpha,
    Beta,
    /// Per value head: the decay's exponent, the decay, the update gate.
    LogDecay,
    Decay,
    Gate,
    /// Queries and keys, a head after another: a Gated-DeltaNet block's L2-normalized, an attention block's
    /// normalized and turned.
    Query,
    Key,
    /// An attention block's values, a head after another.
    Value,
    /// An attention block's query projection, each head's queries and output gate interleaved.
    Projected,
    /// An attention block's output gate, a head after another.
    OutGate,
    /// The read of every head: the memory's read-out, or the attended values.
    Read,
    /// The FFN's gate and up projections, and their gated product.
    FfnGate,
    FfnUp,
    Hidden,
    /// The vocabulary's logits.
    Logits,
}

/// One command of a step.
#[derive(Clone, Debug, PartialEq)]
pub enum Command {
    /// Each row's [`Slot::Residual`] starts as its token's row of the embedding table.
    Embed { table: String },
    /// RMSNorm of each row of `from` by the weights `weight` into `to`.
    Norm { weight: String, from: Slot, to: Slot },
    /// The matrix `tensor` times each row of `from`, into `to`.
    Matvec { tensor: String, from: Slot, to: Slot },
    /// Each row of `from` added into `to`, element by element.
    Add { from: Slot, to: Slot },
    /// `silu(gate)·up` into `to`, element by element.
    SwiGlu { gate: Slot, up: Slot, to: Slot },
    /// Layer `layer`'s causal convolution through SiLU of each row of `from` into `to`, against each row's sequence's
    /// ring of the rows before it; the row as the state stores it then joins the ring.
    Conv { layer: usize, kernel: String, taps: usize, from: Slot, to: Slot },
    /// `heads` heads of `width` elements from element `at` of each row of `from`, each L2-normalized, into `to`.
    L2Heads { from: Slot, at: usize, heads: usize, width: usize, to: Slot },
    /// Per value head, from each row's `alpha` and `beta`: the decay's exponent `softplus(α + dt)·a`, the decay, the
    /// update gate `σ(β)`.
    Gates { alpha: Slot, beta: Slot, dt: String, a: String, log_decay: Slot, decay: Slot, gate: Slot },
    /// Layer `layer`'s memory through each row, a value head a memory: its query and key group's `query` and `key`,
    /// its values from element `value_at` of `value`, its decay and gate; each value head's read-out into `to`.
    Delta {
        layer: usize,
        query: Slot,
        key: Slot,
        value: Slot,
        value_at: usize,
        log_decay: Slot,
        decay: Slot,
        gate: Slot,
        k_heads: usize,
        v_heads: usize,
        width: usize,
        to: Slot,
    },
    /// Each head of `width` elements of `from` RMS-normed by `weight` and gated by `silu(z)`, into `to`.
    GatedNorm { weight: String, from: Slot, z: Slot, width: usize, to: Slot },
    /// Each row of `from`, `heads` heads of `width` queries each followed by its `width` gates, split into `query`
    /// and `gate`.
    Split { from: Slot, heads: usize, width: usize, query: Slot, gate: Slot },
    /// Each head of `width` elements of `slot` RMS-normed by `weight`, in place.
    RmsHeads { weight: String, slot: Slot, width: usize },
    /// Each head of `width` elements of `slot` turned by its row's position ([`super::numerics::rope`]).
    Rotate { slot: Slot, width: usize, rope: Rope },
    /// Each head of `width` elements of `slot` rotated by the Walsh–Hadamard transform, in place.
    Hadamard { slot: Slot, width: usize },
    /// Each row's keys and values, `kv_heads` heads each, appended to layer `layer`'s cache of the row's sequence in
    /// the cache's format.
    CacheAppend { layer: usize, key: Slot, value: Slot, kv_heads: usize, width: usize, value_width: usize },
    /// Layer `layer`'s attention of each row's `heads` query heads over its sequence's cache to the row's own
    /// position, the heads in groups over the `kv_heads` cached heads, scores scaled by `scale`; each head's attended
    /// values into `to`, rotated back when the cache is.
    Attend { layer: usize, query: Slot, heads: usize, kv_heads: usize, width: usize, value_width: usize, scale: f32, to: Slot },
    /// Each element of `slot` times `σ(gate)`, in place.
    SigmoidGate { slot: Slot, gate: Slot },
    /// The positions of every sequence the step advances move past its rows.
    Advance,
}

/// One step of a checkpoint at a knob point.
#[derive(Clone, Debug, PartialEq)]
pub struct Program {
    pub numerics: EngineNumerics,
    pub commands: Vec<Command>,
}

/// Why the engine has no program for a model: the author cannot write its step, the front end cannot lift it, or its
/// step graph holds a block the engine has no unit for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProgramError {
    NotYet(NotYet),
    Lift(String),
    Graph(GraphError),
    Lower(LowerError),
}

impl fmt::Display for ProgramError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ProgramError::NotYet(e) => write!(f, "{e}"),
            ProgramError::Lift(e) => write!(f, "the front end does not lift the step: {e}"),
            ProgramError::Graph(e) => write!(f, "{e}"),
            ProgramError::Lower(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for ProgramError {}

/// A program and the cut of the model's one program it was read off.
#[derive(Clone, Debug, PartialEq)]
pub struct Lowered {
    pub program: Program,
    pub identity: Identity,
}

/// The engine's step of `spec` at `numerics`, read off the decode step the author writes over a cache of `ctx`
/// positions: the model's one program, lifted by the shared front end into its step graph and read block by block.
/// The commands do not depend on `ctx` (the cache's length is the layout's); the identity records the cut read.
pub fn lower(spec: &ModelSpec, numerics: EngineNumerics, ctx: usize) -> Result<Lowered, ProgramError> {
    let (graph, identity) = lifted(spec, numerics, ctx)?;
    let program = lower_graph(&graph, spec, numerics).map_err(ProgramError::Lower)?;
    Ok(Lowered { program, identity })
}

/// The step graph of `spec`'s decode step over a cache of `ctx` positions, and its identity. The graph is the same at
/// every knob point, so it is lifted once a process for each spec and cache length — keyed by the spec's whole
/// contents, never a summary of them — and every point reads the one lift.
fn lifted(spec: &ModelSpec, numerics: EngineNumerics, ctx: usize) -> Result<(Arc<TensorGraph>, Identity), ProgramError> {
    static LIFTED: OnceLock<Mutex<HashMap<(String, usize), (Arc<TensorGraph>, Identity)>>> = OnceLock::new();
    let key = (format!("{spec:?}"), ctx);
    let cache = LIFTED.get_or_init(Default::default);
    if let Some((graph, identity)) = cache.lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
        return Ok((Arc::clone(graph), *identity));
    }
    crate::model::author::check(spec).map_err(ProgramError::NotYet)?;
    let lift = TensorGraph::of_cut(spec, Cut::Decode { ctx }, None, &|_| Storage::Values).map_err(|e| match e {
        GraphError::Unwalked { why } => ProgramError::Lift(why),
        e => ProgramError::Graph(e),
    })?;
    let identity = lift.identity.expect("a cut's graph records its identity");
    let graph = Arc::new(optimize(&lift, &EngineShim { spec, numerics }, &[]).graph);
    cache.lock().unwrap_or_else(|e| e.into_inner()).insert(key, (Arc::clone(&graph), identity));
    Ok((graph, identity))
}

/// The step of `spec` at `numerics` written from the spec directly: the embedding, each layer's mixer and FFN around
/// the residual, the final norm and the head. The oracle [`lower`] is held to, never a program the engine is given.
pub fn hand_lowering(spec: &ModelSpec, numerics: EngineNumerics) -> Program {
    let mut c = vec![Command::Embed { table: "token_embd.weight".into() }];
    for (l, layer) in spec.layers.iter().enumerate() {
        c.push(Command::Norm { weight: format!("blk.{l}.attn_norm.weight"), from: Slot::Residual, to: Slot::Normed });
        match &layer.mixer {
            Mixer::GatedDelta(gd) => delta(&mut c, l, gd),
            Mixer::Attention(a) => attention(&mut c, l, a, numerics),
            Mixer::Mla(_) => unreachable!("the engine refuses multi-head latent attention"),
            Mixer::Recurrent(r) => unreachable!("the engine refuses {}", r.what()),
            Mixer::None => unreachable!("the engine refuses a block with no mixer"),
        }
        c.push(Command::Add { from: Slot::Out, to: Slot::Residual });
        c.push(Command::Norm { weight: format!("blk.{l}.{}.weight", spec.ffn_norm), from: Slot::Residual, to: Slot::Normed });
        c.push(Command::Matvec { tensor: format!("blk.{l}.ffn_gate.weight"), from: Slot::Normed, to: Slot::FfnGate });
        c.push(Command::Matvec { tensor: format!("blk.{l}.ffn_up.weight"), from: Slot::Normed, to: Slot::FfnUp });
        c.push(Command::SwiGlu { gate: Slot::FfnGate, up: Slot::FfnUp, to: Slot::Hidden });
        c.push(Command::Matvec { tensor: format!("blk.{l}.ffn_down.weight"), from: Slot::Hidden, to: Slot::Out });
        c.push(Command::Add { from: Slot::Out, to: Slot::Residual });
    }
    c.push(Command::Advance);
    c.push(Command::Norm { weight: "output_norm.weight".into(), from: Slot::Residual, to: Slot::Normed });
    let head = if spec.tied_output { "token_embd.weight" } else { "output.weight" };
    c.push(Command::Matvec { tensor: head.into(), from: Slot::Normed, to: Slot::Logits });
    Program { numerics, commands: c }
}

/// A Gated-DeltaNet block: the four projections, the convolution, the queries and keys L2-normalized per key head,
/// the gates, the memory, the read normalized and gated, the output projection.
fn delta(c: &mut Vec<Command>, l: usize, gd: &GatedDelta) {
    let (hd, nk, nv) = (gd.head_dim, gd.k_heads, gd.v_heads);
    for (tensor, to) in [("attn_qkv", Slot::Qkv), ("attn_gate", Slot::Z), ("ssm_beta", Slot::Beta), ("ssm_alpha", Slot::Alpha)] {
        c.push(Command::Matvec { tensor: format!("blk.{l}.{tensor}.weight"), from: Slot::Normed, to });
    }
    c.push(Command::Conv { layer: l, kernel: format!("blk.{l}.ssm_conv1d.weight"), taps: gd.conv_taps, from: Slot::Qkv, to: Slot::Mixed });
    c.push(Command::L2Heads { from: Slot::Mixed, at: 0, heads: nk, width: hd, to: Slot::Query });
    c.push(Command::L2Heads { from: Slot::Mixed, at: nk * hd, heads: nk, width: hd, to: Slot::Key });
    c.push(Command::Gates {
        alpha: Slot::Alpha,
        beta: Slot::Beta,
        dt: format!("blk.{l}.ssm_dt.bias"),
        a: format!("blk.{l}.ssm_a"),
        log_decay: Slot::LogDecay,
        decay: Slot::Decay,
        gate: Slot::Gate,
    });
    c.push(Command::Delta {
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
    c.push(Command::GatedNorm { weight: format!("blk.{l}.ssm_norm.weight"), from: Slot::Read, z: Slot::Z, width: hd, to: Slot::Read });
    c.push(Command::Matvec { tensor: format!("blk.{l}.ssm_out.weight"), from: Slot::Read, to: Slot::Out });
}

/// An attention block: the projections, the queries split from their gates, queries and keys normed per head and
/// turned, rotated by the Walsh–Hadamard transform when the cache is, the cache appended, the attention, the gate,
/// the output projection.
fn attention(c: &mut Vec<Command>, l: usize, a: &Attention, numerics: EngineNumerics) {
    let (hd, vd) = (a.head_dim, a.value_dim);
    let rotated = numerics.cache == CacheFormat::Int8Hadamard;
    let projected = if a.output_gate { Slot::Projected } else { Slot::Query };
    c.push(Command::Matvec { tensor: format!("blk.{l}.attn_q.weight"), from: Slot::Normed, to: projected });
    c.push(Command::Matvec { tensor: format!("blk.{l}.attn_k.weight"), from: Slot::Normed, to: Slot::Key });
    c.push(Command::Matvec { tensor: format!("blk.{l}.attn_v.weight"), from: Slot::Normed, to: Slot::Value });
    if a.output_gate {
        c.push(Command::Split { from: Slot::Projected, heads: a.heads, width: hd, query: Slot::Query, gate: Slot::OutGate });
    }
    for (slot, name) in [(Slot::Query, "attn_q_norm"), (Slot::Key, "attn_k_norm")] {
        if a.qk_norm {
            c.push(Command::RmsHeads { weight: format!("blk.{l}.{name}.weight"), slot, width: hd });
        }
        if let Some(rope) = &a.rope {
            c.push(Command::Rotate { slot, width: hd, rope: rope.clone() });
        }
    }
    if rotated {
        c.push(Command::Hadamard { slot: Slot::Key, width: hd });
        c.push(Command::Hadamard { slot: Slot::Value, width: vd });
    }
    c.push(Command::CacheAppend { layer: l, key: Slot::Key, value: Slot::Value, kv_heads: a.kv_heads, width: hd, value_width: vd });
    if rotated {
        c.push(Command::Hadamard { slot: Slot::Query, width: hd });
    }
    c.push(Command::Attend {
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
        c.push(Command::SigmoidGate { slot: Slot::Read, gate: Slot::OutGate });
    }
    c.push(Command::Matvec { tensor: format!("blk.{l}.attn_output.weight"), from: Slot::Read, to: Slot::Out });
}

impl Program {
    /// Every tensor the program reads, each once, in the order it first reads them: the weight image's consumption
    /// order.
    pub fn tensors(&self) -> Vec<&str> {
        let mut seen = std::collections::HashSet::new();
        let mut out = Vec::new();
        for c in &self.commands {
            let names: Vec<&str> = match c {
                Command::Embed { table } => vec![table.as_str()],
                Command::Norm { weight, .. } | Command::RmsHeads { weight, .. } | Command::GatedNorm { weight, .. } => vec![weight.as_str()],
                Command::Matvec { tensor, .. } => vec![tensor.as_str()],
                Command::Conv { kernel, .. } => vec![kernel.as_str()],
                Command::Gates { dt, a, .. } => vec![dt.as_str(), a.as_str()],
                _ => vec![],
            };
            for n in names {
                if seen.insert(n) {
                    out.push(n);
                }
            }
        }
        out
    }

    /// The slots each command reads and the slots it writes.
    pub fn dataflow(command: &Command) -> (Vec<Slot>, Vec<Slot>) {
        use Command as C;
        match command {
            C::Embed { .. } => (vec![], vec![Slot::Residual]),
            C::Norm { from, to, .. } | C::Matvec { from, to, .. } | C::Conv { from, to, .. } | C::L2Heads { from, to, .. } => (vec![*from], vec![*to]),
            C::Add { from, to } => (vec![*from, *to], vec![*to]),
            C::SwiGlu { gate, up, to } => (vec![*gate, *up], vec![*to]),
            C::Gates { alpha, beta, log_decay, decay, gate, .. } => (vec![*alpha, *beta], vec![*log_decay, *decay, *gate]),
            C::Delta { query, key, value, log_decay, decay, gate, to, .. } => (vec![*query, *key, *value, *log_decay, *decay, *gate], vec![*to]),
            C::GatedNorm { from, z, to, .. } => (vec![*from, *z], vec![*to]),
            C::Split { from, query, gate, .. } => (vec![*from], vec![*query, *gate]),
            C::RmsHeads { slot, .. } | C::Rotate { slot, .. } | C::Hadamard { slot, .. } => (vec![*slot], vec![*slot]),
            C::CacheAppend { key, value, .. } => (vec![*key, *value], vec![]),
            C::Attend { query, to, .. } => (vec![*query], vec![*to]),
            C::SigmoidGate { slot, gate } => (vec![*slot, *gate], vec![*slot]),
            C::Advance => (vec![], vec![]),
        }
    }
}

// ── The sequencer's words ───────────────────────────────────────────────────────────────────────────

/// A command as the sequencer reads it: 256 bits, `[7:0]` the command, `[15:8]` its layer, seven 5-bit slots from bit
/// 16, two 20-bit tensors from bit 51 (each its index in [`Program::tensors`], the weight image's order), four 16-bit
/// counts from bit 91, a 20-bit element offset from bit 155, an 8-bit rotary table from bit 175 (its index in
/// [`Program::ropes`]) and a 32-bit f32 constant from bit 183.
pub type Word = [u64; 4];

const SLOTS: [Slot; 21] = [
    Slot::Residual,
    Slot::Normed,
    Slot::Out,
    Slot::Qkv,
    Slot::Mixed,
    Slot::Z,
    Slot::Alpha,
    Slot::Beta,
    Slot::LogDecay,
    Slot::Decay,
    Slot::Gate,
    Slot::Query,
    Slot::Key,
    Slot::Value,
    Slot::Projected,
    Slot::OutGate,
    Slot::Read,
    Slot::FfnGate,
    Slot::FfnUp,
    Slot::Hidden,
    Slot::Logits,
];

/// The fields of a word, as the encoder fills them and the decoder reads them.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Fields {
    op: u8,
    layer: u8,
    slots: [u8; 7],
    tensors: [u32; 2],
    counts: [u16; 4],
    at: u32,
    rope: u8,
    constant: u32,
}

fn slot_code(s: Slot) -> u8 {
    s.code()
}

impl Slot {
    /// Every slot, in the order of its code.
    pub const ALL: [Slot; 21] = SLOTS;

    /// The slot's five-bit code in the sequencer's words.
    pub fn code(self) -> u8 {
        SLOTS.iter().position(|&x| x == self).expect("every slot has a code") as u8
    }

    /// The slot a code stands for.
    pub fn of_code(code: u8) -> Option<Slot> {
        SLOTS.get(code as usize).copied()
    }
}

fn put(w: &mut Word, at: usize, bits: usize, v: u64) {
    assert!(bits == 64 || v < 1 << bits, "{v} past {bits} bits");
    for i in 0..bits {
        if v >> i & 1 == 1 {
            w[(at + i) / 64] |= 1 << ((at + i) % 64);
        }
    }
}

fn get(w: &Word, at: usize, bits: usize) -> u64 {
    (0..bits).fold(0, |v, i| v | ((w[(at + i) / 64] >> ((at + i) % 64) & 1) << i))
}

impl Fields {
    fn word(&self) -> Word {
        let mut w = [0u64; 4];
        put(&mut w, 0, 8, self.op as u64);
        put(&mut w, 8, 8, self.layer as u64);
        for (i, &s) in self.slots.iter().enumerate() {
            put(&mut w, 16 + 5 * i, 5, s as u64);
        }
        for (i, &t) in self.tensors.iter().enumerate() {
            put(&mut w, 51 + 20 * i, 20, t as u64);
        }
        for (i, &c) in self.counts.iter().enumerate() {
            put(&mut w, 91 + 16 * i, 16, c as u64);
        }
        put(&mut w, 155, 20, self.at as u64);
        put(&mut w, 175, 8, self.rope as u64);
        put(&mut w, 183, 32, self.constant as u64);
        w
    }

    fn of(w: &Word) -> Fields {
        let mut f = Fields { op: get(w, 0, 8) as u8, layer: get(w, 8, 8) as u8, ..Fields::default() };
        for i in 0..7 {
            f.slots[i] = get(w, 16 + 5 * i, 5) as u8;
        }
        for i in 0..2 {
            f.tensors[i] = get(w, 51 + 20 * i, 20) as u32;
        }
        for i in 0..4 {
            f.counts[i] = get(w, 91 + 16 * i, 16) as u16;
        }
        f.at = get(w, 155, 20) as u32;
        f.rope = get(w, 175, 8) as u8;
        f.constant = get(w, 183, 32) as u32;
        f
    }
}

impl Program {
    /// Every rotary table the program turns heads by, each once, in the order it first does.
    pub fn ropes(&self) -> Vec<&Rope> {
        let mut out: Vec<&Rope> = Vec::new();
        for c in &self.commands {
            if let Command::Rotate { rope, .. } = c {
                if !out.contains(&rope) {
                    out.push(rope);
                }
            }
        }
        out
    }

    /// The program as the sequencer's words.
    pub fn encode(&self) -> Vec<Word> {
        let tensors: std::collections::HashMap<&str, u32> = self.tensors().into_iter().enumerate().map(|(i, n)| (n, i as u32)).collect();
        let ropes = self.ropes();
        let t = |name: &str| *tensors.get(name).expect("a tensor the program reads");
        let s = slot_code;
        let n16 = |v: usize| u16::try_from(v).expect("a count within 16 bits");
        let n8 = |v: usize| u8::try_from(v).expect("a layer within 8 bits");
        self.commands
            .iter()
            .map(|c| {
                let mut f = Fields::default();
                match c {
                    Command::Embed { table } => (f.op, f.tensors[0]) = (0, t(table)),
                    Command::Norm { weight, from, to } => (f.op, f.tensors[0], f.slots[0], f.slots[1]) = (1, t(weight), s(*from), s(*to)),
                    Command::Matvec { tensor, from, to } => (f.op, f.tensors[0], f.slots[0], f.slots[1]) = (2, t(tensor), s(*from), s(*to)),
                    Command::Add { from, to } => (f.op, f.slots[0], f.slots[1]) = (3, s(*from), s(*to)),
                    Command::SwiGlu { gate, up, to } => (f.op, f.slots[0], f.slots[1], f.slots[2]) = (4, s(*gate), s(*up), s(*to)),
                    Command::Conv { layer, kernel, taps, from, to } => {
                        (f.op, f.layer, f.tensors[0], f.counts[0], f.slots[0], f.slots[1]) = (5, n8(*layer), t(kernel), n16(*taps), s(*from), s(*to))
                    }
                    Command::L2Heads { from, at, heads, width, to } => {
                        (f.op, f.slots[0], f.at, f.counts[0], f.counts[1], f.slots[1]) = (6, s(*from), *at as u32, n16(*heads), n16(*width), s(*to))
                    }
                    Command::Gates { alpha, beta, dt, a, log_decay, decay, gate } => {
                        f.op = 7;
                        f.tensors = [t(dt), t(a)];
                        f.slots[..5].copy_from_slice(&[s(*alpha), s(*beta), s(*log_decay), s(*decay), s(*gate)]);
                    }
                    Command::Delta { layer, query, key, value, value_at, log_decay, decay, gate, k_heads, v_heads, width, to } => {
                        f.op = 8;
                        f.layer = n8(*layer);
                        f.slots = [s(*query), s(*key), s(*value), s(*log_decay), s(*decay), s(*gate), s(*to)];
                        f.at = *value_at as u32;
                        f.counts[..3].copy_from_slice(&[n16(*k_heads), n16(*v_heads), n16(*width)]);
                    }
                    Command::GatedNorm { weight, from, z, width, to } => {
                        (f.op, f.tensors[0], f.slots[0], f.slots[1], f.counts[0], f.slots[2]) = (9, t(weight), s(*from), s(*z), n16(*width), s(*to))
                    }
                    Command::Split { from, heads, width, query, gate } => {
                        (f.op, f.slots[0], f.counts[0], f.counts[1], f.slots[1], f.slots[2]) = (10, s(*from), n16(*heads), n16(*width), s(*query), s(*gate))
                    }
                    Command::RmsHeads { weight, slot, width } => (f.op, f.tensors[0], f.slots[0], f.counts[0]) = (11, t(weight), s(*slot), n16(*width)),
                    Command::Rotate { slot, width, rope } => {
                        let r = ropes.iter().position(|x| *x == rope).expect("a rotary table the program turns by");
                        (f.op, f.slots[0], f.counts[0], f.rope) = (12, s(*slot), n16(*width), r as u8)
                    }
                    Command::Hadamard { slot, width } => (f.op, f.slots[0], f.counts[0]) = (13, s(*slot), n16(*width)),
                    Command::CacheAppend { layer, key, value, kv_heads, width, value_width } => {
                        f.op = 14;
                        f.layer = n8(*layer);
                        f.slots[..2].copy_from_slice(&[s(*key), s(*value)]);
                        f.counts[..3].copy_from_slice(&[n16(*kv_heads), n16(*width), n16(*value_width)]);
                    }
                    Command::Attend { layer, query, heads, kv_heads, width, value_width, scale, to } => {
                        f.op = 15;
                        f.layer = n8(*layer);
                        f.slots[..2].copy_from_slice(&[s(*query), s(*to)]);
                        f.counts = [n16(*heads), n16(*kv_heads), n16(*width), n16(*value_width)];
                        f.constant = scale.to_bits();
                    }
                    Command::SigmoidGate { slot, gate } => (f.op, f.slots[0], f.slots[1]) = (16, s(*slot), s(*gate)),
                    Command::Advance => f.op = 17,
                }
                f.word()
            })
            .collect()
    }

    /// The commands the words stand for, with the tensors and rotary tables they index.
    pub fn decode(words: &[Word], tensors: &[&str], ropes: &[&Rope]) -> Vec<Command> {
        let sl = |c: u8| SLOTS[c as usize];
        words
            .iter()
            .map(|w| {
                let f = Fields::of(w);
                let t = |i: usize| tensors[f.tensors[i] as usize].to_string();
                let [s0, s1, s2, s3, s4, s5, s6] = f.slots.map(sl);
                let [c0, c1, c2, c3] = f.counts.map(|c| c as usize);
                let layer = f.layer as usize;
                match f.op {
                    0 => Command::Embed { table: t(0) },
                    1 => Command::Norm { weight: t(0), from: s0, to: s1 },
                    2 => Command::Matvec { tensor: t(0), from: s0, to: s1 },
                    3 => Command::Add { from: s0, to: s1 },
                    4 => Command::SwiGlu { gate: s0, up: s1, to: s2 },
                    5 => Command::Conv { layer, kernel: t(0), taps: c0, from: s0, to: s1 },
                    6 => Command::L2Heads { from: s0, at: f.at as usize, heads: c0, width: c1, to: s1 },
                    7 => Command::Gates { alpha: s0, beta: s1, dt: t(0), a: t(1), log_decay: s2, decay: s3, gate: s4 },
                    8 => Command::Delta {
                        layer,
                        query: s0,
                        key: s1,
                        value: s2,
                        value_at: f.at as usize,
                        log_decay: s3,
                        decay: s4,
                        gate: s5,
                        k_heads: c0,
                        v_heads: c1,
                        width: c2,
                        to: s6,
                    },
                    9 => Command::GatedNorm { weight: t(0), from: s0, z: s1, width: c0, to: s2 },
                    10 => Command::Split { from: s0, heads: c0, width: c1, query: s1, gate: s2 },
                    11 => Command::RmsHeads { weight: t(0), slot: s0, width: c0 },
                    12 => Command::Rotate { slot: s0, width: c0, rope: ropes[f.rope as usize].clone() },
                    13 => Command::Hadamard { slot: s0, width: c0 },
                    14 => Command::CacheAppend { layer, key: s0, value: s1, kv_heads: c0, width: c1, value_width: c2 },
                    15 => Command::Attend { layer, query: s0, heads: c0, kv_heads: c1, width: c2, value_width: c3, scale: f32::from_bits(f.constant), to: s1 },
                    16 => Command::SigmoidGate { slot: s0, gate: s1 },
                    17 => Command::Advance,
                    op => panic!("no command {op}"),
                }
            })
            .collect()
    }
}
