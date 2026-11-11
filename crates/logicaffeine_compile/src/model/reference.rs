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
use super::quant::{e4m3_to_f32, f16_to_f32, f32_to_e4m3, f32_to_f16, GgmlType, E4M3_MAX};
use crate::tensor_graph::numerics::Keep;
use super::spec::{
    Act, Attention, Dsa, Ffn, GateActivation, GatedDelta, Gating, HyperConnections, Indexer, Layer, LogitScale, Mixer, Mla,
    DeltaAttention, Lightning, Moe, ModelSpec, Norm, Part, Recurrent, Rope, RopeScaling, Rwkv7, Selective, ShortConv, Ssd,
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
    /// A short convolution's state: the last `taps − 1` gated inputs, oldest first.
    Conv { rows: Vec<Vec<f64>> },
    /// A state-space scan's: the last `taps − 1` convolution inputs, oldest first, and each channel's state.
    Scan { conv: Vec<Vec<f64>>, state: Vec<f64> },
    /// A block of attention beside a scan (Falcon-H1's): the attention's cache and the scan's state.
    Parallel { attention: Box<LayerState>, scan: Box<LayerState> },
    /// An RWKV-7 block's: the previous position's normed inputs to its time mix and to its channel mix, and each head's
    /// `head_size × head_size` state (row = value dimension, column = key dimension).
    Rwkv { att_shift: Vec<f64>, ffn_shift: Vec<f64>, state: Vec<f64> },
    /// A block that carries nothing from position to position.
    Stateless,
}

impl LayerState {
    fn cache() -> LayerState {
        LayerState::Attention { k: Vec::new(), v: Vec::new(), ik: Vec::new() }
    }

    fn scan(s: &Ssd) -> LayerState {
        LayerState::Scan {
            conv: vec![vec![0.0; s.heads * s.head_dim + 2 * s.groups * s.state]; s.conv_taps.saturating_sub(1)],
            state: vec![0.0; s.heads * s.head_dim * s.state],
        }
    }

    fn fresh(layer: &Layer) -> LayerState {
        match &layer.mixer {
            Mixer::Attention(_) | Mixer::Mla(_) => LayerState::cache(),
            Mixer::GatedDelta(g) => {
                let channels = 2 * g.k_heads * g.head_dim + g.v_heads * g.head_dim;
                LayerState::Delta {
                    conv: vec![vec![0.0; channels]; g.conv_taps.saturating_sub(1)],
                    memory: vec![0.0; g.v_heads * g.head_dim * g.head_dim],
                }
            }
            Mixer::Recurrent(Recurrent::ShortConv(c)) => LayerState::Conv { rows: vec![Vec::new(); c.taps - 1] },
            Mixer::Recurrent(Recurrent::Ssd(s)) => LayerState::scan(s),
            Mixer::Recurrent(Recurrent::DeltaAttention(d)) => LayerState::Scan {
                conv: vec![vec![0.0; 3 * d.heads * d.head_dim]; d.conv_taps.saturating_sub(1)],
                state: vec![0.0; d.heads * d.head_dim * d.head_dim],
            },
            Mixer::Recurrent(Recurrent::Selective(s)) => LayerState::Scan {
                conv: vec![vec![0.0; s.inner]; s.conv_taps.saturating_sub(1)],
                state: vec![0.0; s.inner * s.state],
            },
            Mixer::Recurrent(Recurrent::Lightning(x)) => LayerState::Scan { conv: Vec::new(), state: vec![0.0; x.heads * x.head_dim * x.head_dim] },
            Mixer::Recurrent(Recurrent::Rwkv7(_)) => LayerState::Rwkv { att_shift: Vec::new(), ffn_shift: Vec::new(), state: Vec::new() },
            Mixer::Recurrent(Recurrent::Parallel(_, s)) => {
                LayerState::Parallel { attention: Box::new(LayerState::cache()), scan: Box::new(LayerState::scan(s)) }
            }
            Mixer::None => LayerState::Stateless,
        }
    }

    fn cached(&self) -> (&[Vec<f64>], &[Vec<f64>]) {
        match self {
            LayerState::Attention { k, v, .. } => (k, v),
            LayerState::Parallel { attention, .. } => attention.cached(),
            _ => (&[], &[]),
        }
    }

    fn memory(&self) -> &[f64] {
        match self {
            LayerState::Delta { memory, .. } | LayerState::Scan { state: memory, .. } | LayerState::Rwkv { state: memory, .. } => memory,
            LayerState::Parallel { scan, .. } => scan.memory(),
            _ => &[],
        }
    }

    fn conv(&self) -> &[Vec<f64>] {
        match self {
            LayerState::Delta { conv, .. } | LayerState::Conv { rows: conv } | LayerState::Scan { conv, .. } => conv,
            LayerState::Parallel { scan, .. } => scan.conv(),
            LayerState::Attention { .. } | LayerState::Rwkv { .. } | LayerState::Stateless => &[],
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
    /// rounded to the nearest, ties to even; read by a 16-bit float matrix, one word of the matrix's own format, rounded
    /// to the nearest from `f32`, ties to even ([`sixteen_bit_activations`]).
    Int8,
    /// The high word as `Int8` (the scale's floor 1e-30, clamped to −128..127) and a low int8 word holding the
    /// remainder `v − hi·s` at a 128th of the scale; read by a 16-bit float matrix, two words of its format, the second
    /// the rest the first leaves in `f32`.
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
    /// Each element rounded to `f32` and then to fp8 e4m3, saturating at ±448, as vLLM's `fp8_e4m3` cache stores the
    /// `f32` rows it computes at a scale of one; every read of the cache takes the rounded row.
    E4m3,
    /// Each head's row — a key's, a value's, a latent's whole row — as int8 codes under one binary16 scale, kept as
    /// `Keep::Int8Rows` keeps it (`Keep::keep_row`); every read of the cache takes the kept row.
    Int8Rows,
    /// [`KvStorage::Int8Rows`] of each head's row turned first by the Walsh–Hadamard transform and turned back as read,
    /// as `Keep::Int8RowsRotated` keeps it — the D5005 engine's cache.
    Int8RowsRotated,
}

impl KvStorage {
    /// `v` as the cache keeps it — to the nearest, or (`flipped`) to its other neighbour — how far its `f32` value lies
    /// from the half-way point between the two (infinitely far for a value the format holds exactly or saturates), and
    /// how far apart the neighbours are.
    fn round(self, v: f64, flipped: bool) -> (f64, f64, f64) {
        match self {
            KvStorage::Exact | KvStorage::Int8Rows | KvStorage::Int8RowsRotated => (v, f64::INFINITY, 0.0),
            KvStorage::F16 => binary16(v, flipped),
            KvStorage::E4m3 => e4m3(v, flipped),
        }
    }

    /// The way a cache in int8 rows keeps a head's row; `None` for a cache that keeps each value alone.
    fn rows(self) -> Option<Keep> {
        match self {
            KvStorage::Int8Rows => Some(Keep::Int8Rows),
            KvStorage::Int8RowsRotated => Some(Keep::Int8RowsRotated),
            _ => None,
        }
    }
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

/// The name a near call gives an activation: the FNV-1a hash of its values' `f32` bits.
fn activation_key(x: &[f64]) -> u64 {
    x.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, &a| {
        (a as f32).to_bits().to_le_bytes().iter().fold(h, |h, &b| (h ^ u64::from(b)).wrapping_mul(0x0100_0000_01b3))
    })
}

/// [`binary16`] for bfloat16.
fn bfloat16(v: f64, flipped: bool) -> (f64, f64, f64) {
    let f = v as f32;
    if f.is_nan() {
        return (f64::from(f), f64::INFINITY, 0.0);
    }
    let bits = f.to_bits();
    let h = (bits.wrapping_add(0x7FFF + ((bits >> 16) & 1)) >> 16) as u16;
    let near = f32::from_bits(u32::from(h) << 16);
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
    let far = f32::from_bits(u32::from(other) << 16);
    let (near, far, f) = (f64::from(near), f64::from(far), f64::from(f));
    (if flipped { far } else { near }, (f - (near + far) / 2.0).abs(), (far - near).abs())
}

/// `x` as a projection by a matrix in the 16-bit float format `ty` (BF16 or F16) reads it under `mode`: as computed under
/// the exact precision; each value rounded to `f32` and then to one word of `ty`, to the nearest, ties to even, under
/// the one-word precision — llama.cpp's precision for a 16-bit matrix; and that word and a second under the two-word
/// precision, the rest the first leaves in `f32` rounded so too. These are the words the lane's 16-bit tensor cores
/// multiply (`lk_h16_words`).
pub fn sixteen_bit_activations(x: &[f64], ty: GgmlType, mode: Activations) -> Vec<f64> {
    x.iter().map(|&v| sixteen_bit(v, ty, mode, false).0).collect()
}

/// One value as [`sixteen_bit_activations`] reads it, its first word taken to the other neighbour where `flipped`, with
/// how far its `f32` value lies from that word's half-way point and how far apart the word's neighbours lie (infinitely
/// far, and none, where no one word decides it).
fn sixteen_bit(v: f64, ty: GgmlType, mode: Activations, flipped: bool) -> (f64, f64, f64) {
    let word = |v: f64, flipped: bool| if ty == GgmlType::Bf16 { bfloat16(v, flipped) } else { binary16(v, flipped) };
    match mode {
        Activations::Exact => (v, f64::INFINITY, 0.0),
        Activations::Int8 => word(v, flipped),
        Activations::Int8TwoPass => {
            let (hi, ..) = word(v, false);
            let (lo, ..) = word(f64::from(v as f32 - hi as f32), false);
            (hi + lo, f64::INFINITY, 0.0)
        }
    }
}

/// [`binary16`] for fp8 e4m3: a value past ±448 saturates exactly.
fn e4m3(v: f64, flipped: bool) -> (f64, f64, f64) {
    let f = v as f32;
    let b = f32_to_e4m3(f);
    let near = e4m3_to_f32(b);
    if near == f || f.is_nan() || f.abs() >= E4M3_MAX {
        return (f64::from(near), f64::INFINITY, 0.0);
    }
    // The neighbour the other side of `f`: a step up in value or down, the sign bit deciding which way the code goes.
    let up = near < f;
    let other = match (b & 0x80 != 0, b & 0x7f, up) {
        (false, _, true) | (true, _, false) => b + 1,
        (false, 0, false) => 0x81,
        (true, 0, true) => 0x01,
        _ => b - 1,
    };
    let far = e4m3_to_f32(other);
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

/// The word a cached value's binary16 rounding is recorded under ([`RoundingCall::word`]), and a cached int8 row's code.
pub const CACHE_WORD: u8 = 2;

/// The word a cached int8 row's scale is recorded under ([`RoundingCall::word`]), its element the row's head.
pub const CACHE_SCALE_WORD: u8 = 3;

/// The words a 16-bit product's one-word rounding is recorded under ([`RoundingCall::word`]): an activation's value as a
/// product by a bfloat16 matrix reads it, and as one by a binary16 matrix reads it.
pub const BF16_WORD: u8 = 3;
pub const F16_WORD: u8 = 4;

/// A rounding the reference made within [`NEAR_ROUNDING`] of a half-way point: value `element` of the activation whose
/// `f32` values hash to `activation` (`word` 0 the int8 or two-pass high word, 1 the low word, [`CACHE_WORD`] a cached
/// row's binary16, [`BF16_WORD`] and [`F16_WORD`] a 16-bit product's one word), first made in the step ending at
/// `position`, `margin` levels from the half-way point (a cached value's and a 16-bit word's in levels of its row's
/// scale, its largest magnitude over 127). An activation is one decision however many projections read it — the lane
/// quantizes it once, and rounds the same `f32` value alike in every product — so a call names the activation, not the
/// read.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RoundingCall {
    pub activation: u64,
    pub element: usize,
    pub word: u8,
    pub position: usize,
    pub margin: f64,
    /// How far the other outcome moves the value, in the levels `margin` is measured in: one for an int8 word, a
    /// cached value's binary16 step for its rounding, a 16-bit word's own step for its.
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
    /// The positions the last full DSA indexer chose for each query of the step running, which a shared layer takes.
    dsa_chosen: Vec<Vec<usize>>,
    /// RWKV-7's first block's value at each position of the step running, which every later block mixes with.
    v_first: Vec<Vec<f64>>,
}

impl Reference {
    pub fn new(spec: ModelSpec, g: impl WeightSource + 'static) -> R<Reference> {
        let altup = spec.parts.iter().any(|p| matches!(p, Part::AltUp { .. }));
        if let Some(part) = spec.parts.iter().find(|p| match p {
            Part::AltUp { .. } | Part::SharedKv { .. } | Part::ValueNorm | Part::GateSparsity { .. } | Part::KeyAsValue { .. } => false,
            Part::PerLayerInput { .. } => spec.hyper.is_some(),
            Part::Laurel { .. } => !altup,
            Part::NormedResidual { .. } | Part::EmbeddingNorm | Part::LayerScale => altup || spec.hyper.is_some(),
        }) {
            return Err(ReferenceError::NotYet(part.what()));
        }
        for layer in &spec.layers {
            let rwkv = matches!(layer.mixer, Mixer::Recurrent(Recurrent::Rwkv7(_)));
            match &layer.ffn {
                // The channel mix's token shift is kept beside its block's time mix.
                Ffn::ChannelMix { .. } if !rwkv => return Err(ReferenceError::NotYet("RWKV-7's channel mix after another mixer")),
                _ => {}
            }
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
            dsa_chosen: Vec::new(),
            v_first: Vec::new(),
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
        self.states[l].cached()
    }

    /// Layer `l`'s recurrent memory: a gated delta's, a scan's state (empty for a layer that keeps none).
    pub fn memory(&self, l: usize) -> &[f64] {
        self.states[l].memory()
    }

    /// Layer `l`'s convolution ring: its last `taps − 1` inputs, oldest first (empty for a layer that convolves
    /// nothing).
    pub fn conv(&self, l: usize) -> &[Vec<f64>] {
        self.states[l].conv()
    }

    /// The per-layer embedding's state: every token so far after its `ngram − 1` leading ends of document, and the last
    /// `(taps − 1)·ngram` normed gated values its dilated convolution reads, oldest first (both empty without one).
    pub fn ple_state(&self) -> (&[u32], &[Vec<f64>]) {
        (&self.ple_tokens, &self.ple_conv)
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
        if self.spec.parts.contains(&Part::EmbeddingNorm) {
            x = self.block_norm(&x, "token_embd_norm")?;
        }
        if let Some(hc) = self.spec.hyper {
            let lanes = self.hyper_layers(tokens, &x, start, hc)?;
            self.pos += tokens.len();
            return Ok(lanes);
        }
        if let Some((streams, active)) = self.spec.parts.iter().find_map(|p| match p {
            Part::AltUp { streams, active } => Some((*streams, *active)),
            _ => None,
        }) {
            let merged = self.altup_layers(tokens, &x, start, streams, active)?;
            self.pos += tokens.len();
            return Ok(merged);
        }
        // Gemma 4's per-layer inputs, each block's slice joining the stream after its FFN.
        let per_layer = match self.spec.parts.iter().find_map(|p| if let Part::PerLayerInput { dim } = p { Some(*dim) } else { None }) {
            Some(dim) => Some((dim, self.per_layer_inputs(tokens, &x, dim)?)),
            None => None,
        };
        let layer_scale = self.spec.parts.contains(&Part::LayerScale);
        // MiniMax-01's sublayers add their outputs to their normed inputs times the scale, in place of their inputs.
        let normed_residual = self.spec.parts.iter().find_map(|p| if let Part::NormedResidual { scale } = p { Some(*scale) } else { None });
        let residual = |x: &mut Vec<Vec<f64>>, xn: Vec<Vec<f64>>, out: &[Vec<f64>]| match normed_residual {
            Some(scale) => {
                *x = xn;
                scale_rows(x, scale);
                add_rows(x, out);
            }
            None => add_rows(x, out),
        };
        for l in 0..self.spec.layers.len() {
            let layer = self.spec.layers[l].clone();
            // A block with no mixer is its FFN alone, normed by the block's one norm; one with no FFN its mixer alone. A
            // block that norms nothing before its sublayers (OLMo 2's) reads the residual stream itself.
            let ffn_norm = if layer.mixer == Mixer::None { "attn_norm" } else { self.spec.ffn_norm };
            if layer.mixer != Mixer::None {
                let xn = if layer.pre_norms { self.block_norm(&x, &format!("blk.{l}.attn_norm"))? } else { x.clone() };
                let mut mixed = self.mixer(l, &layer.mixer, &xn, start)?;
                if layer.post_mixer_norm {
                    mixed = self.block_norm(&mixed, &format!("blk.{l}.post_attention_norm"))?;
                }
                if let Some(r) = self.spec.residual_scale {
                    scale_rows(&mut mixed, r);
                }
                residual(&mut x, xn, &mixed);
            }
            if layer.ffn == Ffn::None {
                continue;
            }
            let xn2 = if layer.pre_norms { self.block_norm(&x, &format!("blk.{l}.{ffn_norm}"))? } else { x.clone() };
            let mut ffn = match &layer.ffn {
                Ffn::DenseAndMoe { dff: _, act, moe } => self.dense_and_moe(l, act, moe, &x, &xn2)?,
                _ => self.ffn(l, &layer, &xn2)?,
            };
            if layer.post_ffn_norm {
                ffn = self.block_norm(&ffn, &format!("blk.{l}.post_ffw_norm"))?;
            }
            if let Some(r) = self.spec.residual_scale {
                scale_rows(&mut ffn, r);
            }
            residual(&mut x, xn2, &ffn);
            if let Some((dim, inputs)) = &per_layer {
                let blk = |t: &str| format!("blk.{l}.{t}");
                let mut gate = self.project(&blk("inp_gate.weight"), &x)?;
                for (row, input) in gate.iter_mut().zip(inputs) {
                    row.iter_mut().zip(&input[l * dim..(l + 1) * dim]).for_each(|(g, p)| *g = gelu_tanh(*g) * p);
                }
                let joined = self.project(&blk("proj.weight"), &gate)?;
                let joined = self.block_norm(&joined, &blk("post_norm"))?;
                add_rows(&mut x, &joined);
            }
            if layer_scale {
                let s = self.tensor(&format!("blk.{l}.layer_output_scale.weight"))?[0];
                scale_rows(&mut x, s);
            }
        }
        self.pos += tokens.len();
        Ok(x)
    }

    /// The logits of each final hidden state: the output norm, the LM head, the model's logit scale on whichever side
    /// of the head its graph applies it, the final softcap.
    fn head(&mut self, x: &[Vec<f64>]) -> R<Vec<Vec<f64>>> {
        let mut xf = match self.spec.hyper {
            // Hyper-connections end in a last mix of the lanes, in place of the output norm.
            Some(hc) => self.mix_lanes(x, "output_hc", hc, false)?.0,
            None => self.block_norm(x, "output_norm")?,
        };
        if let Some(LogitScale::BeforeHead(f)) = self.spec.logit_scale {
            scale_rows(&mut xf, f);
        }
        let head = if self.spec.tied_output {
            "token_embd.weight"
        } else {
            "output.weight"
        };
        let mut logits = self.project(head, &xf)?;
        if self.spec.head_bias {
            let b = self.tensor("output.bias")?;
            add_bias(&mut logits, &b);
        }
        if let Some(LogitScale::AfterHead(f)) = self.spec.logit_scale {
            scale_rows(&mut logits, f);
        }
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
        let eps = self.spec.rms_eps;
        self.norm_rows_at(x, weight, eps)
    }

    /// [`Self::norm_rows`] at `eps` in place of the model's epsilon.
    fn norm_rows_at(&mut self, x: &[Vec<f64>], weight: &str, eps: f64) -> R<Vec<Vec<f64>>> {
        let w = self.tensor(weight)?;
        Ok(x.iter().map(|r| rms_norm(r, &w, eps)).collect())
    }

    /// The model's block norm `stem` over every row of `x`, as the spec's [`Norm`] says: an RMSNorm by `<stem>.weight`,
    /// or a LayerNorm by it plus `<stem>.bias` where the norm is biased.
    fn block_norm(&mut self, x: &[Vec<f64>], stem: &str) -> R<Vec<Vec<f64>>> {
        let w = self.tensor(&format!("{stem}.weight"))?;
        let eps = self.spec.rms_eps;
        Ok(match self.spec.norm {
            Norm::Rms => x.iter().map(|r| rms_norm(r, &w, eps)).collect(),
            Norm::Layer { bias } => {
                let b = if bias { Some(self.tensor(&format!("{stem}.bias"))?) } else { None };
                x.iter().map(|r| layer_norm(r, &w, b.as_deref(), eps)).collect()
            }
        })
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

    /// The activations a projection by weights of type `ty` reads: as computed by `f32` weights, in the matrix's own words
    /// by 16-bit float weights, quantized by quantized ones.
    fn read_by(&mut self, ty: GgmlType, xs: &[Vec<f64>]) -> Vec<Vec<f64>> {
        match ty {
            GgmlType::F32 => xs.to_vec(),
            GgmlType::F16 | GgmlType::Bf16 => xs.iter().map(|x| self.sixteen_row(x, ty)).collect(),
            _ => xs.iter().map(|x| self.quantize_row(x)).collect(),
        }
    }

    /// `x` as a product by a matrix in the 16-bit float format `ty` reads it ([`sixteen_bit_activations`]): under the
    /// one-word precision each rounding within [`NEAR_ROUNDING`] of a half-way point is recorded once an activation
    /// (word [`BF16_WORD`] or [`F16_WORD`]), its margin in levels of the row's scale as a cached value's is, and a
    /// flipped one takes the other neighbour. Two words make no call: a first word rounded the other way leaves a rest
    /// the second takes, so the pair keeps sixteen bits of the value either way.
    fn sixteen_row(&mut self, x: &[f64], ty: GgmlType) -> Vec<f64> {
        if self.activations != Activations::Int8 {
            return sixteen_bit_activations(x, ty, self.activations);
        }
        let word = if ty == GgmlType::Bf16 { BF16_WORD } else { F16_WORD };
        let activation = activation_key(x);
        let position = self.at;
        let level = x.iter().fold(0f64, |m, &v| m.max(f64::from((v as f32).abs()))) / 127.0;
        let mut out = Vec::with_capacity(x.len());
        for (element, &v) in x.iter().enumerate() {
            let key = (activation, element, word);
            let flipped = !self.flipped_roundings.is_empty() && self.flipped_roundings.contains(&key);
            let (kept, distance, apart) = sixteen_bit(v, ty, Activations::Int8, flipped);
            let margin = distance / level;
            if margin <= NEAR_ROUNDING && self.near_roundings.insert(key) {
                self.near.push(NearCall::Rounding(RoundingCall { activation, element, word, position, margin, step: apart / level }));
            }
            out.push(kept);
        }
        out
    }

    /// `row` as the cache keeps it: as computed, or each value rounded to `f32` and then to the cache's narrower float
    /// (binary16, fp8 e4m3), its near roundings recorded (once a row, word [`CACHE_WORD`]) and the flipped ones taken to
    /// the other neighbour. A value's margin is its distance from the half-way point in levels of its row's scale — the
    /// row's largest magnitude over 127, the yardstick an int8 rounding of the row is measured by — since a run's `f32`
    /// arithmetic errs by a share of the scale it computes at, whatever the value: a small value its rotation left after
    /// cancelling errs by more of its own step than a large one.
    ///
    /// A cache in int8 rows keeps each head's `unit` values as [`Keep::keep_row_deciding`] keeps them, its calls each
    /// head's scale and each of its codes: a code's margin its value over the scale's distance from the half-way point,
    /// in levels; the scale's its `f32` value's distance from its binary16 half-way point in 127ths of itself — a run's
    /// arithmetic errs by a share of a level in the head's largest value, and the scale is that value's 127th. A turned
    /// head's width must be a power of two.
    fn kept(&mut self, row: Vec<f64>, unit: usize) -> R<Vec<f64>> {
        if self.kv == KvStorage::Exact {
            return Ok(row);
        }
        let activation = activation_key(&row);
        let position = self.at;
        if let Some(keep) = self.kv.rows() {
            let flipped = &self.flipped_roundings;
            let seen = std::cell::RefCell::new(&mut self.near_roundings);
            let near = std::cell::RefCell::new(&mut self.near);
            let mut out = Vec::with_capacity(row.len());
            for (head, part) in row.chunks(unit).enumerate() {
                let call = |element: usize, word: u8, margin: f64, step: f64| {
                    if margin <= NEAR_ROUNDING && seen.borrow_mut().insert((activation, element, word)) {
                        near.borrow_mut().push(NearCall::Rounding(RoundingCall { activation, element, word, position, margin, step }));
                    }
                    !flipped.is_empty() && flipped.contains(&(activation, element, word))
                };
                let mut scale = |q: f32| {
                    let tick = f64::from(q) / 127.0;
                    let (_, distance, apart) = binary16(f64::from(q), false);
                    let other = tick > 0.0 && call(head, CACHE_SCALE_WORD, distance / tick, apart / tick);
                    binary16(f64::from(q), other).0
                };
                let mut other = |i: usize, r: f64| r.abs() < 127.0 && call(head * unit + i, CACHE_WORD, ((r - r.floor()) - 0.5).abs(), 1.0);
                let kept = keep
                    .keep_row_deciding(part, &mut scale, &mut other)
                    .ok_or(ReferenceError::NotYet("a turned cache row whose width is not a power of two"))?;
                out.extend(kept);
            }
            return Ok(out);
        }
        let level = row.iter().fold(0f64, |m, &v| m.max(f64::from((v as f32).abs()))) / 127.0;
        let mut out = Vec::with_capacity(row.len());
        for (element, &v) in row.iter().enumerate() {
            let key = (activation, element, CACHE_WORD);
            let (kept, distance, apart) = self.kv.round(v, !self.flipped_roundings.is_empty() && self.flipped_roundings.contains(&key));
            let margin = distance / level;
            if margin <= NEAR_ROUNDING && self.near_roundings.insert(key) {
                let step = apart / level;
                self.near.push(NearCall::Rounding(RoundingCall { activation, element, word: CACHE_WORD, position, margin, step }));
            }
            out.push(kept);
        }
        Ok(out)
    }

    /// One row quantized as [`quantize_activations`] does, its near roundings recorded (once an activation) and the
    /// flipped ones taken the other way.
    fn quantize_row(&mut self, x: &[f64]) -> Vec<f64> {
        if self.activations == Activations::Exact {
            return x.to_vec();
        }
        let activation = activation_key(x);
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
        // A layer that attends an earlier layer's cache projects no keys or values of its own.
        let shared = self.shared_cache(l, a);
        let (mut k, mut v) = match (shared, part(&fused, aq, akv), part(&fused, aq + akv, avd)) {
            (Some(_), _, _) => (Vec::new(), Vec::new()),
            (None, Some(k), Some(v)) => (k, v),
            _ => {
                let k = self.project(&format!("blk.{l}.attn_k.weight"), xn)?;
                // Gemma 4's `attention_k_eq_v` layers read their keys, as projected, as their values.
                let keyed = self.spec.parts.iter().any(|p| matches!(p, Part::KeyAsValue { layers } if layers.contains(&l)));
                let v = if keyed { k.clone() } else { self.project(&format!("blk.{l}.attn_v.weight"), xn)? };
                (k, v)
            }
        };
        if a.bias && !a.fused_qkv {
            for (rows, which) in [(&mut q, "attn_q"), (&mut k, "attn_k"), (&mut v, "attn_v")] {
                if rows.is_empty() {
                    continue;
                }
                let b = self.tensor(&format!("blk.{l}.{which}.bias"))?;
                add_bias(rows, &b);
            }
        }
        if self.spec.parts.contains(&Part::ValueNorm) {
            let ones = vec![1.0; vd];
            for row in &mut v {
                for h in 0..a.kv_heads {
                    let n = rms_norm(&row[h * vd..(h + 1) * vd], &ones, self.spec.rms_eps);
                    row[h * vd..(h + 1) * vd].copy_from_slice(&n);
                }
            }
        }
        let (qn, kn) = if a.qk_norm {
            let qw = self.tensor(&format!("blk.{l}.attn_q_norm.weight"))?;
            // A layer attending an earlier layer's cache has no keys of its own to norm.
            let kw = if shared.is_none() { Some(self.tensor(&format!("blk.{l}.attn_k_norm.weight"))?) } else { None };
            (Some(qw), kw)
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
        for (i, qr) in q.iter_mut().enumerate() {
            let pos = start + i;
            if let Some(qw) = &whole.0 {
                *qr = rms_norm(qr, qw, self.spec.rms_eps);
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
        }
        for (i, kr) in k.iter_mut().enumerate() {
            let pos = start + i;
            if let Some(kw) = &whole.1 {
                *kr = rms_norm(kr, kw, self.spec.rms_eps);
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
        let k: Vec<Vec<f64>> = k.into_iter().map(|r| self.kept(r, hd)).collect::<R<_>>()?;
        let v: Vec<Vec<f64>> = v.into_iter().map(|r| self.kept(r, vd)).collect::<R<_>>()?;
        let eps = self.spec.rms_eps;
        let LayerState::Attention { k: kc, v: vc, ik: ikc } = &mut self.states[l] else {
            unreachable!("an attention layer holds an attention cache")
        };
        kc.extend(k);
        vc.extend(v);
        if let Some((_, _, keys, _)) = &index {
            ikc.extend(keys.iter().cloned());
        }
        let LayerState::Attention { ik: ikc, .. } = &self.states[l] else { unreachable!() };
        let LayerState::Attention { k: kc, v: vc, .. } = &self.states[shared.unwrap_or(l)] else {
            unreachable!("a shared cache is an attention layer's")
        };
        // Each query's positions: QSA's selection where the model indexes, else every visible one.
        let kept: Option<Vec<Vec<usize>>> = match index {
            Some((ix, queries, _, kw)) => {
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
    /// its `kv_lora_rank` latent, RMS-normed at the latent norms' own epsilon, and the shared rotated key; each head's unrotated query reaches the latent's
    /// space through the key half of the latent's expansion, attends every cached row with its rotated part at the
    /// model's scale, takes the rows' latents as its values, and leaves through the value half of the expansion. The
    /// halves read their activations unquantized under every precision, so these are the scores and values the
    /// expanded attention takes — each cached latent through both halves, each head attending its own keys and values —
    /// and a lane computes either.
    fn mla(&mut self, l: usize, m: &Mla, xn: &[Vec<f64>], start: usize) -> R<Vec<Vec<f64>>> {
        let (h, nope, rope, vd, lora) = (m.heads, m.qk_nope, m.qk_rope, m.v_dim, m.kv_lora_rank);
        let (qk, kw) = (nope + rope, lora + rope);
        let (q, latent_q) = match m.q_lora_rank {
            None => (self.project(&format!("blk.{l}.attn_q.weight"), xn)?, None),
            Some(_) => {
                let qa = self.project(&format!("blk.{l}.attn_q_a.weight"), xn)?;
                let qa = self.norm_rows_at(&qa, &format!("blk.{l}.attn_q_a_norm.weight"), m.latent_norm_eps)?;
                (self.project(&format!("blk.{l}.attn_q_b.weight"), &qa)?, Some(qa))
            }
        };
        // DeepSeek's sparse attention: each query's positions, its own indexer's choice or the last full one's.
        let chosen: Option<Vec<Vec<usize>>> = match &m.indexer {
            None => None,
            Some(ix) if !ix.full => Some(self.dsa_chosen.clone()),
            Some(ix) => {
                let qa = latent_q.as_ref().ok_or(ReferenceError::NotYet("a DSA indexer over a full-rank query"))?;
                let rope = m.rope.as_ref().ok_or(ReferenceError::NotYet("a DSA indexer beside latent attention that takes no positions"))?;
                let chosen = self.dsa_choose(l, ix, rope, qa, xn, start)?;
                self.dsa_chosen = chosen.clone();
                Some(chosen)
            }
        };
        let kva = self.project(&format!("blk.{l}.attn_kv_a_mqa.weight"), xn)?;
        let latent: Vec<Vec<f64>> = kva.iter().map(|r| r[..lora].to_vec()).collect();
        let latent = self.norm_rows_at(&latent, &format!("blk.{l}.attn_kv_a_norm.weight"), m.latent_norm_eps)?;
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
            if let Some(r) = &m.rope {
                rotate(&mut pe, pos, r, None);
            }
            let row: Vec<f64> = c.iter().chain(&pe).copied().collect();
            let row = self.kept(row.clone(), row.len())?;
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
                if let Some(r) = &m.rope {
                    rotate(&mut qe, pos, r, None);
                }
                let LayerState::Attention { k: cache, .. } = &self.states[l] else { unreachable!() };
                let attended: Vec<usize> = match &chosen {
                    Some(c) => c[i].clone(),
                    None => (0..cache.len()).collect(),
                };
                let mut scores: Vec<f64> = attended
                    .iter()
                    .map(|&j| {
                        let row = &cache[j];
                        let s = absorbed.iter().zip(&row[..lora]).map(|(a, b)| a * b).sum::<f64>()
                            + qe.iter().zip(&row[lora..]).map(|(a, b)| a * b).sum::<f64>();
                        s * m.scale
                    })
                    .collect();
                softmax(&mut scores);
                for (w, &j) in scores.iter().zip(&attended) {
                    for (slot, v) in ol[hh * lora..(hh + 1) * lora].iter_mut().zip(&cache[j][..lora]) {
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

    /// The positions DSA's lightning indexer (llama.cpp `models/deepseek32.cpp`, transformers `DeepseekV32Indexer`) has
    /// each query of `xn` attend: index queries from the query's normed latent `qa` and this step's index keys from the
    /// block's input (a LayerNorm at epsilon 1e-6 by its weight and bias), both rotated over their first `qk_rope`
    /// dimensions, the keys cached; every cached position up to the query's scored `Σ_h w_h·relu(q_h·k)/√(head_dim·heads)`
    /// and the best `top_k` kept (the lower position first between equal scores), in order, every one while no more are
    /// visible.
    fn dsa_choose(&mut self, l: usize, ix: &Dsa, rope: &Rope, qa: &[Vec<f64>], xn: &[Vec<f64>], start: usize) -> R<Vec<Vec<usize>>> {
        let (heads, hd) = (ix.heads, ix.head_dim);
        let iq = self.project(&format!("blk.{l}.indexer.attn_q_b.weight"), qa)?;
        let ik = self.project(&format!("blk.{l}.indexer.attn_k.weight"), xn)?;
        let w = self.project(&format!("blk.{l}.indexer.proj.weight"), xn)?;
        let (nw, nb) = (self.tensor(&format!("blk.{l}.indexer.k_norm.weight"))?, self.tensor(&format!("blk.{l}.indexer.k_norm.bias"))?);
        let rotation = Rope { neox: ix.neox, ..rope.clone() };
        let keys: Vec<Vec<f64>> = ik
            .iter()
            .enumerate()
            .map(|(i, row)| {
                let mut k = layer_norm(row, &nw, Some(&nb), 1e-6);
                rotate(&mut k, start + i, &rotation, None);
                k
            })
            .collect();
        let LayerState::Attention { ik: cached, .. } = &mut self.states[l] else {
            unreachable!("a latent-attention layer holds an attention cache")
        };
        cached.extend(keys);
        let scale = 1.0 / ((hd * heads) as f64).sqrt();
        Ok((0..xn.len())
            .map(|i| {
                let pos = start + i;
                if pos < ix.top_k {
                    return (0..=pos).collect();
                }
                let queries: Vec<Vec<f64>> = (0..heads)
                    .map(|hh| {
                        let mut q = iq[i][hh * hd..(hh + 1) * hd].to_vec();
                        rotate(&mut q, pos, &rotation, None);
                        q
                    })
                    .collect();
                let mut scored: Vec<(f64, usize)> = (0..=pos)
                    .map(|j| {
                        let k = &cached[j];
                        let s = queries
                            .iter()
                            .zip(&w[i])
                            .map(|(q, wh)| wh * q.iter().zip(k).map(|(a, b)| a * b).sum::<f64>().max(0.0))
                            .sum::<f64>();
                        (s * scale, j)
                    })
                    .collect();
                scored.sort_by(|a, b| b.0.total_cmp(&a.0).then(a.1.cmp(&b.1)));
                let mut kept: Vec<usize> = scored[..ix.top_k].iter().map(|&(_, j)| j).collect();
                kept.sort_unstable();
                kept
            })
            .collect())
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
        if self.spec.layers.iter().any(|l| !l.pre_norms) {
            return Err(ReferenceError::NotYet("a hyper-connected residual whose sublayers read it unnormed"));
        }
        let mut h: Vec<Vec<f64>> = x.iter().map(|r| r.repeat(hc.lanes)).collect();
        for l in 0..self.spec.layers.len() {
            let layer = self.spec.layers[l].clone();
            if self.spec.ple.as_ref().is_some_and(|p| p.layer == l) {
                let added = self.ple(l, tokens, &h, hc)?;
                add_rows(&mut h, &added);
            }
            let (input, inject) = self.mix_lanes(&h, &format!("blk.{l}.hc_attn"), hc, true)?;
            let mut out = self.mixer(l, &layer.mixer, &input, start)?;
            // A sublayer's own output norm (Gemma's sandwich) normalizes what joins the lanes, as it does the residual.
            if layer.post_mixer_norm {
                out = self.block_norm(&out, &format!("blk.{l}.post_attention_norm"))?;
            }
            inject_lanes(&mut h, &out, &inject, self.spec.d);
            let (input, inject) = self.mix_lanes(&h, &format!("blk.{l}.hc_ffn"), hc, true)?;
            let mut out = self.ffn(l, &layer, &input)?;
            if layer.post_ffn_norm {
                out = self.block_norm(&out, &format!("blk.{l}.post_ffw_norm"))?;
            }
            inject_lanes(&mut h, &out, &inject, self.spec.d);
        }
        Ok(h)
    }

    /// Gemma's per-layer inputs for each of `tokens` (transformers `Gemma3nTextModel.project_per_layer_inputs`, llama.cpp
    /// `project_per_layer_inputs`): the embedded row `x` through `per_layer_model_proj` over `√d`, each block's `dim` of it
    /// RMS-normed by `per_layer_proj_norm`, plus the token's row of `per_layer_token_embd` times `√dim`, the sum over `√2`;
    /// a row of `layers · dim` a position, block `l`'s from `l · dim`.
    fn per_layer_inputs(&mut self, tokens: &[u32], x: &[Vec<f64>], dim: usize) -> R<Vec<Vec<f64>>> {
        let (layers, eps) = (self.spec.layers.len(), self.spec.rms_eps);
        let (scale_d, scale_dim) = ((self.spec.d as f64).sqrt(), (dim as f64).sqrt());
        let projected = self.project("per_layer_model_proj.weight", x)?;
        let norm = self.tensor("per_layer_proj_norm.weight")?;
        let mut out = Vec::with_capacity(tokens.len());
        for (&t, p) in tokens.iter().zip(&projected) {
            let table = self.rows("per_layer_token_embd.weight", t as usize, 1)?;
            let mut row = Vec::with_capacity(layers * dim);
            for l in 0..layers {
                let scaled: Vec<f64> = p[l * dim..(l + 1) * dim].iter().map(|v| v / scale_d).collect();
                let normed = rms_norm(&scaled, &norm, eps);
                row.extend((0..dim).map(|e| (normed[e] + table[l * dim + e] * scale_dim) / std::f64::consts::SQRT_2));
            }
            out.push(row);
        }
        Ok(out)
    }

    /// The stack over Gemma 3n's alternating updates (transformers `Gemma3nTextModel` and `Gemma3nTextDecoderLayer`,
    /// llama.cpp `models/gemma3n.cpp`): `streams` copies of each position, the embedding and its projections by
    /// `altup_proj` at its root mean square; each block predicts every stream from all of them, computes on the active
    /// prediction — its norm, LAuReL beside its attention, both summed over `√2`, the FFN on that — corrects every stream
    /// by the active one's innovation, and adds its per-layer input, gated by the corrected active stream, to the others;
    /// the head reads the mean of stream 0 and the others through `altup_unembd_proj` at stream 0's root mean square. A
    /// projection's root mean square is taken no smaller than `√1e-5`, as transformers takes it.
    fn altup_layers(&mut self, tokens: &[u32], x: &[Vec<f64>], start: usize, streams: usize, active: usize) -> R<Vec<Vec<f64>>> {
        if self.spec.layers.iter().any(|l| !l.pre_norms) {
            return Err(ReferenceError::NotYet("alternating updates whose sublayers read the stream unnormed"));
        }
        let d = self.spec.d;
        let dim = self
            .spec
            .parts
            .iter()
            .find_map(|p| if let Part::PerLayerInput { dim } = p { Some(*dim) } else { None })
            .ok_or(ReferenceError::NotYet("alternating updates without per-layer inputs"))?;
        let laurel = self.spec.parts.iter().any(|p| matches!(p, Part::Laurel { .. }));
        let rms = |v: &[f64]| (v.iter().map(|x| x * x).sum::<f64>() / v.len() as f64).sqrt();
        let floored = |v: &[f64]| (v.iter().map(|x| x * x).sum::<f64>() / v.len() as f64).max(1e-5).sqrt();
        let slice = |w: &[f64], s: usize, x: &[f64]| -> Vec<f64> { (0..d).map(|o| (0..d).map(|i| w[s * d * d + o * d + i] * x[i]).sum::<f64>()).collect() };
        let per_layer = self.per_layer_inputs(tokens, x, dim)?;
        let up = self.tensor("altup_proj.weight")?;
        // Each position's streams, one after another.
        let mut h: Vec<Vec<Vec<f64>>> = x
            .iter()
            .map(|row| {
                let target = rms(row);
                let mut all = vec![row.clone()];
                for s in 1..streams {
                    let y = slice(&up, s - 1, row);
                    let f = target / floored(&y);
                    all.push(y.into_iter().map(|v| v * f).collect());
                }
                all
            })
            .collect();
        let inv_d = 1.0 / d as f64;
        for l in 0..self.spec.layers.len() {
            let layer = self.spec.layers[l].clone();
            let blk = |t: &str| format!("blk.{l}.{t}");
            let router_norm = self.tensor(&blk("altup_router_norm.weight"))?;
            let eps = self.spec.rms_eps;
            let routed = |me: &mut Self, rows: &[Vec<f64>]| -> R<Vec<Vec<f64>>> {
                let normed: Vec<Vec<f64>> = rows.iter().map(|r| rms_norm(r, &router_norm, eps).into_iter().map(|v| v * inv_d).collect()).collect();
                Ok(me.project(&blk("altup_router.weight"), &normed)?.into_iter().map(|r| r.into_iter().map(f64::tanh).collect()).collect())
            };
            let actives: Vec<Vec<f64>> = h.iter().map(|p| p[active].clone()).collect();
            let modalities = routed(self, &actives)?;
            let coefs = self.project(&blk("altup_predict_coef.weight"), &modalities)?;
            let predictions: Vec<Vec<Vec<f64>>> = h
                .iter()
                .zip(&coefs)
                .map(|(hs, c)| {
                    (0..streams)
                        .map(|p| (0..d).map(|e| hs[p][e] + (0..streams).map(|a| c[p * streams + a] * hs[a][e]).sum::<f64>()).collect())
                        .collect()
                })
                .collect();
            let active_prediction: Vec<Vec<f64>> = predictions.iter().map(|p| p[active].clone()).collect();
            let xn = self.block_norm(&active_prediction, &blk("attn_norm"))?;
            let mut mixed = self.mixer(l, &layer.mixer, &xn, start)?;
            if layer.post_mixer_norm {
                mixed = self.block_norm(&mixed, &blk("post_attention_norm"))?;
            }
            add_rows(&mut mixed, &active_prediction);
            if laurel {
                let low = self.project(&blk("laurel_l.weight"), &xn)?;
                let back = self.project(&blk("laurel_r.weight"), &low)?;
                let mut aug = self.block_norm(&back, &blk("laurel_post_norm"))?;
                add_rows(&mut aug, &xn);
                add_rows(&mut mixed, &aug);
                scale_rows(&mut mixed, std::f64::consts::FRAC_1_SQRT_2);
            }
            let xn2 = self.block_norm(&mixed, &format!("blk.{l}.{}", self.spec.ffn_norm))?;
            let mut ffn = self.ffn(l, &layer, &xn2)?;
            if layer.post_ffn_norm {
                ffn = self.block_norm(&ffn, &blk("post_ffw_norm"))?;
            }
            add_rows(&mut ffn, &mixed);
            let modalities = routed(self, &ffn)?;
            let correct = self.project(&blk("altup_correct_coef.weight"), &modalities)?;
            let scale = self.tensor(&blk("altup_correct_scale.weight"))?;
            let mut corrected: Vec<Vec<Vec<f64>>> = predictions
                .iter()
                .zip(&ffn)
                .zip(&correct)
                .map(|((p, activated), c)| {
                    let innovation: Vec<f64> = activated.iter().zip(&p[active]).map(|(a, b)| a - b).collect();
                    (0..streams).map(|s| (0..d).map(|e| p[s][e] + innovation[e] * (c[s] + 1.0)).collect()).collect()
                })
                .collect();
            let firsts: Vec<Vec<f64>> = corrected.iter().map(|c| c[active].iter().zip(&scale).map(|(v, w)| v * w).collect()).collect();
            let gate = self.project(&blk("inp_gate.weight"), &firsts)?;
            let gated: Vec<Vec<f64>> = gate
                .iter()
                .zip(&per_layer)
                .map(|(g, pl)| g.iter().enumerate().map(|(e, &v)| gelu_tanh(v) * pl[l * dim + e]).collect())
                .collect();
            let projected = self.project(&blk("proj.weight"), &gated)?;
            let first = self.block_norm(&projected, &blk("post_norm"))?;
            for (c, f) in corrected.iter_mut().zip(&first) {
                for stream in c.iter_mut().skip(1) {
                    stream.iter_mut().zip(f).for_each(|(v, a)| *v += a);
                }
            }
            h = corrected;
        }
        let down = self.tensor("altup_unembd_proj.weight")?;
        Ok(h
            .iter()
            .map(|hs| {
                let target = rms(&hs[0]);
                let mut sum = hs[0].clone();
                for s in 1..streams {
                    let y = slice(&down, s - 1, &hs[s]);
                    let f = target / floored(&y);
                    sum.iter_mut().zip(&y).for_each(|(a, b)| *a += b * f);
                }
                sum.into_iter().map(|v| v / streams as f64).collect()
            })
            .collect())
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

    /// The earlier layer whose cache layer `l` attends where the model shares caches (Gemma 3n's and Gemma 4's, llama.cpp's
    /// `layer_reuse`): from `from` on, a sliding-window layer attends layer `from − 2`'s, a global one layer `from − 1`'s.
    fn shared_cache(&self, l: usize, a: &Attention) -> Option<usize> {
        self.spec.parts.iter().find_map(|p| match p {
            Part::SharedKv { from } if l >= *from => Some(if a.window.is_some() { from - 2 } else { from - 1 }),
            _ => None,
        })
    }

    /// Layer `l`'s mixer over the normed rows `xn` at positions from `start`.
    fn mixer(&mut self, l: usize, mixer: &Mixer, xn: &[Vec<f64>], start: usize) -> R<Vec<Vec<f64>>> {
        match mixer {
            Mixer::Attention(a) => self.attention(l, a, xn, start),
            Mixer::GatedDelta(gd) => self.gated_delta(l, gd, xn),
            Mixer::Mla(m) => self.mla(l, m, xn, start),
            Mixer::Recurrent(Recurrent::Parallel(a, s)) => self.parallel(l, a, s, xn, start),
            Mixer::Recurrent(r) => self.recurrent(l, r, xn),
            Mixer::None => Ok(vec![vec![0.0; self.spec.d]; xn.len()]),
        }
    }

    /// Falcon-H1's block (llama.cpp `models/falcon-h1.cpp`): attention and Mamba-2's scan over the same normed rows, each
    /// with its own state, their outputs summed.
    fn parallel(&mut self, l: usize, a: &Attention, s: &Ssd, xn: &[Vec<f64>], start: usize) -> R<Vec<Vec<f64>>> {
        let LayerState::Parallel { attention, scan } = std::mem::replace(&mut self.states[l], LayerState::Stateless) else {
            unreachable!("a parallel block holds an attention cache and a scan's state")
        };
        self.states[l] = *attention;
        let attended = self.attention(l, a, xn, start);
        let attention = std::mem::replace(&mut self.states[l], *scan);
        let scanned = self.ssd(l, s, xn);
        let scan = std::mem::replace(&mut self.states[l], LayerState::Stateless);
        self.states[l] = LayerState::Parallel { attention: Box::new(attention), scan: Box::new(scan) };
        let mut out = attended?;
        add_rows(&mut out, &scanned?);
        Ok(out)
    }

    fn recurrent(&mut self, l: usize, r: &Recurrent, xn: &[Vec<f64>]) -> R<Vec<Vec<f64>>> {
        match r {
            Recurrent::ShortConv(c) => self.short_conv(l, c, xn),
            Recurrent::Ssd(s) => self.ssd(l, s, xn),
            Recurrent::Selective(s) => self.selective(l, s, xn),
            Recurrent::DeltaAttention(d) => self.delta_attention(l, d, xn),
            Recurrent::Lightning(x) => self.lightning(l, x, xn),
            Recurrent::Rwkv7(r) => self.rwkv7(l, r, xn),
            other => Err(ReferenceError::NotYet(other.what())),
        }
    }

    /// MiniMax-01's lightning attention over the rows of `xn` in order (llama.cpp `models/minimax-01.cpp`, MiniMax's
    /// `MiniMaxText01LightningAttention`): `attn_qkv` of the row through SiLU, each head's query, key and value in turn;
    /// per head `h` the memory `S ← e^(−s_h)·S + kᵀv` (key by value) read as `q·S`, `s_h = 2^(−8(h+1)/heads)` times the
    /// depth scale `1 − l/(L − 1) + 10⁻⁵`; the heads' reads RMS-normed together by `attn_norm_2` at 10⁻⁶, gated by
    /// `σ(attn_gate·x)`, projected by `attn_output`.
    fn lightning(&mut self, l: usize, x: &Lightning, xn: &[Vec<f64>]) -> R<Vec<Vec<f64>>> {
        let (heads, hd) = (x.heads, x.head_dim);
        let blk = |t: &str| format!("blk.{l}.{t}");
        let qkv = self.project(&blk("attn_qkv.weight"), xn)?;
        let gate = self.project(&blk("attn_gate.weight"), xn)?;
        let norm = self.tensor(&blk("attn_norm_2.weight"))?;
        let depth = 1.0 - l as f64 / (self.spec.layers.len() - 1) as f64 + 1e-5;
        let decay: Vec<f64> = (0..heads).map(|h| (-(-8.0 * (h + 1) as f64 / heads as f64).exp2() * depth).exp()).collect();
        let LayerState::Scan { state, .. } = &mut self.states[l] else {
            unreachable!("a lightning layer holds each head's memory")
        };
        let mut reads = Vec::with_capacity(xn.len());
        for (t, row) in qkv.iter().enumerate() {
            let mut read = vec![0f64; heads * hd];
            for h in 0..heads {
                let at = h * 3 * hd;
                let (q, k, v): (Vec<f64>, Vec<f64>, Vec<f64>) = (
                    row[at..at + hd].iter().map(|&a| silu(a)).collect(),
                    row[at + hd..at + 2 * hd].iter().map(|&a| silu(a)).collect(),
                    row[at + 2 * hd..at + 3 * hd].iter().map(|&a| silu(a)).collect(),
                );
                let s = &mut state[h * hd * hd..(h + 1) * hd * hd];
                for kk in 0..hd {
                    for vv in 0..hd {
                        s[kk * hd + vv] = decay[h] * s[kk * hd + vv] + k[kk] * v[vv];
                    }
                }
                for vv in 0..hd {
                    read[h * hd + vv] = (0..hd).map(|kk| q[kk] * s[kk * hd + vv]).sum::<f64>();
                }
            }
            let normed = rms_norm(&read, &norm, 1e-6);
            reads.push(normed.iter().zip(&gate[t]).map(|(r, g)| r * sigmoid(*g)).collect());
        }
        self.project(&blk("attn_output.weight"), &reads)
    }

    /// The previous position's row of each of `xn` (layer `l`'s state for the first), and the state left holding the
    /// last: RWKV's token shift, of the time mix's input or (`ffn`) the channel mix's.
    fn token_shift(&mut self, l: usize, xn: &[Vec<f64>], ffn: bool) -> Vec<Vec<f64>> {
        let d = self.spec.d;
        let LayerState::Rwkv { att_shift, ffn_shift, .. } = &mut self.states[l] else {
            unreachable!("an RWKV-7 block holds its token shifts")
        };
        let shift = if ffn { ffn_shift } else { att_shift };
        let mut prev = Vec::with_capacity(xn.len());
        prev.push(if shift.is_empty() { vec![0.0; d] } else { shift.clone() });
        prev.extend(xn.iter().take(xn.len().saturating_sub(1)).cloned());
        if let Some(last) = xn.last() {
            *shift = last.clone();
        }
        prev
    }

    /// RWKV-7's time mix over the rows of `xn` in order (llama.cpp `build_rwkv7_time_mix` and ggml's `rwkv_wkv7`): the
    /// streams lerped from the row and the previous position's; the decay, learning rate, gate and (past the first
    /// block) the value's pull toward the first block's through their low-rank pairs; per head
    /// `S ← S·diag(w) + v kᵀ + (S·(−κ)) (κ⊙a)ᵀ`, `κ` the L2-normed `k⊙k_k`, read as `S r`; the reads group-normed per
    /// head at 64·10⁻⁵, the bonus `v·Σ(k⊙r⊙r_k)` added, gated, projected by `time_mix_output`.
    fn rwkv7(&mut self, l: usize, r: &Rwkv7, xn: &[Vec<f64>]) -> R<Vec<Vec<f64>>> {
        let (d, hs) = (self.spec.d, r.head_size);
        let heads = d / hs;
        let blk = |t: &str| format!("blk.{l}.{t}");
        let prev = self.token_shift(l, xn, false);
        let lerp = self.tensor(&blk("time_mix_lerp_fused.weight"))?;
        let stream = |s: usize| -> Vec<Vec<f64>> {
            xn.iter().zip(&prev).map(|(x, p)| (0..d).map(|i| x[i] + (p[i] - x[i]) * lerp[s * d + i]).collect()).collect()
        };
        let (xr, xw, xk, xv, xa, xg) = (stream(0), stream(1), stream(2), stream(3), stream(4), stream(5));
        let low_rank = |me: &mut Self, down: &str, up: &str, x: &[Vec<f64>], inner: fn(f64) -> f64| -> R<Vec<Vec<f64>>> {
            let mut h = me.project(&blk(down), x)?;
            h.iter_mut().flatten().for_each(|v| *v = inner(*v));
            me.project(&blk(up), &h)
        };
        let rec = self.project(&blk("time_mix_receptance.weight"), &xr)?;
        let w0 = self.tensor(&blk("time_mix_w0.weight"))?;
        let decay = low_rank(self, "time_mix_w1.weight", "time_mix_w2.weight", &xw, f64::tanh)?;
        let key = self.project(&blk("time_mix_key.weight"), &xk)?;
        let mut value = self.project(&blk("time_mix_value.weight"), &xv)?;
        if r.first {
            self.v_first = value.clone();
        } else {
            let v0 = self.tensor(&blk("time_mix_v0.weight"))?;
            let mix = low_rank(self, "time_mix_v1.weight", "time_mix_v2.weight", &xv, |v| v)?;
            for (t, row) in value.iter_mut().enumerate() {
                for i in 0..d {
                    row[i] += (self.v_first[t][i] - row[i]) * sigmoid(mix[t][i] + v0[i]);
                }
            }
        }
        let gate = low_rank(self, "time_mix_g1.weight", "time_mix_g2.weight", &xg, sigmoid)?;
        let a0 = self.tensor(&blk("time_mix_a0.weight"))?;
        let rate = low_rank(self, "time_mix_a1.weight", "time_mix_a2.weight", &xa, |v| v)?;
        let (k_k, k_a, r_k) = (self.tensor(&blk("time_mix_k_k.weight"))?, self.tensor(&blk("time_mix_k_a.weight"))?, self.tensor(&blk("time_mix_r_k.weight"))?);
        let (ln_w, ln_b) = (self.tensor(&blk("time_mix_ln.weight"))?, self.tensor(&blk("time_mix_ln.bias"))?);
        let LayerState::Rwkv { state, .. } = &mut self.states[l] else {
            unreachable!("an RWKV-7 block holds its state")
        };
        if state.is_empty() {
            *state = vec![0.0; heads * hs * hs];
        }
        let mut outs = Vec::with_capacity(xn.len());
        for t in 0..xn.len() {
            let w: Vec<f64> = (0..d).map(|i| (-0.606531 * sigmoid(w0[i] + decay[t][i])).exp()).collect();
            let a: Vec<f64> = (0..d).map(|i| sigmoid(a0[i] + rate[t][i])).collect();
            let kappa: Vec<f64> = (0..d).map(|i| key[t][i] * k_k[i]).collect();
            let k: Vec<f64> = (0..d).map(|i| key[t][i] + (a[i] - 1.0) * key[t][i] * k_a[i]).collect();
            let v = &value[t];
            let mut out = vec![0f64; d];
            for h in 0..heads {
                let span = h * hs..(h + 1) * hs;
                let norm = kappa[span.clone()].iter().map(|x| x * x).sum::<f64>().sqrt().max(1e-12);
                let kk: Vec<f64> = kappa[span.clone()].iter().map(|x| x / norm).collect();
                let s = &mut state[h * hs * hs..(h + 1) * hs * hs];
                let mut read = vec![0f64; hs];
                for i in 0..hs {
                    let row = &mut s[i * hs..(i + 1) * hs];
                    let sa: f64 = (0..hs).map(|j| -kk[j] * row[j]).sum();
                    for j in 0..hs {
                        row[j] = row[j] * w[h * hs + j] + v[h * hs + i] * k[h * hs + j] + sa * kk[j] * a[h * hs + j];
                    }
                    read[i] = (0..hs).map(|j| row[j] * rec[t][h * hs + j]).sum();
                }
                let mean = read.iter().sum::<f64>() / hs as f64;
                let var = read.iter().map(|x| (x - mean) * (x - mean)).sum::<f64>() / hs as f64;
                let scale = 1.0 / (var + 64e-5).sqrt();
                let bonus: f64 = span.clone().map(|j| k[j] * rec[t][j] * r_k[j]).sum();
                for i in 0..hs {
                    let c = h * hs + i;
                    out[c] = ((read[i] - mean) * scale * ln_w[c] + ln_b[c] + v[c] * bonus) * gate[t][c];
                }
            }
            outs.push(out);
        }
        self.project(&blk("time_mix_output.weight"), &outs)
    }

    /// RWKV-7's channel mix over the rows of `xn` in order (llama.cpp `build_rwkv7_channel_mix`): each row lerped toward
    /// the previous position's by `channel_mix_lerp_k`, `relu(key·x)²`, projected by `channel_mix_value`.
    fn channel_mix(&mut self, l: usize, xn: &[Vec<f64>]) -> R<Vec<Vec<f64>>> {
        let d = self.spec.d;
        let prev = self.token_shift(l, xn, true);
        let lerp = self.tensor(&format!("blk.{l}.channel_mix_lerp_k.weight"))?;
        let xk: Vec<Vec<f64>> = xn.iter().zip(&prev).map(|(x, p)| (0..d).map(|i| x[i] + (p[i] - x[i]) * lerp[i]).collect()).collect();
        let mut k = self.project(&format!("blk.{l}.channel_mix_key.weight"), &xk)?;
        k.iter_mut().flatten().for_each(|v| {
            let r = v.max(0.0);
            *v = r * r;
        });
        self.project(&format!("blk.{l}.channel_mix_value.weight"), &k)
    }

    /// Kimi delta attention over the rows of `xn` in order (transformers `KimiLinearDeltaAttention` with
    /// `recurrent_kimi_delta_attention`, ggml's `gated_delta_net` with a decay per channel): the queries, keys and values
    /// each convolved depthwise with the last `taps − 1` of them before it and through SiLU; the queries and keys of each
    /// head L2-normed as `x / √(Σx² + 1e-6)`, the queries over `√head_dim`; each channel's decay
    /// `exp(A_h · softplus(f_b·f_a·x + dt_bias))`, `A = −exp(A_log)` as the file holds it, each head's write strength
    /// `σ(beta·x)`; per head the memory `S` (key by value) decayed row by row, `δ = β(v − Sᵀk)`, `S += k δᵀ`, the read
    /// `Sᵀq`, RMS-normed by `ssm_norm` and gated by `σ(g_b·g_a·x)`, projected out.
    fn delta_attention(&mut self, l: usize, d: &DeltaAttention, xn: &[Vec<f64>]) -> R<Vec<Vec<f64>>> {
        let (heads, hd, taps) = (d.heads, d.head_dim, d.conv_taps);
        let w = heads * hd;
        let blk = |t: &str| format!("blk.{l}.{t}");
        let (q, k, v) = (self.project(&blk("attn_q.weight"), xn)?, self.project(&blk("attn_k.weight"), xn)?, self.project(&blk("attn_v.weight"), xn)?);
        let kernels = [self.tensor(&blk("ssm_conv1d_q.weight"))?, self.tensor(&blk("ssm_conv1d_k.weight"))?, self.tensor(&blk("ssm_conv1d_v.weight"))?];
        let decay = self.project(&blk("ssm_f_a.weight"), xn)?;
        let decay = self.project(&blk("ssm_f_b.weight"), &decay)?;
        let dt_bias = self.tensor(&blk("ssm_dt.bias"))?;
        let a = self.tensor(&blk("ssm_a"))?;
        let beta = self.project(&blk("ssm_beta.weight"), xn)?;
        let gate = self.project(&blk("ssm_g_a.weight"), xn)?;
        let gate = self.project(&blk("ssm_g_b.weight"), &gate)?;
        let norm = self.tensor(&blk("ssm_norm.weight"))?;
        let eps = self.spec.rms_eps;
        let LayerState::Scan { conv, state } = &mut self.states[l] else {
            unreachable!("a delta attention layer holds its convolution's and its memory's state")
        };
        let l2 = |x: &[f64]| -> Vec<f64> {
            let n = (x.iter().map(|v| v * v).sum::<f64>() + 1e-6).sqrt();
            x.iter().map(|v| v / n).collect()
        };
        let scale = 1.0 / (hd as f64).sqrt();
        let mut reads = Vec::with_capacity(xn.len());
        for t in 0..xn.len() {
            let mixed: Vec<f64> = q[t].iter().chain(&k[t]).chain(&v[t]).copied().collect();
            let c: Vec<f64> = (0..3 * w)
                .map(|ch| {
                    let (kernel, at) = (&kernels[ch / w], ch % w);
                    let older = conv.iter().enumerate().map(|(j, r)| r[ch] * kernel[at * taps + j]).sum::<f64>();
                    silu(older + mixed[ch] * kernel[at * taps + taps - 1])
                })
                .collect();
            if taps > 1 {
                conv.remove(0);
                conv.push(mixed);
            }
            let mut out = vec![0f64; w];
            for h in 0..heads {
                let qh: Vec<f64> = l2(&c[h * hd..(h + 1) * hd]).into_iter().map(|x| x * scale).collect();
                let kh = l2(&c[w + h * hd..w + (h + 1) * hd]);
                let vh = &c[2 * w + h * hd..2 * w + (h + 1) * hd];
                let b = sigmoid(beta[t][h]);
                let s = &mut state[h * hd * hd..(h + 1) * hd * hd];
                for kk in 0..hd {
                    let g = (a[h] * softplus(decay[t][h * hd + kk] + dt_bias[h * hd + kk])).exp();
                    s[kk * hd..(kk + 1) * hd].iter_mut().for_each(|x| *x *= g);
                }
                let delta: Vec<f64> = (0..hd).map(|vv| (vh[vv] - (0..hd).map(|kk| s[kk * hd + vv] * kh[kk]).sum::<f64>()) * b).collect();
                for kk in 0..hd {
                    for vv in 0..hd {
                        s[kk * hd + vv] += kh[kk] * delta[vv];
                    }
                }
                let read: Vec<f64> = (0..hd).map(|vv| (0..hd).map(|kk| s[kk * hd + vv] * qh[kk]).sum::<f64>()).collect();
                let normed = rms_norm(&read, &norm, eps);
                for vv in 0..hd {
                    out[h * hd + vv] = normed[vv] * sigmoid(gate[t][h * hd + vv]);
                }
            }
            reads.push(out);
        }
        self.project(&blk("attn_output.weight"), &reads)
    }

    /// Mamba-1's selective scan over the rows of `xn` in order (llama.cpp `build_mamba_layer` and ggml's `ssm_scan`):
    /// `x` and the gate `z` from `ssm_in`; `x` convolved depthwise with the last `taps − 1` of them before it, biased and
    /// through SiLU; `ssm_x` of it into the time step's `dt_rank`, `B` and `C`, each RMS-normed where the scan has its
    /// norms; the time step through `ssm_dt` and its bias; per channel `c` and state `i` the state
    /// `S ← S·exp(Δ_c·A_ci) + Δ_c·x_c·B_i` and the read `Σ_i S·C_i + D_c·x_c`, `Δ = softplus(dt)`; the reads times
    /// `silu(z)` and projected by `ssm_out`.
    fn selective(&mut self, l: usize, s: &Selective, xn: &[Vec<f64>]) -> R<Vec<Vec<f64>>> {
        let (inner, n, rank, taps) = (s.inner, s.state, s.dt_rank, s.conv_taps);
        let blk = |t: &str| format!("blk.{l}.{t}");
        let xz = self.project(&blk("ssm_in.weight"), xn)?;
        let kernel = self.tensor(&blk("ssm_conv1d.weight"))?;
        let bias = self.tensor(&blk("ssm_conv1d.bias"))?;
        let a = self.tensor(&blk("ssm_a"))?;
        let skip = self.tensor(&blk("ssm_d"))?;
        let dt_bias = self.tensor(&blk("ssm_dt.bias"))?;
        let norms = if s.inner_norms {
            Some((self.tensor(&blk("ssm_dt_norm.weight"))?, self.tensor(&blk("ssm_b_norm.weight"))?, self.tensor(&blk("ssm_c_norm.weight"))?))
        } else {
            None
        };
        let eps = self.spec.rms_eps;
        let mut convolved = Vec::with_capacity(xn.len());
        {
            let LayerState::Scan { conv, .. } = &mut self.states[l] else {
                unreachable!("a selective scan layer holds its convolution's and its scan's state")
            };
            for row in &xz {
                let x = &row[..inner];
                let c: Vec<f64> = (0..inner)
                    .map(|ch| {
                        let older = conv.iter().enumerate().map(|(j, r)| r[ch] * kernel[ch * taps + j]).sum::<f64>();
                        silu(older + x[ch] * kernel[ch * taps + taps - 1] + bias[ch])
                    })
                    .collect();
                if taps > 1 {
                    conv.remove(0);
                    conv.push(x.to_vec());
                }
                convolved.push(c);
            }
        }
        let xdb = self.project(&blk("ssm_x.weight"), &convolved)?;
        let mut dts = Vec::with_capacity(xn.len());
        let mut bcs = Vec::with_capacity(xn.len());
        for row in &xdb {
            let (dt, b, c) = (&row[..rank], &row[rank..rank + n], &row[rank + n..rank + 2 * n]);
            let (dt, b, c) = match &norms {
                Some((wd, wb, wc)) => (rms_norm(dt, wd, eps), rms_norm(b, wb, eps), rms_norm(c, wc, eps)),
                None => (dt.to_vec(), b.to_vec(), c.to_vec()),
            };
            dts.push(dt);
            bcs.push((b, c));
        }
        let deltas = self.project(&blk("ssm_dt.weight"), &dts)?;
        let LayerState::Scan { state, .. } = &mut self.states[l] else {
            unreachable!("a selective scan layer holds its convolution's and its scan's state")
        };
        let mut reads = Vec::with_capacity(xn.len());
        for (t, row) in xz.iter().enumerate() {
            let (x, z) = (&convolved[t], &row[inner..2 * inner]);
            let (b, c) = &bcs[t];
            let y: Vec<f64> = (0..inner)
                .map(|ch| {
                    let delta = softplus(deltas[t][ch] + dt_bias[ch]);
                    let st = &mut state[ch * n..(ch + 1) * n];
                    let mut read = 0.0;
                    for i in 0..n {
                        st[i] = st[i] * (delta * a[ch * n + i]).exp() + delta * x[ch] * b[i];
                        read += st[i] * c[i];
                    }
                    (read + skip[ch] * x[ch]) * silu(z[ch])
                })
                .collect();
            reads.push(y);
        }
        self.project(&blk("ssm_out.weight"), &reads)
    }

    /// Mamba-2's scan over the rows of `xn` in order (llama.cpp `build_mamba2_layer` and ggml's `ssm_scan`): `z`,
    /// `x | B | C` and the time steps from `ssm_in`; `x | B | C` convolved depthwise with the last `taps − 1` of them
    /// before it, biased and through SiLU; per head `h` (of group `h / (heads / groups)`) and channel `p` the state
    /// `S ← S·exp(Δ·A_h) + Δ·x·B` and the read `S·C + D_h·x`, `Δ = softplus(dt + dt_bias)`; the reads times `silu(z)`,
    /// RMS-normed group by group by `ssm_norm` where the scan has one, and projected by `ssm_out`.
    fn ssd(&mut self, l: usize, s: &Ssd, xn: &[Vec<f64>]) -> R<Vec<Vec<f64>>> {
        let (heads, hd, n, groups, taps) = (s.heads, s.head_dim, s.state, s.groups, s.conv_taps);
        let inner = heads * hd;
        let width = inner + 2 * groups * n;
        if groups == 0 || heads % groups != 0 || taps == 0 {
            return Err(ReferenceError::NotYet("a scan whose heads do not split into its groups"));
        }
        let blk = |t: &str| format!("blk.{l}.{t}");
        let proj = self.project(&blk("ssm_in.weight"), xn)?;
        let kernel = self.tensor(&blk("ssm_conv1d.weight"))?;
        let bias = self.tensor(&blk("ssm_conv1d.bias"))?;
        let dt_bias = self.tensor(&blk("ssm_dt.bias"))?;
        let a = self.tensor(&blk("ssm_a"))?;
        let skip = self.tensor(&blk("ssm_d"))?;
        let norm = if s.gated_norm { Some(self.tensor(&blk("ssm_norm.weight"))?) } else { None };
        let eps = self.spec.rms_eps;
        let LayerState::Scan { conv, state } = &mut self.states[l] else {
            unreachable!("a scan layer holds its convolution's and its scan's state")
        };
        let mut reads = Vec::with_capacity(xn.len());
        for row in &proj {
            let (z, xbc, dt) = (&row[..inner], &row[inner..inner + width], &row[inner + width..inner + width + heads]);
            let c: Vec<f64> = (0..width)
                .map(|ch| {
                    let older = conv.iter().enumerate().map(|(j, r)| r[ch] * kernel[ch * taps + j]).sum::<f64>();
                    silu(older + xbc[ch] * kernel[ch * taps + taps - 1] + bias[ch])
                })
                .collect();
            if taps > 1 {
                conv.remove(0);
                conv.push(xbc.to_vec());
            }
            let (x, b, cm) = (&c[..inner], &c[inner..inner + groups * n], &c[inner + groups * n..]);
            let mut y = vec![0f64; inner];
            for h in 0..heads {
                let delta = softplus(dt[h] + dt_bias[h]);
                let decay = (delta * a[h]).exp();
                let g = h / (heads / groups);
                for p in 0..hd {
                    let ch = h * hd + p;
                    let xd = x[ch] * delta;
                    let st = &mut state[ch * n..(ch + 1) * n];
                    let mut read = 0.0;
                    for i in 0..n {
                        st[i] = st[i] * decay + b[g * n + i] * xd;
                        read += st[i] * cm[g * n + i];
                    }
                    y[ch] = (read + x[ch] * skip[h]) * silu(z[ch]);
                }
            }
            if let Some(w) = &norm {
                let gw = inner / groups;
                for (seg, w) in y.chunks_mut(gw).zip(w.chunks(gw)) {
                    let normed = rms_norm(seg, w, eps);
                    seg.copy_from_slice(&normed);
                }
            }
            reads.push(y);
        }
        self.project(&blk("ssm_out.weight"), &reads)
    }

    /// LFM2's gated short convolution over the rows of `xn` in order (llama.cpp `models/lfm2.cpp`
    /// `build_shortconv_block`): `B`, `C` and `x` from one projection, `B ⊙ x` convolved depthwise with the last
    /// `taps − 1` of them before it, gated by `C`, projected out.
    fn short_conv(&mut self, l: usize, c: &ShortConv, xn: &[Vec<f64>]) -> R<Vec<Vec<f64>>> {
        let d = self.spec.d;
        let taps = c.taps;
        let bcx = self.project(&format!("blk.{l}.shortconv.in_proj.weight"), xn)?;
        let kernel = self.tensor(&format!("blk.{l}.shortconv.conv.weight"))?;
        let LayerState::Conv { rows } = &mut self.states[l] else {
            unreachable!("a short-convolution layer holds its convolution's state")
        };
        let mut gated = Vec::with_capacity(xn.len());
        for row in &bcx {
            let bx: Vec<f64> = (0..d).map(|ch| row[ch] * row[2 * d + ch]).collect();
            let conv: Vec<f64> = (0..d)
                .map(|ch| {
                    let older = rows.iter().enumerate().map(|(j, r)| r.get(ch).copied().unwrap_or(0.0) * kernel[ch * taps + j]).sum::<f64>();
                    older + bx[ch] * kernel[ch * taps + taps - 1]
                })
                .collect();
            rows.remove(0);
            rows.push(bx);
            gated.push((0..d).map(|ch| row[d + ch] * conv[ch]).collect::<Vec<f64>>());
        }
        self.project(&format!("blk.{l}.shortconv.out_proj.weight"), &gated)
    }

    fn ffn(&mut self, l: usize, layer: &Layer, xn: &[Vec<f64>]) -> R<Vec<Vec<f64>>> {
        match &layer.ffn {
            // relu(up)², the relu as the VM's `max(x, 0)` takes it: a NaN gives 0, and either zero squares to +0.
            Ffn::Dense { act: Act::ReluSquared, .. } => {
                let u = self.project(&format!("blk.{l}.ffn_up.weight"), xn)?;
                let h: Vec<Vec<f64>> = u
                    .iter()
                    .map(|row| {
                        row.iter()
                            .map(|&v| {
                                let r = v.max(0.0);
                                r * r
                            })
                            .collect()
                    })
                    .collect();
                self.project(&format!("blk.{l}.ffn_down.weight"), &h)
            }
            Ffn::Dense { act, .. } => {
                let mut g = self.project(&format!("blk.{l}.ffn_gate.weight"), xn)?;
                // Gemma 3n's activation sparsity: the gate less its mean and `std_mul` of its standard deviation, through
                // a relu. The deviation is the population's, as transformers takes it (`unbiased=False`); llama.cpp's
                // `gaussian_topk` divides by `n − 1`.
                if let Some(std_mul) = self.spec.parts.iter().find_map(|p| match p {
                    Part::GateSparsity { layers, std_mul } if l < *layers => Some(*std_mul),
                    _ => None,
                }) {
                    for row in &mut g {
                        let n = row.len() as f64;
                        let mean = row.iter().sum::<f64>() / n;
                        let std = (row.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / n).sqrt();
                        let cut = mean + std * std_mul;
                        row.iter_mut().for_each(|v| *v = (*v - cut).max(0.0));
                    }
                }
                let u = self.project(&format!("blk.{l}.ffn_up.weight"), xn)?;
                let h: Vec<Vec<f64>> = g.iter().zip(&u).map(|(g, u)| gated(act, g, u)).collect();
                self.project(&format!("blk.{l}.ffn_down.weight"), &h)
            }
            Ffn::Moe(m) => self.moe(l, m, xn),
            Ffn::None => Ok(vec![vec![0.0; self.spec.d]; xn.len()]),
            Ffn::ChannelMix { .. } => self.channel_mix(l, xn),
            // Gemma 4's mixture block reads its unnormed input too, which only the plain block loop hands it.
            Ffn::DenseAndMoe { .. } => Err(ReferenceError::NotYet("Gemma 4's mixture block beside alternating updates")),
        }
    }

    /// A mixture of experts as llama.cpp's `build_moe_ffn` computes it: router logits (plus a router bias), turned
    /// into probabilities by the spec's gating, the top `k` chosen, their weights renormalized (the sum clamped as
    /// llama.cpp clamps it) and scaled, each chosen expert's gated FFN (with its biases) summed by its weight.
    fn moe(&mut self, l: usize, m: &Moe, xn: &[Vec<f64>]) -> R<Vec<Vec<f64>>> {
        let logits = self.project(&format!("blk.{l}.ffn_gate_inp.weight"), xn)?;
        self.routed_experts(l, m, logits, xn, None)
    }

    /// Gemma 4's mixture block (llama.cpp `models/gemma4.cpp`, transformers `Gemma4TextDecoderLayer`): the dense GELU
    /// FFN on the FFN-normed input `xn`, normed by `post_ffw_norm_1`, beside the mixture on the input normed by
    /// `pre_ffw_norm_2`, normed by `post_ffw_norm_2`; the router reads the unnormed input `x` RMS-normed with no weight,
    /// over `√d` and times `ffn_gate_inp.scale`, its softmax's top-k renormalized, each chosen expert's output scaled by
    /// its `ffn_down_exps.scale`.
    fn dense_and_moe(&mut self, l: usize, act: &Act, m: &Moe, x: &[Vec<f64>], xn: &[Vec<f64>]) -> R<Vec<Vec<f64>>> {
        let blk = |t: &str| format!("blk.{l}.{t}");
        let g = self.project(&blk("ffn_gate.weight"), xn)?;
        let u = self.project(&blk("ffn_up.weight"), xn)?;
        let h: Vec<Vec<f64>> = g.iter().zip(&u).map(|(g, u)| gated(act, g, u)).collect();
        let dense = self.project(&blk("ffn_down.weight"), &h)?;
        let mut out = self.block_norm(&dense, &blk("post_ffw_norm_1"))?;
        let scale = self.tensor(&blk("ffn_gate_inp.scale"))?;
        let ones = vec![1.0; self.spec.d];
        let root = (self.spec.d as f64).sqrt();
        let routed: Vec<Vec<f64>> =
            x.iter().map(|r| rms_norm(r, &ones, self.spec.rms_eps).iter().zip(&scale).map(|(v, s)| v / root * s).collect()).collect();
        let logits = self.project(&blk("ffn_gate_inp.weight"), &routed)?;
        let pre = self.block_norm(x, &blk("pre_ffw_norm_2"))?;
        let per_expert = self.tensor(&blk("ffn_down_exps.scale"))?;
        let experts = self.routed_experts(l, m, logits, &pre, Some(&per_expert))?;
        let experts = self.block_norm(&experts, &blk("post_ffw_norm_2"))?;
        add_rows(&mut out, &experts);
        Ok(out)
    }

    /// The experts of a mixture over the rows of `xn`, routed by `logits`, each chosen expert's weight times its entry of
    /// `per_expert` where there is one.
    fn routed_experts(&mut self, l: usize, m: &Moe, mut logits: Vec<Vec<f64>>, xn: &[Vec<f64>], per_expert: Option<&[f64]>) -> R<Vec<Vec<f64>>> {
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
                Gating::SoftmaxOfTopK | Gating::SparseMixer { .. } => {}
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
            let mut w: Vec<f64> = match m.gating {
                Gating::SparseMixer { jitter } => sparse_mixer(&p, chosen, jitter),
                _ => chosen.iter().map(|&e| p[e]).collect(),
            };
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
            if let Some(s) = per_expert {
                w.iter_mut().zip(chosen).for_each(|(v, &e)| *v *= s[e]);
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
        // Every expert's gate and up in one pass over the threads, then every expert's down in another. A checkpoint may
        // store each expert's gate and up as one projection of `2·f` rows, the gate's first (`ffn_gate_up_exps`).
        let fused = self.g.has(&format!("blk.{l}.ffn_gate_up_exps.weight"));
        let mut ups = Vec::new();
        for (&e, rows) in &readers {
            let xs: Vec<Vec<f64>> = rows
                .iter()
                .map(|&t| if scaled_input { xn[t].iter().map(|v| v * weight(t, e)).collect() } else { xn[t].clone() })
                .collect();
            if fused {
                ups.push(self.expert(l, "ffn_gate_up_exps", e, 2 * f, &xs)?);
            } else {
                ups.push(self.expert(l, "ffn_gate_exps", e, f, &xs)?);
                ups.push(self.expert(l, "ffn_up_exps", e, f, &xs)?);
            }
        }
        let gu = products(&ups, self.threads);
        let pairs: Vec<(Vec<Vec<f64>>, Vec<Vec<f64>>)> = if fused {
            gu.into_iter().map(|rows| rows.into_iter().map(|r| (r[..f].to_vec(), r[f..].to_vec())).unzip()).collect()
        } else {
            gu.chunks_exact(2).map(|pair| (pair[0].clone(), pair[1].clone())).collect()
        };
        let mut downs = Vec::new();
        for ((&e, _), (g, u)) in readers.iter().zip(pairs) {
            let (mut g, mut u) = (g, u);
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

/// Each chosen expert's weight under PhiMoE's sparsemixer (transformers `sparsemixer` at inference): the softmax of its
/// logit `m` over the logits `x` of the experts not chosen before it that `(m − x)/max(|x|, m) ≤ 2·jitter` keeps.
fn sparse_mixer(logits: &[f64], chosen: &[usize], jitter: f64) -> Vec<f64> {
    chosen
        .iter()
        .enumerate()
        .map(|(k, &e)| {
            let m = logits[e];
            let total: f64 = (0..logits.len())
                .filter(|j| !chosen[..k].contains(j))
                .filter(|&j| !((m - logits[j]) / logits[j].abs().max(m) > 2.0 * jitter))
                .map(|j| (logits[j] - m).exp())
                .sum();
            1.0 / total
        })
        .collect()
}

/// Every value of `rows` times `f`.
fn scale_rows(rows: &mut [Vec<f64>], f: f64) {
    rows.iter_mut().flatten().for_each(|v| *v *= f);
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

/// `nn.LayerNorm`: `(x − mean)/√(var + eps)·w + b`, the variance the biased one.
fn layer_norm(x: &[f64], w: &[f64], b: Option<&[f64]>, eps: f64) -> Vec<f64> {
    let n = x.len() as f64;
    let mean = x.iter().sum::<f64>() / n;
    let var = x.iter().map(|v| (v - mean) * (v - mean)).sum::<f64>() / n;
    let scale = 1.0 / (var + eps).sqrt();
    x.iter().enumerate().map(|(i, v)| (v - mean) * scale * w[i] + b.map_or(0.0, |b| b[i])).collect()
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

/// GELU in its tanh approximation (PyTorch's `gelu(approximate="tanh")`, ggml's `gelu`).
fn gelu_tanh(g: f64) -> f64 {
    let c = (2.0 / std::f64::consts::PI).sqrt();
    0.5 * g * (1.0 + (c * (g + 0.044715 * g * g * g)).tanh())
}

fn gated(act: &Act, g: &[f64], u: &[f64]) -> Vec<f64> {
    g.iter()
        .zip(u)
        .map(|(&g, &u)| match act {
            Act::Silu => g / (1.0 + (-g).exp()) * u,
            Act::GeluTanh => gelu_tanh(g) * u,
            Act::SwigluOai { alpha, limit } => {
                let gc = g.min(*limit);
                let uc = u.clamp(-limit, *limit);
                gc / (1.0 + (-alpha * gc).exp()) * (uc + 1.0)
            }
            Act::ReluSquared => unreachable!("relu² is not gated: `ffn` squares the relu of the up projection alone"),
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
