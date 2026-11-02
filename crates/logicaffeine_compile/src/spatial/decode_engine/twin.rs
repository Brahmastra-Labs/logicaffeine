//! The decode engine's cycle twin: a step's program priced command by command on the engine's units.
//!
//! Each command asks its units for work ([`Demand`]): the stream engines for its weights from the card's memory and
//! for the state and caches its rows' sequences read and write, each way of moving them at the share of the memory's
//! peak it sustains ([`crate::spatial::serving::Streams`]); the array and the block-scale lanes for a block-format
//! matrix; the vector units for the norms, the attention, the recurrence, a float matrix and the elementwise steps
//! between them. A command takes as long as its slowest unit, the others overlapped with it. Weights stream ahead of
//! the command that reads them into an on-chip window while earlier commands run ([`schedule`]): a command's own state
//! and cache traffic is served first, and the memory time it leaves spare fetches the next commands' weights as far as
//! the window holds.
//!
//! The weights, the traffic of each kind, the array's multiply-adds and the lanes' cycles are the pricer's
//! ([`crate::spatial::serving::PassShape`]) term for term; the twin's vector work is the engine's own, every head of a
//! per-head norm and every elementwise step the pricer leaves out.

use std::collections::BTreeMap;

use serde::Serialize;

use crate::model::spec::{Mixer, ModelSpec};
use crate::spatial::decode::{GgmlType, GgufHeader};
use crate::spatial::serving::{array_passes, is_float, lane_cycles, Card, VALUE_BLOCK};
use crate::spatial::serving::{AttentionMath, StateFlush};

use super::model::{CacheFormat, Replay, StateFormat};
use super::numerics::Words;
use super::program::{Command, Program};

/// f32 operations an element of each of the vector unit's functions takes, as [`super::numerics`] sequences them:
/// `exp` its range reduction, its cubic and its scaling; `recip` and `rsqrt` a seed and two Newton steps;
/// `sigmoid` an `exp`, an add and a `recip`; `silu` a `sigmoid` and a product; `softplus` an `exp` and an `ln1p`.
pub const EXP_OPS: f64 = 10.0;
pub const RECIP_OPS: f64 = 6.0;
pub const RSQRT_OPS: f64 = 8.0;
pub const SIGMOID_OPS: f64 = EXP_OPS + 1.0 + RECIP_OPS;
pub const SILU_OPS: f64 = SIGMOID_OPS + 1.0;
pub const SOFTPLUS_OPS: f64 = EXP_OPS + 12.0;

/// One sequence a step advances: the rows it brings and the positions its caches hold before them.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct SequenceRows {
    pub rows: usize,
    pub position: usize,
    /// Tokens its lazily flushed memories log before the step.
    pub logged: usize,
}

/// The rows a step runs: every sequence it advances, and whether the rows past each sequence's first are drafts the
/// step verifies, so every candidate prefix's state is kept.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Step {
    pub sequences: Vec<SequenceRows>,
    pub drafted: bool,
}

/// The work one command asks of each unit.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize)]
pub struct Demand {
    /// Weight bytes streamed from the card's memory: the command's tensors, the embedding's rows.
    pub weights: f64,
    /// Bytes of the rows' sequences' recurrent states read and written back in place.
    pub rewritten: f64,
    /// Cache, ring, log and state bytes the rows' sequences read in long runs and append their rows after.
    pub appended: f64,
    /// Multiply-adds of the 4-bit array.
    pub array: f64,
    /// Block-scale lane cycles.
    pub lanes: f64,
    /// f32 operations of the vector units.
    pub vector: f64,
}

impl std::ops::AddAssign for Demand {
    fn add_assign(&mut self, o: Demand) {
        self.weights += o.weights;
        self.rewritten += o.rewritten;
        self.appended += o.appended;
        self.array += o.array;
        self.lanes += o.lanes;
        self.vector += o.vector;
    }
}

impl Demand {
    /// Seconds `card`'s memory takes to move the demand's state and cache traffic and `weights` bytes of its weights,
    /// each at its own stream's share of the peak.
    pub fn memory_s(&self, card: &Card, weights: f64) -> f64 {
        let peak = card.memory.bandwidth.value;
        weights / (peak * card.streams.read.value)
            + self.rewritten / (peak * card.streams.rewrite.value)
            + self.appended / (peak * card.streams.append.value)
    }
}

/// Why the twin cannot price a program.
#[derive(Debug, PartialEq)]
pub enum TwinRefusal {
    MissingTensor(String),
    UnknownType { tensor: String, id: u32 },
    /// A matrix in a format neither the array nor the vector units read.
    Undecoded { tensor: String, ty: GgmlType },
}

/// The work of every command of `program` over `step` on the checkpoint `h` describes.
pub fn demands(spec: &ModelSpec, h: &GgufHeader, program: &Program, step: &Step) -> Result<Vec<Demand>, TwinRefusal> {
    let drafted = step.drafted;
    let step = &step.sequences;
    let tensors: BTreeMap<&str, &crate::spatial::decode::TensorRow> = h.tensors.iter().map(|t| (t.name.as_str(), t)).collect();
    let tensor = |name: &str| -> Result<(GgmlType, f64, Vec<u64>), TwinRefusal> {
        let t = tensors.get(name).ok_or_else(|| TwinRefusal::MissingTensor(name.into()))?;
        let unknown = || TwinRefusal::UnknownType { tensor: name.into(), id: t.type_id };
        let ty = GgmlType::from_id(t.type_id).ok_or_else(unknown)?;
        let bytes = ty.bytes(t.dims.iter().product::<u64>()).ok_or_else(unknown)?;
        Ok((ty, bytes as f64, t.dims.clone()))
    };
    let row_bytes = |name: &str| -> Result<f64, TwinRefusal> {
        let (ty, _, dims) = tensor(name)?;
        Ok(ty.bytes(dims[0]).ok_or_else(|| TwinRefusal::UnknownType { tensor: name.into(), id: ty.id() })? as f64)
    };
    let n = program.numerics;
    let words = match n.activations {
        Words::One => 1.0,
        Words::Two => 2.0,
    };
    let state_bytes = match n.state {
        StateFormat::F32 => 4.0,
        StateFormat::F16 => 2.0,
    };
    // A position's cached bytes: every element binary16, or a code a byte and each head row's f16 scale.
    let per_position = |kv_heads: usize, width: usize, value_width: usize| -> f64 {
        match n.cache {
            CacheFormat::F16 => (kv_heads * (width + value_width) * 2) as f64,
            CacheFormat::Int8 | CacheFormat::Int8Hadamard => (kv_heads * (width + value_width + 4)) as f64,
        }
    };
    let rows: f64 = step.iter().map(|s| s.rows as f64).sum();
    let mut out = Vec::with_capacity(program.commands.len());
    for c in &program.commands {
        let mut d = Demand::default();
        match c {
            Command::Embed { table } => d.weights = rows * row_bytes(table)?,
            Command::Norm { weight, .. } => {
                let (_, bytes, dims) = tensor(weight)?;
                d.weights = bytes;
                d.vector = 2.0 * dims[0] as f64 * rows + RSQRT_OPS * rows;
            }
            Command::Matvec { tensor: name, .. } => {
                let (ty, bytes, dims) = tensor(name)?;
                let elems = dims.iter().product::<u64>() as f64;
                d.weights = bytes;
                if let Some(p) = array_passes(ty) {
                    d.array = elems * p as f64 * words * rows;
                    d.lanes = elems / ty.block_elems() as f64 * lane_cycles(ty, n.scaling).expect("a type the array passes") * words * rows;
                } else if is_float(ty) {
                    d.vector = elems * rows;
                } else {
                    return Err(TwinRefusal::Undecoded { tensor: name.clone(), ty });
                }
            }
            Command::Add { .. } => d.vector = spec.d as f64 * rows,
            Command::SwiGlu { .. } => {
                let dff = ffn_width(spec);
                d.vector = dff * rows * (SILU_OPS + 1.0);
            }
            Command::Conv { kernel, taps, .. } => {
                let (_, bytes, dims) = tensor(kernel)?;
                let channels = *dims.last().expect("a kernel's channels") as f64;
                d.weights = bytes;
                // The ring read once a sequence, each row appended, as the state stores it.
                let ring = (*taps as f64 - 1.0) * channels * state_bytes;
                d.appended = step.iter().map(|s| ring + s.rows as f64 * channels * state_bytes).sum();
                d.vector = *taps as f64 * channels * rows + SILU_OPS * channels * rows;
            }
            Command::L2Heads { heads, width, .. } => d.vector = (2.0 * (*heads * *width) as f64 + RSQRT_OPS * *heads as f64) * rows,
            Command::Gates { dt, a, .. } => {
                let (_, b1, _) = tensor(dt)?;
                let (_, b2, dims) = tensor(a)?;
                d.weights = b1 + b2;
                d.vector = dims[0] as f64 * rows * (SOFTPLUS_OPS + 2.0 + EXP_OPS + SIGMOID_OPS);
            }
            Command::Delta { k_heads, v_heads, width, .. } => {
                let state = (*v_heads * *width * *width) as f64;
                let log = (*k_heads * *width + *v_heads * *width + 2 * *v_heads) as f64;
                match n.flush {
                    StateFlush::EveryPass => {
                        // The state read once a sequence and written back once, or every candidate prefix's when the
                        // rows are drafts.
                        d.rewritten = step.iter().map(|s| state * state_bytes * (1.0 + if drafted { s.rows as f64 } else { 1.0 })).sum();
                        d.vector = 3.0 * state * rows;
                    }
                    StateFlush::Every { .. } | StateFlush::Trailing { .. } => {
                        for s in step {
                            let mut logged = s.logged;
                            // Each row reads the state through its folded query and walks the whole log; the rows of a
                            // head its replay names take the log's replayed tokens again.
                            let (mut walked, mut replayed, mut replays) = (0usize, 0usize, 0usize);
                            for _ in 0..s.rows {
                                logged += 1;
                                walked += logged;
                                let replay = Replay::at(n.flush, *width, logged);
                                replayed += replay.rows.len();
                                replays += replay.rows.len() * replay.by;
                                logged -= replay.drop;
                            }
                            // The rows a step replays are read and written back, every replay's; the rest of the state
                            // is read alone.
                            let rewritten = replayed.min(*width) as f64 / *width as f64;
                            d.rewritten += state * state_bytes * (rewritten + replayed as f64 / *width as f64);
                            d.appended += state * state_bytes * (1.0 - rewritten);
                            d.appended += (s.logged + s.rows) as f64 * log * state_bytes;
                            d.vector += s.rows as f64 * state
                                + *v_heads as f64 * walked as f64 * (2.0 * *width as f64 + *width as f64)
                                + 2.0 * (*v_heads * *width * replays) as f64;
                        }
                    }
                }
            }
            Command::GatedNorm { weight, width, .. } => {
                let (_, bytes, _) = tensor(weight)?;
                let heads = gated_heads(spec, *width);
                d.weights = bytes;
                d.vector = heads * *width as f64 * rows * (3.0 + SILU_OPS + 1.0) + RSQRT_OPS * heads * rows;
            }
            Command::Split { .. } => {}
            Command::RmsHeads { weight, width, slot } => {
                let (_, bytes, _) = tensor(weight)?;
                let heads = attention_heads(spec, *slot);
                d.weights = bytes;
                d.vector = (2.0 * heads * *width as f64 + RSQRT_OPS * heads) * rows;
            }
            Command::Rotate { slot, rope, .. } => {
                let heads = attention_heads(spec, *slot);
                // A pair is four products and two adds.
                d.vector = heads * (rope.dims / 2) as f64 * 6.0 * rows;
            }
            Command::Hadamard { slot, width } => {
                let heads = attention_heads(spec, *slot);
                d.vector = heads * transform(*width as f64) * rows;
            }
            Command::CacheAppend { kv_heads, width, value_width, .. } => {
                d.appended = per_position(*kv_heads, *width, *value_width) * rows;
                if n.cache.is_int8() {
                    d.vector = (*kv_heads * (*width + *value_width)) as f64 * rows * 3.0;
                }
            }
            Command::Attend { heads, kv_heads, width, value_width, .. } => {
                let per_position = per_position(*kv_heads, *width, *value_width);
                for s in step {
                    // One read of the cache serves every row of the sequence; each row attends to its own position.
                    let read_to = (s.position + s.rows) as f64;
                    d.appended += per_position * read_to;
                    for r in 0..s.rows {
                        let attended = (s.position + r + 1) as f64;
                        let products = *heads as f64 * (*width + *value_width) as f64 * attended;
                        d.vector += crate::spatial::serving::SOFTMAX_MACS_PER_SCORE * *heads as f64 * attended;
                        match n.attention {
                            AttentionMath::Vector => d.vector += products,
                            AttentionMath::Array => {
                                d.array += 2.0 * products;
                                d.lanes += *heads as f64 * attended * (1.0 + *value_width as f64 / VALUE_BLOCK);
                            }
                        }
                    }
                }
                if n.cache == CacheFormat::Int8Hadamard {
                    d.vector += *heads as f64 * transform(*value_width as f64) * rows;
                }
            }
            Command::SigmoidGate { .. } => {
                let width = attention_gate_width(spec);
                d.vector = width * rows * (SIGMOID_OPS + 1.0);
            }
            Command::Advance => {}
        }
        out.push(d);
    }
    Ok(out)
}

/// A Walsh–Hadamard transform of `n` elements: `n·log2 n` additions.
fn transform(n: f64) -> f64 {
    n * n.max(1.0).log2()
}

fn ffn_width(spec: &ModelSpec) -> f64 {
    spec.layers
        .iter()
        .find_map(|l| match l.ffn {
            crate::model::spec::Ffn::Dense { dff, .. } => Some(dff as f64),
            _ => None,
        })
        .unwrap_or(0.0)
}

fn gated_heads(spec: &ModelSpec, width: usize) -> f64 {
    spec.layers
        .iter()
        .find_map(|l| match &l.mixer {
            Mixer::GatedDelta(g) if g.head_dim == width => Some(g.v_heads as f64),
            _ => None,
        })
        .unwrap_or(0.0)
}

fn attention_heads(spec: &ModelSpec, slot: super::program::Slot) -> f64 {
    spec.layers
        .iter()
        .find_map(|l| match &l.mixer {
            Mixer::Attention(a) => Some(match slot {
                super::program::Slot::Query => a.heads as f64,
                _ => a.kv_heads as f64,
            }),
            _ => None,
        })
        .unwrap_or(0.0)
}

fn attention_gate_width(spec: &ModelSpec) -> f64 {
    spec.layers
        .iter()
        .find_map(|l| match &l.mixer {
            Mixer::Attention(a) => Some((a.heads * a.value_dim) as f64),
            _ => None,
        })
        .unwrap_or(0.0)
}

/// Which unit a command waited on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum Bound {
    Memory,
    Array,
    Lanes,
    Vector,
}

/// One command on the step's timeline.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Span {
    pub start_s: f64,
    pub end_s: f64,
    pub bound: Bound,
    /// Weight bytes of the command already in the window when it started.
    pub prefetched: f64,
}

/// A step on the card: every command's span and the step's length.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Timeline {
    pub spans: Vec<Span>,
    pub step_s: f64,
    /// The time each unit is busy over the step.
    pub memory_s: f64,
    pub array_s: f64,
    pub lanes_s: f64,
    pub vector_s: f64,
}

/// The step's commands in order on `card`: each starts when the one before it ends and lasts as long as its slowest
/// unit; the memory serves the command's own traffic, then its weights not yet fetched, then the following commands'
/// weights in order while `window` bytes hold them.
pub fn schedule(demands: &[Demand], card: &Card, window: f64) -> Timeline {
    // Weight bytes a second the memory reads.
    let reads = card.memory.bandwidth.value * card.streams.read.value;
    let (array, vector, lanes) = (card.array.value, card.vector.value, card.scale.value);
    // Weight bytes of each command already fetched ahead of it.
    let mut fetched = vec![0f64; demands.len()];
    let mut held = 0f64;
    let mut next = 0usize;
    let mut now = 0f64;
    let mut spans = Vec::with_capacity(demands.len());
    let (mut memory_s, mut array_s, mut lanes_s, mut vector_s) = (0f64, 0f64, 0f64, 0f64);
    for (i, d) in demands.iter().enumerate() {
        let prefetched = fetched[i];
        held -= prefetched;
        let own = d.memory_s(card, (d.weights - prefetched).max(0.0));
        let times = [(own, Bound::Memory), (d.array / array, Bound::Array), (d.lanes / lanes, Bound::Lanes), (d.vector / vector, Bound::Vector)];
        let (length, bound) = times.into_iter().fold((0.0, Bound::Memory), |a, b| if b.0 > a.0 { b } else { a });
        memory_s += own;
        array_s += d.array / array;
        lanes_s += d.lanes / lanes;
        vector_s += d.vector / vector;
        // The memory time the command leaves spare fetches the next commands' weights, in order, into the window.
        let mut spare = (length - own) * reads;
        next = next.max(i + 1);
        while spare > 0.0 && next < demands.len() {
            let want = demands[next].weights - fetched[next];
            let room = window - held;
            let take = want.min(room).min(spare);
            if take <= 0.0 {
                break;
            }
            fetched[next] += take;
            held += take;
            spare -= take;
            memory_s += take / reads;
            if fetched[next] >= demands[next].weights {
                next += 1;
            } else {
                break;
            }
        }
        spans.push(Span { start_s: now, end_s: now + length, bound, prefetched });
        now += length;
    }
    Timeline { spans, step_s: now, memory_s, array_s, lanes_s, vector_s }
}

// ── The prediction ──────────────────────────────────────────────────────────────────────────────────

/// The header ledger the prediction prices.
pub const PREDICTED_HEADER: &str = "qwen3.8-27b-q4_k_m";
/// Batches the prediction prices where the card holds them, every sequence at [`PREDICTED_POSITION`]; with them each
/// knob point's largest batch the card holds ([`crate::spatial::serving::max_batch`]).
pub const PREDICTED_BATCHES: [usize; 3] = [1, 16, 64];
/// Positions each sequence's caches hold before the step.
pub const PREDICTED_POSITION: usize = 2047;
/// Each sequence's context limit: the roofline's 2k-of-4k class, its cache paged.
pub const PREDICTED_LIMIT: u64 = 4096;

/// One configuration the twin predicts, beside the roofline's price of it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PredictedRow {
    pub numerics: String,
    pub batch: usize,
    pub position: usize,
    /// On-chip bytes the weights may stream ahead into: the card's on-chip memory less the batch's token-rows.
    pub window_bytes: f64,
    pub step_s: f64,
    pub tokens_per_s: f64,
    /// The pricer's tokens per second for the same serving, the bound no schedule passes.
    pub roofline_tokens_per_s: f64,
    /// The share of the step each unit was the command's bound for.
    pub bound_share: BTreeMap<String, f64>,
    pub memory_s: f64,
    pub array_s: f64,
    pub lanes_s: f64,
    pub vector_s: f64,
}

/// The twin's prediction of the 27B on the D5005's standard plan: every knob point the real checkpoints are held to,
/// at each of [`PREDICTED_BATCHES`] the card holds and at the largest batch it holds, every sequence at
/// [`PREDICTED_POSITION`].
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Predicted {
    pub card: &'static str,
    pub header: &'static str,
    pub rows: Vec<PredictedRow>,
}

/// The serving the pricer describes for a step of `batch` sequences at `n`, each attending to `attended` positions.
pub fn serving_of(spec: &ModelSpec, n: super::model::EngineNumerics, batch: usize, attended: f64) -> crate::spatial::serving::Serving {
    let (hd, vd) = spec
        .layers
        .iter()
        .find_map(|l| match &l.mixer {
            Mixer::Attention(a) => Some((a.head_dim as f64, a.value_dim as f64)),
            _ => None,
        })
        .unwrap_or((0.0, 0.0));
    crate::spatial::serving::Serving {
        batch: batch as u32,
        contexts: crate::spatial::serving::ContextMix::uniform(attended, PREDICTED_LIMIT),
        kv_page: Some(crate::spatial::d5005::KV_PAGE),
        attention: n.attention,
        // A code a byte and each head row's f16 scale, or binary16.
        kv_bytes: if n.cache.is_int8() { (hd + vd + 4.0) / (hd + vd) } else { 2.0 },
        state_bytes: match n.state {
            StateFormat::F32 => 4.0,
            StateFormat::F16 => 2.0,
        },
        activation_words: match n.activations {
            Words::One => 1,
            Words::Two => 2,
        },
        scaling: n.scaling,
        kv_rotated: n.cache == CacheFormat::Int8Hadamard,
        draft: None,
        flush: n.flush,
        host_memory: false,
    }
}

/// The steps a steady decode of `batch` sequences a row each at `position` cycles through: one, or under a lazy flush
/// one for every phase of the flush window, sequence `i` of phase `p` having logged `(i + p) mod tokens` tokens past
/// the window a trailing flush is replaying — so their mean is a mean step.
pub fn steady_steps(n: super::model::EngineNumerics, batch: usize, position: usize) -> Vec<Step> {
    let (window, behind) = match n.flush {
        StateFlush::EveryPass => (1, 0),
        StateFlush::Every { tokens } => (tokens.max(1) as usize, 0),
        StateFlush::Trailing { tokens } => (tokens.max(1) as usize, tokens as usize),
    };
    (0..window)
        .map(|p| Step {
            sequences: (0..batch).map(|i| SequenceRows { rows: 1, position, logged: if window > 1 { behind + (i + p) % window } else { 0 } }).collect(),
            drafted: false,
        })
        .collect()
}

/// The prediction, from the header ledgers under `decode_dir` (`benchmarks/silicon/decode`): the architecture the
/// ledger describes ([`GgufHeader::spec`]), the program of each knob point priced and scheduled.
pub fn predicted(decode_dir: &std::path::Path) -> Result<Predicted, String> {
    use crate::spatial::d5005::{card, EnginePlan};
    let h = GgufHeader::read_ledger(&decode_dir.join(format!("headers/{PREDICTED_HEADER}.json")))?;
    let spec = h.spec().map_err(|e| format!("{PREDICTED_HEADER}: {e}"))?;
    let c = card(&EnginePlan::standard());
    let mut rows = Vec::new();
    for n in super::model::EngineNumerics::real_points() {
        let program = super::program::lower(&spec, n, PREDICTED_POSITION + 1).map_err(|e| format!("{PREDICTED_HEADER}: {e}"))?.program;
        let attended = (PREDICTED_POSITION + 1) as f64;
        let most = crate::spatial::serving::max_batch(&h, &c, &serving_of(&spec, n, 1, attended)).map_err(|e| format!("{e:?}"))? as usize;
        let mut batches: Vec<usize> = PREDICTED_BATCHES.iter().copied().filter(|&b| b < most).collect();
        batches.push(most);
        for batch in batches {
            let shape = crate::spatial::serving::PassShape::of(&h, &serving_of(&spec, n, batch, (PREDICTED_POSITION + 1) as f64))
                .map_err(|e| format!("{e:?}"))?;
            let window = (c.on_chip.capacity.value - batch as f64 * shape.on_chip_per_sequence).max(0.0);
            let steps = steady_steps(n, batch, PREDICTED_POSITION);
            let timelines: Vec<Timeline> = steps
                .iter()
                .map(|step| Ok(schedule(&demands(&spec, &h, &program, step).map_err(|e| format!("{e:?}"))?, &c, window)))
                .collect::<Result<_, String>>()?;
            let mean = |f: &dyn Fn(&Timeline) -> f64| timelines.iter().map(f).sum::<f64>() / timelines.len() as f64;
            let step_s = mean(&|t| t.step_s);
            let roofline = shape.cost(&c, batch as u32, 0).map(|cost| cost.tokens_per_s()).unwrap_or(0.0);
            let mut bound_share = BTreeMap::new();
            for t in &timelines {
                for s in &t.spans {
                    *bound_share.entry(format!("{:?}", s.bound)).or_insert(0.0) += (s.end_s - s.start_s) / (step_s * timelines.len() as f64);
                }
            }
            rows.push(PredictedRow {
                numerics: format!("{n:?}"),
                batch,
                position: PREDICTED_POSITION,
                window_bytes: window,
                step_s,
                tokens_per_s: batch as f64 / step_s,
                roofline_tokens_per_s: roofline,
                bound_share,
                memory_s: mean(&|t| t.memory_s),
                array_s: mean(&|t| t.array_s),
                lanes_s: mean(&|t| t.lanes_s),
                vector_s: mean(&|t| t.vector_s),
            });
        }
    }
    Ok(Predicted { card: c.name, header: PREDICTED_HEADER, rows })
}
