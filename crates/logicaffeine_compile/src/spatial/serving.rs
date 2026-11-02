//! The cost of a pass: batched, speculative decode on one card that holds its tokens and streams its weights.
//!
//! ★ A CARD THAT HOLDS ITS TOKEN-ROWS AND STREAMS ITS WEIGHTS PAYS FOR EACH WEIGHT ONCE A PASS. Every weight
//! crosses the card once and multiplies every token-row the card holds: each sequence's verified token and the
//! tokens drafted after it. What does not amortize is per sequence: the recurrent state it reads and writes
//! back, its KV cache, its conv ring. A pass therefore costs the weights once plus each sequence's own bytes,
//! against the arithmetic of every held token-row, and the batch grows until the card's memory is full.
//!
//! The pass is read off the same [`DecodeStep`] the batch-one price uses — the same stages and bytes, from the
//! checkpoint's header — plus what serving adds: how many sequences ride a pass ([`Serving::batch`]), how many
//! drafted tokens each verifies and how often they are accepted ([`Draft`]), and how the recurrent state is
//! carried between tokens ([`StateFlush`]). The card is facts with sources ([`Card`]): its memory, the shares
//! of that memory's bandwidth its stream engines sustain on each way a pass moves its bytes ([`Streams`]), the
//! multiply-adds its integer array and its f32 vector units retire.
//!
//! The array multiplies 4-bit weight codes by 8-bit activation codes. A wider integer code is split into 4-bit
//! digits and a two-word activation into two words, each digit and word one pass, recombined exactly
//! ([`array_passes`]), so every block format the card decodes rides one datapath. A float matrix rides the
//! vector units. A format the card does not decode is refused by name, never priced as a guess.
//!
//! Every weight block's integer sums leave the array for a block-scale lane: its sub-block scales and mins
//! multiplied in, the block's two scales, the exact sum and its one rounding, the f32 accumulate ([`lane_cycles`]).
//! The lanes are a fifth rate beside the memories and the two kinds of arithmetic: a format of small blocks, or
//! activations scaled per 32 elements against large ones, asks more of them than of the array.

use std::collections::BTreeMap;

use serde::Serialize;

use super::decode::{DecodeRefusal, DecodeStep, Deployment, GgmlType, GgufHeader, HeldClass, Split, StageKind, Tier, WeightBytes};
use super::decode_engine::numerics::ActScaling;
use super::rdu_chip::{Fact, Provenance};

/// How the recurrent state is carried from one token to the next.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum StateFlush {
    /// The dense state is read and written back every pass. With drafts, the state after every candidate prefix
    /// is written and the accepted one kept.
    EveryPass,
    /// The dense state is written back once every `tokens` accepted tokens. Between flushes a log of each token's
    /// key, value, decay exponent and gate carries it, read with the state every pass; a rejected draft truncates
    /// the log.
    Every { tokens: u32 },
    /// The dense state is written back a window behind: each window of `tokens` logged tokens is replayed into the
    /// state over the next window, a `1/tokens` share of every head's rows a token, as the read-out streams them. The
    /// log holds the window being replayed and the one after it, at most `2·tokens` tokens; a row already replayed reads
    /// through the newer window, every other through both. The dense state is [`StateFlush::Every`]'s at the same
    /// `tokens`, a window later.
    Trailing { tokens: u32 },
}

/// The drafted tokens each sequence verifies in a pass.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Draft {
    pub tokens: u32,
    /// `acceptance[i]`: the chance draft `i + 1` is accepted, given every draft before it was.
    pub acceptance: Vec<Fact<f64>>,
    /// Vocabulary rows the drafting head reads (the most frequent tokens), or `None` for the whole head.
    /// Verification always reads the whole head, so the tokens emitted are the greedy tokens either way.
    pub head_rows: Option<u64>,
}

/// A class of the batch's sequences by context: its share of the batch, the mean position its sequences have
/// reached, and the context each of them is allowed.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct ContextClass {
    pub share: f64,
    pub position: f64,
    pub limit: u64,
}

/// The contexts of a batch's sequences. Every per-position cost is linear in position, so a pass reads, and a
/// paged cache holds, what the mean position says; a contiguous cache holds what the mean limit says.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ContextMix {
    pub classes: Vec<ContextClass>,
}

impl ContextMix {
    /// Every sequence at `position` of `limit`.
    pub fn uniform(position: f64, limit: u64) -> ContextMix {
        ContextMix { classes: vec![ContextClass { share: 1.0, position, limit }] }
    }

    pub fn mean_position(&self) -> f64 {
        self.classes.iter().map(|c| c.share * c.position).sum()
    }

    pub fn mean_limit(&self) -> f64 {
        self.classes.iter().map(|c| c.share * c.limit as f64).sum()
    }

    /// The longest context any class is allowed.
    pub fn longest(&self) -> u64 {
        self.classes.iter().map(|c| c.limit).max().unwrap_or(0)
    }
}

/// Where attention's two products over the cache run.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize)]
pub enum AttentionMath {
    /// Scores and the weighted sum in f32 on the vector units, over the cache as stored.
    Vector,
    /// Scores and the weighted sum as integer dot products on the array: int8 queries against an int8 cache, and
    /// 8-bit probabilities against it, two passes each (an 8-bit cache code is two digits). The softmax stays on the
    /// vector units.
    Array,
}

/// Vector multiply-add slots per score a softmax takes: the engine's exponential is twelve f32 operations
/// (`decode_engine::numerics::exp`: the scaling and its rounding, two for the range reduction, six for the cubic, one
/// for the table product, one for the power of two), and the running maximum, the sum and the rescale three more;
/// fifteen operations, two to a slot.
pub const SOFTMAX_MACS_PER_SCORE: f64 = 7.5;

/// One serving configuration of a checkpoint on a card.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Serving {
    /// Sequences that ride a pass.
    pub batch: u32,
    pub contexts: ContextMix,
    /// Positions per KV page when the caches are paged: each sequence then holds its own length and half a page,
    /// on average, instead of its whole limit.
    pub kv_page: Option<u64>,
    pub attention: AttentionMath,
    /// Bytes per KV-cache element.
    pub kv_bytes: f64,
    /// Bytes per recurrent-state element.
    pub state_bytes: f64,
    /// Words each activation crosses the array in: one 8-bit word, or two (a word and its residual).
    pub activation_words: u32,
    /// How the activations a matrix reads are scaled: per 32 elements, or per the matrix format's block.
    pub scaling: ActScaling,
    /// Whether each cached key and value head row is rotated by the Walsh–Hadamard transform before its int8 codes,
    /// the queries rotated alike and every head's weighted sum rotated back.
    pub kv_rotated: bool,
    pub draft: Option<Draft>,
    pub flush: StateFlush,
    /// Whether sequences the card's memory cannot hold, or whose traffic it is better without, may keep their
    /// state and caches in host memory, streamed over the host link each pass.
    pub host_memory: bool,
}

/// The host memory a card reads and writes over its host link.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct HostLink {
    /// Bytes of host memory the serving pins for the card.
    pub capacity: Fact<f64>,
    /// Bytes per second host to card, and card to host: the link's two directions run at once.
    pub read: Fact<f64>,
    pub write: Fact<f64>,
    /// The share of either direction's rate the card's DMA sustains.
    pub stream: Fact<f64>,
}

/// The share of a card memory's bandwidth its stream engines sustain on each way a pass moves its bytes. The ways differ
/// in how often the memory's bus turns between reading and writing and how often a row is opened for few bytes.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
pub struct Streams {
    /// One long read: the weights, the embedding rows.
    pub read: Fact<f64>,
    /// A memory read and written back in place a chunk at a time: a recurrent state its pass rewrites.
    pub rewrite: Fact<f64>,
    /// A long read with the pass's rows appended after it: a cache, a conv ring, a log, a recurrent state read and not
    /// written.
    pub append: Fact<f64>,
}

/// A card that streams its weights, as a pass sees it.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct Card {
    pub name: &'static str,
    /// Off-chip memory: the weights, and every KV cache and recurrent state it holds.
    pub memory: Tier,
    /// On-chip memory: the batch's token-rows through a layer.
    pub on_chip: Tier,
    /// The shares of `memory`'s bandwidth the stream engines sustain.
    pub streams: Streams,
    /// Multiply-adds per second the integer array retires: a 4-bit weight code times an 8-bit activation code.
    pub array: Fact<f64>,
    /// f32 multiply-adds per second the vector units retire: norms, attention, the recurrence, float matrices.
    pub vector: Fact<f64>,
    /// Cycles per second the block-scale lanes retire, all lanes together ([`lane_cycles`]).
    pub scale: Fact<f64>,
    pub host: Option<HostLink>,
    pub sources: &'static [&'static str],
}

impl Card {
    /// Every fact the card carries, with the path a lock names it by.
    pub fn facts(&self) -> Vec<(String, Provenance, &'static str)> {
        let mut out = vec![
            ("memory.capacity".into(), self.memory.capacity.provenance, self.memory.capacity.cite),
            ("memory.bandwidth".into(), self.memory.bandwidth.provenance, self.memory.bandwidth.cite),
            ("on_chip.capacity".into(), self.on_chip.capacity.provenance, self.on_chip.capacity.cite),
            ("on_chip.bandwidth".into(), self.on_chip.bandwidth.provenance, self.on_chip.bandwidth.cite),
            ("streams.read".into(), self.streams.read.provenance, self.streams.read.cite),
            ("streams.rewrite".into(), self.streams.rewrite.provenance, self.streams.rewrite.cite),
            ("streams.append".into(), self.streams.append.provenance, self.streams.append.cite),
            ("array".into(), self.array.provenance, self.array.cite),
            ("vector".into(), self.vector.provenance, self.vector.cite),
            ("scale".into(), self.scale.provenance, self.scale.cite),
        ];
        if let Some(h) = &self.host {
            out.push(("host.capacity".into(), h.capacity.provenance, h.capacity.cite));
            out.push(("host.read".into(), h.read.provenance, h.read.cite));
            out.push(("host.write".into(), h.write.provenance, h.write.cite));
            out.push(("host.stream".into(), h.stream.provenance, h.stream.cite));
        }
        out
    }
}

/// Passes of the 4-bit array one weight of `ty` takes, or `None` for a type the array does not read.
///
/// A block format stores an unsigned code per weight (a signed one is offset by half its range) and per-block
/// scales the block-scale unit applies after the integer sum. Codes of up to four bits are one pass. Five- and
/// six-bit codes split into a 4-bit low digit and a high digit, and eight-bit codes (Q8_0, and the int8 table
/// values IQ4 indexes) into two 4-bit digits: two passes, recombined exactly as `lo + 16·hi`.
pub fn array_passes(ty: GgmlType) -> Option<u32> {
    use GgmlType::*;
    match ty {
        Q2K | Q3K | Q4_0 | Q4_1 | Q4K => Some(1),
        Q5_0 | Q5_1 | Q5K | Q6K | Q8_0 | Iq4Nl | Iq4Xs => Some(2),
        _ => None,
    }
}

/// Sub-block multipliers a block-scale lane applies in one cycle.
pub const LANE_PRODUCTS: f64 = 16.0;

/// Cycles a block-scale lane spends on one weight block of `ty` read against one activation word scaled as `scaling`,
/// or `None` for a type the array does not read.
///
/// A lane takes a block's integer sums from the array, multiplies in its sub-block scales and mins
/// ([`LANE_PRODUCTS`] a cycle), multiplies by the block's two scales times the activation scale, sums the two terms
/// exactly, rounds once and adds the result into its row. Activations scaled per 32 elements against a larger block
/// give the block one term per 32 elements, each aligned into the block's exact sum on its own cycle.
pub fn lane_cycles(ty: GgmlType, scaling: ActScaling) -> Option<f64> {
    use GgmlType::*;
    let products = match ty {
        Q4_0 | Q4_1 | Q5_0 | Q5_1 | Q8_0 | Iq4Nl => 0.0,
        Iq4Xs => 8.0,
        Q3K | Q4K | Q5K | Q6K => 16.0,
        Q2K => 32.0,
        _ => return None,
    };
    let per_block = (products / LANE_PRODUCTS).max(1.0);
    Some(match scaling {
        ActScaling::PerBlock => per_block,
        ActScaling::Sub32 => per_block.max(ty.block_elems() as f64 / 32.0),
    })
}

/// Positions one block of attention's weighted sum on the array spans: its probabilities share a scale.
pub const VALUE_BLOCK: f64 = 32.0;

/// Whether the vector units read `ty` as a float matrix.
pub fn is_float(ty: GgmlType) -> bool {
    matches!(ty, GgmlType::F32 | GgmlType::F16 | GgmlType::Bf16)
}

/// Why a serving could not be priced.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub enum ServingRefusal {
    Step(DecodeRefusal),
    /// A matrix in a format neither the array nor the vector units read.
    Undecoded { tensor: String, ty: GgmlType },
    /// A tensor a stage names that the header does not hold.
    MissingTensor(String),
    /// What the serving holds exceeds the card's memory, and the host's if it may use it.
    NotResident { needed: f64, capacity: f64 },
    /// The batch's token-rows exceed the card's on-chip memory.
    OnChipOverflow { needed: f64, capacity: f64 },
    /// A draft whose acceptance list is not one probability per drafted token.
    BadDraft(String),
    /// A serving of no sequences, or no activation words.
    Empty,
    /// A mixture of experts: which experts a pass reads depends on how its rows route, which the pricer does
    /// not model.
    Routed,
    /// A serving that may use host memory, on a card with no host link.
    NoHostLink,
    /// A context mix whose shares do not sum to one, or a class past its own limit.
    BadContexts(String),
    /// A context limit past the checkpoint's own context length.
    ContextBeyondModel { limit: u64, model: u64 },
    /// Attention on the array over a cache whose elements are wider than its 8-bit codes.
    WideCacheOnArray { kv_bytes: f64 },
}

impl From<DecodeRefusal> for ServingRefusal {
    fn from(r: DecodeRefusal) -> ServingRefusal {
        ServingRefusal::Step(r)
    }
}

/// The term a pass is bound by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum ServingBind {
    Memory,
    Host,
    Array,
    Vector,
    Scale,
}

/// Tokens a sequence emits per pass: its verified token, plus each draft accepted after every draft before it.
pub fn emitted_per_pass(acceptance: &[f64]) -> f64 {
    let mut run = 1.0;
    let mut sum = 1.0;
    for a in acceptance {
        run *= a;
        sum += run;
    }
    sum
}

/// The shape of a Gated-DeltaNet recurrence, from the header: what a log entry and a dense state hold.
struct Recurrence {
    /// Elements of one layer's dense state: one `key × value` matrix per value head.
    state: f64,
    /// Elements one token adds to one layer's log: every key group's key, every value head's value, decay exponent
    /// and gate.
    log: f64,
    key: f64,
    value: f64,
    heads: f64,
}

fn recurrence(h: &GgufHeader) -> Option<Recurrence> {
    let heads = h.meta_u64("ssm.time_step_rank")?;
    let key = h.meta_u64("ssm.state_size")?;
    let inner = h.meta_u64("ssm.inner_size")?;
    let groups = h.meta_u64("ssm.group_count")?;
    let value = inner / heads;
    Some(Recurrence {
        state: (heads * key * value) as f64,
        log: (groups * key + heads * value + 2 * heads) as f64,
        key: key as f64,
        value: value as f64,
        heads: heads as f64,
    })
}

/// An attention head's key and value widths, as the decode step reads them.
struct HeadWidths {
    key: f64,
    value: f64,
}

fn head_widths(h: &GgufHeader) -> HeadWidths {
    let embd = h.meta_u64("embedding_length").unwrap_or(0);
    let heads = h.meta_u64("attention.head_count").unwrap_or(1).max(1);
    HeadWidths {
        key: h.meta_u64("attention.key_length").unwrap_or(embd / heads) as f64,
        value: h.meta_u64("attention.value_length").unwrap_or(embd / heads) as f64,
    }
}

/// A pass as a function of its batch. Everything a pass costs is shared by the batch or paid by each sequence,
/// so the cost is affine in the batch and in how the batch splits between the card's memory and the host's.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PassShape {
    /// Token-rows each sequence carries through a pass: its verified token and its drafts.
    pub rows_per_sequence: u32,
    /// Tokens each sequence emits per pass, expected.
    pub emitted_per_sequence: f64,
    /// Weight bytes a pass streams, whatever its batch.
    pub weight_bytes: f64,
    /// Embedding-table bytes a pass reads for each sequence: a row per verified row and per drafting step.
    pub embed_bytes: f64,
    /// State, cache, log and ring bytes a pass reads and writes for each sequence.
    pub read_bytes: f64,
    pub write_bytes: f64,
    /// Of those, the bytes of each sequence's recurrent states read and written back in place; the rest are read in
    /// long runs with the pass's rows appended after them.
    pub rewrite_bytes: f64,
    pub array_macs: f64,
    pub vector_macs: f64,
    /// Block-scale lane cycles a pass spends for each sequence.
    pub scale_cycles: f64,
    /// Bytes every serving of the checkpoint holds: the weights and the embedding table.
    pub held_shared: f64,
    /// Bytes each sequence holds: its states, its log, its ring, its caches.
    pub held_per_sequence: f64,
    /// Bytes each sequence's token-rows take on chip through a layer.
    pub on_chip_per_sequence: f64,
    /// The weakest acceptance the shape rests on.
    pub provenance: Provenance,
}

/// One pass of a serving on a card.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct ServingCost {
    pub card: &'static str,
    pub batch: u32,
    /// Of the batch, the sequences whose state and caches live in host memory.
    pub host_sequences: u32,
    pub rows_per_sequence: u32,
    pub emitted_per_sequence: f64,
    /// Weight bytes a pass streams, shared by the batch, embedding rows included.
    pub weight_bytes: f64,
    /// Bytes a pass moves for each sequence: its state, its caches, its log, its conv ring.
    pub sequence_bytes: f64,
    /// Of `sequence_bytes`, the recurrent states' bytes read and written back in place.
    pub rewrite_bytes: f64,
    pub array_macs: f64,
    pub vector_macs: f64,
    pub scale_cycles: f64,
    pub memory_s: f64,
    pub host_s: f64,
    pub array_s: f64,
    pub vector_s: f64,
    pub scale_s: f64,
    /// The largest of the five: the units and the memories overlap, so no pass is faster.
    pub pass_s: f64,
    pub binds: ServingBind,
    pub held_bytes: f64,
    pub capacity_bytes: f64,
    pub host_held_bytes: f64,
    pub host_capacity_bytes: f64,
    pub on_chip_bytes: f64,
    pub on_chip_capacity: f64,
    /// The weakest fact the price rests on.
    pub provenance: Provenance,
}

impl ServingCost {
    pub fn tokens_per_s(&self) -> f64 {
        self.batch as f64 * self.emitted_per_sequence / self.pass_s
    }
}

impl PassShape {
    /// The shape of a pass of `s` over the checkpoint `h` describes. The batch and the host split are the
    /// cost's arguments; everything else in `s` is read here.
    pub fn of(h: &GgufHeader, s: &Serving) -> Result<PassShape, ServingRefusal> {
        if s.activation_words == 0 {
            return Err(ServingRefusal::Empty);
        }
        if h.meta_u64("expert_count").is_some_and(|e| e > 0) {
            return Err(ServingRefusal::Routed);
        }
        let k = s.draft.as_ref().map_or(0, |d| d.tokens);
        if let Some(d) = &s.draft {
            if d.acceptance.len() != d.tokens as usize || d.acceptance.iter().any(|a| !(0.0..=1.0).contains(&a.value)) {
                return Err(ServingRefusal::BadDraft(format!(
                    "{} drafted tokens with acceptance {:?}",
                    d.tokens,
                    d.acceptance.iter().map(|a| a.value).collect::<Vec<_>>()
                )));
            }
        }
        let shares: f64 = s.contexts.classes.iter().map(|c| c.share).sum();
        if s.contexts.classes.is_empty()
            || (shares - 1.0).abs() > 1e-9
            || s.contexts.classes.iter().any(|c| !(c.share > 0.0) || !(c.position >= 0.0) || c.position > c.limit as f64)
        {
            return Err(ServingRefusal::BadContexts(format!("{:?}", s.contexts.classes)));
        }
        if let Some(model) = h.meta_u64("context_length") {
            if s.contexts.longest() > model {
                return Err(ServingRefusal::ContextBeyondModel { limit: s.contexts.longest(), model });
            }
        }
        // An 8-bit cache code and its share of a per-row scale: up to one byte and a quarter.
        if s.attention == AttentionMath::Array && s.kv_bytes > 1.25 {
            return Err(ServingRefusal::WideCacheOnArray { kv_bytes: s.kv_bytes });
        }
        // The step at one position: every attention term it prices is then per position, scaled below by the
        // mix's mean position.
        let deployment = Deployment {
            ctx: 1,
            weights: WeightBytes::AsStored,
            kv_bytes: s.kv_bytes,
            state_bytes: s.state_bytes,
            activation_bytes: 2.0,
            speculative: k > 0,
        };
        let step = DecodeStep::of(h, &deployment)?;
        let blocks = h.meta_u64("block_count").unwrap_or(0) as u32;
        let drafts = h.meta_u64("nextn_predict_layers").unwrap_or(0) as u32;
        let is_draft = |layer: Option<u32>| layer.is_some_and(|l| l >= blocks.saturating_sub(drafts));
        let tensors: BTreeMap<&str, &super::decode::TensorRow> = h.tensors.iter().map(|t| (t.name.as_str(), t)).collect();
        let rec = recurrence(h);
        let head = head_widths(h);

        let n = (1 + k) as f64;
        let kf = k as f64;
        let acceptance: Vec<f64> = s.draft.as_ref().map_or(Vec::new(), |d| d.acceptance.iter().map(|a| a.value).collect());
        let emitted = emitted_per_pass(&acceptance);
        let words = s.activation_words as f64;
        let vocab = tensors
            .get("token_embd.weight")
            .and_then(|t| t.dims.get(1).copied())
            .ok_or_else(|| ServingRefusal::MissingTensor("token_embd.weight".into()))? as f64;
        let head_share = s.draft.as_ref().and_then(|d| d.head_rows).map_or(1.0, |r| (r as f64 / vocab).min(1.0));
        // The tokens a flush replays, whether the state is lazy, the windows its log holds at its longest, and the
        // mean log length a pass starts with: the log grows by the emitted tokens each pass from empty to the flush,
        // or under a trailing flush from one window to two, the window being replayed behind the newer one.
        let (flush_tokens, lazy, windows, window) = match s.flush {
            StateFlush::EveryPass => (1.0, false, 0.0, 0.0),
            StateFlush::Every { tokens } => {
                let t = tokens.max(1) as f64;
                (t, true, 1.0, ((t - emitted) / 2.0).max(0.0))
            }
            StateFlush::Trailing { tokens } => {
                let t = tokens.max(1) as f64;
                (t, true, 2.0, t + ((t - emitted) / 2.0).max(0.0))
            }
        };
        let position = s.contexts.mean_position();
        // Positions each sequence's caches hold.
        let kv_positions = match s.kv_page {
            Some(page) => position + page as f64 / 2.0,
            None => s.contexts.mean_limit(),
        };

        let mut shape = PassShape {
            rows_per_sequence: 1 + k,
            emitted_per_sequence: emitted,
            weight_bytes: 0.0,
            embed_bytes: 0.0,
            read_bytes: 0.0,
            write_bytes: 0.0,
            rewrite_bytes: 0.0,
            array_macs: 0.0,
            vector_macs: 0.0,
            scale_cycles: 0.0,
            held_shared: step
                .held
                .iter()
                .filter(|c| matches!(c.class, HeldClass::Weights | HeldClass::EmbeddingTable))
                .map(|c| c.bytes)
                .sum(),
            held_per_sequence: 0.0,
            on_chip_per_sequence: 0.0,
            provenance: s
                .draft
                .as_ref()
                .and_then(|d| d.acceptance.iter().map(|a| a.provenance).max())
                .unwrap_or(Provenance::Published),
        };
        for st in &step.stages {
            let draft = is_draft(st.layer);
            // Rows the stage multiplies per pass, per sequence: every verified row, or one row per drafting step.
            let rows = if draft { kf } else { n };
            match st.kind {
                StageKind::Embed => {
                    // One table row for every verified row and every drafting step's input token.
                    shape.embed_bytes += st.weight_bytes * (n + kf);
                }
                StageKind::Project | StageKind::Head | StageKind::Norm | StageKind::Router => {
                    let reads = if st.kind == StageKind::Head && !draft {
                        // The verifying head, and each drafting step's head over its rows.
                        1.0 + kf * head_share
                    } else if draft {
                        kf
                    } else {
                        1.0
                    };
                    shape.weight_bytes += st.weight_bytes * reads;
                    let head_rows = if st.kind == StageKind::Head { n + kf * head_share } else { rows };
                    for name in &st.tensors {
                        let t = tensors.get(name.as_str()).ok_or_else(|| ServingRefusal::MissingTensor(name.clone()))?;
                        let ty = h.tensor_type(t)?;
                        let elems: f64 = t.dims.iter().product::<u64>() as f64;
                        if st.kind == StageKind::Norm || t.dims.len() < 2 {
                            // A norm: a sum of squares and a scale per element.
                            shape.vector_macs += 2.0 * elems * rows;
                        } else if let Some(p) = array_passes(ty) {
                            shape.array_macs += elems * p as f64 * words * head_rows;
                            let lane = lane_cycles(ty, s.scaling).expect("the array reads every type it passes");
                            shape.scale_cycles += elems / ty.block_elems() as f64 * lane * words * head_rows;
                        } else if is_float(ty) {
                            shape.vector_macs += elems * head_rows;
                        } else {
                            return Err(ServingRefusal::Undecoded { tensor: name.clone(), ty });
                        }
                    }
                }
                StageKind::Attention => {
                    shape.weight_bytes += st.weight_bytes * if draft { kf } else { 1.0 };
                    // One read of the cache serves every row of the pass; every row's key and value are written.
                    let per_position = st.state_write;
                    shape.read_bytes += per_position * position * if draft { kf } else { 1.0 };
                    shape.write_bytes += per_position * rows;
                    // Scores and the weighted sum: a multiply-add per head per key and value element per position.
                    let products = st.ops / 2.0 * position * rows;
                    let heads = match st.split {
                        Split::Heads { q, .. } => q as f64,
                        _ => 1.0,
                    };
                    shape.vector_macs += SOFTMAX_MACS_PER_SCORE * heads * position * rows;
                    match s.attention {
                        AttentionMath::Vector => shape.vector_macs += products,
                        AttentionMath::Array => {
                            shape.array_macs += 2.0 * products;
                            // A lane rounds every score, and every value element's sum over each block of positions.
                            shape.scale_cycles += heads * position * rows * (1.0 + head.value / VALUE_BLOCK);
                        }
                    }
                    if s.kv_rotated {
                        // Every cached key and value head rotated, every query head, every weighted sum rotated back:
                        // a transform of `n` elements is `n·log2 n` additions.
                        let transform = |n: f64| n * n.max(1.0).log2();
                        let kv_heads = match st.split {
                            Split::Heads { kv, .. } => kv as f64,
                            _ => 0.0,
                        };
                        shape.vector_macs +=
                            (kv_heads * (transform(head.key) + transform(head.value)) + heads * (transform(head.key) + transform(head.value))) * rows;
                    }
                    shape.held_per_sequence += per_position * kv_positions;
                }
                StageKind::Recurrence => {
                    shape.weight_bytes += st.weight_bytes;
                    let r = rec.as_ref().ok_or_else(|| {
                        ServingRefusal::Step(DecodeRefusal::MissingMetadata(
                            "ssm.time_step_rank / ssm.state_size / ssm.inner_size / ssm.group_count".into(),
                        ))
                    })?;
                    let dense = r.state * s.state_bytes;
                    // The conv ring is a log of the last inputs: a pass reads the ring and appends its rows.
                    let taps = h.meta_u64("ssm.conv_kernel").unwrap_or(1) as f64;
                    let ring = (st.state_read - dense).max(0.0);
                    let ring_row = ring / (taps - 1.0).max(1.0);
                    shape.read_bytes += dense + ring;
                    shape.write_bytes += ring_row * n;
                    shape.vector_macs += taps * ring_row / s.state_bytes * n;
                    shape.held_per_sequence += ring + ring_row * n;
                    if lazy {
                        let log = r.log * s.state_bytes;
                        // The state is read once; the log since the last flush is read with it; the pass appends
                        // its rows; the state is written once every `flush_tokens` emitted tokens, all at once or
                        // a `1/flush_tokens` share an emitted token.
                        shape.read_bytes += window * log;
                        shape.write_bytes += n * log + emitted / flush_tokens * dense;
                        // A pass that flushes reads the state and writes it back, the others read it alone; under a
                        // trailing flush every pass reads and writes back its emitted tokens' shares.
                        let flushing = (emitted / flush_tokens).min(1.0);
                        shape.rewrite_bytes += flushing * dense + emitted / flush_tokens * dense;
                        // Each row reads the state once through its folded query and walks the log back to the
                        // flush point; a flush folds `flush_tokens` rank-one updates into the state.
                        let walk = window + (n - 1.0) / 2.0;
                        shape.vector_macs += n * r.state + n * r.heads * walk * (2.0 * r.key + r.value) + 2.0 * emitted * r.state;
                        shape.held_per_sequence += dense + (windows * flush_tokens + n) * log;
                    } else {
                        // Every row's decay, rank-one update and read-out; every candidate prefix's state written.
                        shape.write_bytes += n * dense;
                        shape.rewrite_bytes += dense + n * dense;
                        shape.vector_macs += n * 3.0 * r.state;
                        shape.held_per_sequence += n * dense;
                    }
                }
            }
        }
        // Through a layer the card holds each row's f32 residual and its widest activations in their words.
        let width = h.meta_u64("embedding_length").unwrap_or(0) as f64;
        let ffn = h.meta_u64("feed_forward_length").unwrap_or(0) as f64;
        shape.on_chip_per_sequence = n * (4.0 * width + words * (ffn + width));
        Ok(shape)
    }

    /// The pass of `batch` sequences, `host_sequences` of them held in host memory, on `card`.
    pub fn cost(&self, card: &Card, batch: u32, host_sequences: u32) -> Result<ServingCost, ServingRefusal> {
        if batch == 0 {
            return Err(ServingRefusal::Empty);
        }
        let host_sequences = host_sequences.min(batch);
        let b = batch as f64;
        let bh = host_sequences as f64;
        let bd = b - bh;
        let held_bytes = self.held_shared + bd * self.held_per_sequence;
        let capacity_bytes = card.memory.capacity.value;
        let host_held_bytes = bh * self.held_per_sequence;
        let host_capacity_bytes = card.host.map_or(0.0, |h| h.capacity.value);
        if host_sequences > 0 && card.host.is_none() {
            return Err(ServingRefusal::NoHostLink);
        }
        if held_bytes > capacity_bytes {
            return Err(ServingRefusal::NotResident { needed: held_bytes, capacity: capacity_bytes });
        }
        if host_held_bytes > host_capacity_bytes {
            return Err(ServingRefusal::NotResident { needed: host_held_bytes, capacity: host_capacity_bytes });
        }
        let on_chip_bytes = b * self.on_chip_per_sequence;
        let on_chip_capacity = card.on_chip.capacity.value;
        if on_chip_bytes > on_chip_capacity {
            return Err(ServingRefusal::OnChipOverflow { needed: on_chip_bytes, capacity: on_chip_capacity });
        }
        let weight_bytes = self.weight_bytes + b * self.embed_bytes;
        let sequence_bytes = self.read_bytes + self.write_bytes;
        let at = |share: Fact<f64>| card.memory.bandwidth.value * share.value;
        let memory_s = weight_bytes / at(card.streams.read)
            + bd * (self.rewrite_bytes / at(card.streams.rewrite) + (sequence_bytes - self.rewrite_bytes) / at(card.streams.append));
        let host_s = match card.host {
            Some(h) if host_sequences > 0 => {
                (bh * self.read_bytes / (h.read.value * h.stream.value)).max(bh * self.write_bytes / (h.write.value * h.stream.value))
            }
            _ => 0.0,
        };
        let array_macs = b * self.array_macs;
        let vector_macs = b * self.vector_macs;
        let scale_cycles = b * self.scale_cycles;
        let array_s = array_macs / card.array.value;
        let vector_s = vector_macs / card.vector.value;
        let scale_s = scale_cycles / card.scale.value;
        let (pass_s, binds) = [
            (memory_s, ServingBind::Memory),
            (host_s, ServingBind::Host),
            (array_s, ServingBind::Array),
            (vector_s, ServingBind::Vector),
            (scale_s, ServingBind::Scale),
        ]
        .into_iter()
        .max_by(|a, b| a.0.total_cmp(&b.0))
        .expect("five terms");
        let mut facts = vec![
            card.memory.bandwidth,
            card.memory.capacity,
            card.streams.read,
            card.streams.rewrite,
            card.streams.append,
            card.array,
            card.vector,
            card.scale,
            card.on_chip.capacity,
        ];
        if let (Some(h), true) = (card.host, host_sequences > 0) {
            facts.extend([h.capacity, h.read, h.write, h.stream]);
        }
        let provenance = facts.iter().map(|f| f.provenance).max().unwrap_or(Provenance::Published).max(self.provenance);
        Ok(ServingCost {
            card: card.name,
            batch,
            host_sequences,
            rows_per_sequence: self.rows_per_sequence,
            emitted_per_sequence: self.emitted_per_sequence,
            weight_bytes,
            sequence_bytes,
            rewrite_bytes: self.rewrite_bytes,
            array_macs,
            vector_macs,
            scale_cycles,
            memory_s,
            host_s,
            array_s,
            vector_s,
            scale_s,
            pass_s,
            binds,
            held_bytes,
            capacity_bytes,
            host_held_bytes,
            host_capacity_bytes,
            on_chip_bytes,
            on_chip_capacity,
            provenance,
        })
    }

    /// Most sequences the card's memory holds beside the weights.
    fn card_room(&self, card: &Card) -> u32 {
        let room = (card.memory.capacity.value - self.held_shared) / self.held_per_sequence.max(1.0);
        room.max(0.0).floor().min(u32::MAX as f64) as u32
    }

    /// Most sequences the host memory holds.
    fn host_room(&self, card: &Card) -> u32 {
        card.host.map_or(0, |h| (h.capacity.value / self.held_per_sequence.max(1.0)).floor().min(u32::MAX as f64) as u32)
    }

    /// The pass of `batch` sequences split between the card's memory and the host's as fast as the split allows:
    /// the card's memory streams far faster, so it holds sequences until its own traffic would outlast the host
    /// link's.
    pub fn cost_split(&self, card: &Card, batch: u32, host_memory: bool) -> Result<ServingCost, ServingRefusal> {
        if !host_memory {
            return self.cost(card, batch, 0);
        }
        if card.host.is_none() {
            return Err(ServingRefusal::NoHostLink);
        }
        let lo = batch.saturating_sub(self.host_room(card));
        let hi = batch.min(self.card_room(card));
        if lo > hi {
            // Neither memory holds the rest: the cost names what would not fit.
            return self.cost(card, batch, batch - hi);
        }
        // The card's time rises with the sequences it holds and the host's falls: the fastest split is where they
        // cross, or the end of the range nearest it.
        let card_time = |held: u32| self.cost(card, batch, batch - held).map(|c| c.memory_s.max(c.host_s));
        let (mut a, mut z) = (lo, hi);
        while z - a > 2 {
            let m1 = a + (z - a) / 3;
            let m2 = z - (z - a) / 3;
            if card_time(m1)? <= card_time(m2)? {
                z = m2;
            } else {
                a = m1;
            }
        }
        let mut best: Option<ServingCost> = None;
        for held in a..=z {
            let c = self.cost(card, batch, batch - held)?;
            if best.as_ref().map_or(true, |b| c.pass_s < b.pass_s) {
                best = Some(c);
            }
        }
        Ok(best.expect("a nonempty range"))
    }

    /// The batch that makes `card` emit the most tokens per second, and its pass. Ties go to the smaller batch.
    pub fn best(&self, card: &Card, host_memory: bool) -> Result<ServingCost, ServingRefusal> {
        let most = if host_memory { self.card_room(card).saturating_add(self.host_room(card)) } else { self.card_room(card) };
        let mut best: Option<ServingCost> = None;
        for batch in 1..=most.max(1) {
            match self.cost_split(card, batch, host_memory) {
                Ok(c) => {
                    if best.as_ref().map_or(true, |b| c.tokens_per_s() > b.tokens_per_s() * (1.0 + 1e-12)) {
                        best = Some(c);
                    }
                }
                Err(ServingRefusal::NotResident { .. } | ServingRefusal::OnChipOverflow { .. }) => break,
                Err(e) => return Err(e),
            }
        }
        best.map_or_else(|| self.cost_split(card, 1, host_memory), Ok)
    }
}

/// Prices one pass of `s` over the checkpoint `h` describes, on `card`.
pub fn price_serving(h: &GgufHeader, card: &Card, s: &Serving) -> Result<ServingCost, ServingRefusal> {
    if s.batch == 0 {
        return Err(ServingRefusal::Empty);
    }
    PassShape::of(h, s)?.cost_split(card, s.batch, s.host_memory)
}

/// The most sequences `s` can carry on `card`: every larger batch is refused for capacity.
pub fn max_batch(h: &GgufHeader, card: &Card, s: &Serving) -> Result<u32, ServingRefusal> {
    let shape = PassShape::of(h, s)?;
    let mut n = 0;
    let most = if s.host_memory { shape.card_room(card).saturating_add(shape.host_room(card)) } else { shape.card_room(card) };
    for batch in 1..=most.saturating_add(1) {
        match shape.cost_split(card, batch, s.host_memory) {
            Ok(_) => n = batch,
            Err(ServingRefusal::NotResident { .. } | ServingRefusal::OnChipOverflow { .. }) => break,
            Err(e) => return Err(e),
        }
    }
    if n == 0 {
        shape.cost_split(card, 1, s.host_memory)?;
    }
    Ok(n)
}

/// The batch at which `s` emits the most tokens per second on `card`, and its pass.
pub fn best_serving(h: &GgufHeader, card: &Card, s: &Serving) -> Result<ServingCost, ServingRefusal> {
    PassShape::of(h, s)?.best(card, s.host_memory)
}
