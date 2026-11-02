//! The reference forward: what a [`ModelSpec`] computes, evaluated in `f64` on the CPU over the checkpoint's own
//! weights.
//!
//! This is the oracle every compiled lane and every competitor is measured against — the "same-weights" reference:
//! the weights are exactly the checkpoint's (each dequantized as ggml defines its format), and everything after the
//! weights is `f64`, so the only error left in a lane's logits is the lane's own. It runs one prompt as a batch per
//! layer (each weight is read once per pass, not once per token) and then decodes with a KV cache.
//!
//! The operations follow llama.cpp's graph for each architecture (`src/models/<arch>.cpp` at the pinned commit).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::Arc;

use super::gguf::Gguf;
use super::quant::{f16_to_f32, f32_to_f16, GgmlType};
use super::spec::{
    Act, Attention, Ffn, GateActivation, GatedDelta, Gating, HyperConnections, Indexer, Layer, Mixer, Mla, Moe, ModelSpec,
    Rope, RopeScaling,
};

/// Why the reference cannot run a checkpoint.
#[derive(Debug)]
pub enum ReferenceError {
    Io(std::io::Error),
    /// A part of the spec the reference does not evaluate yet, named.
    NotYet(&'static str),
}

impl std::fmt::Display for ReferenceError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ReferenceError::Io(e) => write!(f, "{e}"),
            ReferenceError::NotYet(what) => write!(f, "the reference does not evaluate {what} yet"),
        }
    }
}

impl From<std::io::Error> for ReferenceError {
    fn from(e: std::io::Error) -> Self {
        ReferenceError::Io(e)
    }
}

type R<T> = Result<T, ReferenceError>;

/// What one block carries from position to position.
#[derive(Clone)]
enum LayerState {
    /// An attention block's cache: every position's keys and values so far, and its index key where the model selects
    /// with QSA (unnormed: a block's keys are averaged before their norm).
    Attention { k: Vec<Vec<f64>>, v: Vec<Vec<f64>>, ik: Vec<Vec<f64>> },
    /// A gated-delta block's state: the last `taps − 1` convolution inputs, oldest first, and each value head's
    /// `head_dim × head_dim` memory (row = value dimension, column = key dimension).
    Delta { conv: Vec<Vec<f64>>, memory: Vec<f64> },
}

impl LayerState {
    fn fresh(layer: &Layer) -> LayerState {
        match &layer.mixer {
            Mixer::Attention(_) | Mixer::Mla(_) => LayerState::Attention { k: Vec::new(), v: Vec::new(), ik: Vec::new() },
            Mixer::GatedDelta(g) => {
                let channels = 2 * g.k_heads * g.head_dim + g.v_heads * g.head_dim;
                LayerState::Delta {
                    conv: vec![vec![0.0; channels]; g.conv_taps.saturating_sub(1)],
                    memory: vec![0.0; g.v_heads * g.head_dim * g.head_dim],
                }
            }
        }
    }
}

/// Where the reference reads a model's weights: a checkpoint, or tensors held in memory (a test's tiny model).
/// Tensors are named and laid out as GGUF names and lays them out (`dims[0]` the fastest).
pub trait WeightSource {
    /// A tensor's format and dims.
    fn describe(&self, name: &str) -> std::io::Result<(GgmlType, Vec<u64>)>;
    /// The bytes of rows `[first, first + count)`, a row being the tensor's `dims[0]` elements.
    fn row_bytes(&mut self, name: &str, first: usize, count: usize) -> std::io::Result<Vec<u8>>;
    fn has(&self, name: &str) -> bool;
}

impl WeightSource for Gguf {
    fn describe(&self, name: &str) -> std::io::Result<(GgmlType, Vec<u64>)> {
        let ty = self.tensor_type(name)?;
        Ok((ty, self.dims(name).unwrap_or_default().to_vec()))
    }
    fn row_bytes(&mut self, name: &str, first: usize, count: usize) -> std::io::Result<Vec<u8>> {
        Gguf::row_bytes(self, name, first, count)
    }
    fn has(&self, name: &str) -> bool {
        Gguf::has(self, name)
    }
}

/// Tensors held in memory, each in a GGUF format.
#[derive(Default, Clone)]
pub struct Tensors {
    map: BTreeMap<String, (GgmlType, Vec<u64>, Vec<u8>)>,
}

impl Tensors {
    /// Add an `f32` tensor of `dims` (GGUF order) from its values.
    pub fn f32(&mut self, name: &str, dims: &[u64], values: &[f32]) {
        assert_eq!(dims.iter().product::<u64>() as usize, values.len(), "{name}: dims and values disagree");
        let bytes = values.iter().flat_map(|v| v.to_le_bytes()).collect();
        self.map.insert(name.to_string(), (GgmlType::F32, dims.to_vec(), bytes));
    }

    /// Add a tensor already encoded in `ty`.
    pub fn encoded(&mut self, name: &str, ty: GgmlType, dims: &[u64], bytes: Vec<u8>) {
        let n = dims.iter().product::<u64>() as usize;
        assert_eq!(ty.bytes_for(n), bytes.len(), "{name}: {} bytes for {n} {} weights", bytes.len(), ty.name());
        self.map.insert(name.to_string(), (ty, dims.to_vec(), bytes));
    }

    /// Every tensor, by name.
    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.map.keys().map(String::as_str)
    }

    /// A tensor's values, dequantized.
    pub fn values(&self, name: &str) -> Option<Vec<f32>> {
        self.map.get(name).map(|(ty, _, bytes)| {
            let mut out = Vec::new();
            ty.dequantize(bytes, &mut out);
            out
        })
    }
}

impl WeightSource for Tensors {
    fn describe(&self, name: &str) -> std::io::Result<(GgmlType, Vec<u64>)> {
        self.map
            .get(name)
            .map(|(ty, dims, _)| (*ty, dims.clone()))
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, format!("no tensor {name}")))
    }
    fn row_bytes(&mut self, name: &str, first: usize, count: usize) -> std::io::Result<Vec<u8>> {
        let (ty, dims, bytes) = self
            .map
            .get(name)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, format!("no tensor {name}")))?;
        let row = ty.bytes_for(dims[0] as usize);
        Ok(bytes[first * row..(first + count) * row].to_vec())
    }
    fn has(&self, name: &str) -> bool {
        self.map.contains_key(name)
    }
}

/// The bytes the reference keeps of a checkpoint's tensors by default: every tensor of a model up to this size is
/// read from the file once.
pub const DEFAULT_CACHE_BUDGET: usize = 16 << 30;

/// How the reference's projections by quantized weights read their activations. Exact is the model's arithmetic;
/// the others are the activations a quantized matmul reads under llama.cpp's rule — a quantized weight reads a
/// quantized activation — computed as the GPU lane's kernels compute them, so a lane can be held to its own precision.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Activations {
    /// As computed, in `f64`.
    #[default]
    Exact,
    /// Rounded to `f32`, then each 32 values one int8 word over a shared scale `max|v| / 127` (1 for a zero block),
    /// rounded to the nearest, ties to even.
    Int8,
    /// The high word as `Int8` (the scale's floor 1e-30, clamped to −128..127) and a low int8 word holding the
    /// remainder `v − hi·s` at a 128th of the scale.
    Int8TwoPass,
}

/// How an attention block's cache keeps the rows it is given.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KvStorage {
    /// As computed, in `f64`.
    #[default]
    Exact,
    /// Each element rounded to `f32` and then to binary16, as llama.cpp's default `f16` cache stores the `f32` rows
    /// it computes; every read of the cache takes the rounded row.
    F16,
}

/// `v` rounded to `f32` and then to binary16, to the nearest (ties to even) — or, `flipped`, to its other binary16
/// neighbour — how far the `f32` value lies from the half-way point between the two neighbours (infinitely far for a
/// value binary16 holds exactly, or one past its range), and how far apart the neighbours are.
fn binary16(v: f64, flipped: bool) -> (f64, f64, f64) {
    let f = v as f32;
    let h = f32_to_f16(f);
    let near = f16_to_f32(h);
    if near == f || !near.is_finite() {
        return (f64::from(near), f64::INFINITY, 0.0);
    }
    // The neighbour the other side of `f`: a step up in value or down, the sign bit deciding which way the bits go.
    let up = near < f;
    let other = match (h & 0x8000 != 0, h & 0x7fff, up) {
        (false, _, true) | (true, _, false) => h.wrapping_add(1),
        (false, 0, false) => 0x8001,
        (true, 0, true) => 0x0001,
        _ => h.wrapping_sub(1),
    };
    let far = f16_to_f32(other);
    let (near, far, f) = (f64::from(near), f64::from(far), f64::from(f));
    (if flipped { far } else { near }, (f - (near + far) / 2.0).abs(), (far - near).abs())
}

/// How a gated-delta layer keeps its recurrent memory between steps.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum StateStorage {
    /// As computed, in `f64`.
    #[default]
    Exact,
    /// Each element rounded to `f32` and then to bfloat16 (to the nearest, ties to even) once its step has read it, as a
    /// lane storing its memory in bf16 keeps it.
    Bf16,
}

impl StateStorage {
    /// `v` as the memory keeps it.
    fn keep(self, v: f64) -> f64 {
        match self {
            StateStorage::Exact => v,
            StateStorage::Bf16 => {
                let x = v as f32;
                if x.is_nan() {
                    return f64::from(x);
                }
                let bits = x.to_bits();
                f64::from(f32::from_bits(bits.wrapping_add(0x7FFF + ((bits >> 16) & 1)) & 0xFFFF_0000))
            }
        }
    }
}

/// `x` as a projection by a quantized weight reads it under `mode`, a block of 32 values at a time.
pub fn quantize_activations(x: &[f64], mode: Activations) -> Vec<f64> {
    quantize_deciding(x, mode, |_, _, q| q.round_ties_even())
}

/// [`quantize_activations`] with each word's whole number decided by `round(element, word, q)`: `q` the value in the
/// word's own levels, `word` 0 for the int8 (or two-pass high) word and 1 for the two-pass low word.
fn quantize_deciding(x: &[f64], mode: Activations, mut round: impl FnMut(usize, u8, f32) -> f32) -> Vec<f64> {
    if mode == Activations::Exact {
        return x.to_vec();
    }
    // An int8 word: the decided whole number within −128..127, with no sign on its zero.
    let word = |q: f32| f32::from(q.clamp(-128.0, 127.0) as i8);
    let mut out = Vec::with_capacity(x.len());
    for (b, block) in x.chunks(32).enumerate() {
        let v: Vec<f32> = block.iter().map(|&a| a as f32).collect();
        let amax = v.iter().fold(0f32, |m, a| m.max(a.abs()));
        match mode {
            Activations::Int8 => {
                let s = if amax > 0.0 { amax / 127.0 } else { 1.0 };
                for (i, &a) in v.iter().enumerate() {
                    out.push(f64::from(word(round(b * 32 + i, 0, a / s))) * f64::from(s));
                }
            }
            _ => {
                let s = amax.max(1e-30) / 127.0;
                let sl = s * 0.0078125;
                for (i, &a) in v.iter().enumerate() {
                    let hi = word(round(b * 32 + i, 0, a / s));
                    let lo = word(round(b * 32 + i, 1, (-hi).mul_add(s, a) / sl));
                    out.push(f64::from(hi) * f64::from(s) + f64::from(lo) * f64::from(sl));
                }
            }
        }
    }
    out
}

/// How near a half-way point between two levels a rounding the reference records lies, in levels: within it, a run
/// whose activation differs from the reference's by its own `f32` rounding may take the other level.
pub const NEAR_ROUNDING: f64 = 1e-3;

/// How near a tie, as a fraction of the chosen score, a routing call the reference records lies.
pub const NEAR_ROUTING: f64 = 1e-3;

/// The word a cached value's binary16 rounding is recorded under ([`RoundingCall::word`]).
pub const CACHE_WORD: u8 = 2;

/// A rounding the reference made within [`NEAR_ROUNDING`] of a half-way point: value `element` of the activation whose
/// `f32` values hash to `activation` (`word` 0 the int8 or two-pass high word, 1 the low word, [`CACHE_WORD`] a cached
/// row's binary16), first made in the step ending at `position`, `margin` levels from the half-way point (a cached
/// value's in levels of its row's scale, its largest magnitude over 127). An activation is one decision however many
/// projections read it — the lane quantizes it once — so a call names the activation, not the read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoundingCall {
    pub activation: u64,
    pub element: usize,
    pub word: u8,
    pub position: usize,
    pub margin: f64,
    /// How far the other outcome moves the value, in the levels `margin` is measured in: one for an int8 word, a
    /// cached value's binary16 step for its rounding.
    pub step: f64,
}

/// A decision the reference made near its other outcome — one a run in another precision may make the other way.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum NearCall {
    Rounding(RoundingCall),
    Routing(RoutingCall),
}

impl NearCall {
    /// The position the call was made at (a prompt's last position for a call in its step).
    pub fn position(&self) -> usize {
        match self {
            NearCall::Rounding(c) => c.position,
            NearCall::Routing(c) => c.position,
        }
    }
}

/// The closest call a router made: at `position` (0-based) in layer `layer`, the last expert (or expert group) it
/// chose scored `margin` above the first it passed over, on a selection score of `score`. A run in another precision
/// may choose otherwise wherever `margin` is below what that precision resolves at `score`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoutingCall {
    pub layer: usize,
    pub position: usize,
    pub margin: f64,
    pub score: f64,
}

/// The reference model over a checkpoint.
pub struct Reference {
    pub spec: ModelSpec,
    g: Box<dyn WeightSource>,
    states: Vec<LayerState>,
    pos: usize,
    threads: usize,
    /// Whole tensors' bytes, kept while they fit `budget`; a tensor past it is read again each time it is used.
    cache: HashMap<String, Arc<Vec<u8>>>,
    cached: usize,
    budget: usize,
    closest: Option<RoutingCall>,
    /// Every near call since the last reset, in the order first made, and the roundings among them.
    near: Vec<NearCall>,
    near_roundings: HashSet<(u64, usize, u8)>,
    /// The last position of the step running.
    at: usize,
    /// The calls decided the other way at every run: roundings by (activation, element, word), routings by (layer,
    /// position).
    flipped_roundings: HashSet<(u64, usize, u8)>,
    flipped_routings: HashSet<(usize, usize)>,
    activations: Activations,
    kv: KvStorage,
    state: StateStorage,
    /// The per-layer embedding's state: every token so far after `ngram - 1` ends of document, and the last
    /// `(taps - 1)·ngram` normed gated values its dilated convolution reads, oldest first.
    ple_tokens: Vec<u32>,
    ple_conv: Vec<Vec<f64>>,
}

impl Reference {
    pub fn new(spec: ModelSpec, g: impl WeightSource + 'static) -> R<Reference> {
        for layer in &spec.layers {
            if let Mixer::Attention(a) = &layer.mixer {
                if a.output_gate && a.fused_qkv {
                    return Err(ReferenceError::NotYet("an output gate on a fused query-key-value projection"));
                }
                if a.output_gate && a.value_dim != a.head_dim {
                    return Err(ReferenceError::NotYet("an output gate over values narrower than the queries"));
                }
            }
        }
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
        let (ple_tokens, ple_conv) = Reference::fresh_ple(&spec);
        Ok(Reference {
            ple_tokens,
            ple_conv,
            states: spec.layers.iter().map(LayerState::fresh).collect(),
            spec,
            g: Box::new(g),
            pos: 0,
            threads,
            cache: HashMap::new(),
            cached: 0,
            budget: DEFAULT_CACHE_BUDGET,
            closest: None,
            near: Vec::new(),
            near_roundings: HashSet::new(),
            at: 0,
            flipped_roundings: HashSet::new(),
            flipped_routings: HashSet::new(),
            activations: Activations::Exact,
            kv: KvStorage::Exact,
            state: StateStorage::Exact,
        })
    }

    /// The same reference with its quantized projections reading `mode`'s activations.
    pub fn with_activations(mut self, mode: Activations) -> Reference {
        self.activations = mode;
        self
    }

    /// The same reference keeping its attention caches' rows as `kv` stores them.
    pub fn with_kv_storage(mut self, kv: KvStorage) -> Reference {
        self.kv = kv;
        self
    }

    /// The same reference keeping its gated-delta memories as `state` stores them.
    pub fn with_state_storage(mut self, state: StateStorage) -> Reference {
        self.state = state;
        self
    }

    /// The same reference keeping at most `bytes` of tensor data in memory.
    pub fn with_cache_budget(mut self, bytes: usize) -> Reference {
        self.budget = bytes;
        self
    }

    /// A whole tensor's bytes, from the cache or the source (and cached when they fit the budget).
    fn tensor_bytes(&mut self, name: &str) -> R<Arc<Vec<u8>>> {
        if let Some(b) = self.cache.get(name) {
            return Ok(b.clone());
        }
        let (_, dims) = self.g.describe(name)?;
        let row = dims.first().copied().unwrap_or(1).max(1) as usize;
        let rows = dims.iter().product::<u64>() as usize / row;
        let bytes = Arc::new(self.g.row_bytes(name, 0, rows)?);
        if self.cached + bytes.len() <= self.budget {
            self.cached += bytes.len();
            self.cache.insert(name.to_string(), bytes.clone());
        }
        Ok(bytes)
    }

    /// The number of positions run so far.
    pub fn position(&self) -> usize {
        self.pos
    }

    /// Layer `l`'s cached keys and values, one row per position (empty for a layer that is not attention).
    pub fn cached(&self, l: usize) -> (&[Vec<f64>], &[Vec<f64>]) {
        match &self.states[l] {
            LayerState::Attention { k, v, .. } => (k, v),
            LayerState::Delta { .. } => (&[], &[]),
        }
    }

    /// Layer `l`'s gated-delta memory (empty for a layer that is not gated delta).
    pub fn memory(&self, l: usize) -> &[f64] {
        match &self.states[l] {
            LayerState::Delta { memory, .. } => memory,
            LayerState::Attention { .. } => &[],
        }
    }

    /// Layer `l`'s convolution ring: its last `taps − 1` inputs, oldest first (empty for a layer that is not gated
    /// delta).
    pub fn conv(&self, l: usize) -> &[Vec<f64>] {
        match &self.states[l] {
            LayerState::Delta { conv, .. } => conv,
            LayerState::Attention { .. } => &[],
        }
    }

    /// Forget every position: empty caches, zero recurrent state, no routing or near calls. The calls decided the other
    /// way stay decided so.
    pub fn reset(&mut self) {
        self.states = self.spec.layers.iter().map(LayerState::fresh).collect();
        (self.ple_tokens, self.ple_conv) = Reference::fresh_ple(&self.spec);
        self.pos = 0;
        self.closest = None;
        self.near.clear();
        self.near_roundings.clear();
    }

    /// Every rounding and routing call since the last [`reset`](Self::reset) made near its other outcome, in order.
    pub fn near_calls(&self) -> &[NearCall] {
        &self.near
    }

    /// Decide a near call the other way at every run past it: a rounding takes the other level, a routing call keeps
    /// the first expert it would pass over in place of the last it would keep.
    pub fn flip(&mut self, call: NearCall) {
        match call {
            NearCall::Rounding(c) => {
                self.flipped_roundings.insert((c.activation, c.element, c.word));
            }
            NearCall::Routing(c) => self.flip_routing(c),
        }
    }

    /// The closest routing call since the last [`reset`](Self::reset) — `None` for a model without experts.
    pub fn closest_routing(&self) -> Option<RoutingCall> {
        self.closest
    }

    /// Decide one routing call the other way at every run past it: at `call`'s layer and position the router keeps
    /// the first expert it would pass over in place of the last it would keep — how a run in another precision that
    /// resolves the call the other way routes.
    pub fn flip_routing(&mut self, call: RoutingCall) {
        self.flipped_routings.insert((call.layer, call.position));
    }

    /// Note a routing decision: `chosen` scored `margin` above the first passed over.
    fn routed(&mut self, layer: usize, position: usize, chosen: f64, passed: f64) {
        if !passed.is_finite() {
            return;
        }
        let call = RoutingCall { layer, position, margin: chosen - passed, score: chosen };
        if self.closest.is_none_or(|c| call.margin < c.margin) {
            self.closest = Some(call);
        }
        if call.margin <= NEAR_ROUTING * call.score.abs() {
            self.near.push(NearCall::Routing(call));
        }
    }

    /// Run `tokens` at the next positions, appending them to the state; returns the logits at the last one.
    pub fn step(&mut self, tokens: &[u32]) -> R<Vec<f64>> {
        let x = self.layers(tokens)?;
        let last = x.last().ok_or(ReferenceError::NotYet("an empty step"))?;
        Ok(self.head(std::slice::from_ref(last))?.remove(0))
    }

    /// Run `tokens` at the next positions, appending them to the state; returns the logits after every one of them —
    /// the distributions a perplexity or a divergence over a text is taken from.
    pub fn step_every(&mut self, tokens: &[u32]) -> R<Vec<Vec<f64>>> {
        let x = self.layers(tokens)?;
        self.head(&x)
    }

    /// The final hidden state of each of `tokens`, run at the next positions.
    fn layers(&mut self, tokens: &[u32]) -> R<Vec<Vec<f64>>> {
        let start = self.pos;
        self.at = start + tokens.len().max(1) - 1;
        let mut x: Vec<Vec<f64>> = Vec::with_capacity(tokens.len());
        for &t in tokens {
            let mut row = self.rows("token_embd.weight", t as usize, 1)?;
            if let Some(scale) = self.spec.embedding_scale {
                row.iter_mut().for_each(|v| *v *= scale);
            }
            x.push(row);
        }
        if let Some(hc) = self.spec.hyper {
            let lanes = self.hyper_layers(tokens, &x, start, hc)?;
            self.pos += tokens.len();
            return Ok(lanes);
        }
        for l in 0..self.spec.layers.len() {
            let layer = self.spec.layers[l].clone();
            let xn = self.norm_rows(&x, &format!("blk.{l}.attn_norm.weight"))?;
            let mut mixed = match &layer.mixer {
                Mixer::Attention(a) => self.attention(l, a, &xn, start)?,
                Mixer::GatedDelta(gd) => self.gated_delta(l, gd, &xn)?,
                Mixer::Mla(m) => self.mla(l, m, &xn, start)?,
            };
            if layer.post_mixer_norm {
                mixed = self.norm_rows(&mixed, &format!("blk.{l}.post_attention_norm.weight"))?;
            }
            add_rows(&mut x, &mixed);
            let xn2 = self.norm_rows(&x, &format!("blk.{l}.{}.weight", self.spec.ffn_norm))?;
            let mut ffn = self.ffn(l, &layer, &xn2)?;
            if layer.post_ffn_norm {
                ffn = self.norm_rows(&ffn, &format!("blk.{l}.post_ffw_norm.weight"))?;
            }
            add_rows(&mut x, &ffn);
        }
        self.pos += tokens.len();
        Ok(x)
    }

    /// The logits of each final hidden state: the output norm, the LM head, the final softcap.
    fn head(&mut self, x: &[Vec<f64>]) -> R<Vec<Vec<f64>>> {
        let xf = match self.spec.hyper {
            // Hyper-connections end in a last mix of the lanes, in place of the output norm.
            Some(hc) => self.mix_lanes(x, "output_hc", hc, false)?.0,
            None => self.norm_rows(x, "output_norm.weight")?,
        };
        let head = if self.spec.tied_output {
            "token_embd.weight"
        } else {
            "output.weight"
        };
        let mut logits = self.project(head, &xf)?;
        if let Some(cap) = self.spec.final_logit_softcap {
            logits.iter_mut().flatten().for_each(|v| *v = cap * (*v / cap).tanh());
        }
        Ok(logits)
    }

    /// Greedy continuation: the prompt in one step, then `n` tokens one step each.
    pub fn generate(&mut self, prompt: &[u32], n: usize) -> R<Vec<u32>> {
        let mut logits = self.step(prompt)?;
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let t = argmax(&logits);
            out.push(t);
            if i + 1 < n {
                logits = self.step(&[t])?;
            }
        }
        Ok(out)
    }

    /// A whole tensor's values.
    fn tensor(&mut self, name: &str) -> R<Vec<f64>> {
        let (ty, _) = self.g.describe(name)?;
        let raw = self.tensor_bytes(name)?;
        let mut out = Vec::new();
        ty.dequantize(&raw, &mut out);
        Ok(out.into_iter().map(f64::from).collect())
    }

    /// Rows `[first, first + count)` of a tensor.
    fn rows(&mut self, name: &str, first: usize, count: usize) -> R<Vec<f64>> {
        let (ty, dims) = self.g.describe(name)?;
        let raw = match self.cache.get(name) {
            Some(b) => {
                let row = ty.bytes_for(dims.first().copied().unwrap_or(1) as usize);
                b[first * row..(first + count) * row].to_vec()
            }
            None => self.g.row_bytes(name, first, count)?,
        };
        let mut out = Vec::new();
        ty.dequantize(&raw, &mut out);
        Ok(out.into_iter().map(f64::from).collect())
    }

    fn norm_rows(&mut self, x: &[Vec<f64>], weight: &str) -> R<Vec<Vec<f64>>> {
        let w = self.tensor(weight)?;
        Ok(x.iter().map(|r| rms_norm(r, &w, self.spec.rms_eps)).collect())
    }

    /// `y = W·x` for every row of `xs`: the weight's bytes read once, each output row dequantized as ggml defines
    /// its format and contracted in `f64`, the rows split across threads.
    fn project(&mut self, name: &str, xs: &[Vec<f64>]) -> R<Vec<Vec<f64>>> {
        let (ty, dims) = self.g.describe(name)?;
        let [in_f, out_f] = dims[..] else {
            return Err(ReferenceError::NotYet("a projection that is not a matrix"));
        };
        let (in_f, out_f) = (in_f as usize, out_f as usize);
        let raw = self.tensor_bytes(name)?;
        let xs = self.read_by(ty, xs);
        Ok(products(&[Product { raw, from: 0, ty, in_f, out_f, xs }], self.threads).pop().expect("the product"))
    }

    /// The activations a projection by weights of type `ty` reads: quantized when the weights are.
    fn read_by(&mut self, ty: GgmlType, xs: &[Vec<f64>]) -> Vec<Vec<f64>> {
        match ty {
            GgmlType::F32 | GgmlType::F16 | GgmlType::Bf16 => xs.to_vec(),
            _ => xs.iter().map(|x| self.quantize_row(x)).collect(),
        }
    }

    /// `row` as the cache keeps it: as computed, or each value rounded to `f32` and then to binary16, its near roundings
    /// recorded (once a row, word [`CACHE_WORD`]) and the flipped ones taken to the other neighbour. A value's margin is
    /// its distance from the half-way point in levels of its row's scale — the row's largest magnitude over 127, the
    /// yardstick an int8 rounding of the row is measured by — since a run's `f32` arithmetic errs by a share of the
    /// scale it computes at, whatever the value: a small value its rotation left after cancelling errs by more of its
    /// own binary16 step than a large one.
    fn kept(&mut self, row: Vec<f64>) -> Vec<f64> {
        if self.kv == KvStorage::Exact {
            return row;
        }
        let activation = row.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &a| {
            (a as f32).to_bits().to_le_bytes().iter().fold(h, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
        });
        let position = self.at;
        let level = row.iter().fold(0f64, |m, &v| m.max(f64::from((v as f32).abs()))) / 127.0;
        let mut out = Vec::with_capacity(row.len());
        for (element, &v) in row.iter().enumerate() {
            let key = (activation, element, CACHE_WORD);
            let (kept, distance, apart) = binary16(v, !self.flipped_roundings.is_empty() && self.flipped_roundings.contains(&key));
            let margin = distance / level;
            if margin <= NEAR_ROUNDING && self.near_roundings.insert(key) {
                let step = apart / level;
                self.near.push(NearCall::Rounding(RoundingCall { activation, element, word: CACHE_WORD, position, margin, step }));
            }
            out.push(kept);
        }
        out
    }

    /// One row quantized as [`quantize_activations`] does, its near roundings recorded (once an activation) and the
    /// flipped ones taken the other way.
    fn quantize_row(&mut self, x: &[f64]) -> Vec<f64> {
        if self.activations == Activations::Exact {
            return x.to_vec();
        }
        let activation = x.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &a| {
            (a as f32).to_bits().to_le_bytes().iter().fold(h, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
        });
        let position = self.at;
        let (flipped, recorded) = (&self.flipped_roundings, &mut self.near_roundings);
        let mut near = Vec::new();
        let out = quantize_deciding(x, self.activations, |element, word, q| {
            let r = q.round_ties_even();
            let margin = f64::from(((q - q.floor()) - 0.5).abs());
            if margin <= NEAR_ROUNDING && recorded.insert((activation, element, word)) {
                near.push(NearCall::Rounding(RoundingCall { activation, element, word, position, margin, step: 1.0 }));
            }
            match !flipped.is_empty() && flipped.contains(&(activation, element, word)) {
                true if r > q => r - 1.0,
                true => r + 1.0,
                false => r,
            }
        });
        self.near.extend(near);
        out
    }

    fn attention(&mut self, l: usize, a: &Attention, xn: &[Vec<f64>], start: usize) -> R<Vec<Vec<f64>>> {
        let (hd, vd) = (a.head_dim, a.value_dim);
        let (aq, akv, avd) = (a.heads * hd, a.kv_heads * hd, a.kv_heads * vd);
        // A fused projection holds the queries, keys and values one after another in each output row.
        let fused = if a.fused_qkv {
            let mut qkv = self.project(&format!("blk.{l}.attn_qkv.weight"), xn)?;
            if a.bias {
                let b = self.tensor(&format!("blk.{l}.attn_qkv.bias"))?;
                add_bias(&mut qkv, &b);
            }
            Some(qkv)
        } else {
            None
        };
        let part = |rows: &Option<Vec<Vec<f64>>>, from: usize, n: usize| -> Option<Vec<Vec<f64>>> {
            rows.as_ref().map(|r| r.iter().map(|row| row[from..from + n].to_vec()).collect())
        };
        let projected = match part(&fused, 0, aq) {
            Some(q) => q,
            None => self.project(&format!("blk.{l}.attn_q.weight"), xn)?,
        };
        // With an output gate the query projection holds, per head, the head's queries then its gate.
        let (mut q, gate): (Vec<Vec<f64>>, Option<Vec<Vec<f64>>>) = if a.output_gate {
            let part = |r: &Vec<f64>, at: usize| -> Vec<f64> {
                (0..a.heads).flat_map(|h| r[h * 2 * hd + at..h * 2 * hd + at + hd].to_vec()).collect()
            };
            (projected.iter().map(|r| part(r, 0)).collect(), Some(projected.iter().map(|r| part(r, hd)).collect()))
        } else {
            (projected, None)
        };
        let (mut k, mut v) = match (part(&fused, aq, akv), part(&fused, aq + akv, avd)) {
            (Some(k), Some(v)) => (k, v),
            _ => (
                self.project(&format!("blk.{l}.attn_k.weight"), xn)?,
                self.project(&format!("blk.{l}.attn_v.weight"), xn)?,
            ),
        };
        if a.bias && !a.fused_qkv {
            for (rows, which) in [(&mut q, "attn_q"), (&mut k, "attn_k"), (&mut v, "attn_v")] {
                let b = self.tensor(&format!("blk.{l}.{which}.bias"))?;
                add_bias(rows, &b);
            }
        }
        let (qn, kn) = if a.qk_norm {
            let qw = self.tensor(&format!("blk.{l}.attn_q_norm.weight"))?;
            let kw = self.tensor(&format!("blk.{l}.attn_k_norm.weight"))?;
            (Some(qw), Some(kw))
        } else {
            (None, None)
        };
        let factors = match a.rope.as_ref().map(|r| &r.scaling) {
            Some(RopeScaling::Factors) => Some(self.tensor("rope_freqs.weight")?),
            _ => None,
        };
        let sinks = if a.sinks { Some(self.tensor(&format!("blk.{l}.attn_sinks.weight"))?) } else { None };
        // QSA's index queries, normed and rotated head by head, and this step's index keys (transformers
        // `Qwen4ExpTextQSAIndexer`, llama.cpp `qwen4exp`).
        let index = match self.spec.indexer.clone() {
            Some(ix) if ix.layers.contains(&l) => {
                let iq = self.project(&format!("blk.{l}.indexer.q_proj.weight"), xn)?;
                let ik = self.project(&format!("blk.{l}.indexer.k_proj.weight"), xn)?;
                let qw = self.tensor(&format!("blk.{l}.indexer.q_norm.weight"))?;
                let kw = self.tensor(&format!("blk.{l}.indexer.k_norm.weight"))?;
                let eps = self.spec.rms_eps;
                let queries: Vec<Vec<Vec<f64>>> = iq
                    .iter()
                    .enumerate()
                    .map(|(i, row)| {
                        (0..ix.heads)
                            .map(|h| {
                                let mut q = rms_norm(&row[h * ix.head_dim..(h + 1) * ix.head_dim], &qw, eps);
                                if let Some(r) = &a.rope {
                                    rotate(&mut q, start + i, r, factors.as_deref());
                                }
                                q
                            })
                            .collect()
                    })
                    .collect();
                Some((ix, queries, ik, kw))
            }
            _ => None,
        };
        // Llama 4's weightless norm follows the rotation (`Llama4TextL2Norm`), and its temperature follows the norm.
        let ones = vec![1.0; hd];
        // A norm across every head at once (MiniMax-M2) runs over the whole projection, before the rotation.
        let (whole, per_head) = if a.qk_norm_whole { ((qn, kn), (None, None)) } else { ((None, None), (qn, kn)) };
        let (qn, kn) = per_head;
        for (i, (qr, kr)) in q.iter_mut().zip(k.iter_mut()).enumerate() {
            let pos = start + i;
            if let (Some(qw), Some(kw)) = &whole {
                *qr = rms_norm(qr, qw, self.spec.rms_eps);
                *kr = rms_norm(kr, kw, self.spec.rms_eps);
            }
            for h in 0..a.heads {
                let s = &mut qr[h * hd..(h + 1) * hd];
                if let Some(w) = &qn {
                    let n = rms_norm(s, w, self.spec.rms_eps);
                    s.copy_from_slice(&n);
                }
                if let Some(r) = &a.rope {
                    rotate(s, pos, r, factors.as_deref());
                }
                if a.qk_unit_norm {
                    let n = rms_norm(s, &ones, self.spec.rms_eps);
                    s.copy_from_slice(&n);
                }
                if let Some(t) = a.temperature {
                    let f = t.at(pos);
                    s.iter_mut().for_each(|v| *v *= f);
                }
            }
            for h in 0..a.kv_heads {
                let s = &mut kr[h * hd..(h + 1) * hd];
                if let Some(w) = &kn {
                    let n = rms_norm(s, w, self.spec.rms_eps);
                    s.copy_from_slice(&n);
                }
                if let Some(r) = &a.rope {
                    rotate(s, pos, r, factors.as_deref());
                }
                if a.qk_unit_norm {
                    let n = rms_norm(s, &ones, self.spec.rms_eps);
                    s.copy_from_slice(&n);
                }
            }
        }
        let k: Vec<Vec<f64>> = k.into_iter().map(|r| self.kept(r)).collect();
        let v: Vec<Vec<f64>> = v.into_iter().map(|r| self.kept(r)).collect();
        let eps = self.spec.rms_eps;
        let LayerState::Attention { k: kc, v: vc, ik: ikc } = &mut self.states[l] else {
            unreachable!("an attention layer holds an attention cache")
        };
        kc.extend(k);
        vc.extend(v);
        let (kc, vc) = (&*kc, &*vc);
        // Each query's positions: QSA's selection where the model indexes, else every visible one.
        let kept: Option<Vec<Vec<usize>>> = match index {
            Some((ix, queries, keys, kw)) => {
                ikc.extend(keys);
                let ikc = &*ikc;
                Some(
                    queries
                        .iter()
                        .enumerate()
                        .map(|(i, q)| qsa_positions(&ix, q, ikc, &kw, a.rope.as_ref(), factors.as_deref(), eps, start + i))
                        .collect(),
                )
            }
            None => None,
        };
        let group = a.heads / a.kv_heads;
        let mut out = vec![vec![0f64; a.heads * vd]; xn.len()];
        for (i, o) in out.iter_mut().enumerate() {
            let pos = start + i;
            // A sliding window hides every key `window` or more positions back (llama.cpp's standard SWA mask); a chunk keeps
            // the keys of the query's own chunk (Llama 4's chunked mask, `q / C == k / C`).
            let first = a.window.map_or(0, |w| w.first(pos + 1) - 1);
            let positions: Vec<usize> = match &kept {
                Some(kept) => kept[i].clone(),
                None => (first..=pos).collect(),
            };
            for h in 0..a.heads {
                let g = h / group;
                let qh = &q[i][h * hd..(h + 1) * hd];
                let mut scores: Vec<f64> = positions
                    .iter()
                    .map(|&j| {
                        let kj = &kc[j][g * hd..(g + 1) * hd];
                        let s = qh.iter().zip(kj).map(|(x, y)| x * y).sum::<f64>() * a.scale;
                        match a.softcap {
                            Some(cap) => cap * (s / cap).tanh(),
                            None => s,
                        }
                    })
                    .collect();
                match &sinks {
                    // A sink joins the softmax's denominator and takes no value (ggml's `soft_max_ext` with sinks).
                    Some(sk) => {
                        let m = scores.iter().cloned().fold(sk[h], f64::max);
                        let total = scores.iter().map(|s| (s - m).exp()).sum::<f64>() + (sk[h] - m).exp();
                        scores.iter_mut().for_each(|s| *s = (*s - m).exp() / total);
                    }
                    None => softmax(&mut scores),
                }
                let oh = &mut o[h * vd..(h + 1) * vd];
                for (&j, w) in positions.iter().zip(&scores) {
                    for (e, slot) in oh.iter_mut().enumerate() {
                        *slot += w * vc[j][g * vd + e];
                    }
                }
            }
        }
        if let Some(gate) = gate {
            for (o, g) in out.iter_mut().zip(&gate) {
                for (v, &g) in o.iter_mut().zip(g) {
                    *v *= sigmoid(g);
                }
            }
        }
        let mut projected = self.project(&format!("blk.{l}.attn_output.weight"), &out)?;
        if a.output_bias {
            let b = self.tensor(&format!("blk.{l}.attn_output.bias"))?;
            add_bias(&mut projected, &b);
        }
        Ok(projected)
    }

    /// Multi-head latent attention (llama.cpp `models/deepseek2.cpp`), computed absorbed: each position's cache row is
    /// its RMS-normed `kv_lora_rank` latent and the shared rotated key; each head's unrotated query reaches the latent's
    /// space through the key half of the latent's expansion, attends every cached row with its rotated part at the
    /// model's scale, takes the rows' latents as its values, and leaves through the value half of the expansion. The
    /// halves read their activations unquantized under every precision, so these are the scores and values the
    /// expanded attention takes — each cached latent through both halves, each head attending its own keys and values —
    /// and a lane computes either.
    fn mla(&mut self, l: usize, m: &Mla, xn: &[Vec<f64>], start: usize) -> R<Vec<Vec<f64>>> {
        let (h, nope, rope, vd, lora) = (m.heads, m.qk_nope, m.qk_rope, m.v_dim, m.kv_lora_rank);
        let (qk, kw) = (nope + rope, lora + rope);
        let q = match m.q_lora_rank {
            None => self.project(&format!("blk.{l}.attn_q.weight"), xn)?,
            Some(_) => {
                let qa = self.project(&format!("blk.{l}.attn_q_a.weight"), xn)?;
                let qa = self.norm_rows(&qa, &format!("blk.{l}.attn_q_a_norm.weight"))?;
                self.project(&format!("blk.{l}.attn_q_b.weight"), &qa)?
            }
        };
        let kva = self.project(&format!("blk.{l}.attn_kv_a_mqa.weight"), xn)?;
        let latent: Vec<Vec<f64>> = kva.iter().map(|r| r[..lora].to_vec()).collect();
        let latent = self.norm_rows(&latent, &format!("blk.{l}.attn_kv_a_norm.weight"))?;
        // The expansion's halves, each head's block of rows: `(tensor, first row of head 0, rows a head)`, its rows the
        // head's outputs or its inputs.
        let (key_half, value_half) = if m.kv_b_split {
            ((format!("blk.{l}.attn_k_b.weight"), 0, lora, true), (format!("blk.{l}.attn_v_b.weight"), 0, vd, true))
        } else {
            let kv = format!("blk.{l}.attn_kv_b.weight");
            ((kv.clone(), 0, nope, false), (kv, nope, vd, true))
        };
        let head_stride = |half: &(String, usize, usize, bool)| if m.kv_b_split { half.2 } else { nope + vd };
        let (ks, vs) = (head_stride(&key_half), head_stride(&value_half));
        let mut out = Vec::with_capacity(xn.len());
        for (i, (qr, (kr, c))) in q.iter().zip(kva.iter().zip(&latent)).enumerate() {
            let pos = start + i;
            let mut pe = kr[lora..kw].to_vec();
            rotate(&mut pe, pos, &m.rope, None);
            let row = self.kept(c.iter().chain(&pe).copied().collect());
            let LayerState::Attention { k: cache, .. } = &mut self.states[l] else {
                unreachable!("a latent-attention layer holds an attention cache")
            };
            cache.push(row);
            // Each head's unrotated query, absorbed.
            let qn: Vec<f64> = (0..h).flat_map(|hh| qr[hh * qk..hh * qk + nope].to_vec()).collect();
            let mut ol = vec![0f64; h * lora];
            for hh in 0..h {
                let (name, first, count, rows_out) = &key_half;
                let x = &qn[hh * nope..(hh + 1) * nope];
                let absorbed = self.head_product(name, hh * ks + first, *count, *rows_out, x)?;
                let mut qe = qr[hh * qk + nope..(hh + 1) * qk].to_vec();
                rotate(&mut qe, pos, &m.rope, None);
                let LayerState::Attention { k: cache, .. } = &self.states[l] else { unreachable!() };
                let mut scores: Vec<f64> = cache
                    .iter()
                    .map(|row| {
                        let s = absorbed.iter().zip(&row[..lora]).map(|(a, b)| a * b).sum::<f64>()
                            + qe.iter().zip(&row[lora..]).map(|(a, b)| a * b).sum::<f64>();
                        s * m.scale
                    })
                    .collect();
                softmax(&mut scores);
                for (w, row) in scores.iter().zip(cache) {
                    for (slot, v) in ol[hh * lora..(hh + 1) * lora].iter_mut().zip(&row[..lora]) {
                        *slot += w * v;
                    }
                }
            }
            // Each head's attended latent through the value half.
            let mut ct = Vec::with_capacity(h * vd);
            for hh in 0..h {
                let (name, first, count, rows_out) = &value_half;
                ct.extend(self.head_product(name, hh * vs + first, *count, *rows_out, &ol[hh * lora..(hh + 1) * lora])?);
            }
            out.push(ct);
        }
        self.project(&format!("blk.{l}.attn_output.weight"), &out)
    }

    /// One head's product with its block of `count` rows of `name` from `first`: each row dotted with `x` when the rows
    /// are the head's outputs, the rows weighed by `x` and summed when they are its inputs — in `f64`.
    fn head_product(&mut self, name: &str, first: usize, count: usize, rows_out: bool, x: &[f64]) -> R<Vec<f64>> {
        let w = self.rows(name, first, count)?;
        let width = w.len() / count.max(1);
        Ok(if rows_out {
            w.chunks(width).map(|row| row.iter().zip(x).map(|(a, b)| a * b).sum()).collect()
        } else {
            let mut y = vec![0f64; width];
            for (row, xi) in w.chunks(width).zip(x) {
                for (slot, v) in y.iter_mut().zip(row) {
                    *slot += xi * v;
                }
            }
            y
        })
    }

    /// One Gated DeltaNet block over the rows of `xn` in order (llama.cpp `models/qwen35.cpp`
    /// `build_layer_attn_linear`): the fused `q|k|v` projection through a depthwise causal convolution and SiLU, q
    /// and k L2-normalized per key head, then per value head the delta rule — decay the memory by
    /// `exp(softplus(α + dt)·a)`, write `β·(v − decayed memory·k)` along `k`, read with `q/√head_dim` — and the
    /// read normalized and gated by `silu(z)`.
    /// The per-layer embedding's fresh state: `ngram - 1` ends of document before the first token, and a convolution
    /// that has read only zeros.
    fn fresh_ple(spec: &ModelSpec) -> (Vec<u32>, Vec<Vec<f64>>) {
        match (&spec.ple, spec.hyper) {
            (Some(p), Some(hc)) => {
                (vec![p.eos; p.ngram - 1], vec![vec![0.0; hc.lanes * spec.d]; (p.conv_taps - 1) * p.ngram])
            }
            _ => (Vec::new(), Vec::new()),
        }
    }

    /// The stack over hyper-connection lanes (transformers `Qwen4ExpTextDecoderLayer`): `lanes` copies of each
    /// embedding; at its layer the per-layer embedding joins the lanes; each sublayer reads the lanes' mix and its output
    /// joins every lane, weighted. Each position's lanes, `lanes·d` wide.
    fn hyper_layers(&mut self, tokens: &[u32], x: &[Vec<f64>], start: usize, hc: HyperConnections) -> R<Vec<Vec<f64>>> {
        let mut h: Vec<Vec<f64>> = x.iter().map(|r| r.repeat(hc.lanes)).collect();
        for l in 0..self.spec.layers.len() {
            let layer = self.spec.layers[l].clone();
            if self.spec.ple.as_ref().is_some_and(|p| p.layer == l) {
                let added = self.ple(l, tokens, &h, hc)?;
                add_rows(&mut h, &added);
            }
            let (input, inject) = self.mix_lanes(&h, &format!("blk.{l}.hc_attn"), hc, true)?;
            let out = match &layer.mixer {
                Mixer::Attention(a) => self.attention(l, a, &input, start)?,
                Mixer::GatedDelta(gd) => self.gated_delta(l, gd, &input)?,
                Mixer::Mla(m) => self.mla(l, m, &input, start)?,
            };
            inject_lanes(&mut h, &out, &inject, self.spec.d);
            let (input, inject) = self.mix_lanes(&h, &format!("blk.{l}.hc_ffn"), hc, true)?;
            let out = self.ffn(l, &layer, &input)?;
            inject_lanes(&mut h, &out, &inject, self.spec.d);
        }
        Ok(h)
    }

    /// A hyper-connection's mix of each position's lanes (transformers `Qwen4ExpTextGatedResidual`): the lanes normed
    /// each on its own (`<stem>_norm`, one weight across all of them), weighted by `σ(up·silu(down·x̂ / lanes))` and
    /// averaged into one input; with `inject`, each lane's share of the sublayer's output, `2σ(inject·x̂ / lanes)`.
    fn mix_lanes(&mut self, h: &[Vec<f64>], stem: &str, hc: HyperConnections, inject: bool) -> R<(Vec<Vec<f64>>, Vec<Vec<f64>>)> {
        let (d, lanes) = (self.spec.d, hc.lanes as f64);
        let norm = self.tensor(&format!("{stem}_norm.weight"))?;
        let normed: Vec<Vec<f64>> = h.iter().map(|r| lane_norm(r, &norm, d, self.spec.rms_eps)).collect();
        let down = self.project(&format!("{stem}_down.weight"), &normed)?;
        let mid: Vec<Vec<f64>> = down.iter().map(|r| r.iter().map(|v| silu(v / lanes)).collect()).collect();
        let up = self.project(&format!("{stem}_up.weight"), &mid)?;
        let inputs = normed
            .iter()
            .zip(&up)
            .map(|(x, w)| (0..d).map(|j| (0..hc.lanes).map(|s| sigmoid(w[s * d + j]) * x[s * d + j]).sum::<f64>() / lanes).collect())
            .collect();
        let shares = if inject {
            let raw = self.project(&format!("{stem}_inject.weight"), &normed)?;
            raw.iter().map(|r| r.iter().map(|v| 2.0 * sigmoid(v / lanes)).collect()).collect()
        } else {
            Vec::new()
        };
        Ok((inputs, shares))
    }

    /// The per-layer embedding of `tokens` (transformers `Qwen4ExpTextPLELayer`), added to layer `l`'s lanes `h`: each
    /// position's n-grams hashed into the table, the rows a value and a key for each lane, the lane's normed state gating
    /// the value, and the dilated depthwise convolution of the normed gated values after it.
    fn ple(&mut self, l: usize, tokens: &[u32], h: &[Vec<f64>], hc: HyperConnections) -> R<Vec<Vec<f64>>> {
        let p = self.spec.ple.clone().expect("a per-layer embedding");
        let (d, lanes) = (self.spec.d, hc.lanes);
        let width = lanes * d;
        let mut rows = Vec::with_capacity(tokens.len());
        for &t in tokens {
            self.ple_tokens.push(t);
            let w = &self.ple_tokens[self.ple_tokens.len() - p.ngram..];
            let mut emb = Vec::with_capacity((p.ngram - 1) * p.heads_per_ngram * p.head_dim);
            for id in ngram_rows(&p, w) {
                emb.extend(self.rows("per_layer_token_embd.weight", id as usize, 1)?);
            }
            rows.push(emb);
        }
        let key = self.project(&format!("blk.{l}.ple_key.weight"), &rows)?;
        let value = self.project(&format!("blk.{l}.ple_value.weight"), &rows)?;
        let eps = self.spec.rms_eps;
        let (nk, nq, nc) = (
            self.tensor(&format!("blk.{l}.ple_norm_key.weight"))?,
            self.tensor(&format!("blk.{l}.ple_norm_query.weight"))?,
            self.tensor(&format!("blk.{l}.ple_norm_conv.weight"))?,
        );
        let kernel = self.tensor(&format!("blk.{l}.ple_conv1d.weight"))?;
        let mut out = Vec::with_capacity(tokens.len());
        for (i, x) in h.iter().enumerate() {
            let key = lane_norm(&key[i], &nk, d, eps);
            let query = lane_norm(x, &nq, d, eps);
            let mut gated = vec![0.0; width];
            for s in 0..lanes {
                let g = (0..d).map(|j| key[s * d + j] * query[s * d + j]).sum::<f64>() / (d as f64).sqrt();
                let sign = if g > 0.0 { 1.0 } else if g < 0.0 { -1.0 } else { 0.0 };
                let gate = sigmoid(sign * g.abs().max(1e-6).sqrt());
                for j in 0..d {
                    gated[s * d + j] = gate * value[i][j];
                }
            }
            let normed = lane_norm(&gated, &nc, d, eps);
            // Tap `k` of `taps` reads the normed values `(taps - 1 - k)·ngram` positions back.
            self.ple_conv.push(normed);
            let history = &self.ple_conv;
            let last = history.len() - 1;
            let row: Vec<f64> = (0..width)
                .map(|c| {
                    let conv = (0..p.conv_taps)
                        .map(|k| kernel[c * p.conv_taps + k] * history[last - (p.conv_taps - 1 - k) * p.ngram][c])
                        .sum::<f64>();
                    gated[c] + silu(conv)
                })
                .collect();
            self.ple_conv.remove(0);
            out.push(row);
        }
        Ok(out)
    }

    fn gated_delta(&mut self, l: usize, gd: &GatedDelta, xn: &[Vec<f64>]) -> R<Vec<Vec<f64>>> {
        let (nk, nv, hd, taps) = (gd.k_heads, gd.v_heads, gd.head_dim, gd.conv_taps);
        let (qk, vw) = (nk * hd, nv * hd);
        let channels = 2 * qk + vw;
        let qkv = self.project(&format!("blk.{l}.attn_qkv.weight"), xn)?;
        let z = self.project(&format!("blk.{l}.attn_gate.weight"), xn)?;
        let beta = self.project(&format!("blk.{l}.ssm_beta.weight"), xn)?;
        let alpha = self.project(&format!("blk.{l}.ssm_alpha.weight"), xn)?;
        let kernel = self.tensor(&format!("blk.{l}.ssm_conv1d.weight"))?;
        let dt = self.tensor(&format!("blk.{l}.ssm_dt.bias"))?;
        let a = self.tensor(&format!("blk.{l}.ssm_a"))?;
        let norm = self.tensor(&format!("blk.{l}.ssm_norm.weight"))?;
        let eps = self.spec.rms_eps;
        let keep_state = self.state;
        let LayerState::Delta { conv, memory } = &mut self.states[l] else {
            unreachable!("a gated-delta layer holds a recurrent state")
        };
        let l2 = |x: &[f64]| -> Vec<f64> {
            let den = (x.iter().map(|v| v * v).sum::<f64>() + eps).sqrt();
            x.iter().map(|v| v / den).collect()
        };
        let mut reads = Vec::with_capacity(xn.len());
        for t in 0..xn.len() {
            let c: Vec<f64> = (0..channels)
                .map(|ch| {
                    let older = conv.iter().enumerate().map(|(j, row)| row[ch] * kernel[ch * taps + j]).sum::<f64>();
                    silu(older + qkv[t][ch] * kernel[ch * taps + taps - 1])
                })
                .collect();
            if taps > 1 {
                conv.remove(0);
                conv.push(qkv[t].clone());
            }
            let q: Vec<f64> = (0..nk).flat_map(|h| l2(&c[h * hd..(h + 1) * hd])).collect();
            let k: Vec<f64> = (0..nk).flat_map(|h| l2(&c[qk + h * hd..qk + (h + 1) * hd])).collect();
            let v = &c[2 * qk..];
            let mut read = vec![0.0; vw];
            let scale = 1.0 / (hd as f64).sqrt();
            for h in 0..nv {
                let (kq, kk) = (&q[(h % nk) * hd..(h % nk + 1) * hd], &k[(h % nk) * hd..(h % nk + 1) * hd]);
                let decay = (softplus(alpha[t][h] + dt[h]) * a[h]).exp();
                let b = sigmoid(beta[t][h]);
                let s = &mut memory[h * hd * hd..(h + 1) * hd * hd];
                for r in 0..hd {
                    let row = &mut s[r * hd..(r + 1) * hd];
                    let u = decay * row.iter().zip(kk).map(|(m, k)| m * k).sum::<f64>();
                    let coef = b * (v[h * hd + r] - u);
                    for (m, &kc) in row.iter_mut().zip(kk) {
                        *m = decay * *m + coef * kc;
                    }
                }
                let o: Vec<f64> =
                    (0..hd).map(|r| s[r * hd..(r + 1) * hd].iter().zip(kq).map(|(m, q)| m * q).sum::<f64>() * scale).collect();
                for m in s.iter_mut() {
                    *m = keep_state.keep(*m);
                }
                let on = rms_norm(&o, &norm, eps);
                for r in 0..hd {
                    let g = z[t][h * hd + r];
                    read[h * hd + r] = on[r]
                        * match gd.gate {
                            GateActivation::Silu => silu(g),
                            GateActivation::Sigmoid => sigmoid(g),
                        };
                }
            }
            reads.push(read);
        }
        self.project(&format!("blk.{l}.ssm_out.weight"), &reads)
    }

    fn ffn(&mut self, l: usize, layer: &Layer, xn: &[Vec<f64>]) -> R<Vec<Vec<f64>>> {
        match &layer.ffn {
            Ffn::Dense { act, .. } => {
                let g = self.project(&format!("blk.{l}.ffn_gate.weight"), xn)?;
                let u = self.project(&format!("blk.{l}.ffn_up.weight"), xn)?;
                let h: Vec<Vec<f64>> = g.iter().zip(&u).map(|(g, u)| gated(act, g, u)).collect();
                self.project(&format!("blk.{l}.ffn_down.weight"), &h)
            }
            Ffn::Moe(m) => self.moe(l, m, xn),
        }
    }

    /// A mixture of experts as llama.cpp's `build_moe_ffn` computes it: router logits (plus a router bias), turned
    /// into probabilities by the spec's gating, the top `k` chosen, their weights renormalized (the sum clamped as
    /// llama.cpp clamps it) and scaled, each chosen expert's gated FFN (with its biases) summed by its weight.
    fn moe(&mut self, l: usize, m: &Moe, xn: &[Vec<f64>]) -> R<Vec<Vec<f64>>> {
        // A router ranks scores largest first, a NaN below every number and two NaNs alike: the chip's order, so a NaN
        // score is chosen only when no number remains.
        fn descending(a: f64, b: f64) -> std::cmp::Ordering {
            match (a.is_nan(), b.is_nan()) {
                (false, false) => b.partial_cmp(&a).expect("two numbers compare"),
                (true, true) => std::cmp::Ordering::Equal,
                (true, false) => std::cmp::Ordering::Greater,
                (false, true) => std::cmp::Ordering::Less,
            }
        }
        let selection_bias = match m.gating {
            Gating::Sigmoid { bias: true } => Some(self.tensor(&format!("blk.{l}.exp_probs_b.bias"))?),
            _ => None,
        };
        let mut logits = self.project(&format!("blk.{l}.ffn_gate_inp.weight"), xn)?;
        let bias = |me: &mut Self, which: &str| -> R<Option<Vec<f64>>> {
            if m.biases {
                Ok(Some(me.tensor(&format!("blk.{l}.{which}.bias"))?))
            } else {
                Ok(None)
            }
        };
        if let Some(b) = bias(self, "ffn_gate_inp")? {
            add_bias(&mut logits, &b);
        }
        let (bg, bu, bd) = (bias(self, "ffn_gate_exps")?, bias(self, "ffn_up_exps")?, bias(self, "ffn_down_exps")?);
        let d = self.spec.d;
        let f = m.expert_dff;
        // Each row's experts and their weights, in the order the row sums them.
        let mut routes: Vec<Vec<(usize, f64)>> = Vec::with_capacity(xn.len());
        for (t, lg) in logits.iter().enumerate() {
            let mut p = lg.clone();
            match m.gating {
                Gating::Softmax => softmax(&mut p),
                Gating::Sigmoid { .. } | Gating::SigmoidScaledInput => p.iter_mut().for_each(|v| *v = sigmoid(*v)),
                Gating::SoftmaxOfTopK => {}
            }
            // Selection may read biased scores; the weights read the unbiased ones.
            let mut sel: Vec<f64> = match &selection_bias {
                Some(b) => p.iter().zip(b).map(|(v, b)| v + b).collect(),
                None => p.clone(),
            };
            if let Some((groups, used)) = m.groups {
                let per = p.len() / groups;
                let score = |g: usize| {
                    let mut v: Vec<f64> = sel[g * per..(g + 1) * per].to_vec();
                    v.sort_by(|a, b| descending(*a, *b));
                    v[0] + v.get(1).copied().unwrap_or(0.0)
                };
                let mut ranked: Vec<usize> = (0..groups).collect();
                ranked.sort_by(|&a, &b| descending(score(a), score(b)).then(a.cmp(&b)));
                if used < groups {
                    let (kept, dropped) = (score(ranked[used - 1]), score(ranked[used]));
                    self.routed(l, self.pos + t, kept, dropped);
                }
                for &g in &ranked[used..] {
                    sel[g * per..(g + 1) * per].iter_mut().for_each(|v| *v = f64::NEG_INFINITY);
                }
            }
            let mut order: Vec<usize> = (0..p.len()).collect();
            order.sort_by(|&a, &b| descending(sel[a], sel[b]).then(a.cmp(&b)));
            if let Some(&passed) = order.get(m.top_k) {
                self.routed(l, self.pos + t, sel[order[m.top_k - 1]], sel[passed]);
                if self.flipped_routings.contains(&(l, self.pos + t)) {
                    order.swap(m.top_k - 1, m.top_k);
                }
            }
            let chosen = &order[..m.top_k];
            let mut w: Vec<f64> = chosen.iter().map(|&e| p[e]).collect();
            if matches!(m.gating, Gating::SoftmaxOfTopK) {
                softmax(&mut w);
            }
            if m.norm_top_k {
                let total = w.iter().sum::<f64>().max(6.103515625e-5);
                w.iter_mut().for_each(|v| *v /= total);
            }
            if m.weights_scale != 0.0 && m.weights_scale != 1.0 {
                w.iter_mut().for_each(|v| *v *= m.weights_scale);
            }
            routes.push(chosen.iter().copied().zip(w).collect());
        }
        // Every expert runs once, on all the rows that chose it: its weights are read once for the block.
        let mut readers: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for (t, route) in routes.iter().enumerate() {
            for &(e, _) in route {
                readers.entry(e).or_default().push(t);
            }
        }
        // A gating that scales the input weighs each row's input to an expert, not the expert's output.
        let scaled_input = m.gating == Gating::SigmoidScaledInput;
        let weight = |t: usize, e: usize| routes[t].iter().find(|(x, _)| *x == e).map(|(_, w)| *w).expect("a chosen expert");
        // Every expert's gate and up in one pass over the threads, then every expert's down in another.
        let mut ups = Vec::new();
        for (&e, rows) in &readers {
            let xs: Vec<Vec<f64>> = rows
                .iter()
                .map(|&t| if scaled_input { xn[t].iter().map(|v| v * weight(t, e)).collect() } else { xn[t].clone() })
                .collect();
            ups.push(self.expert(l, "ffn_gate_exps", e, f, &xs)?);
            ups.push(self.expert(l, "ffn_up_exps", e, f, &xs)?);
        }
        let gu = products(&ups, self.threads);
        let mut downs = Vec::new();
        for ((&e, _), pair) in readers.iter().zip(gu.chunks_exact(2)) {
            let (mut g, mut u) = (pair[0].clone(), pair[1].clone());
            if let (Some(bg), Some(bu)) = (&bg, &bu) {
                g.iter_mut().for_each(|g| g.iter_mut().zip(&bg[e * f..(e + 1) * f]).for_each(|(v, b)| *v += b));
                u.iter_mut().for_each(|u| u.iter_mut().zip(&bu[e * f..(e + 1) * f]).for_each(|(v, b)| *v += b));
            }
            let h: Vec<Vec<f64>> = g.iter().zip(&u).map(|(g, u)| gated(&m.act, g, u)).collect();
            downs.push(self.expert(l, "ffn_down_exps", e, d, &h)?);
        }
        let mut ys: HashMap<(usize, usize), Vec<f64>> = HashMap::new();
        for ((&e, rows), y) in readers.iter().zip(products(&downs, self.threads)) {
            for (&t, mut y) in rows.iter().zip(y) {
                if let Some(bd) = &bd {
                    y.iter_mut().zip(&bd[e * d..(e + 1) * d]).for_each(|(v, b)| *v += b);
                }
                ys.insert((t, e), y);
            }
        }
        let mut out = vec![vec![0f64; d]; xn.len()];
        for (t, route) in routes.iter().enumerate() {
            for &(e, we) in route {
                let we = if scaled_input { 1.0 } else { we };
                for (slot, v) in out[t].iter_mut().zip(&ys[&(t, e)]) {
                    *slot += we * v;
                }
            }
        }
        if m.shared_experts > 0 {
            let g = self.project(&format!("blk.{l}.ffn_gate_shexp.weight"), xn)?;
            let u = self.project(&format!("blk.{l}.ffn_up_shexp.weight"), xn)?;
            let h: Vec<Vec<f64>> = g.iter().zip(&u).map(|(g, u)| gated(&m.act, g, u)).collect();
            let mut y = self.project(&format!("blk.{l}.ffn_down_shexp.weight"), &h)?;
            // Qwen's shared experts are scaled by `σ(x·g)`, the gate a vector over the input.
            if m.shared_gate {
                let g = self.tensor(&format!("blk.{l}.ffn_gate_inp_shexp.weight"))?;
                for (row, x) in y.iter_mut().zip(xn) {
                    let gate = sigmoid(x.iter().zip(&g).map(|(a, b)| a * b).sum());
                    row.iter_mut().for_each(|v| *v *= gate);
                }
            }
            add_rows(&mut out, &y);
        }
        Ok(out)
    }

    /// Expert `e`'s projection of every row of `xs`, out of a stacked `[in, out, experts]` tensor, as a product to run.
    fn expert(&mut self, l: usize, which: &str, e: usize, out_f: usize, xs: &[Vec<f64>]) -> R<Product> {
        let name = format!("blk.{l}.{which}.weight");
        let (ty, _) = self.g.describe(&name)?;
        let in_f = xs[0].len();
        let row = ty.bytes_for(in_f);
        let raw = self.tensor_bytes(&name)?;
        let xs = self.read_by(ty, xs);
        Ok(Product { raw, from: e * out_f * row, ty, in_f, out_f, xs })
    }
}

/// The multiply-adds one thread of [`products`] is given at least: a smaller pass runs on fewer threads (one, below
/// this), so a thread is never started for less work than starting it costs.
const MATVEC_GRAIN: usize = 1 << 20;

/// `y_b = W·x_b` for each `x_b`, `W` the `out_f` rows of `ty` bytes from byte `from` of `raw`.
struct Product {
    raw: Arc<Vec<u8>>,
    from: usize,
    ty: GgmlType,
    in_f: usize,
    out_f: usize,
    xs: Vec<Vec<f64>>,
}

/// Every product's `y_b`, the products' rows split across threads together as the work warrants: each thread takes a
/// run of the rows of the products in turn, decodes them one at a time into one reused row and contracts it with every
/// `x_b` of its product, so no more than a row of a weight is ever decoded at once. A row's sums are its own, however
/// the rows are split.
fn products(ps: &[Product], threads: usize) -> Vec<Vec<Vec<f64>>> {
    let work = |p: &Product| p.in_f * p.xs.len().max(1);
    let total: usize = ps.iter().map(|p| p.out_f * work(p)).sum();
    let tasks = threads.min(total.div_ceil(MATVEC_GRAIN)).max(1);
    // Each task's runs of rows, `(product, first, rows)`, cut so the tasks' work is even.
    let share = total.div_ceil(tasks);
    let mut runs: Vec<Vec<(usize, usize, usize)>> = vec![Vec::new()];
    let mut load = 0;
    for (i, p) in ps.iter().enumerate() {
        let mut first = 0;
        while first < p.out_f {
            if load >= share && runs.len() < tasks {
                runs.push(Vec::new());
                load = 0;
            }
            let rows = if runs.len() == tasks {
                p.out_f - first
            } else {
                (share - load.min(share)).div_ceil(work(p)).clamp(1, p.out_f - first)
            };
            runs.last_mut().expect("a task").push((i, first, rows));
            load += rows * work(p);
            first += rows;
        }
    }
    let run = |task: &[(usize, usize, usize)]| -> Vec<(usize, usize, Vec<Vec<f64>>)> {
        task.iter()
            .map(|&(i, first, rows)| {
                let p = &ps[i];
                let row_bytes = p.ty.bytes_for(p.in_f);
                let mut w: Vec<f32> = Vec::with_capacity(p.in_f);
                let mut ys: Vec<Vec<f64>> = p.xs.iter().map(|_| Vec::with_capacity(rows)).collect();
                for r in first..first + rows {
                    let at = p.from + r * row_bytes;
                    w.clear();
                    p.ty.dequantize(&p.raw[at..at + row_bytes], &mut w);
                    for (y, x) in ys.iter_mut().zip(&p.xs) {
                        y.push(w.iter().zip(x).map(|(&a, b)| a as f64 * b).sum());
                    }
                }
                (i, first, ys)
            })
            .collect()
    };
    let parts: Vec<(usize, usize, Vec<Vec<f64>>)> = if runs.len() == 1 {
        run(&runs[0])
    } else {
        std::thread::scope(|s| {
            let handles: Vec<_> = runs.iter().map(|task| s.spawn(move || run(task))).collect();
            handles.into_iter().flat_map(|h| h.join().expect("product thread")).collect()
        })
    };
    let mut out: Vec<Vec<Vec<f64>>> = ps.iter().map(|p| vec![vec![0f64; p.out_f]; p.xs.len()]).collect();
    for (i, first, ys) in parts {
        for (col, y) in out[i].iter_mut().zip(ys) {
            col[first..first + y.len()].copy_from_slice(&y);
        }
    }
    out
}

fn add_rows(x: &mut [Vec<f64>], y: &[Vec<f64>]) {
    for (a, b) in x.iter_mut().zip(y) {
        for (u, v) in a.iter_mut().zip(b) {
            *u += v;
        }
    }
}

/// Each lane's share of a sublayer's output joins it: `h[s·d + j] += out[j]·share[s]`.
fn inject_lanes(h: &mut [Vec<f64>], out: &[Vec<f64>], shares: &[Vec<f64>], d: usize) {
    for ((row, o), share) in h.iter_mut().zip(out).zip(shares) {
        for (s, &w) in share.iter().enumerate() {
            for j in 0..d {
                row[s * d + j] += o[j] * w;
            }
        }
    }
}

/// An RMSNorm of each `d`-wide lane of `x` on its own, by one weight as wide as every lane.
fn lane_norm(x: &[f64], w: &[f64], d: usize, eps: f64) -> Vec<f64> {
    x.chunks(d).zip(w.chunks(d)).flat_map(|(lane, w)| rms_norm(lane, w, eps)).collect()
}

/// The table rows of the n-grams ending at the last of `w` (the last `ngram` tokens, oldest first): for each order `n`
/// from 2, `t₀·m₀ ⊕ t₁·m₁ ⊕ … ⊕ t₍ₙ₋₁₎·m₍ₙ₋₁₎` (`t_k` the token `k` back, the end of document where an end of
/// document lies between, in two's-complement 64-bit arithmetic as the reference implementation's `long`s) modulo each
/// of its heads' primes, from the head's offset.
fn ngram_rows(p: &super::spec::Ple, w: &[u32]) -> Vec<u64> {
    let last = w.len() - 1;
    let back = |k: usize| -> i64 {
        if w[last - k..last].contains(&p.eos) {
            i64::from(p.eos)
        } else {
            i64::from(w[last - k])
        }
    };
    let mut rows = Vec::with_capacity((p.ngram - 1) * p.heads_per_ngram);
    for n in 2..=p.ngram {
        let mixed = (0..n).fold(0i64, |acc, k| {
            let t = if k == 0 { i64::from(w[last]) } else { back(k) };
            acc ^ t.wrapping_mul(p.multipliers[k] as i64)
        });
        for j in 0..p.heads_per_ngram {
            let head = (n - 2) * p.heads_per_ngram + j;
            rows.push(mixed.rem_euclid(p.head_vocab[head] as i64) as u64 + p.head_offsets[head]);
        }
    }
    rows
}

/// The positions a query at `pos` attends under QSA: every visible one while the complete blocks of `ix.block` fit the
/// budget; past it the best `budget / block` blocks by `Σ_h relu(q_h·k̄) / √head_dim` — a block's index keys averaged,
/// normed and rotated at its first position — the lower block first between equal scores, and every position after the
/// last complete block.
#[allow(clippy::too_many_arguments)]
fn qsa_positions(
    ix: &Indexer,
    queries: &[Vec<f64>],
    keys: &[Vec<f64>],
    norm: &[f64],
    rope: Option<&Rope>,
    factors: Option<&[f64]>,
    eps: f64,
    pos: usize,
) -> Vec<usize> {
    let blocks = (pos + 1) / ix.block;
    let keep = ix.budget / ix.block;
    if blocks <= keep {
        return (0..=pos).collect();
    }
    let mut scored: Vec<(f64, usize)> = (0..blocks)
        .map(|b| {
            let first = b * ix.block;
            let mean: Vec<f64> = (0..ix.head_dim)
                .map(|e| keys[first..first + ix.block].iter().map(|k| k[e]).sum::<f64>() / ix.block as f64)
                .collect();
            let mut k = rms_norm(&mean, norm, eps);
            if let Some(r) = rope {
                rotate(&mut k, first, r, factors);
            }
            let score = queries.iter().map(|q| q.iter().zip(&k).map(|(a, b)| a * b).sum::<f64>().max(0.0)).sum::<f64>()
                / (ix.head_dim as f64).sqrt();
            (score, b)
        })
        .collect();
    scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
    let mut chosen: Vec<usize> = scored[..keep].iter().map(|&(_, b)| b).collect();
    chosen.sort_unstable();
    chosen.into_iter().flat_map(|b| b * ix.block..(b + 1) * ix.block).chain(blocks * ix.block..=pos).collect()
}

fn add_bias(rows: &mut [Vec<f64>], b: &[f64]) {
    for r in rows {
        r.iter_mut().zip(b).for_each(|(v, b)| *v += b);
    }
}

fn rms_norm(x: &[f64], w: &[f64], eps: f64) -> Vec<f64> {
    let ms = x.iter().map(|v| v * v).sum::<f64>() / x.len() as f64;
    let scale = 1.0 / (ms + eps).sqrt();
    x.iter().zip(w).map(|(v, w)| v * scale * w).collect()
}

fn softmax(xs: &mut [f64]) {
    let m = xs.iter().cloned().fold(f64::NEG_INFINITY, f64::max);
    let mut sum = 0.0;
    for v in xs.iter_mut() {
        *v = (*v - m).exp();
        sum += *v;
    }
    for v in xs.iter_mut() {
        *v /= sum;
    }
}

fn sigmoid(x: f64) -> f64 {
    1.0 / (1.0 + (-x).exp())
}

fn silu(x: f64) -> f64 {
    x * sigmoid(x)
}

/// `ln(1 + eˣ)`, without overflowing for large `x`.
fn softplus(x: f64) -> f64 {
    if x > 0.0 {
        x + (-x).exp().ln_1p()
    } else {
        x.exp().ln_1p()
    }
}

fn gated(act: &Act, g: &[f64], u: &[f64]) -> Vec<f64> {
    g.iter()
        .zip(u)
        .map(|(&g, &u)| match act {
            Act::Silu => g / (1.0 + (-g).exp()) * u,
            Act::GeluTanh => {
                let c = (2.0 / std::f64::consts::PI).sqrt();
                0.5 * g * (1.0 + (c * (g + 0.044715 * g * g * g)).tanh()) * u
            }
            Act::SwigluOai { alpha, limit } => {
                let gc = g.min(*limit);
                let uc = u.clamp(-limit, *limit);
                gc / (1.0 + (-alpha * gc).exp()) * (uc + 1.0)
            }
        })
        .collect()
}

/// Rotate one head vector at position `pos`: the first `rope.dims` dimensions in pairs (NEOX: `i` with
/// `i + dims/2`; otherwise `2i` with `2i+1`) by [`Rope::rotation`], the rest untouched.
fn rotate(v: &mut [f64], pos: usize, rope: &Rope, factors: Option<&[f64]>) {
    let half = rope.dims / 2;
    for i in 0..half {
        let (c, s) = rope.rotation(i, pos, factors.map(|f| f[i]));
        let (a, b) = if rope.neox { (i, i + half) } else { (2 * i, 2 * i + 1) };
        let (x0, x1) = (v[a], v[b]);
        v[a] = x0 * c - x1 * s;
        v[b] = x0 * s + x1 * c;
    }
}

/// The index of the largest logit (the first of equals).
pub fn argmax(logits: &[f64]) -> u32 {
    let mut best = 0;
    for (i, &v) in logits.iter().enumerate() {
        if v > logits[best] {
            best = i;
        }
    }
    best as u32
}
