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
    /// Each sublayer's output multiplied by this before it joins the residual (Granite's `residual_multiplier`,
    /// MiniCPM's `scale_depth / √layers`).
    pub residual_scale: Option<f64>,
    /// A multiplier on the logits, and where the model applies it.
    pub logit_scale: Option<LogitScale>,
    /// How the block norms normalize: before each sublayer, after it where the block has one, and before the head.
    pub norm: Norm,
    /// A bias on the LM head (`output.bias`): PhiMoE's `lm_head_bias`.
    pub head_bias: bool,
    /// The model's parts past its blocks' mixers and FFNs, each at most once, in [`Part`]'s order.
    pub parts: Vec<Part>,
}

/// How a model's block norms normalize their input: an RMSNorm, or a LayerNorm (the mean taken out, then the root of
/// the variance), each times its weight and, where `bias`, plus its bias (`<norm>.bias`). Every family's norms are
/// RMSNorms but PhiMoE's (`nn.LayerNorm`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Norm {
    Rms,
    Layer { bias: bool },
}

/// A model's logit multiplier, at the place its own graph applies it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum LogitScale {
    /// On the output norm's result, before the head (MiniCPM: `h / (d / dim_model_base)`).
    BeforeHead(f64),
    /// On the logits, after the head and before any soft cap (Granite: `logits / logits_scaling`).
    AfterHead(f64),
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

/// A part of a model past its blocks' mixers and FFNs: what Gemma 3n, Gemma 4, MiniMax-01 and RWKV-7 add to the stream
/// the blocks share. A lowering that does not carry one refuses it by [`Part::what`], so a part added here needs no arm
/// of its own anywhere else.
#[derive(Clone, Debug, PartialEq)]
pub enum Part {
    /// The token embedding normed by `token_embd_norm` before the first block (RWKV-7's `ln0`), a norm of the model's
    /// [`Norm`].
    EmbeddingNorm,
    /// Per-layer inputs of `dim` (Gemma 3n's and Gemma 4's): each token's row of `per_layer_token_embd` times `√dim`,
    /// plus the embedded stream through `per_layer_model_proj` over `√d` and normed by `per_layer_proj_norm`, the sum
    /// over `√2`, a `dim`-wide slice for each block. After its FFN a block gates its slice by
    /// `gelu(inp_gate·x)`, projects it by `proj`, norms it by `post_norm` and adds it to the stream (to each stream but
    /// the active one where [`Part::AltUp`] is).
    PerLayerInput { dim: usize },
    /// Gemma 3n's alternating updates: `streams` copies of the stream, the others made from the embedding by
    /// `altup_proj` at its magnitude. Each block predicts every stream from all of them by coefficients its router reads
    /// off the active one (`altup_router_norm`, `altup_router`, tanh, `altup_predict_coef`), computes on the active
    /// prediction alone, and corrects every stream by the active one's innovation (`altup_correct_coef` plus one); the
    /// head reads the mean of the active stream and the others through `altup_unembd_proj` at its magnitude. A
    /// projection's magnitude is its root mean square taken no smaller than `√1e-5`, as transformers takes it; llama.cpp
    /// floors it at nothing.
    AltUp { streams: usize, active: usize },
    /// Gemma 3n's learned augmented residual: beside the attention, the normed input through `laurel_l` to `rank` and
    /// back by `laurel_r`, normed by `laurel_post_norm` and added to the input; the attention's residual sum joins it
    /// and the two are scaled by `1/√2` before the FFN.
    Laurel { rank: usize },
    /// The layers from `from` on keep no cache and attend an earlier layer's (Gemma 3n's and Gemma 4's shared KV): a
    /// sliding-window layer layer `from − 2`'s, a global one layer `from − 1`'s (llama.cpp's `layer_reuse`).
    SharedKv { from: usize },
    /// Every attention layer's values RMS-normed per head, with no weight, before they are cached (Gemma 3n, Gemma 4).
    ValueNorm,
    /// The values of the attention layers in `layers` are their keys as projected, before the key norm and the rotation
    /// (Gemma 4's `attention_k_eq_v`: those layers hold no `attn_v`).
    KeyAsValue { layers: Vec<usize> },
    /// Each block's output multiplied by its own `layer_output_scale` (Gemma 4).
    LayerScale,
    /// Gemma 3n's activation sparsity: in the first `layers` blocks the FFN's gate keeps only what exceeds its mean by
    /// `std_mul` of its standard deviations, as `relu(g − μ − std_mul·σ)` before the GELU. σ is the population's, as
    /// transformers takes it; llama.cpp's `gaussian_topk` divides by `n − 1`, and the spec follows transformers.
    GateSparsity { layers: usize, std_mul: f64 },
    /// MiniMax-01's residual: each sublayer adds its output to its normed input times `scale`, not to its input.
    NormedResidual { scale: f64 },
}

impl Part {
    /// What the part is, by name: a lowering that does not carry it refuses it so.
    pub fn what(&self) -> &'static str {
        match self {
            Part::EmbeddingNorm => "a norm on the token embedding (RWKV-7's)",
            Part::PerLayerInput { .. } => "per-layer inputs (Gemma 3n's and Gemma 4's)",
            Part::AltUp { .. } => "alternating updates over several streams (Gemma 3n's AltUp)",
            Part::Laurel { .. } => "a learned augmented residual (Gemma 3n's LAuReL)",
            Part::SharedKv { .. } => "layers that attend an earlier layer's cache",
            Part::ValueNorm => "a weightless norm on the values",
            Part::KeyAsValue { .. } => "values that are the keys",
            Part::LayerScale => "a learned scale on each block's output",
            Part::GateSparsity { .. } => "a sparsified FFN gate (Gemma 3n's activation sparsity)",
            Part::NormedResidual { .. } => "a residual from the normed input (MiniMax-01's)",
        }
    }

    /// The tensors the part reads in a checkpoint of `spec`, each with its GGUF dimensions.
    pub fn tensors(&self, spec: &ModelSpec) -> Vec<(String, Vec<u64>)> {
        let (d, layers) = (spec.d as u64, spec.layers.len());
        let each = |names: &[(&str, Vec<u64>)]| -> Vec<(String, Vec<u64>)> {
            (0..layers).flat_map(|l| names.iter().map(move |(n, dims)| (format!("blk.{l}.{n}"), dims.clone()))).collect()
        };
        match self {
            Part::EmbeddingNorm => {
                let mut t = vec![("token_embd_norm.weight".to_string(), vec![d])];
                if spec.norm == (Norm::Layer { bias: true }) {
                    t.push(("token_embd_norm.bias".to_string(), vec![d]));
                }
                t
            }
            Part::PerLayerInput { dim } => {
                let (w, all) = (*dim as u64, (*dim * layers) as u64);
                let mut t = vec![
                    ("per_layer_token_embd.weight".to_string(), vec![all, spec.vocab as u64]),
                    ("per_layer_model_proj.weight".to_string(), vec![d, all]),
                    ("per_layer_proj_norm.weight".to_string(), vec![w]),
                ];
                t.extend(each(&[("inp_gate.weight", vec![d, w]), ("proj.weight", vec![w, d]), ("post_norm.weight", vec![d])]));
                t
            }
            Part::AltUp { streams, .. } => {
                let n = *streams as u64;
                let mut t = vec![
                    ("altup_proj.weight".to_string(), vec![d, d, n - 1]),
                    ("altup_unembd_proj.weight".to_string(), vec![d, d, n - 1]),
                ];
                t.extend(each(&[
                    ("altup_correct_coef.weight", vec![n, n]),
                    ("altup_correct_scale.weight", vec![d]),
                    ("altup_predict_coef.weight", vec![n, n * n]),
                    ("altup_router.weight", vec![d, n]),
                    ("altup_router_norm.weight", vec![d]),
                ]));
                t
            }
            Part::Laurel { rank } => {
                let r = *rank as u64;
                each(&[("laurel_l.weight", vec![d, r]), ("laurel_r.weight", vec![r, d]), ("laurel_post_norm.weight", vec![d])])
            }
            Part::LayerScale => each(&[("layer_output_scale.weight", vec![1])]),
            Part::SharedKv { .. } | Part::ValueNorm | Part::KeyAsValue { .. } | Part::GateSparsity { .. } | Part::NormedResidual { .. } => {
                Vec::new()
            }
        }
    }
}

/// One block: its token mixer, its feed-forward block, and its norms.
#[derive(Clone, Debug, PartialEq)]
pub struct Layer {
    pub mixer: Mixer,
    pub ffn: Ffn,
    /// A norm before the mixer and one before the FFN (`attn_norm`, `ffn_norm`), each sublayer reading its input
    /// normed: every family's but OLMo 2's, whose sublayers read the residual stream itself.
    pub pre_norms: bool,
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
    Recurrent(Recurrent),
    /// No mixer: the block is its FFN alone, normed by the block's one norm (`attn_norm`): Nemotron-H's MLP blocks.
    None,
}

/// A mixer of the families past attention, latent attention and Gated DeltaNet — the convolutions, state-space scans,
/// linear attentions and recurrences the reference computes and the author does not write yet. A lowering that does not
/// carry one refuses it by [`Recurrent::what`], so a kind added here needs no arm of its own anywhere else.
#[derive(Clone, Debug, PartialEq)]
pub enum Recurrent {
    ShortConv(ShortConv),
    Ssd(Ssd),
    Selective(Selective),
    /// Falcon-H1's block: attention and Mamba-2's scan side by side on one normed input, their outputs summed.
    Parallel(Box<Attention>, Ssd),
    DeltaAttention(DeltaAttention),
    Lightning(Lightning),
    Rwkv7(Rwkv7),
}

impl Recurrent {
    /// What the mixer is, by name: a lowering that does not carry it refuses it so.
    pub fn what(&self) -> &'static str {
        match self {
            Recurrent::ShortConv(_) => "a gated short convolution (LFM2's)",
            Recurrent::Ssd(_) => "Mamba-2's state-space scan (SSD)",
            Recurrent::Selective(_) => "Mamba-1's selective scan",
            Recurrent::Parallel(..) => "attention beside a state-space scan in one block (Falcon-H1's)",
            Recurrent::DeltaAttention(_) => "Kimi delta attention (KDA)",
            Recurrent::Lightning(_) => "lightning attention (MiniMax-01's)",
            Recurrent::Rwkv7(_) => "RWKV-7's time mix",
        }
    }

    /// The tensors the mixer reads in a block of width `d`, named within the block (`blk.<l>.` before each), each with
    /// its GGUF dimensions as llama.cpp's `load_arch_tensors` creates them.
    pub fn tensors(&self, d: usize) -> Vec<(String, Vec<u64>)> {
        let d = d as u64;
        let t = |name: &str, dims: &[u64]| (name.to_string(), dims.to_vec());
        match self {
            Recurrent::ShortConv(c) => vec![
                t("shortconv.in_proj.weight", &[d, 3 * d]),
                t("shortconv.conv.weight", &[c.taps as u64, d]),
                t("shortconv.out_proj.weight", &[d, d]),
            ],
            Recurrent::Ssd(s) => s.tensors(d),
            Recurrent::Selective(s) => {
                let (inner, state, rank, taps) = (s.inner as u64, s.state as u64, s.dt_rank as u64, s.conv_taps as u64);
                let mut v = vec![
                    t("ssm_in.weight", &[d, 2 * inner]),
                    t("ssm_conv1d.weight", &[taps, inner]),
                    t("ssm_conv1d.bias", &[inner]),
                    t("ssm_x.weight", &[inner, rank + 2 * state]),
                    t("ssm_dt.weight", &[rank, inner]),
                    t("ssm_dt.bias", &[inner]),
                    t("ssm_a", &[state, inner]),
                    t("ssm_d", &[inner]),
                    t("ssm_out.weight", &[inner, d]),
                ];
                if s.inner_norms {
                    v.extend([t("ssm_dt_norm.weight", &[rank]), t("ssm_b_norm.weight", &[state]), t("ssm_c_norm.weight", &[state])]);
                }
                v
            }
            Recurrent::Parallel(a, s) => {
                let (q, kv, o) = ((a.heads * a.head_dim) as u64, (a.kv_heads * a.head_dim) as u64, (a.heads * a.value_dim) as u64);
                let mut v = vec![
                    t("attn_q.weight", &[d, q]),
                    t("attn_k.weight", &[d, kv]),
                    t("attn_v.weight", &[d, (a.kv_heads * a.value_dim) as u64]),
                    t("attn_output.weight", &[o, d]),
                ];
                v.extend(s.tensors(d));
                v
            }
            Recurrent::DeltaAttention(k) => {
                let (hd, w, taps) = (k.head_dim as u64, (k.heads * k.head_dim) as u64, k.conv_taps as u64);
                vec![
                    t("attn_q.weight", &[d, w]),
                    t("attn_k.weight", &[d, w]),
                    t("attn_v.weight", &[d, w]),
                    t("ssm_conv1d_q.weight", &[taps, 1, w, 1]),
                    t("ssm_conv1d_k.weight", &[taps, 1, w, 1]),
                    t("ssm_conv1d_v.weight", &[taps, 1, w, 1]),
                    t("ssm_f_a.weight", &[d, hd]),
                    t("ssm_f_b.weight", &[hd, w]),
                    t("ssm_beta.weight", &[d, k.heads as u64]),
                    t("ssm_a", &[1, k.heads as u64, 1, 1]),
                    t("ssm_dt.bias", &[w]),
                    t("ssm_g_a.weight", &[d, hd]),
                    t("ssm_g_b.weight", &[hd, w]),
                    t("ssm_norm.weight", &[hd]),
                    t("attn_output.weight", &[w, d]),
                ]
            }
            Recurrent::Lightning(x) => {
                let w = (x.heads * x.head_dim) as u64;
                vec![
                    t("attn_norm_2.weight", &[w]),
                    t("attn_qkv.weight", &[d, 3 * w]),
                    t("attn_gate.weight", &[d, w]),
                    t("attn_output.weight", &[w, d]),
                ]
            }
            Recurrent::Rwkv7(r) => {
                let value_rank = if r.first { r.iclr_lora } else { r.value_res_lora } as u64;
                let (decay, iclr, gate) = (r.decay_lora as u64, r.iclr_lora as u64, r.gate_lora as u64);
                vec![
                    t("time_mix_w0.weight", &[d]),
                    t("time_mix_w1.weight", &[d, decay]),
                    t("time_mix_w2.weight", &[decay, d]),
                    t("time_mix_a0.weight", &[d]),
                    t("time_mix_a1.weight", &[d, iclr]),
                    t("time_mix_a2.weight", &[iclr, d]),
                    t("time_mix_v0.weight", &[d]),
                    t("time_mix_v1.weight", &[d, value_rank]),
                    t("time_mix_v2.weight", &[value_rank, d]),
                    t("time_mix_g1.weight", &[d, gate]),
                    t("time_mix_g2.weight", &[gate, d]),
                    t("time_mix_lerp_fused.weight", &[d, 1, 1, 6]),
                    t("time_mix_k_k.weight", &[d]),
                    t("time_mix_k_a.weight", &[d]),
                    t("time_mix_r_k.weight", &[d]),
                    t("time_mix_key.weight", &[d, d]),
                    t("time_mix_value.weight", &[d, d]),
                    t("time_mix_receptance.weight", &[d, d]),
                    t("time_mix_ln.weight", &[d]),
                    t("time_mix_ln.bias", &[d]),
                    t("time_mix_output.weight", &[d, d]),
                ]
            }
        }
    }
}

impl Ssd {
    /// The scan's tensors in a block of width `d` (llama.cpp `llama_model_mamba2::load_arch_tensors`): `ssm_in` into
    /// `z`, `x | B | C` and the time steps, the convolution and its bias, the time-step bias, `A` (as `−exp(A_log)`) and
    /// `D` a value a head, the grouped gated norm where there is one, and `ssm_out`.
    fn tensors(&self, d: u64) -> Vec<(String, Vec<u64>)> {
        let (inner, heads, groups) = ((self.heads * self.head_dim) as u64, self.heads as u64, self.groups as u64);
        let conv = inner + 2 * groups * self.state as u64;
        let mut v: Vec<(String, Vec<u64>)> = [
            ("ssm_in.weight", vec![d, inner + conv + heads]),
            ("ssm_conv1d.weight", vec![self.conv_taps as u64, conv]),
            ("ssm_conv1d.bias", vec![conv]),
            ("ssm_dt.bias", vec![heads]),
            ("ssm_a", vec![1, heads]),
            ("ssm_d", vec![1, heads]),
        ]
        .into_iter()
        .map(|(n, dims)| (n.to_string(), dims))
        .collect();
        if self.gated_norm {
            v.push(("ssm_norm.weight".to_string(), vec![inner / groups, groups]));
        }
        v.push(("ssm_out.weight".to_string(), vec![inner, d]));
        v
    }
}

/// Mamba-2's mixer (llama.cpp `build_mamba2_layer`, transformers `Mamba2Mixer`): the block's input through `ssm_in` into
/// a gate `z` (`heads·head_dim` wide), the convolved `x`, `B` and `C` (`groups·state` each), and a time step per head;
/// `x | B | C` through a depthwise causal convolution of `conv_taps` with its bias and SiLU; per head `h` and channel
/// `p`, the state `S ← S·exp(Δ·A) + Δ·x·B` and the read `y = S·C + D·x`, `Δ = softplus(dt + dt_bias)`, `B` and `C` the
/// head's group's; `y·silu(z)` RMS-normed in `groups` groups where `gated_norm` (by `ssm_norm`), and projected out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ssd {
    pub heads: usize,
    pub head_dim: usize,
    pub state: usize,
    pub groups: usize,
    pub conv_taps: usize,
    pub gated_norm: bool,
}

/// Mamba-1's selective scan (llama.cpp `build_mamba_layer`, transformers `JambaMambaMixer`): the input through
/// `ssm_in` into `x` and a gate `z`, `inner` wide each; `x` through a depthwise causal convolution of `conv_taps` with
/// its bias and SiLU; `x` through `ssm_x` into a time step of `dt_rank`, `B` and `C` of `state` — each RMS-normed where
/// `inner_norms` (Jamba's `dt`, `B` and `C` norms) — the time step through `ssm_dt` and softplus; per channel the state
/// `S ← S·exp(Δ·A) + Δ·x·B`, `A` a row per channel, the read `S·C + D·x`; gated by `silu(z)` and projected out.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Selective {
    pub inner: usize,
    pub state: usize,
    pub dt_rank: usize,
    pub conv_taps: usize,
    pub inner_norms: bool,
}

/// Kimi delta attention (llama.cpp `models/kimi-linear.cpp`, transformers `KimiLinearDeltaAttention`): `heads` heads of
/// `head_dim`; the queries, keys and values each through a depthwise causal convolution of `conv_taps` and SiLU, the
/// queries and keys L2-normed as `x / √(Σx² + 1e-6)` (transformers'; llama.cpp's `l2_norm` floors the norm at the model's
/// epsilon instead); a decay per key channel from a low-rank projection of the input, a write strength per head by
/// sigmoid, the delta rule over a `head_dim × head_dim` memory per head; the read RMS-normed and gated by the sigmoid of
/// a low-rank output gate.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeltaAttention {
    pub heads: usize,
    pub head_dim: usize,
    pub conv_taps: usize,
}

/// MiniMax-01's lightning attention (llama.cpp `models/minimax-01.cpp`, MiniMax's `MiniMaxText01LightningAttention`):
/// `heads` heads of `head_dim`; the input through `attn_qkv` and SiLU into each head's query, key and value, in that
/// order a head; per head `h` a `head_dim × head_dim` memory `S ← e^(−s)·S + kᵀv` read as `q·S`, the rate `s` ALiBi's
/// slope `2^(−8(h+1)/heads)` times the block's depth scale `1 − l/(L − 1) + 10⁻⁵` (block `l` of `L`); the heads' reads
/// RMS-normed together by `attn_norm_2` at an epsilon of 10⁻⁶ (MiniMax's code and transformers'; llama.cpp norms at the
/// model's), gated by the sigmoid of `attn_gate` over the input, and projected out by `attn_output`. transformers takes
/// the depth scale over `L − 1 + 10⁻⁵`; the spec takes MiniMax's and llama.cpp's.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lightning {
    pub heads: usize,
    pub head_dim: usize,
}

/// RWKV-7's time mix (llama.cpp `models/rwkv7-base.cpp`): heads of `head_size`; the normed input `x` and the previous
/// position's `x'` lerped (`x + (x' − x)·μ`, `time_mix_lerp_fused`) into six streams — receptance, decay, key, value,
/// in-context learning rate, gate; the decay `exp(−0.606531·σ(w0 + w2·tanh(w1·x_w)))`, the learning rate
/// `a = σ(a0 + a2·a1·x_a)`, the gate `g2·σ(g1·x_g)`, each through its low-rank pair (`decay_lora`, `iclr_lora`,
/// `gate_lora`), and past the `first` block the value pulled toward the first block's by `σ(v0 + v2·v1·x_v)`
/// (`value_res_lora`); the key's removal direction `κ = k⊙k_k` L2-normed per head (floored at 10⁻¹²), the key
/// `k + (a − 1)·k⊙k_a`; per head the state `S ← S·diag(w) + v kᵀ − (S κ)(κ⊙a)ᵀ` read as `S r`, group-normed per head at
/// 64·10⁻⁵ by `time_mix_ln` with its bias, plus the bonus `v·Σ(k⊙r⊙r_k)` of each head, times the gate, projected by
/// `time_mix_output`. The first block mixes with no other value, and its value-mix pair (`time_mix_v0`–`v2`, stored at
/// `iclr_lora`, the converter's copy of the learning rate's) goes unread.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rwkv7 {
    pub head_size: usize,
    pub decay_lora: usize,
    pub iclr_lora: usize,
    pub value_res_lora: usize,
    pub gate_lora: usize,
    pub first: bool,
}

/// LFM2's gated short convolution (llama.cpp `models/lfm2.cpp`, transformers `Lfm2ShortConv`): the block's input
/// through `shortconv.in_proj` into `B`, `C` and `x`, each `d` wide, in that order; `B ⊙ x` through a depthwise causal
/// convolution of `taps` (`shortconv.conv`, no bias), its state the last `taps − 1` of them; that gated by `C` and
/// projected by `shortconv.out_proj`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShortConv {
    pub taps: usize,
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
    /// The rotation of each head's last `qk_rope` dimensions, or `None` where they do not rotate (Kimi-Linear's
    /// latent attention, which takes no positions).
    pub rope: Option<Rope>,
    /// The score scale: `1/√(qk_nope + qk_rope)`, times YaRN's magnitude squared.
    pub scale: f64,
    /// The checkpoint stores the latent's expansion split into its key half (`attn_k_b`, each head's `kv_lora_rank`
    /// rows of `qk_nope`) and its value half (`attn_v_b`, each head's `v_dim` rows of `kv_lora_rank`), rather than as
    /// one `attn_kv_b` of `qk_nope + v_dim` rows a head.
    pub kv_b_split: bool,
    /// DeepSeek's sparse attention: the lightning indexer that chooses the positions each query attends.
    pub indexer: Option<Dsa>,
    /// The epsilon of the latent's RMSNorms (`attn_q_a_norm`, `attn_kv_a_norm`), which the model's own code may build
    /// apart from every other norm's.
    pub latent_norm_eps: f64,
}

/// The latent norms' epsilon in every latent-attention arm: the publishers' modeling code (DeepSeek-V2's and V3's, Kimi's,
/// GLM-5's) and transformers' build `q_a_layernorm` and `kv_a_layernorm` at their RMSNorm's default 10⁻⁶ whatever
/// `rms_norm_eps` says. llama.cpp's `deepseek2`, `deepseek32`, `glm-dsa` and `kimi-linear` graphs norm them at the
/// model's epsilon, and the GGUF carries no other, so a release whose `rms_norm_eps` is not 10⁻⁶ (GLM-5's, Kimi-Linear's,
/// Kimi K2-0905's, all 10⁻⁵) computes otherwise under llama.cpp; the arms read what the model computes.
pub const MLA_LATENT_NORM_EPS: f64 = 1e-6;

/// DeepSeek's sparse attention (DSA; DeepSeek-V3.2's and GLM-5's lightning indexer): `heads` index queries of `head_dim`
/// from the query's normed low-rank latent (`indexer.attn_q_b`) and one index key from the block's input
/// (`indexer.attn_k`, then a LayerNorm by `indexer.k_norm`'s weight and bias at epsilon 1e-6), each rotated over its first
/// `qk_rope` dimensions by the layer's rotation; every cached position scored `Σ_h w_h·relu(q_h·k)/√(head_dim·heads)`,
/// `w` the block's input through `indexer.proj`; a query attends its best `top_k` positions, every one it sees while it
/// sees no more. A shared layer (`full` false, GLM-5.2's) takes the positions the last full layer before it chose.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Dsa {
    pub heads: usize,
    pub head_dim: usize,
    pub top_k: usize,
    /// The index rotation pairs dimension `i` with `i + qk_rope/2` (DeepSeek-V3.2's) or `2i` with `2i + 1` (GLM-5's).
    pub neox: bool,
    pub full: bool,
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

/// Llama 4's attention temperature: the query at position `p` (0-based) scaled by
/// `1 + scale·ln(⌊(p + offset)/floor⌋ + 1)`, so short contexts are untouched and long ones sharpen
/// (llama.cpp `llm_graph_input_attn_temp`): Llama 4's rotation-free layers count positions from one (`offset` 1,
/// `attn_temperature_tuning`), Ministral 3's every layer from zero (`offset` 0, transformers' `get_llama_4_attn_scale`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Temperature {
    pub floor: f64,
    pub scale: f64,
    pub offset: f64,
}

impl Temperature {
    /// The factor a query at position `p` (0-based) is scaled by.
    pub fn at(self, p: usize) -> f64 {
        1.0 + self.scale * (((p as f64 + self.offset) / self.floor).floor() + 1.0).ln()
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
    /// No FFN: the block is its mixer alone (Mamba-2's blocks, Nemotron-H's mixer blocks).
    None,
    /// RWKV-7's channel mix: the normed input and the previous position's lerped by `channel_mix_lerp_k`,
    /// `relu(key·x)²` of `dff`, projected by `channel_mix_value`.
    ChannelMix {
        dff: usize,
    },
    /// Gemma 4's mixture block (llama.cpp `models/gemma4.cpp`): a dense FFN of `dff` gated by `act` on the input normed
    /// by the FFN norm, then normed by `post_ffw_norm_1`, beside `moe` on the input normed by `pre_ffw_norm_2`, then
    /// normed by `post_ffw_norm_2`; the two summed. Its router reads the block's unnormed input RMS-normed with no
    /// weight, over `√d`, times `ffn_gate_inp.scale`; the softmax's top `moe.top_k` are renormalized and each chosen
    /// expert's output scaled by its `ffn_down_exps.scale`. The experts' gate and up may be stored as one
    /// `ffn_gate_up_exps` of `2·expert_dff` rows an expert, the gate's first.
    DenseAndMoe {
        dff: usize,
        act: Act,
        moe: Moe,
    },
}

impl Ffn {
    /// The tensors the FFN reads in a block of width `d`, named within the block (`blk.<l>.` before each), each with its
    /// GGUF dimensions; its input norm is the block's, not the FFN's.
    pub fn tensors(&self, d: usize) -> Vec<(String, Vec<u64>)> {
        let d = d as u64;
        let mut v: Vec<(String, Vec<u64>)> = Vec::new();
        let mut t = |name: &str, dims: &[u64]| v.push((name.to_string(), dims.to_vec()));
        let dense = |t: &mut dyn FnMut(&str, &[u64]), dff: usize, act: &Act| {
            let f = dff as u64;
            if *act != Act::ReluSquared {
                t("ffn_gate.weight", &[d, f]);
            }
            t("ffn_up.weight", &[d, f]);
            t("ffn_down.weight", &[f, d]);
        };
        match self {
            Ffn::Dense { dff, act } => dense(&mut t, *dff, act),
            Ffn::Moe(m) => m.tensors(d, &mut t),
            Ffn::None => {}
            Ffn::ChannelMix { dff } => {
                let f = *dff as u64;
                t("channel_mix_lerp_k.weight", &[d, 1, 1]);
                t("channel_mix_key.weight", &[d, f]);
                t("channel_mix_value.weight", &[f, d]);
            }
            Ffn::DenseAndMoe { dff, act, moe } => {
                dense(&mut t, *dff, act);
                t("post_ffw_norm_1.weight", &[d]);
                t("pre_ffw_norm_2.weight", &[d]);
                t("post_ffw_norm_2.weight", &[d]);
                t("ffn_gate_inp.scale", &[d]);
                t("ffn_down_exps.scale", &[moe.experts as u64]);
                moe.tensors(d, &mut t);
            }
        }
        v
    }
}

impl Moe {
    /// The mixture's tensors in a block of width `d`: the router, the experts (no gate where they are relu²), the
    /// latent projections, the selection bias, the shared expert and its gate, and the biases where the mixture has them.
    fn tensors(&self, d: u64, t: &mut dyn FnMut(&str, &[u64])) {
        let (e, f, w) = (self.experts as u64, self.expert_dff as u64, self.latent.map_or(d, |l| l as u64));
        let gated = self.act != Act::ReluSquared;
        t("ffn_gate_inp.weight", &[d, e]);
        if gated {
            t("ffn_gate_exps.weight", &[w, f, e]);
        }
        t("ffn_up_exps.weight", &[w, f, e]);
        t("ffn_down_exps.weight", &[f, w, e]);
        if self.latent.is_some() {
            t("ffn_latent_down.weight", &[d, w]);
            t("ffn_latent_up.weight", &[w, d]);
        }
        if self.gating == (Gating::Sigmoid { bias: true }) {
            t("exp_probs_b.bias", &[e]);
        }
        if self.shared_gate {
            t("ffn_gate_inp_shexp.weight", &[d]);
        }
        if self.shared_experts > 0 {
            let s = self.shared_dff as u64;
            if gated {
                t("ffn_gate_shexp.weight", &[d, s]);
            }
            t("ffn_up_shexp.weight", &[d, s]);
            t("ffn_down_shexp.weight", &[s, d]);
        }
        if self.biases {
            t("ffn_gate_inp.bias", &[e]);
            t("ffn_gate_exps.bias", &[f, e]);
            t("ffn_up_exps.bias", &[f, e]);
            t("ffn_down_exps.bias", &[d, e]);
        }
    }
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
    /// The experts compute `latent` wide: the input projected down by `ffn_latent_down` before them, their weighted sum
    /// up by `ffn_latent_up` after them, the router and the shared expert on the input as it is (Nemotron 3's).
    pub latent: Option<usize>,
}

/// A gated activation `act(gate) · up`.
#[derive(Clone, Debug, PartialEq)]
pub enum Act {
    Silu,
    GeluTanh,
    /// gpt-oss: `clamp(gate)·σ(α·gate) · (clamp(up)+1)`.
    SwigluOai { alpha: f64, limit: f64 },
    /// Arcee's and Nemotron's `relu(up)²`, which is not gated: the FFN has no gate projection.
    ReluSquared,
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
    /// PhiMoE's sparsemixer at inference (transformers `sparsemixer`): two experts by logit, each weighed by the softmax
    /// over the experts not yet chosen whose logits `x` lie within `2·jitter` of its own `m` as `(m − x)/max(|x|, m)`
    /// measures it; never renormalized.
    SparseMixer { jitter: f64 },
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
/// Which of `n_layers` blocks are recurrent, as llama.cpp's `get_key_or_arr` reads `attention.recurrent_layers`: a flag
/// a block, or one flag for every block; where the key is unwritten, every block but each `full_attention_interval`-th
/// (`default` where that is unwritten too).
fn recurrent_layers<S: SpecSource + ?Sized>(k: &Keys<'_, S>, n_layers: usize, default: usize) -> Result<Vec<bool>, SpecError> {
    let flag = |v: &MetaValue| v.as_bool().or_else(|| v.as_u64().map(|x| x != 0));
    let missing = || SpecError::MissingKey(k.key("attention.recurrent_layers"));
    match k.get("attention.recurrent_layers") {
        Some(MetaValue::Array { items, .. }) => items.iter().take(n_layers).map(flag).collect::<Option<_>>().ok_or_else(missing),
        Some(one) => Ok(vec![flag(one).ok_or_else(missing)?; n_layers]),
        None => {
            let every = k.usize_or("full_attention_interval", default);
            if every == 0 {
                return Err(SpecError::Inconsistent("a full_attention_interval of 0".into()));
            }
            Ok((0..n_layers).map(|l| (l + 1) % every != 0).collect())
        }
    }
}

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
        // A model with no attention (Mamba-2's) has no heads to divide the width by.
        let head_dim = k.usize_or("attention.key_length", d.checked_div(heads).unwrap_or(0));
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
                latent: None,
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
                latent: None,
            }))
        };
        // The model's parts past its blocks, which an arm reads with its layers.
        let mut parts: Vec<Part> = Vec::new();
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
                        latent: None,
                    }),
                    _ => dense(Act::Silu)?,
                };
                (0..n_layers)
                    .map(|_| Layer {
                        mixer: mixer.clone(),
                        ffn: ffn.clone(),
                        pre_norms: true,
                        post_mixer_norm: false,
                        post_ffn_norm: false,
                    })
                    .collect()
            }
            // `models/mistral3.cpp` (Ministral 3, Mistral Small 3.1 and 3.2): Llama's interleaved rotation over every
            // dimension of each head (the graph asserts it) and scores scaled by `1/√head_dim` (the loader reads no
            // attention scale); where `attention.temperature_scale` is set, every layer's rotated queries take the
            // attention temperature, its floor the rotation's original context and its positions counted from zero; a
            // dense SwiGLU, or for a checkpoint that counts experts a softmax router over experts as wide as the FFN whose
            // kept weights are renormalized and unscaled (the loader reads no routed scale). A dense FFN's biases, which
            // the graph adds where the checkpoint carries them, are refused: the spec reads none.
            "mistral3" => {
                let attn = attention(false, false, 0)?;
                let rotated = attn.rope.as_ref().map_or(0, |r| r.dims);
                if rotated != head_dim || value_dim != head_dim {
                    return Err(SpecError::Inconsistent(format!(
                        "mistral3 rotates {rotated} of heads {head_dim} wide with values {value_dim} wide; its graph rotates \
                         every dimension of heads as wide as their values"
                    )));
                }
                if let Some(bias) = ["ffn_gate.bias", "ffn_up.bias", "ffn_down.bias"].into_iter().find(|b| has_any(g, b)) {
                    return Err(SpecError::Inconsistent(format!("a dense FFN's `{bias}`: the spec reads no FFN biases")));
                }
                let beta = k.f64_or("attention.temperature_scale", 0.0);
                let temperature = if beta == 0.0 {
                    None
                } else {
                    let floor = k.usize_or("rope.scaling.original_context_length", k.usize_or("context_length", 0));
                    if floor == 0 {
                        return Err(SpecError::Inconsistent(
                            "an attention temperature with no original context to count its positions' floors by".into(),
                        ));
                    }
                    Some(Temperature { floor: floor as f64, scale: beta, offset: 0.0 })
                };
                let mixer = Mixer::Attention(Attention { temperature, ..attn });
                let ffn = match k.usize_or("expert_count", 0) {
                    0 => dense(Act::Silu)?,
                    experts => Ffn::Moe(Moe {
                        experts,
                        top_k: k.usize("expert_used_count")?,
                        expert_dff: k.usize("feed_forward_length")?,
                        act: Act::Silu,
                        gating: Gating::Softmax,
                        norm_top_k: true,
                        weights_scale: 1.0,
                        shared_experts: 0,
                        shared_dff: 0,
                        shared_gate: false,
                        biases: false,
                        groups: None,
                        latent: None,
                    }),
                };
                (0..n_layers)
                    .map(|_| Layer { mixer: mixer.clone(), ffn: ffn.clone(), pre_norms: true, post_mixer_norm: false, post_ffn_norm: false })
                    .collect()
            }
            // `models/granite.cpp`, whose graph MiniCPM shares (`models/minicpm.cpp`): Llama's interleaved rotation and
            // SwiGLU FFN; Granite's scores times `attention.scale` where the file sets one, else 1/√head_dim (MiniCPM's
            // loader reads no attention scale). The multipliers follow the layers.
            "granite" | "minicpm" => {
                let set = if arch == "granite" { k.f64_or("attention.scale", 0.0) } else { 0.0 };
                let scale = if set != 0.0 { set } else { 1.0 / (head_dim as f64).sqrt() };
                let mixer = Mixer::Attention(Attention { scale, ..attention(false, false, 0)? });
                let ffn = dense(Act::Silu)?;
                (0..n_layers)
                    .map(|_| Layer { mixer: mixer.clone(), ffn: ffn.clone(), pre_norms: true, post_mixer_norm: false, post_ffn_norm: false })
                    .collect()
            }
            // `models/arcee.cpp`: Llama's interleaved rotation, and an FFN of relu² of the up projection with no gate.
            "arcee" => {
                let mixer = Mixer::Attention(attention(false, false, 0)?);
                let ffn = dense(Act::ReluSquared)?;
                (0..n_layers)
                    .map(|_| Layer { mixer: mixer.clone(), ffn: ffn.clone(), pre_norms: true, post_mixer_norm: false, post_ffn_norm: false })
                    .collect()
            }
            // `models/olmo2.cpp` (OLMo 2, and OLMo 3's windows): no norm before either sublayer, each reading the residual
            // stream itself, and a norm after the mixer and after the FFN before each residual add; the queries and the
            // keys each normed across every head at once (`attn_q_norm` as wide as the query projection, `attn_k_norm` as
            // the keys'); a NEOX rotation; a SiLU-gated FFN. With `attention.sliding_window` set, all but every
            // `sliding_window_pattern`-th block (default 4) attends a sliding window, turning by `rope.freq_base_swa`
            // (default the global base) unscaled.
            "olmo2" => {
                let global = attention(true, false, 0)?;
                let width = |name: &str| g.dims(&format!("blk.0.{name}.weight")).and_then(|d| d.first().copied()).map(|w| w as usize);
                if !global.qk_norm || width("attn_q_norm") != Some(heads * head_dim) || width("attn_k_norm") != Some(kv_heads * head_dim) {
                    return Err(SpecError::Inconsistent(format!(
                        "attn_q_norm is {:?} and attn_k_norm {:?} wide: OLMo 2 norms its {heads} query heads' {} and its {kv_heads} KV heads' {} at once",
                        width("attn_q_norm"),
                        width("attn_k_norm"),
                        heads * head_dim,
                        kv_heads * head_dim
                    )));
                }
                let global = Attention { qk_norm_whole: true, ..global };
                let window = k.usize_or("attention.sliding_window", 0);
                let swa = sliding_layers(&k, n_layers, 4, window)?;
                let local = Attention {
                    window: Some(Window::Sliding(window)),
                    rope: global.rope.clone().map(|r| Rope { theta: k.f64_or("rope.freq_base_swa", r.theta), scaling: RopeScaling::None, ..r }),
                    ..global.clone()
                };
                let ffn = dense(Act::Silu)?;
                swa.iter()
                    .map(|&s| Layer {
                        mixer: Mixer::Attention(if s { local.clone() } else { global.clone() }),
                        ffn: ffn.clone(),
                        pre_norms: false,
                        post_mixer_norm: true,
                        post_ffn_norm: true,
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
                    latent: None,
                });
                (0..n_layers)
                    .map(|_| Layer {
                        mixer: Mixer::Attention(attn.clone()),
                        ffn: ffn.clone(),
                        pre_norms: true,
                        post_mixer_norm: false,
                        post_ffn_norm: false,
                    })
                    .collect()
            }
            // `models/minimax-01.cpp`: lightning attention (`Lightning`) on the recurrent blocks, `attention.recurrent_layers`
            // or all but every `full_attention_interval`-th (default 8); attention under a NEOX rotation of
            // `rope.dimension_count` of each head on the rest; in every block a mixture of `expert_count` experts as wide as
            // `feed_forward_length`, the softmax's kept weights renormalized (the graph fixes it); each sublayer's residual its
            // normed input times `residual_scale` (`Part::NormedResidual`). The lightning decay's depth scale divides by the
            // blocks after the first, so a one-block stack is refused.
            "minimax-01" => {
                let recurrent = recurrent_layers(&k, n_layers, 8)?;
                if value_dim != head_dim {
                    return Err(SpecError::Inconsistent(format!("values of {value_dim} beside keys of {head_dim}; minimax-01 asserts them equal")));
                }
                if n_layers < 2 && recurrent.iter().any(|&r| r) {
                    return Err(SpecError::Inconsistent("a lightning block in a one-block stack, whose decay's depth scale is 0/0".into()));
                }
                let at = recurrent.iter().position(|r| !r).unwrap_or(0);
                let attn = attention(true, false, at)?;
                let lightning = Lightning { heads, head_dim };
                let ffn = Ffn::Moe(Moe {
                    experts: k.usize("expert_count")?,
                    top_k: k.usize("expert_used_count")?,
                    expert_dff: k.usize("feed_forward_length")?,
                    act: Act::Silu,
                    gating: Gating::Softmax,
                    norm_top_k: true,
                    weights_scale: k.get("expert_weights_scale").and_then(decimal).filter(|&s| s != 0.0).unwrap_or(1.0),
                    shared_experts: 0,
                    shared_dff: 0,
                    shared_gate: false,
                    biases: false,
                    groups: None,
                    latent: None,
                });
                parts.push(Part::NormedResidual { scale: k.f64("residual_scale")? });
                recurrent
                    .iter()
                    .map(|&r| Layer {
                        mixer: if r { Mixer::Recurrent(Recurrent::Lightning(lightning.clone())) } else { Mixer::Attention(attn.clone()) },
                        ffn: ffn.clone(),
                        pre_norms: true,
                        post_mixer_norm: false,
                        post_ffn_norm: false,
                    })
                    .collect()
            }
            // `models/gemma4.cpp`: Gemma 3's sandwich-normed blocks with QK norms and unscaled scores, the window on the
            // blocks `attention.sliding_window_pattern` marks; a windowed block's heads `key_length_swa` wide under the NEOX
            // rotation of `rope.dimension_count_swa` turning by `rope.freq_base_swa`, a global block's `key_length` wide
            // under `rope_freqs`' rotation of `rope.dimension_count`, each block its own KV heads; the values normed with no
            // weight, a KV block holding no `attn_v` reading its keys as its values, the blocks from `n − shared_kv_layers`
            // on attending earlier blocks' caches; every block a GELU FFN of its `feed_forward_length`, beside Gemma 4's
            // mixture where it holds a router; per-layer inputs of `embedding_length_per_layer_input` where that is not 0;
            // each block's output scaled where it holds `layer_output_scale`; the embedding scaled by `√d`.
            "gemma4" => {
                let swa: Vec<bool> = match k.get("attention.sliding_window_pattern") {
                    Some(MetaValue::Array { items, .. }) => items
                        .iter()
                        .take(n_layers)
                        .map(|v| v.as_bool().or_else(|| v.as_u64().map(|x| x != 0)))
                        .collect::<Option<_>>()
                        .ok_or_else(|| SpecError::MissingKey(k.key("attention.sliding_window_pattern")))?,
                    _ => return Err(SpecError::MissingKey(k.key("attention.sliding_window_pattern"))),
                };
                if swa.len() != n_layers {
                    return Err(SpecError::Inconsistent(format!("a window pattern of {} for {n_layers} blocks", swa.len())));
                }
                let per_block = |suffix: &str| -> Result<Vec<usize>, SpecError> {
                    let v: Vec<usize> = match k.get(suffix) {
                        Some(MetaValue::Array { items, .. }) => items
                            .iter()
                            .take(n_layers)
                            .map(|v| v.as_u64().map(|x| x as usize))
                            .collect::<Option<_>>()
                            .ok_or_else(|| SpecError::MissingKey(k.key(suffix)))?,
                        Some(one) => vec![one.as_u64().ok_or_else(|| SpecError::MissingKey(k.key(suffix)))? as usize; n_layers],
                        None => return Err(SpecError::MissingKey(k.key(suffix))),
                    };
                    if v.len() != n_layers {
                        return Err(SpecError::Inconsistent(format!("{} values of `{}` for {n_layers} blocks", v.len(), k.key(suffix))));
                    }
                    Ok(v)
                };
                let (kv, dff) = (per_block("attention.head_count_kv")?, per_block("feed_forward_length")?);
                let (key_full, key_swa) = (head_dim, k.usize_or("attention.key_length_swa", head_dim));
                if value_dim != key_full || k.usize_or("attention.value_length_swa", key_swa) != key_swa {
                    return Err(SpecError::Inconsistent("values unlike the keys; llama.cpp's gemma4 requires them alike".into()));
                }
                let rot_full = k.usize_or("rope.dimension_count", key_full);
                let theta_full = k.f64_or("rope.freq_base", 10000.0);
                let global = Rope {
                    dims: rot_full,
                    theta: theta_full,
                    neox: true,
                    scaling: if g.has("rope_freqs.weight") { RopeScaling::Factors } else { RopeScaling::None },
                    sections: None,
                };
                let local = Rope {
                    dims: k.usize_or("rope.dimension_count_swa", rot_full),
                    theta: k.f64_or("rope.freq_base_swa", theta_full),
                    neox: true,
                    scaling: RopeScaling::None,
                    sections: None,
                };
                let window = k.usize("attention.sliding_window")?;
                let shared = k.usize_or("attention.shared_kv_layers", 0);
                let from = n_layers.saturating_sub(shared);
                if shared > 0 && (from < 2 || !(swa[from - 2] && !swa[from - 1])) {
                    return Err(SpecError::Inconsistent(format!(
                        "the blocks before the shared ones ({from}) are not a windowed block and a global one, the caches llama.cpp's gemma4 shares"
                    )));
                }
                let ple = k.usize("embedding_length_per_layer_input")?;
                let mixture = Moe {
                    experts: k.usize_or("expert_count", 0),
                    top_k: k.usize_or("expert_used_count", 0),
                    expert_dff: k.usize_or("expert_feed_forward_length", 0),
                    act: Act::GeluTanh,
                    gating: Gating::Softmax,
                    norm_top_k: true,
                    weights_scale: 1.0,
                    shared_experts: 0,
                    shared_dff: 0,
                    shared_gate: false,
                    biases: false,
                    groups: None,
                    latent: None,
                };
                let mut keyed = Vec::new();
                let mut layers = Vec::with_capacity(n_layers);
                for l in 0..n_layers {
                    let attn = Attention {
                        kv_heads: kv[l],
                        head_dim: if swa[l] { key_swa } else { key_full },
                        value_dim: if swa[l] { key_swa } else { key_full },
                        rope: Some(if swa[l] { local.clone() } else { global.clone() }),
                        qk_norm: true,
                        window: swa[l].then_some(Window::Sliding(window)),
                        scale: 1.0,
                        ..attention(true, false, l)?
                    };
                    if l < from && !g.has(&format!("blk.{l}.attn_v.weight")) && !g.has(&format!("blk.{l}.attn_qkv.weight")) {
                        keyed.push(l);
                    }
                    let ffn = if g.has(&format!("blk.{l}.ffn_gate_inp.weight")) {
                        if mixture.experts == 0 || mixture.top_k == 0 || mixture.expert_dff == 0 {
                            return Err(SpecError::Inconsistent(format!("block {l} routes, but the mixture is {mixture:?}")));
                        }
                        Ffn::DenseAndMoe { dff: dff[l], act: Act::GeluTanh, moe: mixture.clone() }
                    } else {
                        Ffn::Dense { dff: dff[l], act: Act::GeluTanh }
                    };
                    layers.push(Layer { mixer: Mixer::Attention(attn), ffn, pre_norms: true, post_mixer_norm: true, post_ffn_norm: true });
                }
                if ple > 0 {
                    parts.push(Part::PerLayerInput { dim: ple });
                }
                if shared > 0 {
                    parts.push(Part::SharedKv { from });
                }
                parts.push(Part::ValueNorm);
                if !keyed.is_empty() {
                    parts.push(Part::KeyAsValue { layers: keyed });
                }
                if g.has("blk.0.layer_output_scale.weight") {
                    parts.push(Part::LayerScale);
                }
                layers
            }
            // `models/rwkv7.cpp` and `models/rwkv7-base.cpp`: every block RWKV-7's time mix (`Rwkv7`, heads of
            // `wkv.head_size` and the four low-rank pairs' ranks, the first block's value the one every later block mixes
            // with) then its channel mix of `feed_forward_length`; the embedding LayerNormed (`token_embd_norm`), and
            // every norm a LayerNorm with its bias at `attention.layer_norm_epsilon`, the channel mix's `attn_norm_2`.
            // The graph asserts two token shifts a block (`token_shift_count`).
            "rwkv7" => {
                let head_size = k.usize("wkv.head_size")?;
                if head_size == 0 || d % head_size != 0 {
                    return Err(SpecError::Inconsistent(format!("RWKV-7 heads of {head_size} over a width of {d}")));
                }
                if k.usize_or("token_shift_count", 2) != 2 {
                    return Err(SpecError::Inconsistent(format!(
                        "a token_shift_count of {}; llama.cpp's rwkv7 asserts 2",
                        k.usize_or("token_shift_count", 2)
                    )));
                }
                let time = Rwkv7 {
                    head_size,
                    decay_lora: k.usize("attention.decay_lora_rank")?,
                    iclr_lora: k.usize("attention.iclr_lora_rank")?,
                    value_res_lora: k.usize("attention.value_residual_mix_lora_rank")?,
                    gate_lora: k.usize("attention.gate_lora_rank")?,
                    first: false,
                };
                let ffn = Ffn::ChannelMix { dff: k.usize("feed_forward_length")? };
                parts.push(Part::EmbeddingNorm);
                (0..n_layers)
                    .map(|l| Layer {
                        mixer: Mixer::Recurrent(Recurrent::Rwkv7(Rwkv7 { first: l == 0, ..time.clone() })),
                        ffn: ffn.clone(),
                        pre_norms: true,
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
                        pre_norms: true,
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
                        pre_norms: true,
                        post_mixer_norm: false,
                        post_ffn_norm: false,
                    })
                    .collect()
            }
            // transformers' PhiMoE (`PhimoeForCausalLM`), which llama.cpp's `models/phimoe.cpp` does not compute: llama.cpp
            // runs phi3's graph over it — RMSNorms plus their biases, a softmax router whose two kept weights are
            // renormalized, no window — where PhiMoE's norms are LayerNorms, its router sparsemixer, and its attention
            // windowed. A phimoe GGUF is a PhiMoE checkpoint converted, read here as the model it holds: every block norm a
            // LayerNorm with its bias; attention with a NEOX rotation over `rope.dimension_count` of each head, its
            // projections biased where the checkpoint carries biases, windowed where `attention.sliding_window` is short of
            // the context; in every block sparsemixer's two of `expert_count` experts, its jitter PhiMoE's 0.01, which the
            // GGUF does not carry; the head biased where the checkpoint carries `output.bias`. LongRoPE's factor tables are
            // refused, as phi3's are.
            "phimoe" => {
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
                let top_k = k.usize("expert_used_count")?;
                if top_k != 2 {
                    return Err(SpecError::Inconsistent(format!("{top_k} experts a token: PhiMoE's sparsemixer chooses two")));
                }
                let ffn = Ffn::Moe(Moe {
                    experts: k.usize("expert_count")?,
                    top_k,
                    expert_dff: k.usize("feed_forward_length")?,
                    act: Act::Silu,
                    gating: Gating::SparseMixer { jitter: 0.01 },
                    norm_top_k: false,
                    weights_scale: 1.0,
                    shared_experts: 0,
                    shared_dff: 0,
                    shared_gate: false,
                    biases: false,
                    groups: None,
                    latent: None,
                });
                (0..n_layers)
                    .map(|_| Layer { mixer: Mixer::Attention(attn.clone()), ffn: ffn.clone(), pre_norms: true, post_mixer_norm: false, post_ffn_norm: false })
                    .collect()
            }
            // `models/lfm2.cpp`: a gated short convolution of `shortconv.l_cache` taps on every block whose
            // `attention.head_count_kv` (an array, a block each) is 0, else attention with a NEOX rotation and per-head QK
            // norms, windowed where `attention.sliding_window` is set; a dense SwiGLU everywhere. The checkpoint stores its
            // output norm as `token_embd_norm`, which `model::open_checkpoint` presents as `output_norm` (a checkpoint read
            // raw is refused); a decision head, a bidirectional encoder and ColBERT's projection are refused.
            "lfm2" => {
                if g.has("token_embd_norm.weight") && !g.has("output_norm.weight") {
                    return Err(SpecError::Inconsistent(
                        "an lfm2 checkpoint read raw: its output norm is `token_embd_norm`; `model::open_checkpoint` presents \
                         it as `output_norm`"
                            .into(),
                    ));
                }
                if k.usize_or("decision_block_count", 0) > 0 {
                    return Err(SpecError::Inconsistent("LFM2's decision head (`decision_block_count`) is not read".into()));
                }
                if k.get("attention.causal").and_then(MetaValue::as_bool) == Some(false) {
                    return Err(SpecError::Inconsistent("a bidirectional LFM2 (`attention.causal` false): a causal decoder is read here".into()));
                }
                if g.has("dense_2_out.weight") {
                    return Err(SpecError::Inconsistent("LFM2-ColBERT's output projection (`dense_2_out`) is not read".into()));
                }
                let kv: Vec<usize> = match k.get("attention.head_count_kv") {
                    Some(MetaValue::Array { items, .. }) => items
                        .iter()
                        .take(n_layers)
                        .map(|v| v.as_u64().map(|x| x as usize))
                        .collect::<Option<_>>()
                        .ok_or_else(|| SpecError::MissingKey(k.key("attention.head_count_kv")))?,
                    Some(v) => vec![v.as_u64().ok_or_else(|| SpecError::MissingKey(k.key("attention.head_count_kv")))? as usize; n_layers],
                    None => vec![heads; n_layers],
                };
                if kv.len() != n_layers {
                    return Err(SpecError::Inconsistent(format!("{} KV head counts for {n_layers} blocks", kv.len())));
                }
                let taps = k.usize("shortconv.l_cache")?;
                if taps < 2 {
                    return Err(SpecError::Inconsistent(format!("a short convolution of {taps} taps")));
                }
                let at = kv.iter().position(|&n| n > 0).unwrap_or(0);
                let mut attn = attention(true, false, at)?;
                let window = k.usize_or("attention.sliding_window", 0);
                if window > 0 {
                    attn.window = Some(Window::Sliding(window));
                }
                let ffn = dense(Act::Silu)?;
                kv.iter()
                    .map(|&n| Layer {
                        mixer: if n == 0 {
                            Mixer::Recurrent(Recurrent::ShortConv(ShortConv { taps }))
                        } else {
                            Mixer::Attention(Attention { kv_heads: n, ..attn.clone() })
                        },
                        ffn: ffn.clone(),
                        pre_norms: true,
                        post_mixer_norm: false,
                        post_ffn_norm: false,
                    })
                    .collect()
            }
            // `models/mamba2.cpp` (the graph `models/mamba.cpp` builds for both Mambas): each block its norm and Mamba-2's
            // scan alone, with no FFN.
            "mamba2" => {
                let scan = ssd(&k, g, 0)?;
                if !scan.gated_norm {
                    return Err(SpecError::MissingTensor("blk.0.ssm_norm.weight".into()));
                }
                (0..n_layers)
                    .map(|_| Layer { mixer: Mixer::Recurrent(Recurrent::Ssd(scan.clone())), ffn: Ffn::None, pre_norms: true, post_mixer_norm: false, post_ffn_norm: false })
                    .collect()
            }
            // `models/kimi-linear.cpp`: Kimi delta attention where a block has no KV heads (`kda.head_dim` heads of the
            // model's heads, a convolution of `ssm.conv_kernel` taps), latent attention with no rotation where it has them
            // (its queries full-rank or low-rank, its keys `key_length_mla` wide with `rope.dimension_count` of them shared
            // and unrotated, scored by `1/√key_length_mla`); the first `leading_dense_block_count` blocks a dense SwiGLU,
            // the rest a mixture under `expert_gating_func`'s scores with the selection bias, its kept weights always
            // renormalized and scaled by `expert_weights_scale`, beside `expert_shared_count` shared experts. The latent
            // norm at 10⁻⁶, as Kimi's code builds it, where this graph norms it at the model's epsilon
            // ([`MLA_LATENT_NORM_EPS`]).
            "kimi-linear" => {
                let kv: Vec<usize> = match k.get("attention.head_count_kv") {
                    Some(MetaValue::Array { items, .. }) => items
                        .iter()
                        .take(n_layers)
                        .map(|v| v.as_u64().map(|x| x as usize))
                        .collect::<Option<_>>()
                        .ok_or_else(|| SpecError::MissingKey(k.key("attention.head_count_kv")))?,
                    _ => return Err(SpecError::MissingKey(k.key("attention.head_count_kv"))),
                };
                if kv.len() != n_layers {
                    return Err(SpecError::Inconsistent(format!("{} KV head counts for {n_layers} blocks", kv.len())));
                }
                let delta = DeltaAttention { heads, head_dim: k.usize("kda.head_dim")?, conv_taps: k.usize("ssm.conv_kernel")? };
                if delta.head_dim == 0 || delta.conv_taps == 0 {
                    return Err(SpecError::Inconsistent(format!("Kimi delta attention of {delta:?}")));
                }
                let key = k.usize("attention.key_length_mla")?;
                let rope_dims = k.usize("rope.dimension_count")?;
                if rope_dims >= key {
                    return Err(SpecError::Inconsistent(format!("latent keys of {key} with {rope_dims} shared")));
                }
                let mla = Mla {
                    heads,
                    q_lora_rank: k.get("attention.q_lora_rank").and_then(MetaValue::as_u64).map(|v| v as usize),
                    kv_lora_rank: k.usize("attention.kv_lora_rank")?,
                    qk_nope: key - rope_dims,
                    qk_rope: rope_dims,
                    v_dim: k.usize("attention.value_length_mla")?,
                    rope: None,
                    scale: 1.0 / (key as f64).sqrt(),
                    kv_b_split: has_any(g, "attn_k_b.weight"),
                    indexer: None,
                    latent_norm_eps: MLA_LATENT_NORM_EPS,
                };
                let lead = k.usize_or("leading_dense_block_count", 0);
                let dense_ffn = Ffn::Dense { dff: k.usize("feed_forward_length")?, act: Act::Silu };
                let expert_dff = k.usize("expert_feed_forward_length")?;
                let shared = k.usize_or("expert_shared_count", 0);
                let moe = Moe {
                    experts: k.usize("expert_count")?,
                    top_k: k.usize("expert_used_count")?,
                    expert_dff,
                    act: Act::Silu,
                    gating: match k.usize_or("expert_gating_func", 1) {
                        2 => Gating::Sigmoid { bias: has_any(g, "exp_probs_b.bias") },
                        1 => Gating::Softmax,
                        other => return Err(SpecError::Inconsistent(format!("an expert gating function of {other}"))),
                    },
                    norm_top_k: true,
                    weights_scale: k.get("expert_weights_scale").and_then(decimal).filter(|&s| s != 0.0).unwrap_or(1.0),
                    shared_experts: shared,
                    shared_dff: expert_dff * shared.max(1),
                    shared_gate: false,
                    biases: false,
                    groups: None,
                    latent: None,
                };
                (0..n_layers)
                    .map(|l| Layer {
                        mixer: if kv[l] == 0 { Mixer::Recurrent(Recurrent::DeltaAttention(delta.clone())) } else { Mixer::Mla(mla.clone()) },
                        ffn: if l < lead { dense_ffn.clone() } else { Ffn::Moe(moe.clone()) },
                        pre_norms: true,
                        post_mixer_norm: false,
                        post_ffn_norm: false,
                    })
                    .collect()
            }
            // `models/jamba.cpp`: Mamba-1's selective scan where a block has no KV heads (its time step, B and C each
            // RMS-normed where the block holds their norms), attention with no rotation where it has them; after either a
            // dense SwiGLU, or where the block has a router a mixture of `expert_count` experts, the softmax over every
            // expert's logit, the top `expert_used_count` kept as they stand.
            "jamba" => {
                let kv: Vec<usize> = match k.get("attention.head_count_kv") {
                    Some(MetaValue::Array { items, .. }) => items
                        .iter()
                        .take(n_layers)
                        .map(|v| v.as_u64().map(|x| x as usize))
                        .collect::<Option<_>>()
                        .ok_or_else(|| SpecError::MissingKey(k.key("attention.head_count_kv")))?,
                    _ => return Err(SpecError::MissingKey(k.key("attention.head_count_kv"))),
                };
                if kv.len() != n_layers {
                    return Err(SpecError::Inconsistent(format!("{} KV head counts for {n_layers} blocks", kv.len())));
                }
                let scan = match (0..n_layers).find(|&l| kv[l] == 0) {
                    None => None,
                    Some(at) => {
                        let s = Selective {
                            inner: k.usize("ssm.inner_size")?,
                            state: k.usize("ssm.state_size")?,
                            dt_rank: k.usize("ssm.time_step_rank")?,
                            conv_taps: k.usize("ssm.conv_kernel")?,
                            inner_norms: g.has(&format!("blk.{at}.ssm_dt_norm.weight")),
                        };
                        if s.inner == 0 || s.state == 0 || s.dt_rank == 0 || s.conv_taps == 0 {
                            return Err(SpecError::Inconsistent(format!("a selective scan of {s:?}")));
                        }
                        Some(s)
                    }
                };
                let at = kv.iter().position(|&n| n > 0).unwrap_or(0);
                let mut attn = attention(false, false, at)?;
                attn.rope = None;
                let dff = k.usize("feed_forward_length")?;
                let experts = k.usize_or("expert_count", 0);
                (0..n_layers)
                    .map(|l| {
                        let mixer = match kv[l] {
                            0 => Mixer::Recurrent(Recurrent::Selective(scan.clone().expect("a scan block reads its scan"))),
                            n => Mixer::Attention(Attention { kv_heads: n, ..attn.clone() }),
                        };
                        let ffn = if g.has(&format!("blk.{l}.ffn_gate_inp.weight")) {
                            Ffn::Moe(Moe {
                                experts,
                                top_k: k.usize("expert_used_count")?,
                                expert_dff: dff,
                                act: Act::Silu,
                                gating: Gating::Softmax,
                                norm_top_k: false,
                                weights_scale: k.get("expert_weights_scale").and_then(decimal).filter(|&s| s != 0.0).unwrap_or(1.0),
                                shared_experts: 0,
                                shared_dff: 0,
                                shared_gate: false,
                                biases: false,
                                groups: None,
                                latent: None,
                            })
                        } else {
                            Ffn::Dense { dff, act: Act::Silu }
                        };
                        Ok(Layer { mixer, ffn, pre_norms: true, post_mixer_norm: false, post_ffn_norm: false })
                    })
                    .collect::<Result<Vec<_>, SpecError>>()?
            }
            // `models/nemotron-h.cpp`: blocks of one sublayer each under the block's one norm (`attn_norm`): Mamba-2's
            // scan where the block has neither KV heads nor an FFN width, attention with no rotation where it has KV
            // heads, an FFN of relu² with no gate where it has an FFN width. Nemotron 3's (`nemotron_h_moe`, the same
            // graph) has a mixture where Nemotron-H has the FFN, which the arm does not read yet and refuses by name.
            "nemotron_h" | "nemotron_h_moe" => {
                let per_block = |suffix: &str| -> Result<Vec<usize>, SpecError> {
                    let v: Vec<usize> = match k.get(suffix) {
                        Some(MetaValue::Array { items, .. }) => items
                            .iter()
                            .map(|v| v.as_u64().map(|x| x as usize))
                            .collect::<Option<_>>()
                            .ok_or_else(|| SpecError::MissingKey(k.key(suffix)))?,
                        Some(v) => vec![v.as_u64().ok_or_else(|| SpecError::MissingKey(k.key(suffix)))? as usize; n_layers],
                        None => return Err(SpecError::MissingKey(k.key(suffix))),
                    };
                    if v.len() < n_layers {
                        return Err(SpecError::Inconsistent(format!("{} of `{}` for {n_layers} blocks", v.len(), k.key(suffix))));
                    }
                    Ok(v)
                };
                let (kv, ff) = (per_block("attention.head_count_kv")?, per_block("feed_forward_length")?);
                if has_any(g, "ffn_gate_inp.weight") {
                    return Err(SpecError::Inconsistent("Nemotron 3's expert mixture (`ffn_gate_inp`) is not read yet".into()));
                }
                for (bias, what) in [("ffn_up.bias", "an FFN bias"), ("ffn_down.bias", "an FFN bias")] {
                    if has_any(g, bias) {
                        return Err(SpecError::Inconsistent(format!("`{bias}`: {what}, which the spec's dense FFN does not add")));
                    }
                }
                let scan = (0..n_layers).find(|&l| kv[l] == 0 && ff[l] == 0).map(|l| ssd(&k, g, l)).transpose()?;
                if let Some(s) = &scan {
                    if !s.gated_norm {
                        return Err(SpecError::MissingTensor("ssm_norm.weight of a scan block".into()));
                    }
                }
                let at = (0..n_layers).find(|&l| kv[l] > 0).unwrap_or(0);
                let mut attn = attention(false, false, at)?;
                attn.rope = None;
                if let Some(scale) = k.get("attention.scale").and_then(decimal).filter(|&s| s != 0.0) {
                    attn.scale = scale;
                }
                (0..n_layers)
                    .map(|l| {
                        let (mixer, ffn) = match (kv[l], ff[l]) {
                            (0, 0) => (Mixer::Recurrent(Recurrent::Ssd(scan.clone().expect("a scan block reads its scan"))), Ffn::None),
                            (n, 0) => (Mixer::Attention(Attention { kv_heads: n, ..attn.clone() }), Ffn::None),
                            (0, dff) => (Mixer::None, Ffn::Dense { dff, act: Act::ReluSquared }),
                            (n, dff) => {
                                return Err(SpecError::Inconsistent(format!("block {l} has {n} KV heads and an FFN of {dff}: one sublayer a block")))
                            }
                        };
                        Ok(Layer { mixer, ffn, pre_norms: true, post_mixer_norm: false, post_ffn_norm: false })
                    })
                    .collect::<Result<Vec<_>, _>>()?
            }
            // `models/falcon-h1.cpp`: each block's one normed input through attention under a NEOX rotation and through
            // Mamba-2's scan beside it, their outputs summed into the residual, then a dense SwiGLU; the converter folds
            // every MuP multiplier into the weights. The FFN norm is stored without its `.weight` suffix, as llama.cpp
            // loads it; `model::layout` presents it with one. llama.cpp's graph adds no FFN bias and no attention output
            // bias whatever the checkpoint holds.
            "falcon-h1" => {
                if g.has("blk.0.ffn_norm") && !g.has("blk.0.ffn_norm.weight") {
                    return Err(SpecError::Inconsistent(
                        "a falcon-h1 checkpoint read raw: its FFN norms are `blk.<n>.ffn_norm`; `model::open_checkpoint` presents \
                         them as `blk.<n>.ffn_norm.weight`"
                            .into(),
                    ));
                }
                for (bias, what) in [
                    ("ffn_gate.bias", "an FFN bias, which llama.cpp's falcon-h1 graph adds and the spec's dense FFN does not"),
                    ("ffn_up.bias", "an FFN bias, which llama.cpp's falcon-h1 graph adds and the spec's dense FFN does not"),
                    ("ffn_down.bias", "an FFN bias, which llama.cpp's falcon-h1 graph adds and the spec's dense FFN does not"),
                    ("attn_output.bias", "an attention output bias, which llama.cpp's falcon-h1 graph does not add"),
                ] {
                    if has_any(g, bias) {
                        return Err(SpecError::Inconsistent(format!("`{bias}`: {what}")));
                    }
                }
                let attn = attention(true, false, 0)?;
                let scan = ssd(&k, g, 0)?;
                let ffn = dense(Act::Silu)?;
                (0..n_layers)
                    .map(|_| Layer {
                        mixer: Mixer::Recurrent(Recurrent::Parallel(Box::new(attn.clone()), scan.clone())),
                        ffn: ffn.clone(),
                        pre_norms: true,
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
                    .map(|_| Layer { mixer: Mixer::Attention(attn.clone()), ffn: ffn.clone(), pre_norms: true, post_mixer_norm, post_ffn_norm })
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
                    latent: None,
                });
                (0..n_layers)
                    .map(|_| Layer {
                        mixer: mixer.clone(),
                        ffn: ffn.clone(),
                        pre_norms: true,
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
                        pre_norms: true,
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
                let recurrent = recurrent_layers(&k, n_layers, 4)?;
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
                        pre_norms: true,
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
                        pre_norms: true,
                        post_mixer_norm: true,
                        post_ffn_norm: true,
                    })
                    .collect()
            }
            // `models/gemma3n.cpp`: Gemma 3's blocks — a window on the blocks the pattern marks, turning by their own base;
            // per-head QK norms; a norm after the attention and after the FFN; a GELU-gated FFN as wide as its block's
            // entry — the scores unscaled, with Gemma 3n's parts: the values normed with no weight, four AltUp streams
            // (the first active), LAuReL of rank 64 beside the attention, per-layer inputs of 256, the first ten blocks'
            // gates sparsified at the 95th percentile, and the blocks from the twentieth on attending the cache of block 18
            // (a sliding-window block) or 19 (a global one). llama.cpp fixes every one of those numbers whatever the file
            // says, and a file that says others is refused, naming them.
            "gemma3n" => {
                let window = k.usize_or("attention.sliding_window", 0);
                let swa = sliding_layers(&k, n_layers, 5, window)?;
                let global = Attention { scale: 1.0, ..attention(true, false, 0)? };
                let local = Attention {
                    window: Some(Window::Sliding(window)),
                    rope: Some(Rope {
                        theta: k.f64_or("rope.freq_base_swa", 10000.0),
                        scaling: RopeScaling::None,
                        ..global.rope.clone().expect("gemma3n rotates")
                    }),
                    ..global.clone()
                };
                let dff: Vec<usize> = match k.get("feed_forward_length") {
                    Some(MetaValue::Array { items, .. }) => items
                        .iter()
                        .take(n_layers)
                        .map(|v| v.as_u64().map(|x| x as usize))
                        .collect::<Option<_>>()
                        .ok_or_else(|| SpecError::MissingKey(k.key("feed_forward_length")))?,
                    _ => vec![k.usize("feed_forward_length")?; n_layers],
                };
                if dff.len() != n_layers {
                    return Err(SpecError::Inconsistent(format!("{} FFN widths for {n_layers} blocks", dff.len())));
                }
                let fixed = |suffix: &str, want: usize, what: &str| -> Result<(), SpecError> {
                    match k.get(suffix).and_then(MetaValue::as_u64) {
                        Some(v) if v as usize != want => Err(SpecError::Inconsistent(format!(
                            "`{}` is {v}: llama.cpp's gemma3n computes {what} of {want} whatever the file says",
                            k.key(suffix)
                        ))),
                        _ => Ok(()),
                    }
                };
                fixed("altup.num_inputs", GEMMA3N_ALTUP_STREAMS, "AltUp streams")?;
                fixed("altup.active_idx", GEMMA3N_ALTUP_ACTIVE, "an active stream")?;
                fixed("embedding_length_per_layer_input", GEMMA3N_PER_LAYER_INPUT, "per-layer inputs")?;
                let rank = g.dims("blk.0.laurel_l.weight").and_then(|d| d.get(1).copied()).ok_or_else(|| SpecError::MissingTensor("blk.0.laurel_l.weight".into()))?;
                if rank as usize != GEMMA3N_LAUREL_RANK {
                    return Err(SpecError::Inconsistent(format!("a LAuReL of rank {rank}: llama.cpp's gemma3n reads one of {GEMMA3N_LAUREL_RANK}")));
                }
                let shared = k.usize_or("attention.shared_kv_layers", 0);
                let from = GEMMA3N_KV_FROM;
                if shared != n_layers.saturating_sub(from) {
                    return Err(SpecError::Inconsistent(format!(
                        "`{}` is {shared}: llama.cpp's gemma3n shares the cache of every block from the {from}th, {} of these {n_layers}",
                        k.key("attention.shared_kv_layers"),
                        n_layers.saturating_sub(from)
                    )));
                }
                if n_layers > from && !(swa[from - 2] && !swa[from - 1]) {
                    return Err(SpecError::Inconsistent(format!(
                        "blocks {} and {} are not a sliding-window block and a global one, the caches llama.cpp's gemma3n shares",
                        from - 2,
                        from - 1
                    )));
                }
                // The multiplier is the converter's (`Normal.icdf(0.95)` in f32, as transformers computes it), which
                // llama.cpp's `f_sparsity_std_mul` rounds the other way in its last place; the spec takes the file's.
                let sparse = n_layers.min(GEMMA3N_SPARSE_LAYERS);
                let scales: Vec<f32> = match k.get("activation_sparsity_scale") {
                    Some(MetaValue::Array { items, .. }) => items.iter().map(|v| if let MetaValue::F32(f) = v { Some(*f) } else { v.as_f64().map(|x| x as f32) }).collect::<Option<_>>().unwrap_or_default(),
                    _ => Vec::new(),
                };
                let std_mul = scales.first().copied().unwrap_or(f32::NEG_INFINITY);
                let percentile = (std_mul - GEMMA3N_SPARSITY_STD_MUL).abs() <= 4.0 * f32::EPSILON;
                let fixed_scales = scales.len() >= n_layers
                    && (0..n_layers).all(|l| if l < sparse { scales[l] == std_mul && percentile } else { scales[l] == f32::NEG_INFINITY });
                if !fixed_scales {
                    return Err(SpecError::Inconsistent(format!(
                        "`{}` is {scales:?}: llama.cpp's gemma3n sparsifies the first {GEMMA3N_SPARSE_LAYERS} blocks' gates at the 95th percentile and no other",
                        k.key("activation_sparsity_scale")
                    )));
                }
                parts = vec![
                    Part::PerLayerInput { dim: GEMMA3N_PER_LAYER_INPUT },
                    Part::AltUp { streams: GEMMA3N_ALTUP_STREAMS, active: GEMMA3N_ALTUP_ACTIVE },
                    Part::Laurel { rank: GEMMA3N_LAUREL_RANK },
                ];
                if n_layers > from {
                    parts.push(Part::SharedKv { from });
                }
                parts.push(Part::ValueNorm);
                if sparse > 0 {
                    parts.push(Part::GateSparsity { layers: sparse, std_mul: std_mul as f64 });
                }
                swa.iter()
                    .zip(&dff)
                    .map(|(&s, &dff)| Layer {
                        mixer: Mixer::Attention(if s { local.clone() } else { global.clone() }),
                        ffn: Ffn::Dense { dff, act: Act::GeluTanh },
                        pre_norms: true,
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
                    .map(|_| Layer { mixer: Mixer::Attention(attn.clone()), ffn: ffn.clone(), pre_norms: true, post_mixer_norm: false, post_ffn_norm: false })
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
                        pre_norms: true,
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
                    latent: None,
                });
                (0..n_layers)
                    .map(|l| Layer {
                        mixer: Mixer::Attention(Attention {
                            window: (every == 0 || l % every < every - 1).then_some(Window::Sliding(window)),
                            ..attn.clone()
                        }),
                        ffn: ffn.clone(),
                        pre_norms: true,
                        post_mixer_norm: false,
                        post_ffn_norm: false,
                    })
                    .collect()
            }
            // `models/deepseek2.cpp`: multi-head latent attention with interleaved rotation; the first
            // `leading_dense_block_count` blocks a dense SiLU FFN, the rest a routed mixture with shared experts —
            // softmax routing (V2) or sigmoid with a selection bias and grouped choice (V3), per
            // `expert_gating_func`.
            // `models/deepseek32.cpp` and `models/glm-dsa.cpp` (DeepSeek-V3.2, GLM-5): the same, the query always low-rank
            // and the latent's expansion split, with DeepSeek's sparse attention on every layer ([`Dsa`]): DeepSeek-V3.2's
            // index rotation NEOX and every layer's indexer its own, `expert_gating_func` required; GLM-5's index rotation
            // interleaved, its experts scored by sigmoids where the file names no gating, and its layers' indexers full
            // or shared as `attention.indexer.types` says — where it says nothing, all full below a trained context of
            // 1,048,576 (GLM-5 and 5.1) and GLM-5.2's pattern from it. The latent norms at 10⁻⁶, as the models' own code
            // builds them, where these graphs norm them at the model's epsilon ([`MLA_LATENT_NORM_EPS`]).
            "deepseek2" | "deepseek32" | "glm-dsa" => {
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
                    rope: Some(rope),
                    scale: mscale * mscale / (key as f64).sqrt(),
                    kv_b_split: g.has("blk.0.attn_k_b.weight"),
                    indexer: None,
                    latent_norm_eps: MLA_LATENT_NORM_EPS,
                };
                let sparse = arch != "deepseek2";
                if sparse {
                    if mla.q_lora_rank.is_none() {
                        return Err(SpecError::MissingKey(k.key("attention.q_lora_rank")));
                    }
                    if !mla.kv_b_split {
                        return Err(SpecError::MissingTensor("blk.0.attn_k_b.weight".into()));
                    }
                }
                let full: Vec<bool> = match (arch.as_str(), k.get("attention.indexer.types").and_then(MetaValue::as_array)) {
                    ("glm-dsa", Some(a)) => a
                        .iter()
                        .take(n_layers)
                        .map(|v| v.as_bool().or_else(|| v.as_u64().map(|x| x != 0)))
                        .collect::<Option<_>>()
                        .ok_or_else(|| SpecError::MissingKey(k.key("attention.indexer.types")))?,
                    ("glm-dsa", None) if k.usize_or("context_length", 0) >= 1_048_576 => (0..n_layers).map(glm52_indexer_full).collect(),
                    _ => vec![true; n_layers],
                };
                if sparse && full.first() != Some(&true) {
                    return Err(SpecError::Inconsistent("a shared indexer layer before any full one, whose choice it would take".into()));
                }
                let dsa = |l: usize| -> Result<Dsa, SpecError> {
                    let dsa = Dsa {
                        heads: k.usize("attention.indexer.head_count")?,
                        head_dim: k.usize("attention.indexer.key_length")?,
                        top_k: k.usize("attention.indexer.top_k")?,
                        neox: arch == "deepseek32",
                        full: full.get(l).copied().unwrap_or(true),
                    };
                    if dsa.head_dim < rope_dims || dsa.heads == 0 || dsa.top_k == 0 {
                        return Err(SpecError::Inconsistent(format!(
                            "an indexer of {} heads of {} choosing {} positions under a rotation of {rope_dims}",
                            dsa.heads, dsa.head_dim, dsa.top_k
                        )));
                    }
                    Ok(dsa)
                };
                if arch == "deepseek32" {
                    k.usize("expert_gating_func")?;
                }
                let moe = routed(if arch == "glm-dsa" { 2 } else { 1 })?;
                let dense_lead = k.usize_or("leading_dense_block_count", 0);
                let dense_ffn = dense(Act::Silu)?;
                (0..n_layers)
                    .map(|l| {
                        Ok(Layer {
                            mixer: Mixer::Mla(Mla { indexer: if sparse { Some(dsa(l)?) } else { None }, ..mla.clone() }),
                            ffn: if l < dense_lead { dense_ffn.clone() } else { moe.clone() },
                            pre_norms: true,
                            post_mixer_norm: false,
                            post_ffn_norm: false,
                        })
                    })
                    .collect::<Result<_, SpecError>>()?
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
                    temperature: chunked.then_some(Temperature { floor: 8192.0, scale: 0.1, offset: 1.0 }),
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
                    latent: None,
                });
                let dense_ffn = dense(Act::Silu)?;
                (0..n_layers)
                    .map(|l| Layer {
                        mixer: Mixer::Attention(if (l + 1) % no_rope_step == 0 { full.clone() } else { rotated.clone() }),
                        ffn: if step > 0 && (l + 1) % step == 0 { moe.clone() } else { dense_ffn.clone() },
                        pre_norms: true,
                        post_mixer_norm: false,
                        post_ffn_norm: false,
                    })
                    .collect()
            }
            other => return Err(SpecError::UnknownArchitecture(other.to_string())),
        };
        let ffn_norm = match arch.as_str() {
            "qwen35" | "qwen35moe" | "qwen3next" | "gpt-oss" => "post_attention_norm",
            "rwkv7" => "attn_norm_2",
            _ => "ffn_norm",
        };
        let (hyper, ple, indexer) = if arch == "qwen4exp" { qwen4exp_parts(&k, &layers)? } else { (None, None, None) };
        // Granite's and MiniCPM's multipliers (`embedding_scale`, `residual_scale`, `logit_scale`): a zero or absent
        // Granite multiplier applies none and its logit scale is required; MiniCPM's default to `models/minicpm.cpp`'s
        // 12, 1.4/√layers and 256/d. Granite divides the logits after the head; MiniCPM's graph divides the output
        // norm's result before it.
        let nonzero = |key: &str| k.get(key).and_then(decimal).filter(|&v| v != 0.0);
        let embedding_scale = match arch.as_str() {
            "gemma" | "gemma2" | "gemma3" | "gemma3n" | "gemma4" => Some((d as f64).sqrt()),
            "granite" => nonzero("embedding_scale"),
            "minicpm" => Some(k.f64_or("embedding_scale", 12.0)),
            _ => None,
        };
        let (residual_scale, logit_scale) = match arch.as_str() {
            "granite" => (nonzero("residual_scale"), Some(LogitScale::AfterHead(1.0 / k.f64("logit_scale")?))),
            "minicpm" => (
                Some(k.f64_or("residual_scale", 1.4 / (n_layers as f64).sqrt())),
                Some(LogitScale::BeforeHead(1.0 / k.f64_or("logit_scale", 256.0 / d as f64))),
            ),
            _ => (None, None),
        };
        // Gemma 2's graph caps its logits whatever the file says, at 30 where it says nothing.
        let final_logit_softcap = match k.get("final_logit_softcapping").and_then(decimal) {
            None if matches!(arch.as_str(), "gemma2" | "gemma3n") => Some(30.0),
            cap => cap,
        };
        let norm = match arch.as_str() {
            "phimoe" => Norm::Layer { bias: g.has("blk.0.attn_norm.bias") },
            "rwkv7" => Norm::Layer { bias: true },
            _ => Norm::Rms,
        };
        // phi3's graph adds a head bias where the checkpoint carries one, and PhiMoE's head has one.
        let head_bias = matches!(arch.as_str(), "phi3" | "phimoe") && g.has("output.bias");
        let rms_eps = match k.get("attention.layer_norm_rms_epsilon").and_then(decimal) {
            Some(eps) => eps,
            // Nemotron-H's converter writes the epsilon as a LayerNorm's, which llama.cpp's RMSNorms take.
            None if matches!(arch.as_str(), "nemotron_h" | "nemotron_h_moe") => k.f64_or("attention.layer_norm_epsilon", 1e-6),
            // RWKV-7's LayerNorms take the converter's `layer_norm_epsilon`, which its loader reads with no default.
            None if arch == "rwkv7" => k.f64("attention.layer_norm_epsilon")?,
            None => 1e-6,
        };
        Ok(ModelSpec {
            arch,
            d,
            vocab,
            ctx_len: k.usize_or("context_length", 0),
            rms_eps,
            embedding_scale,
            tied_output: !g.has("output.weight"),
            final_logit_softcap,
            layers,
            mtp_blocks,
            ffn_norm,
            hyper,
            ple,
            indexer,
            residual_scale,
            logit_scale,
            norm,
            head_bias,
            parts,
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

/// Gemma 3n's numbers as llama.cpp fixes them (`llama_hparams`, `llama_model_gemma3n`): four AltUp streams, the first
/// active, LAuReL of rank 64, per-layer inputs of 256, the cache shared from block 20, and the first ten blocks' gates
/// sparsified at the standard normal's 95th percentile (`f_sparsity_std_mul`, which the converter's own value meets to
/// within its last places).
const GEMMA3N_ALTUP_STREAMS: usize = 4;
const GEMMA3N_ALTUP_ACTIVE: usize = 0;
const GEMMA3N_LAUREL_RANK: usize = 64;
const GEMMA3N_PER_LAYER_INPUT: usize = 256;
const GEMMA3N_KV_FROM: usize = 20;
const GEMMA3N_SPARSE_LAYERS: usize = 10;
const GEMMA3N_SPARSITY_STD_MUL: f32 = 1.644_853_4;

/// Whether GLM-5.2's layer `l` runs its own indexer where its file names no indexer types (llama.cpp's
/// `GLM_5_2_DEFAULT_INDEXER_TYPES`): the first three of its 78 layers, then every fourth from layer 6.
fn glm52_indexer_full(l: usize) -> bool {
    l < 3 || (6..78).contains(&l) && (l - 2) % 4 == 0
}

/// Mamba-2's scan as llama.cpp reads it (`build_mamba2_layer`): `ssm.time_step_rank` heads of
/// `ssm.inner_size / heads`, `B` and `C` of `ssm.state_size` in `ssm.group_count` groups, a convolution of
/// `ssm.conv_kernel` taps, and the grouped gated norm where block `at` holds `ssm_norm`. A head reads its group's `B` and
/// `C` as `h / (heads / groups)`, which is transformers' `repeat_interleave` only where the groups split the heads.
fn ssd<S: SpecSource + ?Sized>(k: &Keys<S>, g: &S, at: usize) -> Result<Ssd, SpecError> {
    let inner = k.usize("ssm.inner_size")?;
    let heads = k.usize("ssm.time_step_rank")?;
    let groups = k.usize_or("ssm.group_count", 1);
    let scan = Ssd {
        heads,
        head_dim: inner.checked_div(heads).unwrap_or(0),
        state: k.usize("ssm.state_size")?,
        groups,
        conv_taps: k.usize("ssm.conv_kernel")?,
        gated_norm: g.has(&format!("blk.{at}.ssm_norm.weight")),
    };
    if heads == 0 || groups == 0 || inner % heads != 0 || heads % groups != 0 || scan.state == 0 || scan.conv_taps == 0 {
        return Err(SpecError::Inconsistent(format!(
            "a scan of {inner} channels in {heads} heads and {groups} groups, a state of {} and a convolution of {} taps",
            scan.state, scan.conv_taps
        )));
    }
    Ok(scan)
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
            // The DeepSeek-family loaders read the stored multiplier as the tenth the converter writes and keep ten times
            // it ([TAG_DEEPSEEK2_YARN_LOG_MUL_FIX]); llama-context takes it as `mscale_all_dim` with `mscale` 1, but for
            // DeepSeek-V2, whose `mscale` it takes as the same multiplier when that is not 1.
            let stored = k.f64_or("rope.scaling.yarn_log_multiplier", 0.0);
            let log_mul = if matches!(k.g.arch(), "deepseek2" | "deepseek32" | "glm-dsa") { stored / 0.1 } else { stored };
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
