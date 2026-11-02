//! The decode engine's model: a hybrid Gated-DeltaNet checkpoint (`qwen35`, Qwen3.5 and Qwen3.8) run over a batch of
//! sequences in the engine's arithmetic, every number from [`super::numerics`].
//!
//! Each sequence carries its own state: per Gated-DeltaNet block its conv ring and its memory, per attention block
//! its key and value caches, held in the formats the knobs name ([`EngineNumerics`]). A step takes any rows of any
//! sequences; a sequence's rows run in order through its own state, and every row's arithmetic is its own, so a
//! sequence's numbers never depend on what else rides the batch.
//!
//! The matrices run as the array runs them: a block-format row's exact sums against its activations quantized as the
//! knob names, per 32 elements or per the matrix's own block (`numerics::dot`), an f32 matrix's dot in the vector
//! units' lanes.
//!
//! A step is the checkpoint's program at the model's knob point ([`super::program::lower`]: the model's one program,
//! read off its step graph), its commands run in order over the step's token-rows: the model is the interpreter of the
//! program the engine's sequencer issues.

use std::collections::HashMap;
use std::sync::Arc;

use crate::model::quant::{f16_to_f32, GgmlType};
use crate::model::reference::WeightSource;
use crate::model::spec::{Ffn, Layer, Mixer, ModelSpec, Rope, RopeScaling};
use crate::spatial::serving::{AttentionMath, StateFlush};

use super::numerics::{
    attend_int8, attend_vector, cache_row, f32_to_f16, fdot, gdn_flush, gdn_fold_trailing, gdn_read, gdn_row, hadamard, ldexp,
    quantize, quantize_blocks, ActRow, ActScaling, CodedRow, FpUnit, Logged, Words, GROUP,
};
use super::program::{Command, Program, Slot};
use super::vector;

/// How a sequence's recurrent memory and conv ring are stored between tokens.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StateFormat {
    F32,
    F16,
}

/// How an attention block's keys and values are stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CacheFormat {
    /// Each element binary16.
    F16,
    /// Each head row of a position int8 codes under one f16 scale ([`cache_row`]).
    Int8,
    /// Each head row rotated by the Walsh–Hadamard transform ([`hadamard`]) before its int8 codes, so a few large
    /// channels spread across the row and its one scale fits every element; the query rotated alike (a score is the
    /// rotated dot over the head's length) and the weighted sum of values rotated back.
    Int8Hadamard,
}

impl CacheFormat {
    /// Whether the cache holds int8 codes, so attention can run on the array.
    pub fn is_int8(self) -> bool {
        matches!(self, CacheFormat::Int8 | CacheFormat::Int8Hadamard)
    }
}

/// The knobs of the engine's arithmetic.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct EngineNumerics {
    pub activations: Words,
    /// How the activations a matrix reads are scaled: per 32 elements, or per the matrix format's own block.
    pub scaling: ActScaling,
    pub state: StateFormat,
    pub cache: CacheFormat,
    pub attention: AttentionMath,
    /// How a Gated-DeltaNet memory is carried: rewritten every token, or held at its last flush with a log of the
    /// tokens since, read through the log and flushed once every `tokens`, or a window of `tokens` behind ([`Replay`]).
    pub flush: StateFlush,
    /// The multiplier every f32 product is computed by: the engine's own, or the card's floating-point DSP block.
    pub fp_unit: FpUnit,
}

impl EngineNumerics {
    /// The most precise knob point: two-word activations scaled per 32 elements, an f32 state, an f16 cache, attention
    /// on the vector units.
    pub fn precise() -> EngineNumerics {
        EngineNumerics {
            activations: Words::Two,
            scaling: ActScaling::Sub32,
            state: StateFormat::F32,
            cache: CacheFormat::F16,
            attention: AttentionMath::Vector,
            flush: StateFlush::EveryPass,
            fp_unit: FpUnit::Soft,
        }
    }

    /// The fastest knob point the serving roofline prices: one-word activations scaled per weight block, an f16 state
    /// flushed every 16 tokens, a rotated int8 cache, attention on the array.
    pub fn fastest() -> EngineNumerics {
        EngineNumerics {
            activations: Words::One,
            scaling: ActScaling::PerBlock,
            state: StateFormat::F16,
            cache: CacheFormat::Int8Hadamard,
            attention: AttentionMath::Array,
            flush: StateFlush::Every { tokens: FLUSH_TOKENS },
            fp_unit: FpUnit::Soft,
        }
    }

    /// The knob points held to the real checkpoints, both word counts at their most and least precise storage:
    /// [`precise`](Self::precise); two words over an f16 state and an int8 cache on the array; one word over an f32
    /// state and an f16 cache; [`fastest`](Self::fastest).
    pub fn real_points() -> [EngineNumerics; 4] {
        let at = |activations, state, cache, attention| EngineNumerics {
            activations,
            scaling: ActScaling::Sub32,
            state,
            cache,
            attention,
            flush: StateFlush::EveryPass,
            fp_unit: FpUnit::Soft,
        };
        [
            EngineNumerics::precise(),
            at(Words::Two, StateFormat::F16, CacheFormat::Int8, AttentionMath::Array),
            at(Words::One, StateFormat::F32, CacheFormat::F16, AttentionMath::Vector),
            EngineNumerics::fastest(),
        ]
    }

    /// The knob points a real checkpoint of `spec` is held to: [`real_points`](Self::real_points) when every attention
    /// layer norms its queries and keys, and otherwise the same points with the two-word point's int8 cache rotated and
    /// the family's [`fastest_for`](Self::fastest_for): un-normed keys carry outlier channels an unrotated int8 cache
    /// loses and the rotation spreads. Measured on Llama-3.2-1B against llama.cpp's runs of its corpus: two words on an
    /// int8 cache followed the f32 run 458 steps to the shipped run's 462, rotated 501.
    pub fn real_points_for(spec: &ModelSpec) -> [EngineNumerics; 4] {
        let mut points = Self::real_points();
        if !keys_normed(spec) {
            points[1].cache = CacheFormat::Int8Hadamard;
            points[3] = Self::fastest_for(spec);
        }
        points
    }

    /// The fastest knob point `spec`'s family is held to: [`fastest`](Self::fastest), its attention on the vector units
    /// when a layer's keys are not normed (Llama-3.2-1B's logits after the prompt sat 0.098 from llama.cpp's f32 run's
    /// on the vector units, within the shipped run's 0.080 by two standard errors, and 0.112 on the array).
    pub fn fastest_for(spec: &ModelSpec) -> EngineNumerics {
        let fastest = Self::fastest();
        if keys_normed(spec) {
            fastest
        } else {
            EngineNumerics { attention: AttentionMath::Vector, ..fastest }
        }
    }

    /// Every knob point the engine runs.
    pub fn all() -> Vec<EngineNumerics> {
        let mut out = Vec::new();
        for activations in [Words::One, Words::Two] {
            for scaling in [ActScaling::Sub32, ActScaling::PerBlock] {
                for state in [StateFormat::F32, StateFormat::F16] {
                    for cache in [CacheFormat::F16, CacheFormat::Int8, CacheFormat::Int8Hadamard] {
                        for attention in [AttentionMath::Vector, AttentionMath::Array] {
                            if attention == AttentionMath::Array && !cache.is_int8() {
                                continue;
                            }
                            for flush in [StateFlush::EveryPass, StateFlush::Every { tokens: FLUSH_TOKENS }, StateFlush::Trailing { tokens: TRAILING_TOKENS }] {
                                out.push(EngineNumerics { activations, scaling, state, cache, attention, flush, fp_unit: FpUnit::Soft });
                            }
                        }
                    }
                }
            }
        }
        out
    }
}

/// Whether every attention layer of `spec` norms its queries and keys (a model with none, vacuously).
fn keys_normed(spec: &ModelSpec) -> bool {
    spec.layers.iter().all(|l| match &l.mixer {
        Mixer::Attention(a) => a.qk_norm,
        _ => true,
    })
}

/// Tokens a lazily flushed memory carries in its log between flushes.
pub const FLUSH_TOKENS: u32 = 16;

/// Tokens of each window a trailing flush replays: half of [`FLUSH_TOKENS`], so its log, two windows at its longest,
/// is as long as a flush's every [`FLUSH_TOKENS`].
pub const TRAILING_TOKENS: u32 = FLUSH_TOKENS / 2;

/// What a lazily flushed head does at a token whose log, the token's own entry appended, holds `count` tokens
/// ([`Replay::at`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Replay {
    /// The log's oldest tokens being replayed into the memory: rows below `replayed` are, and read through the fold
    /// over the tokens after these alone; every other row reads through the whole log.
    pub older: usize,
    pub replayed: usize,
    /// The rows the token replays the log's first `by` tokens into, each stored once.
    pub rows: std::ops::Range<usize>,
    pub by: usize,
    /// Tokens dropped from the log's front once the token is through.
    pub drop: usize,
}

impl Replay {
    /// The token of a `width`-row head whose log holds `count` tokens under `flush`. Every `tokens`: every row reads
    /// through the whole log, and once it holds `tokens` every row replays it and it empties. Trailing `tokens`: once
    /// the log holds a window past the first, the token at `count` replays the first window into the rows of share
    /// `j = count − tokens − 1`, from row `j·width/tokens` to row `(j+1)·width/tokens`; the rows of the shares before
    /// read through the newer tokens; at `2·tokens` every row has replayed it and the window is dropped.
    pub fn at(flush: StateFlush, width: usize, count: usize) -> Replay {
        match flush {
            StateFlush::EveryPass => unreachable!("a memory rewritten every pass keeps no log"),
            StateFlush::Every { tokens } => {
                let full = count >= tokens as usize;
                Replay { older: 0, replayed: 0, rows: if full { 0..width } else { 0..0 }, by: count, drop: if full { count } else { 0 } }
            }
            StateFlush::Trailing { tokens } => {
                let t = tokens as usize;
                if count <= t {
                    return Replay { older: 0, replayed: 0, rows: 0..0, by: 0, drop: 0 };
                }
                let j = count - t - 1;
                let (lo, hi) = (j * width / t, (j + 1) * width / t);
                Replay { older: t, replayed: lo, rows: lo..hi, by: t, drop: if count == 2 * t { t } else { 0 } }
            }
        }
    }

    /// The tokens a lazily flushed memory's log holds at its longest under `flush`.
    pub fn capacity(flush: StateFlush) -> usize {
        match flush {
            StateFlush::EveryPass => 0,
            StateFlush::Every { tokens } => tokens as usize,
            StateFlush::Trailing { tokens } => 2 * tokens as usize,
        }
    }
}

/// Why the engine's model cannot run a checkpoint.
#[derive(Debug)]
pub enum EngineError {
    Io(std::io::Error),
    /// A feature of the architecture the engine does not run yet, by name.
    NotYet(&'static str),
    /// A matrix in a format the array does not read and the vector units do not hold.
    Undecoded { tensor: String, ty: GgmlType },
    /// A block whose scale is not finite.
    Unreadable(String),
    /// A sequence the model does not hold.
    NoSequence(usize),
    /// A run past the checkpoint's context.
    PastContext { sequence: usize, position: usize },
    /// A rotation scaled by frequency factors whose table (`rope_freqs.weight`) holds `found` factors for its `pairs`.
    Factors { found: usize, pairs: usize },
    /// No program for the model: its one program does not lower to the engine.
    Program(super::program::ProgramError),
}

impl From<std::io::Error> for EngineError {
    fn from(e: std::io::Error) -> Self {
        EngineError::Io(e)
    }
}

impl From<super::program::ProgramError> for EngineError {
    fn from(e: super::program::ProgramError) -> Self {
        EngineError::Program(e)
    }
}

type R<T> = Result<T, EngineError>;

/// One position's head rows of an attention cache: as stored, and as the vector units read them, decoded once.
#[derive(Clone)]
struct CacheRows {
    stored: Stored,
    values: Vec<Vec<f32>>,
}

/// A position's head rows as the cache stores them.
#[derive(Clone)]
enum Stored {
    F16(Vec<Vec<u16>>),
    Int8(Vec<(u16, Vec<i8>)>),
}

impl CacheRows {
    fn of(heads: &[&[f32]], format: CacheFormat, fp: FpUnit) -> CacheRows {
        let stored = match format {
            CacheFormat::F16 => Stored::F16(heads.iter().map(|h| h.iter().map(|&v| f32_to_f16(v)).collect()).collect()),
            CacheFormat::Int8 | CacheFormat::Int8Hadamard => Stored::Int8(heads.iter().map(|h| cache_row(fp, h)).collect()),
        };
        let values = match &stored {
            Stored::F16(rows) => rows.iter().map(|r| r.iter().map(|&h| f16_to_f32(h)).collect()).collect(),
            Stored::Int8(rows) => rows
                .iter()
                .map(|(s, codes)| {
                    let s = f16_to_f32(*s);
                    codes.iter().map(|&c| fp.mul(c as f32, s)).collect()
                })
                .collect(),
        };
        CacheRows { stored, values }
    }

    /// Head `g`'s row as the vector units read it.
    fn values(&self, g: usize) -> &[f32] {
        &self.values[g]
    }

    /// Head `g`'s int8 row as the array reads it: its scale's bits and its codes.
    fn int8_row(&self, g: usize) -> (u16, &[i8]) {
        let (s, codes) = self.int8(g);
        (*s, codes.as_slice())
    }

    /// Head `g`'s int8 row and its scale, as the array reads it.
    fn int8(&self, g: usize) -> &(u16, Vec<i8>) {
        match &self.stored {
            Stored::Int8(rows) => &rows[g],
            Stored::F16(_) => unreachable!("attention on the array reads an int8 cache"),
        }
    }
}

/// One token's entry in a lazily flushed memory's log: every key group's key, every value head's value, the log of
/// its decay and its gate, as stored. The decay is logged as its exponent `softplus(α + dt)·a`, whose stored
/// significand holds the decay's distance from 1 to its own precision: a decay near 1 stored as itself would keep
/// only the eleven bits of `1 − decay` an f16 has below 1.
#[derive(Clone)]
struct LogEntry {
    k: Vec<f32>,
    v: Vec<f32>,
    log_decay: Vec<f32>,
    beta: Vec<f32>,
}

/// What one block carries from position to position for one sequence.
#[derive(Clone)]
enum Carried {
    Delta { ring: Vec<Vec<f32>>, memory: Vec<f32>, log: Vec<LogEntry> },
    Attention { keys: Vec<CacheRows>, values: Vec<CacheRows> },
}

impl Carried {
    fn fresh(layer: &Layer) -> Carried {
        match &layer.mixer {
            Mixer::GatedDelta(g) => {
                let channels = 2 * g.k_heads * g.head_dim + g.v_heads * g.head_dim;
                Carried::Delta {
                    ring: vec![vec![0.0; channels]; g.conv_taps.saturating_sub(1)],
                    memory: vec![0.0; g.v_heads * g.head_dim * g.head_dim],
                    log: Vec::new(),
                }
            }
            _ => Carried::Attention { keys: Vec::new(), values: Vec::new() },
        }
    }
}

struct Sequence {
    position: usize,
    layers: Vec<Carried>,
}

/// Rows of a matrix a projection carries against its token-rows at once.
const ROW_TILE: usize = 16;

/// The default bytes of tensor data the model keeps in memory.
pub const CACHE_BUDGET: usize = 24 << 30;

/// The default bytes of decoded rows the model keeps: a coded weight takes four bytes.
pub const CODED_BUDGET: usize = 16 << 30;

/// A hybrid Gated-DeltaNet checkpoint in the engine's arithmetic.
pub struct EngineModel {
    pub spec: ModelSpec,
    pub numerics: EngineNumerics,
    source: Box<dyn WeightSource>,
    tensors: HashMap<String, Arc<Vec<u8>>>,
    cached: usize,
    budget: usize,
    /// Matrices' coded rows, decoded once while they fit `coded_budget`; a matrix past it is decoded each use.
    coded: HashMap<String, Arc<Vec<CodedRow>>>,
    coded_bytes: usize,
    coded_budget: usize,
    sequences: Vec<Sequence>,
    threads: usize,
    /// The step this model runs, at its knob point.
    program: Arc<Program>,
}

/// Whether the engine runs `spec` at `numerics`, or what of it the engine does not run yet, by name.
pub fn runs(spec: &ModelSpec, numerics: EngineNumerics) -> R<()> {
    if numerics.attention == AttentionMath::Array && !numerics.cache.is_int8() {
        return Err(EngineError::NotYet("attention on the array over a cache that is not int8"));
    }
    if spec.embedding_scale.is_some() || spec.final_logit_softcap.is_some() {
        return Err(EngineError::NotYet("an embedding scale or a final softcap"));
    }
    for layer in &spec.layers {
        match &layer.mixer {
            Mixer::GatedDelta(_) => {}
            Mixer::Attention(a) => {
                if a.fused_qkv || a.bias || a.output_bias || a.sinks || a.softcap.is_some() || a.window.is_some() {
                    return Err(EngineError::NotYet("a fused, biased, windowed, softcapped or sinked attention"));
                }
                if a.qk_norm_whole {
                    return Err(EngineError::NotYet("a QK norm across every head"));
                }
                if numerics.cache == CacheFormat::Int8Hadamard && !(a.head_dim.is_power_of_two() && a.value_dim.is_power_of_two()) {
                    return Err(EngineError::NotYet("a rotated cache of heads whose width is not a power of two"));
                }
            }
            Mixer::Mla(_) => return Err(EngineError::NotYet("multi-head latent attention")),
        }
        if !matches!(layer.ffn, Ffn::Dense { act: crate::model::spec::Act::Silu, .. }) {
            return Err(EngineError::NotYet("a mixture of experts or a non-SiLU FFN"));
        }
        if layer.post_mixer_norm || layer.post_ffn_norm {
            return Err(EngineError::NotYet("a sandwich norm"));
        }
    }
    Ok(())
}

impl EngineModel {
    pub fn new(spec: ModelSpec, source: impl WeightSource + 'static, numerics: EngineNumerics) -> R<EngineModel> {
        runs(&spec, numerics)?;
        let threads = std::thread::available_parallelism().map_or(4, |n| n.get());
        let program = Arc::new(super::program::lower(&spec, numerics, spec.ctx_len)?.program);
        Ok(EngineModel {
            spec,
            numerics,
            source: Box::new(source),
            tensors: HashMap::new(),
            cached: 0,
            budget: CACHE_BUDGET,
            coded: HashMap::new(),
            coded_bytes: 0,
            coded_budget: CODED_BUDGET,
            sequences: Vec::new(),
            threads,
            program,
        })
    }

    /// A fresh sequence at position 0; its handle.
    pub fn sequence(&mut self) -> usize {
        let layers = self.spec.layers.iter().map(Carried::fresh).collect();
        self.sequences.push(Sequence { position: 0, layers });
        self.sequences.len() - 1
    }

    pub fn position(&self, sequence: usize) -> usize {
        self.sequences[sequence].position
    }

    /// The step this model runs: the program [`step`](Self::step) interprets.
    pub fn program(&self) -> &Program {
        &self.program
    }

    /// A Gated-DeltaNet block's memory for a sequence, as stored at its last write, and the tokens its log holds.
    pub fn memory(&self, sequence: usize, layer: usize) -> Option<(&[f32], usize)> {
        match &self.sequences.get(sequence)?.layers.get(layer)? {
            Carried::Delta { memory, log, .. } => Some((memory, log.len())),
            Carried::Attention { .. } => None,
        }
    }

    fn bytes(&mut self, name: &str) -> R<Arc<Vec<u8>>> {
        if let Some(b) = self.tensors.get(name) {
            return Ok(b.clone());
        }
        let (_, dims) = self.source.describe(name)?;
        let row = dims.first().copied().unwrap_or(1).max(1) as usize;
        let rows = dims.iter().product::<u64>() as usize / row;
        let bytes = Arc::new(self.source.row_bytes(name, 0, rows)?);
        if self.cached + bytes.len() <= self.budget {
            self.cached += bytes.len();
            self.tensors.insert(name.to_string(), bytes.clone());
        }
        Ok(bytes)
    }

    /// A vector tensor's values in f32.
    fn vector(&mut self, name: &str) -> R<Vec<f32>> {
        let (ty, _) = self.source.describe(name)?;
        let raw = self.bytes(name)?;
        let mut out = Vec::new();
        ty.dequantize(&raw, &mut out);
        Ok(out)
    }

    /// Row `r` of a table, each element its exact weight rounded to f32 (which is ggml's dequantized value).
    fn table_row(&mut self, name: &str, r: usize) -> R<Vec<f32>> {
        let (ty, dims) = self.source.describe(name)?;
        let raw = self.bytes(name)?;
        let row = ty.bytes_for(dims[0] as usize);
        let mut out = Vec::new();
        ty.dequantize(&raw[r * row..(r + 1) * row], &mut out);
        Ok(out)
    }

    /// `W·x` for every row of `xs`, as the array (a block-format `W`) or the vector units (a float `W`) compute it.
    fn project(&mut self, name: &str, xs: &[Vec<f32>]) -> R<Vec<Vec<f32>>> {
        let fp = self.numerics.fp_unit;
        let (ty, dims) = self.source.describe(name)?;
        let [in_f, out_f] = dims[..] else { return Err(EngineError::NotYet("a projection that is not a matrix")) };
        let (in_f, out_f) = (in_f as usize, out_f as usize);
        let raw = self.bytes(name)?;
        let row_bytes = ty.bytes_for(in_f);
        enum Input {
            Coded(Vec<ActRow>),
            Float(Vec<Vec<f32>>),
        }
        let input = match ty {
            GgmlType::F32 | GgmlType::F16 | GgmlType::Bf16 => Input::Float(xs.to_vec()),
            _ if super::numerics::reads(ty) => {
                let words = self.numerics.activations;
                match self.numerics.scaling {
                    ActScaling::Sub32 => {
                        if in_f % GROUP != 0 {
                            return Err(EngineError::NotYet("a matrix row that is not a whole number of 256-element groups"));
                        }
                        Input::Coded(xs.iter().map(|x| quantize(x, words)).collect())
                    }
                    ActScaling::PerBlock => Input::Coded(xs.iter().map(|x| quantize_blocks(x, words, ty.block_len())).collect()),
                }
            }
            _ => return Err(EngineError::Undecoded { tensor: name.to_string(), ty }),
        };
        let coded = match &input {
            Input::Coded(_) => self.coded_rows(name, ty, &raw, out_f, row_bytes)?,
            Input::Float(_) => None,
        };
        // A thread for every million multiply-adds, up to the threads there are: a small matrix runs on one.
        let workers = (out_f * in_f * xs.len() / (1 << 20)).clamp(1, self.threads);
        let chunk = out_f.div_ceil(workers).max(1);
        let results: Vec<R<Vec<Vec<f32>>>> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..out_f)
                .step_by(chunk)
                .map(|first| {
                    let (raw, input, coded) = (&raw, &input, &coded);
                    scope.spawn(move || -> R<Vec<Vec<f32>>> {
                        let last = (first + chunk).min(out_f);
                        let mut out = vec![Vec::with_capacity(last - first); xs.len()];
                        // A tile of rows against every token-row in turn, so each token-row's codes are read once a
                        // tile and the tile's rows stay in cache across the token-rows.
                        for tile in (first..last).step_by(ROW_TILE) {
                            let end = (tile + ROW_TILE).min(last);
                            match input {
                                Input::Coded(acts) => {
                                    let decoded: Vec<CodedRow> = match coded {
                                        Some(_) => Vec::new(),
                                        None => (tile..end)
                                            .map(|r| {
                                                CodedRow::decode(ty, &raw[r * row_bytes..(r + 1) * row_bytes])
                                                    .expect("a format the array reads")
                                                    .map_err(|_| EngineError::Unreadable(format!("{name} row {r}")))
                                            })
                                            .collect::<R<_>>()?,
                                    };
                                    let rows: &[CodedRow] = match coded {
                                        Some(rows) => &rows[tile..end],
                                        None => &decoded,
                                    };
                                    for (t, a) in acts.iter().enumerate() {
                                        for row in rows {
                                            out[t].push(super::numerics::dot(row, a));
                                        }
                                    }
                                }
                                Input::Float(rows) => {
                                    let ws: Vec<Vec<f32>> = (tile..end)
                                        .map(|r| {
                                            let mut w = Vec::with_capacity(in_f);
                                            ty.dequantize(&raw[r * row_bytes..(r + 1) * row_bytes], &mut w);
                                            w
                                        })
                                        .collect();
                                    for (t, x) in rows.iter().enumerate() {
                                        for w in &ws {
                                            out[t].push(fdot(fp, w, x));
                                        }
                                    }
                                }
                            }
                        }
                        Ok(out)
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().expect("a projection thread")).collect()
        });
        let mut out = vec![Vec::with_capacity(out_f); xs.len()];
        for part in results {
            for (t, rows) in part?.into_iter().enumerate() {
                out[t].extend(rows);
            }
        }
        Ok(out)
    }

    /// A matrix's coded rows from the cache, or decoded across the threads and cached when they fit its budget; `None`
    /// when they do not fit, so each use decodes its rows.
    fn coded_rows(&mut self, name: &str, ty: GgmlType, raw: &[u8], rows: usize, row_bytes: usize) -> R<Option<Arc<Vec<CodedRow>>>> {
        if let Some(c) = self.coded.get(name) {
            return Ok(Some(c.clone()));
        }
        let elems = rows * ty.block_len() * (row_bytes / ty.block_bytes());
        if self.coded_bytes + 4 * elems > self.coded_budget {
            return Ok(None);
        }
        let chunk = rows.div_ceil(self.threads).max(1);
        let parts: Vec<R<Vec<CodedRow>>> = std::thread::scope(|scope| {
            let handles: Vec<_> = (0..rows)
                .step_by(chunk)
                .map(|first| {
                    scope.spawn(move || -> R<Vec<CodedRow>> {
                        (first..(first + chunk).min(rows))
                            .map(|r| {
                                CodedRow::decode(ty, &raw[r * row_bytes..(r + 1) * row_bytes])
                                    .expect("a format the array reads")
                                    .map_err(|_| EngineError::Unreadable(format!("{name} row {r}")))
                            })
                            .collect()
                    })
                })
                .collect();
            handles.into_iter().map(|h| h.join().expect("a decoding thread")).collect()
        });
        let mut all = Vec::with_capacity(rows);
        for p in parts {
            all.extend(p?);
        }
        let all = Arc::new(all);
        self.coded_bytes += 4 * elems;
        self.coded.insert(name.to_string(), all.clone());
        Ok(Some(all))
    }

    /// Run each listed sequence's tokens at its next positions; returns, per entry, the logits after each token.
    ///
    /// The step is the program's commands in order over the step's rows ([`super::program`]): every row of every
    /// entry, an entry's rows in its tokens' order.
    pub fn step(&mut self, rows: &[(usize, Vec<u32>)]) -> R<Vec<Vec<Vec<f32>>>> {
        let mut seen = std::collections::HashSet::new();
        for (s, _) in rows {
            if !seen.insert(*s) {
                return Err(EngineError::NotYet("one sequence twice in a step"));
            }
        }
        for (s, toks) in rows {
            let seq = self.sequences.get(*s).ok_or(EngineError::NoSequence(*s))?;
            if seq.position + toks.len() > self.spec.ctx_len {
                return Err(EngineError::PastContext { sequence: *s, position: seq.position + toks.len() });
            }
        }
        let mut plan = Vec::new();
        for (e, (s, toks)) in rows.iter().enumerate() {
            for (i, &t) in toks.iter().enumerate() {
                plan.push(Row { entry: e, sequence: *s, position: self.sequences[*s].position + i, token: t });
            }
        }
        let program = self.program.clone();
        let mut store = Store::default();
        for command in &program.commands {
            self.run(command, &plan, &mut store)?;
        }
        let mut out: Vec<Vec<Vec<f32>>> = rows.iter().map(|_| Vec::new()).collect();
        for (logits, row) in store.take(Slot::Logits).into_iter().zip(&plan) {
            out[row.entry].push(logits);
        }
        Ok(out)
    }

    /// Greedy continuation of one sequence: the prompt in one step, then `n` tokens a step each.
    pub fn generate(&mut self, sequence: usize, prompt: &[u32], n: usize) -> R<Vec<u32>> {
        let mut logits = self.step(&[(sequence, prompt.to_vec())])?.remove(0).pop().expect("a prompt token");
        let mut out = Vec::with_capacity(n);
        for i in 0..n {
            let t = argmax(&logits);
            out.push(t);
            if i + 1 < n {
                logits = self.step(&[(sequence, vec![t])])?.remove(0).pop().expect("one token");
            }
        }
        Ok(out)
    }

    /// One command over every row of the step, in the step's row order.
    fn run(&mut self, command: &Command, plan: &[Row], store: &mut Store) -> R<()> {
        let (eps, fp) = (self.spec.rms_eps as f32, self.numerics.fp_unit);
        match command {
            Command::Embed { table } => {
                let rows = plan.iter().map(|row| self.table_row(table, row.token as usize)).collect::<R<Vec<_>>>()?;
                store.put(Slot::Residual, rows);
            }
            Command::Norm { weight, from, to } => {
                let w = self.vector(weight)?;
                let program = vector::rmsnorm(fp, 0, 1, 2, 3, w.len(), eps);
                let rows = store.get(*from).iter().map(|x| simd(fp, &program, [x.clone(), w.clone()], 2)).collect();
                store.put(*to, rows);
            }
            Command::Matvec { tensor, from, to } => {
                let rows = self.project(tensor, store.get(*from))?;
                store.put(*to, rows);
            }
            Command::Add { from, to } => {
                let add = [vector::Op::Vec { op: vector::Bin::Add, a: 0, b: 1, d: 2 }];
                let rows = store.get(*to).iter().zip(store.get(*from)).map(|(x, y)| simd(fp, &add, [x.clone(), y.clone()], 2)).collect();
                store.put(*to, rows);
            }
            Command::SwiGlu { gate, up, to } => {
                let program = vector::swiglu(0, 1, 2);
                let rows = store.get(*gate).iter().zip(store.get(*up)).map(|(g, u)| simd(fp, &program, [g.clone(), u.clone()], 2)).collect();
                store.put(*to, rows);
            }
            Command::Conv { layer, kernel, taps, from, to } => {
                let kernel = self.vector(kernel)?;
                let taps = *taps;
                let channels = kernel.len() / taps;
                // The kernel tap by tap, as the vector unit reads it.
                let by_tap: Vec<Vec<f32>> = (0..taps).map(|j| (0..channels).map(|ch| kernel[ch * taps + j]).collect()).collect();
                let ring_regs: Vec<vector::VReg> = (0..taps as u8 - 1).collect();
                let x_reg = taps as u8 - 1;
                let tap_regs: Vec<vector::VReg> = (taps as u8..2 * taps as u8).collect();
                let (acc, conv_out) = (2 * taps as u8, 2 * taps as u8 + 1);
                let program = vector::conv(&ring_regs, x_reg, &tap_regs, acc, conv_out);
                let stored_as = match self.numerics.state {
                    StateFormat::F32 => None,
                    StateFormat::F16 => Some([vector::Op::Map { f: vector::Func::F16, a: 0, d: 1 }]),
                };
                let mut out = Vec::with_capacity(plan.len());
                for (x, row) in store.get(*from).iter().zip(plan) {
                    let stored = match &stored_as {
                        None => x.clone(),
                        Some(p) => simd(fp, p, [x.clone()], 1),
                    };
                    let Carried::Delta { ring, .. } = &mut self.sequences[row.sequence].layers[*layer] else { unreachable!("a delta block") };
                    let inputs: Vec<Vec<f32>> = ring.iter().cloned().chain(std::iter::once(x.clone())).chain(by_tap.iter().cloned()).collect();
                    out.push(simd(fp, &program, inputs, conv_out));
                    if taps > 1 {
                        ring.remove(0);
                        ring.push(stored);
                    }
                }
                store.put(*to, out);
            }
            Command::L2Heads { from, at, heads, width, to } => {
                let program = vector::l2norm(0, 1, eps);
                let rows = store
                    .get(*from)
                    .iter()
                    .map(|c| (0..*heads).flat_map(|h| simd(fp, &program, [c[at + h * width..at + (h + 1) * width].to_vec()], 1)).collect())
                    .collect();
                store.put(*to, rows);
            }
            Command::Gates { alpha, beta, dt, a, log_decay, decay, gate } => {
                let (dt, a) = (self.vector(dt)?, self.vector(a)?);
                let program = vector::delta_gates(0, 1, 2, 3, 4, 5, 6);
                let (mut exponents, mut decays, mut gates) = (Vec::new(), Vec::new(), Vec::new());
                for (al, be) in store.get(*alpha).iter().zip(store.get(*beta)) {
                    let mut regs = vector::Regs::new(16, 8);
                    regs.v[..4].clone_from_slice(&[al.clone(), dt.clone(), a.clone(), be.clone()]);
                    vector::run(fp, &program, &mut regs);
                    exponents.push(std::mem::take(&mut regs.v[4]));
                    decays.push(std::mem::take(&mut regs.v[5]));
                    gates.push(std::mem::take(&mut regs.v[6]));
                }
                store.put(*log_decay, exponents);
                store.put(*decay, decays);
                store.put(*gate, gates);
            }
            Command::Delta { layer, query, key, value, value_at, log_decay, decay, gate, k_heads, v_heads, width, to } => {
                let reads = self.delta(*layer, plan, store, [*query, *key, *value, *log_decay, *decay, *gate], *value_at, (*k_heads, *v_heads, *width));
                store.put(*to, reads);
            }
            Command::GatedNorm { weight, from, z, width, to } => {
                let norm = self.vector(weight)?;
                let program = vector::gdn_out(fp, 0, 1, 2, 3, 4, *width, eps);
                let rows = store
                    .get(*from)
                    .iter()
                    .zip(store.get(*z))
                    .map(|(o, z)| {
                        o.chunks(*width)
                            .zip(z.chunks(*width))
                            .flat_map(|(oh, zh)| simd(fp, &program, [oh.to_vec(), norm.clone(), zh.to_vec()], 3))
                            .collect()
                    })
                    .collect();
                store.put(*to, rows);
            }
            Command::Split { from, heads, width, query, gate } => {
                let (mut qs, mut gs) = (Vec::with_capacity(plan.len()), Vec::with_capacity(plan.len()));
                for p in store.get(*from) {
                    qs.push((0..*heads).flat_map(|h| p[h * 2 * width..h * 2 * width + width].iter().copied()).collect());
                    gs.push((0..*heads).flat_map(|h| p[h * 2 * width + width..(h + 1) * 2 * width].iter().copied()).collect());
                }
                store.put(*query, qs);
                store.put(*gate, gs);
            }
            Command::RmsHeads { weight, slot, width } => {
                let w = self.vector(weight)?;
                let program = vector::rmsnorm(fp, 0, 1, 2, 3, *width, eps);
                let mut rows = store.take(*slot);
                for head in rows.iter_mut().flat_map(|r| r.chunks_mut(*width)) {
                    let normed = simd(fp, &program, [head.to_vec(), w.clone()], 2);
                    head.copy_from_slice(&normed);
                }
                store.put(*slot, rows);
            }
            Command::Rotate { slot, width, rope } => {
                let program = vector::rotary(0, 1, 2, 3, rope.neox);
                let factors = match rope.scaling {
                    RopeScaling::Factors => Some(factors_for(rope, self.vector(FACTORS)?)?),
                    _ => None,
                };
                let mut rows = store.take(*slot);
                for (r, row) in rows.iter_mut().zip(plan) {
                    let (cos, sin) = rotation_table(row.position, rope, factors.as_deref());
                    for head in r.chunks_mut(*width) {
                        let turned = simd(fp, &program, [head.to_vec(), cos.clone(), sin.clone()], 3);
                        head.copy_from_slice(&turned);
                    }
                }
                store.put(*slot, rows);
            }
            Command::Hadamard { slot, width } => {
                let program = [vector::Op::Hadamard { a: 0, d: 1 }];
                let mut rows = store.take(*slot);
                for head in rows.iter_mut().flat_map(|r| r.chunks_mut(*width)) {
                    let turned = simd(fp, &program, [head.to_vec()], 1);
                    head.copy_from_slice(&turned);
                }
                store.put(*slot, rows);
            }
            Command::CacheAppend { layer, key, value, kv_heads, width, value_width } => {
                let cache = self.numerics.cache;
                for ((k, v), row) in store.get(*key).iter().zip(store.get(*value)).zip(plan) {
                    let (kh, vh): (Vec<&[f32]>, Vec<&[f32]>) = (k.chunks(*width).collect(), v.chunks(*value_width).collect());
                    assert!(kh.len() == *kv_heads && vh.len() == *kv_heads, "a row of {kv_heads} cached heads");
                    let Carried::Attention { keys, values } = &mut self.sequences[row.sequence].layers[*layer] else { unreachable!("an attention block") };
                    keys.push(CacheRows::of(&kh, cache, fp));
                    values.push(CacheRows::of(&vh, cache, fp));
                }
            }
            Command::Attend { layer, query, heads, kv_heads, width, value_width, scale, to } => {
                let (hd, vd) = (*width, *value_width);
                let group = heads / kv_heads;
                let (math, words, scaling) = (self.numerics.attention, self.numerics.activations, self.numerics.scaling);
                let rotated = self.numerics.cache == CacheFormat::Int8Hadamard;
                // A rotated score is the dot of rotated rows, `hd` times the plain one: the scale takes `2^-log2 hd`.
                let score_scale = if rotated { ldexp(*scale, -(hd.trailing_zeros() as i32)) } else { *scale };
                let mut outs = Vec::with_capacity(plan.len());
                for (q, row) in store.get(*query).iter().zip(plan) {
                    let Carried::Attention { keys: kc, values: vc } = &self.sequences[row.sequence].layers[*layer] else { unreachable!("an attention block") };
                    let mut out = vec![0f32; heads * vd];
                    for h in 0..*heads {
                        let g = h / group;
                        let qh = &q[h * hd..(h + 1) * hd];
                        let mut oh = match math {
                            AttentionMath::Vector => {
                                let keys: Vec<&[f32]> = (0..=row.position).map(|j| kc[j].values(g)).collect();
                                let values: Vec<&[f32]> = (0..=row.position).map(|j| vc[j].values(g)).collect();
                                attend_vector(fp, qh, &keys, &values, score_scale)
                            }
                            AttentionMath::Array => {
                                let keys: Vec<(u16, &[i8])> = (0..=row.position).map(|j| kc[j].int8_row(g)).collect();
                                let values: Vec<(u16, &[i8])> = (0..=row.position).map(|j| vc[j].int8_row(g)).collect();
                                attend_int8(fp, qh, &keys, &values, score_scale, words, scaling)
                            }
                        };
                        if rotated {
                            hadamard(&mut oh);
                            let back = -(vd.trailing_zeros() as i32);
                            oh.iter_mut().for_each(|v| *v = ldexp(*v, back));
                        }
                        out[h * vd..(h + 1) * vd].copy_from_slice(&oh);
                    }
                    outs.push(out);
                }
                store.put(*to, outs);
            }
            Command::SigmoidGate { slot, gate } => {
                let program = vector::sigmoid_gate(0, 1, 2, 3);
                let rows = store.get(*slot).iter().zip(store.get(*gate)).map(|(o, g)| simd(fp, &program, [o.clone(), g.clone()], 2)).collect();
                store.put(*slot, rows);
            }
            Command::Advance => {
                for row in plan {
                    self.sequences[row.sequence].position += 1;
                }
            }
        }
        Ok(())
    }

    /// A Gated-DeltaNet block's memory through each row, each sequence's rows in order: per value head the row's dots
    /// with its key and query group's key and query, the rank-one update written back and the read-out
    /// `decay·(S·q) + coef·(k·q)` — or, with a lazy flush, the token logged, the read through the log, and the rows
    /// its [`Replay`] names replayed into the memory. The value heads run on the threads, each over its
    /// own memory. `slots` are the query, key, value, decay exponent, decay and gate.
    fn delta(&mut self, layer: usize, plan: &[Row], store: &Store, slots: [Slot; 6], value_at: usize, heads: (usize, usize, usize)) -> Vec<Vec<f32>> {
        let (nk, nv, hd) = heads;
        let fp = self.numerics.fp_unit;
        let [query, key, value, log_decay, decay, gate] = slots.map(|s| store.get(s));
        let scale = (1.0 / (hd as f64).sqrt()) as f32;
        let (state, flush, threads) = (self.numerics.state, self.numerics.flush, self.threads);
        let round = move |v: f32| match state {
            StateFormat::F32 => v,
            StateFormat::F16 => f16_to_f32(f32_to_f16(v)),
        };
        let mut reads = Vec::with_capacity(plan.len());
        for (k, row) in plan.iter().enumerate() {
            let (q, kk, v) = (&query[k], &key[k], &value[k][value_at..]);
            let (log_decays, decays, betas) = (&log_decay[k], &decay[k], &gate[k]);
            let Carried::Delta { memory, log, .. } = &mut self.sequences[row.sequence].layers[layer] else { unreachable!("a delta block") };
            let mut outs = vec![vec![0f32; hd]; nv];
            let per = nv.div_ceil(threads.min(nv).max(1));
            match flush {
                StateFlush::EveryPass => {
                    std::thread::scope(|scope| {
                        for (chunk, (mems, outs)) in memory.chunks_mut(per * hd * hd).zip(outs.chunks_mut(per)).enumerate() {
                            scope.spawn(move || {
                                for (i, (mem, out)) in mems.chunks_mut(hd * hd).zip(outs.iter_mut()).enumerate() {
                                    let h = chunk * per + i;
                                    let g = h % nk;
                                    let (hq, hk) = (&q[g * hd..(g + 1) * hd], &kk[g * hd..(g + 1) * hd]);
                                    let kq = fdot(fp, hk, hq);
                                    for (r, row) in mem.chunks_mut(hd).enumerate() {
                                        out[r] = gdn_row(fp, row, hk, hq, decays[h], betas[h], v[h * hd + r], kq, scale, round);
                                    }
                                }
                            });
                        }
                    });
                }
                StateFlush::Every { .. } | StateFlush::Trailing { .. } => {
                    log.push(LogEntry {
                        k: kk.iter().map(|&x| round(x)).collect(),
                        v: v.iter().map(|&x| round(x)).collect(),
                        log_decay: log_decays.iter().map(|&x| round(x)).collect(),
                        beta: betas.iter().map(|&x| round(x)).collect(),
                    });
                    let replay = Replay::at(flush, hd, log.len());
                    let (log_entries, replay_at) = (&*log, &replay);
                    std::thread::scope(|scope| {
                        for (chunk, (mems, outs)) in memory.chunks_mut(per * hd * hd).zip(outs.chunks_mut(per)).enumerate() {
                            scope.spawn(move || {
                                for (i, (mem, out)) in mems.chunks_mut(hd * hd).zip(outs.iter_mut()).enumerate() {
                                    let h = chunk * per + i;
                                    let g = h % nk;
                                    let head: Vec<Logged> = log_entries
                                        .iter()
                                        .map(|e| Logged {
                                            k: &e.k[g * hd..(g + 1) * hd],
                                            v: &e.v[h * hd..(h + 1) * hd],
                                            log_decay: e.log_decay[h],
                                            beta: e.beta[h],
                                        })
                                        .collect();
                                    let folds = gdn_fold_trailing(fp, &q[g * hd..(g + 1) * hd], &head, hd, replay_at.older);
                                    for (r, row) in mem.chunks(hd).enumerate() {
                                        let (y, carried) = &folds[usize::from(r >= replay_at.replayed)];
                                        out[r] = gdn_read(fp, row, y, carried[r], scale);
                                    }
                                    for r in replay_at.rows.clone() {
                                        gdn_flush(fp, &mut mem[r * hd..(r + 1) * hd], &head[..replay_at.by], r, round);
                                    }
                                }
                            });
                        }
                    });
                    log.drain(..replay.drop);
                }
            }
            reads.push(outs.concat());
        }
        reads
    }
}

/// One token-row of a step: the entry it came in, its sequence, its position there and its token.
#[derive(Clone, Copy, Debug)]
struct Row {
    entry: usize,
    sequence: usize,
    position: usize,
    token: u32,
}

/// The token-row store of a step: each slot's vector for every row, in the step's row order.
#[derive(Default)]
struct Store(HashMap<Slot, Vec<Vec<f32>>>);

impl Store {
    fn get(&self, slot: Slot) -> &[Vec<f32>] {
        self.0.get(&slot).unwrap_or_else(|| panic!("{slot:?} read before the program writes it"))
    }

    fn put(&mut self, slot: Slot, rows: Vec<Vec<f32>>) {
        self.0.insert(slot, rows);
    }

    fn take(&mut self, slot: Slot) -> Vec<Vec<f32>> {
        self.0.remove(&slot).unwrap_or_else(|| panic!("{slot:?} read before the program writes it"))
    }
}

/// The host's rotary table at `pos`: the f32 cosine and sine of [`rotation`] for each pair of the first `rope.dims`
/// dimensions.
fn rotation_table(pos: usize, rope: &Rope, factors: Option<&[f32]>) -> (Vec<f32>, Vec<f32>) {
    (0..rope.dims / 2)
        .map(|i| {
            let (c, s) = rotation(rope, i, pos, factors);
            (c as f32, s as f32)
        })
        .unzip()
}

/// The checkpoint's table of per-frequency factors, one a pair, that a rotation scaled by them divides its angles by.
pub(crate) const FACTORS: &str = "rope_freqs.weight";

/// `(cos, sin)` of pair `i` at `pos` ([`Rope::rotation`]), its angle divided by the pair's factor when the rotation
/// is scaled by `factors`.
pub(crate) fn rotation(rope: &Rope, i: usize, pos: usize, factors: Option<&[f32]>) -> (f64, f64) {
    rope.rotation(i, pos, factors.map(|f| f64::from(f[i])))
}

/// The factors `rope` divides its angles by, read from `source` when the rotation is scaled by them, none otherwise.
pub(crate) fn rotary_factors(rope: &Rope, source: &mut dyn WeightSource) -> R<Option<Vec<f32>>> {
    if rope.scaling != RopeScaling::Factors {
        return Ok(None);
    }
    let (ty, dims) = source.describe(FACTORS)?;
    let row = dims.first().copied().unwrap_or(1).max(1) as usize;
    let raw = source.row_bytes(FACTORS, 0, dims.iter().product::<u64>() as usize / row)?;
    let mut factors = Vec::new();
    ty.dequantize(&raw, &mut factors);
    factors_for(rope, factors).map(Some)
}

/// `factors` as the table of `rope`'s pairs: one a pair, or refused.
fn factors_for(rope: &Rope, factors: Vec<f32>) -> R<Vec<f32>> {
    let pairs = rope.dims / 2;
    if factors.len() != pairs {
        return Err(EngineError::Factors { found: factors.len(), pairs });
    }
    Ok(factors)
}

/// `program` on the vector unit over `inputs` loaded into its vector registers from 0; register `output` after.
fn simd(fp: FpUnit, program: &[vector::Op], inputs: impl IntoIterator<Item = Vec<f32>>, output: vector::VReg) -> Vec<f32> {
    let mut regs = vector::Regs::new(16, 8);
    for (r, v) in inputs.into_iter().enumerate() {
        regs.v[r] = v;
    }
    vector::run(fp, program, &mut regs);
    std::mem::take(&mut regs.v[output as usize])
}

/// The greedy token: the index of the largest logit ([`super::numerics::argmax`]).
pub fn argmax(logits: &[f32]) -> u32 {
    super::numerics::argmax(logits) as u32
}
