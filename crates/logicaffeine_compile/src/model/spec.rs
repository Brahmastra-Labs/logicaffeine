//! A decoder-only language model's architecture, as data.
//!
//! [`ModelSpec`] says what a checkpoint computes — the width, the vocabulary, and for every layer its token mixer
//! (attention or a gated-delta recurrence), its feed-forward block (dense or a mixture of experts) and how its
//! norms are arranged. It is read from the checkpoint's own metadata ([`ModelSpec::from_gguf`]), so bringing up a
//! model is reading its file: the per-architecture knowledge lives in one match over `general.architecture`, and
//! everything downstream (the authored `.lg`, the reference forward, the weight binding) is generic over the spec.
//!
//! The meaning of each field follows llama.cpp's graph for that architecture (`src/models/<arch>.cpp` at the pinned
//! commit), which is the reference the conformance locks hold the model to.

use super::gguf::{Gguf, MetaValue};
pub use crate::tensor_recognize::qwen::Window;

/// Qwen3.8-Flash-Next's model-wide parts from its keys (llama.cpp `qwen4exp`): the hyper-connections, the per-layer
/// embedding (one layer, as the GGUF holds one table) and QSA's indexer, which selects on the attention layers whose
/// compression ratio is not zero, one ratio for them all (the draft block's among them), as llama.cpp selects.
#[allow(clippy::type_complexity)]
fn qwen4exp_parts<S: SpecSource + ?Sized>(
    k: &Keys<'_, S>,
    layers: &[Layer],
) -> Result<(Option<HyperConnections>, Option<Ple>, Option<Indexer>), SpecError> {
    let hyper = HyperConnections { lanes: k.usize("hyper_connection.count")?, rank: k.usize("hyper_connection.low_rank")? };
    if hyper.lanes < 2 || hyper.rank == 0 {
        return Err(SpecError::Inconsistent(format!("hyper-connections of {} lanes over a rank of {}", hyper.lanes, hyper.rank)));
    }
    let ints = |suffix: &str| -> Result<Vec<u64>, SpecError> {
        k.get(suffix)
            .and_then(MetaValue::as_array)
            .and_then(|a| a.iter().map(MetaValue::as_u64).collect::<Option<Vec<u64>>>())
            .ok_or_else(|| SpecError::MissingKey(k.key(suffix)))
    };
    let ratios = ints("attention.compress_ratios")?;
    if ratios.len() < layers.len() {
        return Err(SpecError::Inconsistent(format!("{} compression ratios for {} layers", ratios.len(), layers.len())));
    }
    let mut block = None;
    let mut selecting = Vec::new();
    for (l, &r) in ratios.iter().enumerate() {
        let r = r as usize;
        if r == 0 {
            continue;
        }
        if *block.get_or_insert(r) != r {
            return Err(SpecError::Inconsistent(format!("QSA's blocks of {r} on block {l}, of {} before it", block.unwrap())));
        }
        match layers.get(l).map(|layer| &layer.mixer) {
            Some(Mixer::Attention(_)) => selecting.push(l),
            Some(_) => return Err(SpecError::Inconsistent(format!("layer {l} is not attention but compresses by {r}"))),
            // The draft block's.
            None => {}
        }
    }
    let indexer = Indexer {
        heads: k.usize("attention.indexer.head_count")?,
        head_dim: k.usize("attention.indexer.key_length")?,
        budget: k.usize("attention.indexer.top_k")?,
        block: block.ok_or_else(|| SpecError::Inconsistent("QSA's indexer with no block that selects".into()))?,
        layers: selecting,
    };
    if indexer.block < 2 || indexer.budget % indexer.block != 0 {
        return Err(SpecError::Inconsistent(format!("QSA's budget of {} in blocks of {}", indexer.budget, indexer.block)));
    }
    let ple = match k.get("ple.layers").and_then(MetaValue::as_array) {
        None => None,
        Some(_) => {
            let layer = match ints("ple.layers")?[..] {
                [l] => l as usize,
                ref other => return Err(SpecError::Inconsistent(format!("the per-layer embedding on layers {other:?}: one table, one layer"))),
            };
            if !matches!(layers.get(layer).map(|l| &l.mixer), Some(Mixer::GatedDelta(_))) {
                return Err(SpecError::Inconsistent(format!("the per-layer embedding on layer {layer}, not a Gated DeltaNet layer")));
            }
            let ple = Ple {
                layer,
                ngram: k.usize("ple.ngram_size")?,
                heads_per_ngram: k.usize("ple.heads_per_ngram")?,
                head_dim: k.usize("embedding_length_per_layer_input")?,
                conv_taps: k.usize("ple.conv_kernel")?,
                eos: k.usize("ple.eos_token_id")? as u32,
                multipliers: ints("ple.layer_multipliers")?,
                head_offsets: ints("ple.head_offsets")?,
                head_vocab: ints("ple.head_vocab_sizes")?,
            };
            let heads = (ple.ngram.max(1) - 1) * ple.heads_per_ngram;
            if ple.ngram < 2 || ple.multipliers.len() != ple.ngram || ple.head_offsets.len() != heads || ple.head_vocab.len() != heads {
                return Err(SpecError::Inconsistent(format!(
                    "n-grams of {} over {heads} heads with {} multipliers, {} offsets and {} moduli",
                    ple.ngram,
                    ple.multipliers.len(),
                    ple.head_offsets.len(),
                    ple.head_vocab.len()
                )));
            }
            Some(ple)
        }
    };
    Ok((Some(hyper), ple, Some(indexer)))
}

/// What a checkpoint computes.
#[derive(Clone, Debug, PartialEq)]
pub struct ModelSpec {
    /// `general.architecture`.
    pub arch: String,
    /// Hidden width.
    pub d: usize,
    /// Vocabulary size (rows of the token embedding).
    pub vocab: usize,
    /// The trained context window.
    pub ctx_len: usize,
    /// The RMSNorm epsilon every norm of the model uses.
    pub rms_eps: f64,
    /// A scale applied to the token embedding as it is read (Gemma: `√d`).
    pub embedding_scale: Option<f64>,
    /// The output projection is the token embedding (no `output.weight` in the file).
    pub tied_output: bool,
    /// Soft-capping of the final logits (`cap · tanh(x / cap)`).
    pub final_logit_softcap: Option<f64>,
    pub layers: Vec<Layer>,
    /// Draft blocks for multi-token prediction appended after the model's layers.
    pub mtp_blocks: usize,
    /// The tensor stem of the norm on each block's FFN input: `ffn_norm`, or `post_attention_norm` in Qwen3.5,
    /// whose converter names the pre-FFN norm after the attention it follows.
    pub ffn_norm: &'static str,
    /// The residual stream's hyper-connections, in place of every norm (Qwen3.8-Flash-Next); `None` for one residual.
    pub hyper: Option<HyperConnections>,
    /// The hashed n-gram embedding one layer's input takes (Qwen3.8-Flash-Next's per-layer embedding).
    pub ple: Option<Ple>,
    /// The sparse selection of the positions every attention layer's queries attend (Qwen's QSA).
    pub indexer: Option<Indexer>,
}

/// Hyper-connections: the residual is `lanes` copies of the hidden state. Before each sublayer the lanes are normed
/// each on its own (one weight across all of them), weighted by `σ(up·silu(down·x̂ / lanes))` through a bottleneck of
/// `rank` and averaged into the sublayer's input; the sublayer's output joins every lane scaled by
/// `2σ(inject·x̂ / lanes)`. A last mix with no injection feeds the head in place of an output norm.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HyperConnections {
    pub lanes: usize,
    pub rank: usize,
}

/// The per-layer n-gram embedding: each position's n-grams of 2 to `ngram` tokens, read back no further than the last
/// end of document, are hashed `heads_per_ngram` ways each (`t₀·m₀ ⊕ t₁·m₁ ⊕ …` modulo a head's prime, from the head's
/// offset in one table); the heads' rows, side by side, project to a value and to a key for each hyper-connection lane,
/// the lane's normed state gating the value by `σ(sign(g)·√|g|)` of its agreement `g` with the key, and a dilated
/// depthwise convolution over the normed gated values adds local context. Its sum joins the layer's residual input.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ple {
    /// The layer whose input it joins.
    pub layer: usize,
    /// The longest n-gram: its context reaches `ngram - 1` tokens back, and the convolution's dilation is `ngram`.
    pub ngram: usize,
    pub heads_per_ngram: usize,
    /// A head's row in the table.
    pub head_dim: usize,
    pub conv_taps: usize,
    /// The token that ends a document: no n-gram reaches back across it, and it stands in before the first token.
    pub eos: u32,
    /// The odd multiplier each position back is hashed by.
    pub multipliers: Vec<u64>,
    /// Each head's first row in the table.
    pub head_offsets: Vec<u64>,
    /// Each head's prime modulus.
    pub head_vocab: Vec<u64>,
}

/// Qwen's sparse attention selection: `heads` index queries of `head_dim` over one index key a position, the keys of
/// each complete block of `block` positions averaged, normed and rotated at the block's first position, each block
/// scored by `Σ_h relu(q_h·k̄)/√head_dim`; a query attends the best `budget / block` blocks and the positions after the
/// last complete block, every visible position while the complete blocks are no more than that. The attention layers
/// in `layers` select; any other attends every position.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Indexer {
    pub heads: usize,
    pub head_dim: usize,
    pub budget: usize,
    pub block: usize,
    pub layers: Vec<usize>,
}

/// One block: its token mixer, its feed-forward block, and its norms.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub mixer: Mixer,
    pub ffn: Ffn,
    /// A norm applied to the mixer's output before the residual add (Gemma's sandwich norm,
    /// `post_attention_norm`).
    pub post_mixer_norm: bool,
    /// A norm applied to the feed-forward output before the residual add (`post_ffw_norm`).
    pub post_ffn_norm: bool,
}

/// How a block mixes information across positions.
#[derive(Clone, Debug, PartialEq)]
pub enum Mixer {
    Attention(Attention),
    GatedDelta(GatedDelta),
    Mla(Mla),
}

/// Multi-head latent attention (DeepSeek V2/V3): the keys and values of every head are expanded from one shared
/// low-rank latent per position (`kv_lora_rank` wide, RMS-normed), and each head's query and key carry a further
/// `qk_rope` dimensions that rotate — the key's shared by every head. The query is a full projection, or itself
/// low-rank (`q_lora_rank`, normed between its two halves).
#[derive(Clone, Debug, PartialEq)]
pub struct Mla {
    pub heads: usize,
    pub q_lora_rank: Option<usize>,
    pub kv_lora_rank: usize,
    /// Per-head query/key dimensions that do not rotate.
    pub qk_nope: usize,
    /// Per-head query/key dimensions that rotate (the trailing ones).
    pub qk_rope: usize,
    pub v_dim: usize,
    pub rope: Rope,
    /// The score scale: `1/√(qk_nope + qk_rope)`, times YaRN's magnitude squared.
    pub scale: f64,
    /// The checkpoint stores the latent's expansion split into its key half (`attn_k_b`, each head's `kv_lora_rank`
    /// rows of `qk_nope`) and its value half (`attn_v_b`, each head's `v_dim` rows of `kv_lora_rank`), rather than as
    /// one `attn_kv_b` of `qk_nope + v_dim` rows a head.
    pub kv_b_split: bool,
}

/// Grouped-query attention, with the variants checkpoints use.
#[derive(Clone, Debug, PartialEq)]
pub struct Attention {
    pub heads: usize,
    pub kv_heads: usize,
    /// Query/key head width.
    pub head_dim: usize,
    /// Value head width.
    pub value_dim: usize,
    /// The rotary embedding, or `None` for a layer without positions (NoPE).
    pub rope: Option<Rope>,
    /// Per-head RMSNorm of the queries and keys before the rotation.
    pub qk_norm: bool,
    /// The query projection also produces a per-element gate, and the attention output is multiplied by its
    /// sigmoid (Qwen3.5/3.8).
    pub output_gate: bool,
    /// The projections are one fused tensor (`attn_qkv`) rather than three.
    pub fused_qkv: bool,
    /// Biases on the query, key and value projections.
    pub bias: bool,
    /// A bias on the output projection (`attn_output.bias`): gpt-oss's; Qwen2's attention has none.
    pub output_bias: bool,
    /// The positions each query keeps: the last `W` (Mistral, Gemma 2 and 3, gpt-oss), or its own chunk of `C`
    /// (Llama 4's rotated layers).
    pub window: Option<Window>,
    /// A learned per-head logit joins each softmax's denominator (gpt-oss).
    pub sinks: bool,
    /// Soft-capping of the attention scores.
    pub softcap: Option<f64>,
    /// The score scale (`1/√head_dim` unless the checkpoint says otherwise).
    pub scale: f64,
    /// Each query scaled by a factor of its position before it scores (Llama 4's rotation-free layers).
    pub temperature: Option<Temperature>,
    /// A per-head RMSNorm of the queries and keys with no weight (Llama 4's rotated layers, after the rotation: the
    /// same as before it, a rotation keeping each head's norm).
    pub qk_unit_norm: bool,
    /// With `qk_norm`, the queries and the keys are each normed across every head at once, their weights as wide as the
    /// projections (MiniMax-M2's `qk_norm_type` "per_layer"), rather than head by head.
    pub qk_norm_whole: bool,
}

/// Llama 4's attention temperature: the query at position `p` (0-based) scaled by `1 + scale·ln(⌊(p + 1)/floor⌋ + 1)`,
/// so short contexts are untouched and long ones sharpen (`attn_temperature_tuning`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Temperature {
    pub floor: f64,
    pub scale: f64,
}

impl Temperature {
    /// The factor a query at position `p` (0-based) is scaled by.
    pub fn at(self, p: usize) -> f64 {
        1.0 + self.scale * (((p as f64 + 1.0) / self.floor).floor() + 1.0).ln()
    }
}

/// The rotary position embedding.
#[derive(Clone, Debug, PartialEq)]
pub struct Rope {
    /// How many of each head's dimensions rotate (the rest pass through).
    pub dims: usize,
    pub theta: f64,
    /// NEOX pairs dimension `i` with `i + dims/2`; the original form pairs `2i` with `2i+1`.
    pub neox: bool,
    pub scaling: RopeScaling,
    /// Interleaved multi-section rotation (`rope.dimension_sections`, Qwen3.5): frequency `i` reads position stream
    /// `i mod Σsections` as llama.cpp's IMROPE assigns it. A text token's streams are `[p, p, p, 0]`, so a frequency
    /// assigned to the fourth stream does not turn.
    pub sections: Option<[usize; 4]>,
}

impl Rope {
    /// `(cos, sin)` of frequency `i` at text position `pos`, each scaled by the rotation's magnitude, as ggml's CPU
    /// rope computes them (`rope_yarn`): the angle is `pos·θ^(−2i/dims)`, divided by the checkpoint's frequency
    /// factor `factor` when it stores one; linear scaling divides it by its factor; YaRN blends the divided and the
    /// undivided angle by a ramp over the correction dimensions and scales the pair by its magnitude.
    pub fn rotation(&self, i: usize, pos: usize, factor: Option<f64>) -> (f64, f64) {
        let mut freq = self.theta.powf(-2.0 * i as f64 / self.dims as f64);
        if let Some(f) = factor {
            freq /= f;
        }
        let extrapolated = self.text_position(i, pos) as f64 * freq;
        let (angle, scale) = match self.scaling {
            RopeScaling::Linear { factor } => (extrapolated / factor, 1.0),
            RopeScaling::Yarn { factor, original_ctx, beta_fast, beta_slow, magnitude } => {
                let dims = self.dims as f64;
                let corr = |beta: f64| {
                    dims * (original_ctx as f64 / (beta * 2.0 * std::f64::consts::PI)).ln() / (2.0 * self.theta.ln())
                };
                let low = corr(beta_fast).floor().max(0.0);
                let high = corr(beta_slow).ceil().min(dims - 1.0);
                let ramp = 1.0 - ((i as f64 - low) / (high - low).max(0.001)).clamp(0.0, 1.0);
                (extrapolated / factor * (1.0 - ramp) + extrapolated * ramp, magnitude)
            }
            RopeScaling::None | RopeScaling::Factors => (extrapolated, 1.0),
        };
        let (s, c) = angle.sin_cos();
        (c * scale, s * scale)
    }

    /// The position frequency `i` turns by for a text token at `pos`.
    pub fn text_position(&self, i: usize, pos: usize) -> usize {
        let Some(sec) = self.sections else { return pos };
        let total: usize = sec.iter().sum();
        let sector = i % total;
        let streams = [pos, pos, pos, 0];
        let stream = if sector % 3 == 1 && sector < 3 * sec[1] {
            1
        } else if sector % 3 == 2 && sector < 3 * sec[2] {
            2
        } else if sector % 3 == 0 && sector < 3 * sec[0] {
            0
        } else {
            3
        };
        streams[stream]
    }
}

/// Context extension applied to the rotary frequencies.
#[derive(Clone, Debug, PartialEq)]
pub enum RopeScaling {
    None,
    /// Per-frequency factors stored in the checkpoint (`rope_freqs.weight`, Llama 3.1+).
    Factors,
    Linear { factor: f64 },
    /// YaRN as ggml applies it: each frequency blended between interpolated (`/factor`) and extrapolated by a ramp
    /// over the correction dimensions `beta_fast`/`beta_slow` give at `original_ctx`, and the rotation scaled by
    /// `magnitude` (llama.cpp's `get_mscale` ratio times `rope.scaling.attn_factor`, with its own cancelling factor
    /// folded in).
    Yarn {
        factor: f64,
        original_ctx: usize,
        beta_fast: f64,
        beta_slow: f64,
        magnitude: f64,
    },
}

/// The Gated DeltaNet recurrence (Qwen3.5/3.8 linear-attention layers).
#[derive(Clone, Debug, PartialEq)]
pub struct GatedDelta {
    pub k_heads: usize,
    pub v_heads: usize,
    pub head_dim: usize,
    pub conv_taps: usize,
    /// The activation of the gate each head's normed read is multiplied by.
    pub gate: GateActivation,
}

/// A gate's activation: SiLU (Qwen3.5's and Qwen3-Next's Gated DeltaNet) or the sigmoid (Qwen3.8-Flash-Next's).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateActivation {
    Silu,
    Sigmoid,
}

/// A block's feed-forward network.
#[derive(Clone, Debug, PartialEq)]
pub enum Ffn {
    Dense {
        dff: usize,
        act: Act,
    },
    Moe(Moe),
}

/// A mixture of experts.
#[derive(Clone, Debug, PartialEq)]
pub struct Moe {
    pub experts: usize,
    pub top_k: usize,
    pub expert_dff: usize,
    pub act: Act,
    pub gating: Gating,
    /// The chosen experts' weights are renormalized to sum to one.
    pub norm_top_k: bool,
    /// A scale applied to the routing weights.
    pub weights_scale: f64,
    pub shared_experts: usize,
    pub shared_dff: usize,
    /// The shared experts' output scaled by `σ(x·g)`, `g` the checkpoint's `ffn_gate_inp_shexp` (Qwen's mixtures).
    pub shared_gate: bool,
    /// Biases on the router and on every expert's projections (gpt-oss).
    pub biases: bool,
    /// Experts chosen within groups (DeepSeek V3): the experts split into `groups` of equal size, each group scored
    /// by its two best experts, only the best `used` groups' experts eligible for the top-k.
    pub groups: Option<(usize, usize)>,
}

/// A gated activation `act(gate) · up`.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    Silu,
    GeluTanh,
    /// gpt-oss: `clamp(gate)·σ(α·gate) · (clamp(up)+1)`.
    SwigluOai { alpha: f64, limit: f64 },
}

/// How a router turns logits into expert weights, and where a weight applies: to its expert's output, unless the gating
/// says otherwise.
#[derive(Clone, Debug, PartialEq)]
pub enum Gating {
    /// Softmax over all experts, then the top-k.
    Softmax,
    /// The top-k logits, then a softmax over them (gpt-oss).
    SoftmaxOfTopK,
    /// Sigmoid scores, optionally with a learned selection bias (DeepSeek V3).
    Sigmoid { bias: bool },
    /// The top-k logits, each chosen expert's INPUT scaled by the sigmoid of its logit and its output summed as it
    /// stands (Llama 4; llama.cpp's `weight_before_ffn`).
    SigmoidScaledInput,
}

/// Why a checkpoint is not a model this crate describes.
#[derive(Clone, Debug, PartialEq)]
pub enum SpecError {
    /// `general.architecture` names an architecture no arm reads yet.
    UnknownArchitecture(String),
    /// A key the architecture requires is absent or of the wrong type.
    MissingKey(String),
    /// A tensor the architecture requires is absent.
    MissingTensor(String),
    /// The keys describe a shape the architecture's graph does not compute.
    Inconsistent(String),
}

impl std::fmt::Display for SpecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SpecError::UnknownArchitecture(a) => write!(f, "architecture `{a}` is not described yet"),
            SpecError::MissingKey(k) => write!(f, "metadata key `{k}` is missing"),
            SpecError::MissingTensor(t) => write!(f, "tensor `{t}` is missing"),
            SpecError::Inconsistent(why) => write!(f, "the checkpoint is inconsistent: {why}"),
        }
    }
}

/// A float key's value. A converter writes the model's hyperparameters as decimals (`1e-6`) and GGUF stores most of
/// them as f32, which cannot hold most decimals; the value meant is the shortest decimal that rounds to the stored
/// f32 — so `rms_eps` reads as `1e-6`, not as the f32 nearest it.
/// Which of `n_layers` blocks slide, as llama.cpp's `load_swa_pattern` reads it: `attention.sliding_window_pattern` as
/// one flag a block, or as a period `every` (`default` where unwritten) in which all but the last block slide (every
/// block at a period of 0) — none where the window is 0.
fn sliding_layers<S: SpecSource + ?Sized>(k: &Keys<'_, S>, n_layers: usize, default: usize, window: usize) -> Result<Vec<bool>, SpecError> {
    Ok(match k.get("attention.sliding_window_pattern") {
        Some(MetaValue::Array { items: a, .. }) => a
            .iter()
            .take(n_layers)
            .map(|v| v.as_bool().or_else(|| v.as_u64().map(|x| x != 0)))
            .collect::<Option<_>>()
            .ok_or_else(|| SpecError::MissingKey(k.key("attention.sliding_window_pattern")))?,
        other => {
            let every = other.and_then(MetaValue::as_u64).map_or(default, |v| v as usize);
            (0..n_layers).map(|l| window > 0 && (every == 0 || l % every < every - 1)).collect()
        }
    })
}

fn decimal(v: &MetaValue) -> Option<f64> {
    match *v {
        MetaValue::F32(f) => f.to_string().parse().ok(),
        _ => v.as_f64(),
    }
}

/// What a checkpoint's architecture is read from: its architecture's name, its keys and its tensor directory — a
/// GGUF checkpoint, or anything describing one.
pub trait SpecSource {
    /// `general.architecture`.
    fn arch(&self) -> &str;
    /// The architecture key `{arch}.{suffix}`.
    fn arch_meta(&self, suffix: &str) -> Option<&MetaValue>;
    /// A tensor's dims, `dims[0]` the fastest-varying.
    fn dims(&self, name: &str) -> Option<&[u64]>;
    /// Every tensor's name, in the directory's order.
    fn tensor_names(&self) -> Vec<&str>;
    fn has(&self, name: &str) -> bool {
        self.dims(name).is_some()
    }
}

impl SpecSource for Gguf {
    fn arch(&self) -> &str {
        &self.arch.arch
    }
    fn arch_meta(&self, suffix: &str) -> Option<&MetaValue> {
        Gguf::arch_meta(self, suffix)
    }
    fn dims(&self, name: &str) -> Option<&[u64]> {
        Gguf::dims(self, name)
    }
    fn tensor_names(&self) -> Vec<&str> {
        self.tensors().iter().map(|t| t.name.as_str()).collect()
    }
    fn has(&self, name: &str) -> bool {
        Gguf::has(self, name)
    }
}

/// Typed reads of a checkpoint's architecture keys.
struct Keys<'a, S: SpecSource + ?Sized> {
    g: &'a S,
}

impl<S: SpecSource + ?Sized> Keys<'_, S> {
    fn key(&self, suffix: &str) -> String {
        format!("{}.{suffix}", self.g.arch())
    }
    fn get(&self, suffix: &str) -> Option<&MetaValue> {
        self.g.arch_meta(suffix)
    }
    fn usize(&self, suffix: &str) -> Result<usize, SpecError> {
        self.get(suffix)
            .and_then(MetaValue::as_u64)
            .map(|v| v as usize)
            .ok_or_else(|| SpecError::MissingKey(self.key(suffix)))
    }
    fn usize_or(&self, suffix: &str, default: usize) -> usize {
        self.get(suffix)
            .and_then(MetaValue::as_u64)
            .map_or(default, |v| v as usize)
    }
    fn f64(&self, suffix: &str) -> Result<f64, SpecError> {
        self.get(suffix)
            .and_then(decimal)
            .ok_or_else(|| SpecError::MissingKey(self.key(suffix)))
    }
    fn f64_or(&self, suffix: &str, default: f64) -> f64 {
        self.get(suffix).and_then(decimal).unwrap_or(default)
    }
}

impl ModelSpec {
    /// Read the architecture of a GGUF checkpoint.
    pub fn from_gguf(g: &Gguf) -> Result<ModelSpec, SpecError> {
        ModelSpec::from_source(g)
    }

    /// Read the architecture of a checkpoint from anything that describes it ([`SpecSource`]).
    pub fn from_source<S: SpecSource + ?Sized>(g: &S) -> Result<ModelSpec, SpecError> {
        let k = Keys { g };
        let arch = g.arch().to_string();
        let d = k.usize("embedding_length")?;
        let vocab = g
            .dims("token_embd.weight")
            .and_then(|dims| dims.get(1).copied())
            .ok_or_else(|| SpecError::MissingTensor("token_embd.weight".into()))? as usize;
        let mtp_blocks = k.usize_or("nextn_predict_layers", 0);
        let n_layers = k.usize("block_count")? - mtp_blocks;
        let heads = k.usize("attention.head_count")?;
        let kv_heads = k.usize_or("attention.head_count_kv", heads);
        let head_dim = k.usize_or("attention.key_length", d / heads);
        let value_dim = k.usize_or("attention.value_length", head_dim);
        let sections = match k.get("rope.dimension_sections").and_then(MetaValue::as_array) {
            None => None,
            Some(a) => {
                let v: Vec<usize> = a.iter().filter_map(MetaValue::as_u64).map(|x| x as usize).collect();
                match v[..] {
                    [a, b, c, d] if a + b + c + d > 0 => Some([a, b, c, d]),
                    _ => return Err(SpecError::MissingKey(k.key("rope.dimension_sections"))),
                }
            }
        };
        let rope = |neox: bool| -> Result<Rope, SpecError> {
            Ok(Rope {
                dims: k.usize_or("rope.dimension_count", head_dim),
                theta: k.f64_or("rope.freq_base", 10000.0),
                neox,
                scaling: rope_scaling(&k, g)?,
                sections,
            })
        };
        // The attention features are read off an attention layer's own tensors (`at`): in a hybrid model block 0
        // may be a recurrent one.
        let attention = |neox: bool, output_gate: bool, at: usize| -> Result<Attention, SpecError> {
            Ok(Attention {
                heads,
                kv_heads,
                head_dim,
                value_dim,
                rope: Some(rope(neox)?),
                qk_norm: g.has(&format!("blk.{at}.attn_q_norm.weight")),
                output_gate,
                fused_qkv: g.has(&format!("blk.{at}.attn_qkv.weight")),
                bias: g.has(&format!("blk.{at}.attn_q.bias")),
                output_bias: g.has(&format!("blk.{at}.attn_output.bias")),
                window: None,
                sinks: false,
                softcap: None,
                scale: 1.0 / (head_dim as f64).sqrt(),
                temperature: None,
                qk_unit_norm: false,
                qk_norm_whole: false,
            })
        };
        let dense = |act: Act| -> Result<Ffn, SpecError> {
            Ok(Ffn::Dense {
                dff: k.usize("feed_forward_length")?,
                act,
            })
        };
        // DeepSeek's mixture (llama.cpp `build_moe_ffn` with `exp_probs_b`, `models/deepseek2.cpp` and
        // `models/glm4-moe.cpp`): experts scored as `expert_gating_func` says (`gating` where it is unset), chosen by
        // score plus the per-expert bias where the checkpoint carries one and within the best `expert_group_used_count`
        // of `expert_group_count` groups where there is more than one, the kept weights renormalized as
        // `expert_weights_norm` says and scaled by `expert_weights_scale`, beside `expert_shared_count` ungated shared
        // experts as wide as one routed expert each.
        let routed = |gating: usize| -> Result<Ffn, SpecError> {
            let gating = match k.usize_or("expert_gating_func", gating) {
                2 => Gating::Sigmoid { bias: g.has("blk.1.exp_probs_b.bias") || has_any(g, "exp_probs_b.bias") },
                _ => Gating::Softmax,
            };
            let shared = k.usize_or("expert_shared_count", 0);
            let expert_dff = k.usize("expert_feed_forward_length")?;
            let groups = match (k.usize_or("expert_group_count", 0), k.usize_or("expert_group_used_count", 0)) {
                (n, used) if n > 1 => Some((n, used)),
                _ => None,
            };
            Ok(Ffn::Moe(Moe {
                experts: k.usize("expert_count")?,
                top_k: k.usize("expert_used_count")?,
                expert_dff,
                act: Act::Silu,
                gating,
                norm_top_k: k.get("expert_weights_norm").and_then(MetaValue::as_bool).unwrap_or(false),
                weights_scale: k.f64_or("expert_weights_scale", 1.0),
                shared_experts: shared,
                shared_dff: expert_dff * shared,
                shared_gate: false,
                biases: false,
                groups,
            }))
        };
        // Qwen's mixture (llama.cpp `models/qwen2moe.cpp`, `models/qwen35moe.cpp`): a softmax router whose top-k weights
        // are renormalized or kept as they are by architecture, each expert as wide as `expert_feed_forward_length`
        // (block 0's of a per-block array, as llama.cpp reads it for every block; unset or 0, the FFN length over the
        // experts a token takes), beside one shared expert as wide as `expert_shared_feed_forward_length` (unset or 0, the
        // FFN length) scaled by `σ(x·g)`. llama.cpp reads no routed scale for it.
        let qwen_mixture = |renormalized: bool| -> Result<Ffn, SpecError> {
            let first = |suffix: &str| {
                k.get(suffix)
                    .and_then(|v| v.as_u64().or_else(|| v.as_array().and_then(|a| a.first()).and_then(MetaValue::as_u64)))
                    .filter(|&w| w > 0)
                    .map(|w| w as usize)
            };
            let experts = k.usize("expert_count")?;
            let top_k = k.usize("expert_used_count")?;
            if experts == 0 || top_k == 0 {
                return Err(SpecError::Inconsistent(format!("{experts} experts, {top_k} a token: {arch} routes to at least one")));
            }
            let ff = || k.usize("feed_forward_length");
            Ok(Ffn::Moe(Moe {
                experts,
                top_k,
                expert_dff: match first("expert_feed_forward_length") {
                    Some(w) => w,
                    None => ff()? / top_k,
                },
                act: Act::Silu,
                gating: Gating::Softmax,
                norm_top_k: renormalized,
                weights_scale: 1.0,
                shared_experts: 1,
                shared_dff: match first("expert_shared_feed_forward_length") {
                    Some(w) => w,
                    None => ff()?,
                },
                shared_gate: true,
                biases: false,
                groups: None,
            }))
        };
        let layers: Vec<Layer> = match arch.as_str() {
            // llama.cpp `models/qwen3.cpp`, `models/qwen2.cpp`, `models/llama.cpp`: attention with NEOX (Qwen) or
            // interleaved (Llama) rotation — Qwen2's with biased projections, read off its tensors — and a SiLU-gated
            // FFN: dense, or for a Llama checkpoint that counts experts (Mixtral; Qwen's mixtures have arches of their
            // own) a softmax router whose top-k weights are renormalized, each expert as wide as
            // `expert_feed_forward_length` or else the FFN length.
            "qwen3" | "qwen2" | "llama" => {
                let mixer = Mixer::Attention(attention(arch != "llama", false, 0)?);
                let ffn = match (arch.as_str(), k.usize_or("expert_count", 0)) {
                    ("llama", experts) if experts > 0 => Ffn::Moe(Moe {
                        experts,
                        top_k: k.usize("expert_used_count")?,
                        expert_dff: match k.get("expert_feed_forward_length").and_then(MetaValue::as_u64) {
                            Some(dff) => dff as usize,
                            None => k.usize("feed_forward_length")?,
                        },
                        act: Act::Silu,
                        gating: Gating::Softmax,
                        norm_top_k: true,
                        weights_scale: 1.0,
                        shared_experts: 0,
                        shared_dff: 0,
                        shared_gate: false,
                        biases: false,
                        groups: None,
                    }),
                    _ => dense(Act::Silu)?,
                };
                (0..n_layers)
                    .map(|_| Layer {
                        mixer: mixer.clone(),
                        ffn: ffn.clone(),
                        post_mixer_norm: false,
                        post_ffn_norm: false,
                    })
                    .collect()
            }
            // `models/minimax-m2.cpp`: attention with a partial NEOX rotation whose queries and keys are each normed across
            // every head at once (`attn_q_norm` as wide as the query projection), and in every block experts scored as
            // `expert_gating_func` says (MiniMax-M2's sigmoids), chosen by score plus `exp_probs_b` where the checkpoint
            // carries it, the kept weights renormalized (the graph fixes it) and unscaled; no shared expert.
            "minimax-m2" => {
                let mut attn = attention(true, false, 0)?;
                let width = |name: &str| g.dims(&format!("blk.0.{name}.weight")).and_then(|d| d.first().copied()).map(|w| w as usize);
                attn.qk_norm_whole = attn.qk_norm && width("attn_q_norm") == Some(heads * head_dim);
                if attn.qk_norm && !attn.qk_norm_whole && width("attn_q_norm") != Some(head_dim) {
                    return Err(SpecError::Inconsistent(format!(
                        "attn_q_norm is {:?} wide, neither a head of {head_dim} nor the {heads} heads' {}",
                        width("attn_q_norm"),
                        heads * head_dim
                    )));
                }
                let gating = match k.usize_or("expert_gating_func", 2) {
                    1 => Gating::Softmax,
                    2 => Gating::Sigmoid { bias: has_any(g, "exp_probs_b.bias") },
                    other => {
                        return Err(SpecError::Inconsistent(format!("expert_gating_func {other} is neither a softmax (1) nor a sigmoid (2)")))
                    }
                };
                let ffn = Ffn::Moe(Moe {
                    experts: k.usize("expert_count")?,
                    top_k: k.usize("expert_used_count")?,
                    expert_dff: k.usize("expert_feed_forward_length")?,
                    act: Act::Silu,
                    gating,
                    norm_top_k: true,
                    weights_scale: 1.0,
                    shared_experts: 0,
                    shared_dff: 0,
                    shared_gate: false,
                    biases: false,
                    groups: None,
                });
                (0..n_layers)
                    .map(|_| Layer {
                        mixer: Mixer::Attention(attn.clone()),
                        ffn: ffn.clone(),
                        post_mixer_norm: false,
                        post_ffn_norm: false,
                    })
                    .collect()
            }
            // `models/glm4-moe.cpp`: attention with NEOX rotation over `rope.dimension_count` of each head (half, in
            // GLM-4.5), its projections biased and its query and key normed per head where the checkpoint carries them; the
            // first `leading_dense_block_count` blocks a dense SwiGLU, every other DeepSeek's mixture, its experts scored
            // by sigmoids where `expert_gating_func` is unset. The checkpoint stores the FFN's norm as
            // `post_attention_norm`, which `model::open_checkpoint` presents as `ffn_norm` (a checkpoint read raw is
            // refused); the draft blocks after the stack are not read, and GLM-4.5V's multimodal rotation is refused.
            "glm4moe" => {
                if has_any(g, "post_attention_norm.weight") && !has_any(g, "ffn_norm.weight") {
                    return Err(SpecError::Inconsistent(
                        "a glm4moe checkpoint read raw: its FFN's norm is `post_attention_norm`; `model::open_checkpoint` \
                         presents it as `ffn_norm`"
                            .into(),
                    ));
                }
                if sections.is_some() {
                    return Err(SpecError::Inconsistent("GLM-4.5V's multimodal rotation (`rope.dimension_sections`) is not read yet".into()));
                }
                let attn = attention(true, false, 0)?;
                let moe = routed(2)?;
                let dense_lead = k.usize_or("leading_dense_block_count", 0);
                let dense_ffn = dense(Act::Silu)?;
                (0..n_layers)
                    .map(|l| Layer {
                        mixer: Mixer::Attention(attn.clone()),
                        ffn: if l < dense_lead { dense_ffn.clone() } else { moe.clone() },
                        post_mixer_norm: false,
                        post_ffn_norm: false,
                    })
                    .collect()
            }
            // `models/phi3.cpp`: attention with NEOX rotation over `rope.dimension_count` of each head, its query, key and
            // value one projection or three, a window where `attention.sliding_window` is short of the context (0 for
            // none, Phi-4's), and a dense SwiGLU. The checkpoint keeps the gate and up projections as one `ffn_up`, which
            // `model::open_checkpoint` presents apart; a checkpoint read raw is refused, and so are LongRoPE's factor
            // tables (Phi-3.5 and Phi-4-mini), which are not read yet.
            "phi3" => {
                if has_any(g, "ffn_up.weight") && !has_any(g, "ffn_gate.weight") {
                    return Err(SpecError::Inconsistent(
                        "a phi3 checkpoint read raw: its gate and up projections are one `ffn_up`; `model::open_checkpoint` \
                         presents them apart"
                            .into(),
                    ));
                }
                if g.has("rope_factors_long.weight") || g.has("rope_factors_short.weight") {
                    return Err(SpecError::Inconsistent(
                        "LongRoPE's factor tables (`rope_factors_long`, `rope_factors_short`) are not read yet".into(),
                    ));
                }
                let mut attn = attention(true, false, 0)?;
                let window = k.usize_or("attention.sliding_window", 0);
                if window > 0 && window < k.usize_or("context_length", usize::MAX) {
                    attn.window = Some(Window::Sliding(window));
                }
                let ffn = dense(Act::Silu)?;
                (0..n_layers)
                    .map(|_| Layer {
                        mixer: Mixer::Attention(attn.clone()),
                        ffn: ffn.clone(),
                        post_mixer_norm: false,
                        post_ffn_norm: false,
                    })
                    .collect()
            }
            // `models/glm4.cpp`: attention with interleaved rotation over `rope.dimension_count` of each head (half, in
            // GLM-4), its projections biased where the checkpoint carries biases, a norm on the attention's output and on
            // the FFN's (`post_attention_norm`, `post_ffw_norm`) before each residual add, and a dense SwiGLU whose gate and
            // up projections the checkpoint keeps as one `ffn_up` (`model::layout` presents them apart; read raw, refused).
            // GLM-4V's multimodal rotation is refused by name.
            "glm4" => {
                if has_any(g, "ffn_up.weight") && !has_any(g, "ffn_gate.weight") {
                    return Err(SpecError::Inconsistent(
                        "a glm4 checkpoint read raw: its gate and up projections are one `ffn_up`; `model::open_checkpoint` \
                         presents them apart"
                            .into(),
                    ));
                }
                if sections.is_some() {
                    return Err(SpecError::Inconsistent("GLM-4V's multimodal rotation (`rope.dimension_sections`) is not read yet".into()));
                }
                let attn = attention(false, false, 0)?;
                let ffn = dense(Act::Silu)?;
                let (post_mixer_norm, post_ffn_norm) = (has_any(g, "post_attention_norm.weight"), has_any(g, "post_ffw_norm.weight"));
                (0..n_layers)
                    .map(|_| Layer { mixer: Mixer::Attention(attn.clone()), ffn: ffn.clone(), post_mixer_norm, post_ffn_norm })
                    .collect()
            }
            // `models/qwen3moe.cpp`: Qwen3 attention, a softmax router whose top-k weights are renormalized.
            "qwen3moe" => {
                let mixer = Mixer::Attention(attention(true, false, 0)?);
                let ffn = Ffn::Moe(Moe {
                    experts: k.usize("expert_count")?,
                    top_k: k.usize("expert_used_count")?,
                    expert_dff: k.usize("expert_feed_forward_length")?,
                    act: Act::Silu,
                    gating: Gating::Softmax,
                    norm_top_k: true,
                    weights_scale: 1.0,
                    shared_experts: 0,
                    shared_dff: 0,
                    shared_gate: false,
                    biases: false,
                    groups: None,
                });
                (0..n_layers)
                    .map(|_| Layer {
                        mixer: mixer.clone(),
                        ffn: ffn.clone(),
                        post_mixer_norm: false,
                        post_ffn_norm: false,
                    })
                    .collect()
            }
            // `models/qwen2moe.cpp`: Qwen2's attention (NEOX, its biased projections read off its tensors) and in every
            // block Qwen's mixture, its kept softmax weights as they are.
            "qwen2moe" => {
                let mixer = Mixer::Attention(attention(true, false, 0)?);
                let ffn = qwen_mixture(false)?;
                (0..n_layers)
                    .map(|_| Layer {
                        mixer: mixer.clone(),
                        ffn: ffn.clone(),
                        post_mixer_norm: false,
                        post_ffn_norm: false,
                    })
                    .collect()
            }
            // `models/qwen35.cpp`, `models/qwen35moe.cpp`, `models/qwen3next.cpp`: gated attention (partial NEOX
            // rotation, Qwen3.5's with IMROPE sections, a sigmoid output gate) where `attention.recurrent_layers` says a
            // block is not recurrent, else every `full_attention_interval`-th block (default 4); Gated DeltaNet
            // elsewhere; a dense SiLU FFN everywhere, or for `qwen35moe` and `qwen3next` Qwen's mixture with its kept
            // softmax weights renormalized. A Qwen3-Next checkpoint is read as `model::layout` presents it, in Qwen3.5's
            // tiled layout; one still holding its grouped layout's `ssm_ba` is refused.
            // `models/qwen4exp.cpp` reads Qwen3.5's hybrid with Gated DeltaNet's gate the sigmoid, Qwen's mixture in
            // every block, and hyper-connections, QSA and the per-layer embedding read below.
            "qwen35" | "qwen35moe" | "qwen3next" | "qwen4exp" => {
                if arch == "qwen3next" && has_any(g, "ssm_ba.weight") {
                    return Err(SpecError::Inconsistent(
                        "a Qwen3-Next checkpoint read raw: its value heads are grouped by key head and its betas and alphas \
                         one tensor (`ssm_ba`); `model::open_checkpoint` presents it in Qwen3.5's tiled layout"
                            .into(),
                    ));
                }
                let recurrent: Vec<bool> = match k.get("attention.recurrent_layers").and_then(MetaValue::as_array) {
                    Some(a) => a
                        .iter()
                        .take(n_layers)
                        .map(|v| v.as_bool().or_else(|| v.as_u64().map(|x| x != 0)))
                        .collect::<Option<_>>()
                        .ok_or_else(|| SpecError::MissingKey(k.key("attention.recurrent_layers")))?,
                    None => {
                        let every = k.usize_or("full_attention_interval", 4);
                        (0..n_layers).map(|l| (l + 1) % every != 0).collect()
                    }
                };
                let gdn = GatedDelta {
                    k_heads: k.usize("ssm.group_count")?,
                    v_heads: k.usize("ssm.time_step_rank")?,
                    head_dim: k.usize("ssm.state_size")?,
                    conv_taps: k.usize("ssm.conv_kernel")?,
                    gate: if arch == "qwen4exp" { GateActivation::Sigmoid } else { GateActivation::Silu },
                };
                let inner = k.usize("ssm.inner_size")?;
                if inner != gdn.v_heads * gdn.head_dim {
                    return Err(SpecError::Inconsistent(format!(
                        "ssm.inner_size {inner} is not {} value heads of {}",
                        gdn.v_heads, gdn.head_dim
                    )));
                }
                let at = recurrent.iter().position(|r| !r).unwrap_or(0);
                let attn = attention(true, true, at)?;
                let ffn = if arch == "qwen35" { dense(Act::Silu)? } else { qwen_mixture(true)? };
                recurrent
                    .iter()
                    .map(|&r| Layer {
                        mixer: if r { Mixer::GatedDelta(gdn.clone()) } else { Mixer::Attention(attn.clone()) },
                        ffn: ffn.clone(),
                        post_mixer_norm: false,
                        post_ffn_norm: false,
                    })
                    .collect()
            }
            // `models/gemma3.cpp`: sliding-window attention on all but every `sliding_window_pattern`-th block
            // (default 6), those turning by `rope.freq_base_swa` (default 10000) unscaled; QK-norm; a norm after the
            // mixer and after the FFN as well as before each; a GELU-gated FFN; the embedding scaled by `√d`.
            "gemma3" => {
                let window = k.usize_or("attention.sliding_window", 0);
                let swa = sliding_layers(&k, n_layers, 6, window)?;
                let global = attention(true, false, 0)?;
                let scale = if n_layers == 62 { 1.0 / ((d / heads) as f64).sqrt() } else { 1.0 / (head_dim as f64).sqrt() };
                let local = Attention {
                    window: Some(Window::Sliding(window)),
                    rope: Some(Rope {
                        theta: k.f64_or("rope.freq_base_swa", 10000.0),
                        scaling: RopeScaling::None,
                        ..global.rope.clone().expect("gemma3 rotates")
                    }),
                    scale,
                    ..global.clone()
                };
                let global = Attention { scale, ..global };
                let ffn = dense(Act::GeluTanh)?;
                swa.iter()
                    .map(|&s| Layer {
                        mixer: Mixer::Attention(if s { local.clone() } else { global.clone() }),
                        ffn: ffn.clone(),
                        post_mixer_norm: true,
                        post_ffn_norm: true,
                    })
                    .collect()
            }
            // `models/gemma.cpp`: one rotation, the scores scaled by `1/√head_dim`, a GELU-gated FFN, the embedding scaled
            // by `√d`.
            "gemma" => {
                let attn = attention(true, false, 0)?;
                let ffn = dense(Act::GeluTanh)?;
                (0..n_layers)
                    .map(|_| Layer { mixer: Mixer::Attention(attn.clone()), ffn: ffn.clone(), post_mixer_norm: false, post_ffn_norm: false })
                    .collect()
            }
            // `models/gemma2.cpp`: a sliding window (`attention.sliding_window`, default 4096) on all but every
            // `sliding_window_pattern`-th block (default 2: the first block local), one rotation, each score capped at
            // `attn_logit_softcapping` (default 50) and the logits at `final_logit_softcapping` (default 30), a norm
            // after the mixer and after the FFN as well as before each, a GELU-gated FFN, the embedding scaled by `√d`;
            // the scores scaled by `1/√(d/heads)` at 27B's 46 blocks, else by `1/√head_dim`.
            "gemma2" => {
                let window = k.usize_or("attention.sliding_window", 4096);
                let swa = sliding_layers(&k, n_layers, 2, window)?;
                let scale = if n_layers == 46 { 1.0 / ((d / heads) as f64).sqrt() } else { 1.0 / (head_dim as f64).sqrt() };
                let softcap = Some(k.f64_or("attn_logit_softcapping", 50.0));
                let global = Attention { scale, softcap, ..attention(true, false, 0)? };
                let local = Attention { window: Some(Window::Sliding(window)), ..global.clone() };
                let ffn = dense(Act::GeluTanh)?;
                swa.iter()
                    .map(|&s| Layer {
                        mixer: Mixer::Attention(if s { local.clone() } else { global.clone() }),
                        ffn: ffn.clone(),
                        post_mixer_norm: true,
                        post_ffn_norm: true,
                    })
                    .collect()
            }
            // `models/openai-moe.cpp`: attention with biases and a learned sink per head, a 128-token window on every
            // other block (pattern 2) under one YaRN rotation, and a biased mixture of experts whose top-k router
            // logits are softmaxed after selection, each expert a clamped SwiGLU.
            "gpt-oss" => {
                let window = k.usize("attention.sliding_window")?;
                let every = k.usize_or("attention.sliding_window_pattern", 2);
                let attn = Attention { sinks: true, ..attention(true, false, 0)? };
                let ffn = Ffn::Moe(Moe {
                    experts: k.usize("expert_count")?,
                    top_k: k.usize("expert_used_count")?,
                    expert_dff: k.usize("expert_feed_forward_length")?,
                    act: Act::SwigluOai { alpha: 1.702, limit: 7.0 },
                    gating: Gating::SoftmaxOfTopK,
                    norm_top_k: false,
                    weights_scale: 1.0,
                    shared_experts: 0,
                    shared_dff: 0,
                    shared_gate: false,
                    biases: g.has("blk.0.ffn_gate_inp.bias"),
                    groups: None,
                });
                (0..n_layers)
                    .map(|l| Layer {
                        mixer: Mixer::Attention(Attention {
                            window: (every == 0 || l % every < every - 1).then_some(Window::Sliding(window)),
                            ..attn.clone()
                        }),
                        ffn: ffn.clone(),
                        post_mixer_norm: false,
                        post_ffn_norm: false,
                    })
                    .collect()
            }
            // `models/deepseek2.cpp`: multi-head latent attention with interleaved rotation; the first
            // `leading_dense_block_count` blocks a dense SiLU FFN, the rest a routed mixture with shared experts —
            // softmax routing (V2) or sigmoid with a selection bias and grouped choice (V3), per
            // `expert_gating_func`.
            "deepseek2" => {
                let rope_dims = k.usize("rope.dimension_count")?;
                let key = k.usize_or("attention.key_length_mla", head_dim);
                let rope = Rope {
                    dims: rope_dims,
                    theta: k.f64_or("rope.freq_base", 10000.0),
                    neox: false,
                    scaling: rope_scaling(&k, g)?,
                    sections: None,
                };
                // llama.cpp pre-scales the scores by YaRN's magnitude (its log multiplier, stored as a tenth) squared.
                let mscale = match rope.scaling {
                    RopeScaling::Yarn { factor, magnitude, .. } => {
                        let log_mul = k.f64_or("rope.scaling.yarn_log_multiplier", 0.0) / 0.1;
                        magnitude * (1.0 + 0.1 * log_mul * factor.ln())
                    }
                    _ => 1.0,
                };
                let mla = Mla {
                    heads,
                    q_lora_rank: k.get("attention.q_lora_rank").and_then(MetaValue::as_u64).map(|v| v as usize),
                    kv_lora_rank: k.usize("attention.kv_lora_rank")?,
                    qk_nope: key - rope_dims,
                    qk_rope: rope_dims,
                    v_dim: k.usize_or("attention.value_length_mla", value_dim),
                    rope,
                    scale: mscale * mscale / (key as f64).sqrt(),
                    kv_b_split: g.has("blk.0.attn_k_b.weight"),
                };
                let moe = routed(1)?;
                let dense_lead = k.usize_or("leading_dense_block_count", 0);
                let dense_ffn = dense(Act::Silu)?;
                (0..n_layers)
                    .map(|l| Layer {
                        mixer: Mixer::Mla(mla.clone()),
                        ffn: if l < dense_lead { dense_ffn.clone() } else { moe.clone() },
                        post_mixer_norm: false,
                        post_ffn_norm: false,
                    })
                    .collect()
            }
            // `models/llama4.cpp`: three layers in chunks of 8192 positions, rotated (interleaved pairs, the checkpoint's
            // frequency factors) with a weightless QK norm unless the model has 128 experts, then one full layer with no
            // rotation whose queries take the attention temperature (floor 8192, scale 0.1) — llama.cpp's constants, which
            // no key restates. `attention.sliding_window` 0 turns the chunks off: every layer rotates but the last (the
            // no-rotation step is the layer count) and no temperature applies.
            // Every `interleave_moe_layer_step`-th block is a mixture whose top-k logits' sigmoids scale their experts'
            // inputs, beside a shared expert as wide as one; every other block a dense SiLU FFN.
            "llama4" => {
                let experts = k.usize("expert_count")?;
                let chunked = k.get("attention.sliding_window").and_then(MetaValue::as_u64) != Some(0);
                let step = k.usize("interleave_moe_layer_step")?;
                let expert_dff = k.usize("expert_feed_forward_length")?;
                let base = attention(false, false, 0)?;
                let rotated = Attention {
                    window: chunked.then_some(Window::Chunked(8192)),
                    qk_unit_norm: experts != 128,
                    qk_norm_whole: false,
                    ..base.clone()
                };
                let full = Attention {
                    rope: None,
                    temperature: chunked.then_some(Temperature { floor: 8192.0, scale: 0.1 }),
                    ..base.clone()
                };
                let no_rope_step = if chunked { 4 } else { n_layers };
                let moe = Ffn::Moe(Moe {
                    experts,
                    top_k: k.usize("expert_used_count")?,
                    expert_dff,
                    act: Act::Silu,
                    gating: Gating::SigmoidScaledInput,
                    norm_top_k: false,
                    weights_scale: 1.0,
                    shared_experts: 1,
                    shared_dff: expert_dff,
                    shared_gate: false,
                    biases: false,
                    groups: None,
                });
                let dense_ffn = dense(Act::Silu)?;
                (0..n_layers)
                    .map(|l| Layer {
                        mixer: Mixer::Attention(if (l + 1) % no_rope_step == 0 { full.clone() } else { rotated.clone() }),
                        ffn: if step > 0 && (l + 1) % step == 0 { moe.clone() } else { dense_ffn.clone() },
                        post_mixer_norm: false,
                        post_ffn_norm: false,
                    })
                    .collect()
            }
            other => return Err(SpecError::UnknownArchitecture(other.to_string())),
        };
        let ffn_norm =
            if matches!(arch.as_str(), "qwen35" | "qwen35moe" | "qwen3next" | "gpt-oss") { "post_attention_norm" } else { "ffn_norm" };
        let (hyper, ple, indexer) = if arch == "qwen4exp" { qwen4exp_parts(&k, &layers)? } else { (None, None, None) };
        let embedding_scale = matches!(arch.as_str(), "gemma" | "gemma2" | "gemma3").then(|| (d as f64).sqrt());
        // Gemma 2's graph caps its logits whatever the file says, at 30 where it says nothing.
        let final_logit_softcap = match k.get("final_logit_softcapping").and_then(decimal) {
            None if arch == "gemma2" => Some(30.0),
            cap => cap,
        };
        Ok(ModelSpec {
            arch,
            d,
            vocab,
            ctx_len: k.usize_or("context_length", 0),
            rms_eps: k.f64_or("attention.layer_norm_rms_epsilon", 1e-6),
            embedding_scale,
            tied_output: !g.has("output.weight"),
            final_logit_softcap,
            layers,
            mtp_blocks,
            ffn_norm,
            hyper,
            ple,
            indexer,
        })
    }

    /// The indices of the attention layers.
    pub fn attention_layers(&self) -> Vec<usize> {
        self.layers
            .iter()
            .enumerate()
            .filter(|(_, l)| matches!(l.mixer, Mixer::Attention(_)))
            .map(|(i, _)| i)
            .collect()
    }
}

/// Whether any block holds a tensor named `blk.<n>.<suffix>`.
fn has_any<S: SpecSource + ?Sized>(g: &S, suffix: &str) -> bool {
    g.tensor_names().iter().any(|t| t.starts_with("blk.") && t.ends_with(suffix))
}

/// The rotary scaling the checkpoint declares.
fn rope_scaling<S: SpecSource + ?Sized>(k: &Keys<S>, g: &S) -> Result<RopeScaling, SpecError> {
    if g.has("rope_freqs.weight") {
        return Ok(RopeScaling::Factors);
    }
    let kind = k.get("rope.scaling.type").and_then(MetaValue::as_str).unwrap_or("none");
    Ok(match kind {
        "none" => RopeScaling::None,
        "linear" => RopeScaling::Linear {
            factor: k.f64("rope.scaling.factor")?,
        },
        "yarn" => {
            let factor = k.f64("rope.scaling.factor")?;
            // llama-context.cpp: `get_mscale(f, m) = 0.1·m·ln f + 1`; with a log multiplier the ratio of two of them
            // (DeepSeek V2 takes the multiplier for both when it is not 1), else `get_mscale(f, 1)`.
            let get_mscale = |m: f64| if factor <= 1.0 { 1.0 } else { 0.1 * m * factor.ln() + 1.0 };
            let log_mul = k.f64_or("rope.scaling.yarn_log_multiplier", 0.0);
            let ratio = if log_mul != 0.0 {
                let m = if k.g.arch() == "deepseek2" && log_mul != 1.0 { log_mul } else { 1.0 };
                get_mscale(m) / get_mscale(log_mul)
            } else {
                get_mscale(1.0)
            };
            RopeScaling::Yarn {
                factor,
                original_ctx: k.usize("rope.scaling.original_context_length")?,
                beta_fast: k.f64_or("rope.scaling.yarn_beta_fast", 32.0),
                beta_slow: k.f64_or("rope.scaling.yarn_beta_slow", 1.0),
                magnitude: ratio * k.f64_or("rope.scaling.attn_factor", 1.0),
            }
        }
        other => return Err(SpecError::MissingKey(format!("rope.scaling.type `{other}` is not described yet"))),
    })
}
