//! Small models of every architecture the spec reads, written as real GGUF checkpoints: the projections and tables
//! in Q8_0, the norms and the recurrence's small tensors in F32, every weight exact in both (a Q8_0 block's scale is
//! 2⁻⁷ and its values whole multiples of it), so a lane reading the file and the `f64` reference reading it see the
//! same numbers. Widths are multiples of 32, as Q8_0 rows need.

#![allow(dead_code)]

use std::path::Path;

use logicaffeine_compile::model::gguf::{self, MetaValue, TensorOut};
use logicaffeine_compile::model::quant::GgmlType;
use logicaffeine_compile::model::spec::{
    Act, Attention, Ffn, GateActivation, GatedDelta, Gating, Layer, LogitScale, Mixer, Mla, ModelSpec, Moe, Norm, Part,
    Recurrent, Rope, RopeScaling, Window, MLA_LATENT_NORM_EPS,
};

/// The architectures a tiny model comes in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tiny {
    /// Qwen3: QK-norm, a NEOX rotation, a dense SiLU FFN.
    Qwen3,
    /// Llama: an interleaved rotation, no QK-norm.
    Llama,
    /// Llama with the output head tied to the embedding.
    LlamaTied,
    /// Qwen3-MoE: a softmax top-2 of 4 experts.
    Qwen3Moe,
    /// Qwen3.5's hybrid: two Gated DeltaNet blocks, then gated attention with a partial multi-section rotation.
    Qwen35,
    /// Gemma 3: two sliding-window layers turning by their own base, then a global one; a norm after the mixer and
    /// after the FFN as well as before each; a GeLU-gated FFN; the embedding scaled by `√d`; the head tied.
    Gemma3,
    /// gpt-oss: windowed and global layers alternating, attention sinks, biased projections, a YaRN rotation; a mixture
    /// with an `f32` router, a bias on the router and on every expert projection, and the clamped SwiGLU.
    GptOss,
    /// DeepSeek-V2 (Lite): multi-head latent attention — a full query projection, the latent and the shared rotated key
    /// from one projection, the latent's expansion stored as one `attn_kv_b` — under an interleaved YaRN rotation and a
    /// given score scale; a dense first layer, then a mixture weighted by the softmax over every expert, not
    /// renormalized, with a shared expert.
    DeepSeek2,
    /// DeepSeek-V2 as a newer conversion stores it: a low-rank query normed between its halves, and the latent's
    /// expansion split into its key half (`attn_k_b`) and its value half (`attn_v_b`).
    DeepSeek2Split,
    /// Granite: Llama's shape with its multipliers — the embedding times 4, every score times 1/8, each sublayer's output
    /// times 1/4 before the residual, the logits times 1/8 after the head.
    Granite,
    /// MiniCPM: Llama's shape, each sublayer's output times 1/2 before the residual, the output norm's result times 1/4
    /// (llama.cpp's 256/d at this width) before the head.
    MiniCpm,
    /// Arcee: Llama's shape with an FFN of relu² of the up projection and no gate projection.
    Arcee,
    /// Gemma 2: Gemma 3's sandwich norms, GeLU and tied head under one rotation base, a window on the even layers, every
    /// score soft-capped at 50 and the logits at 30.
    Gemma2,
    /// OLMo 2: no norm before either sublayer, each reading the residual stream itself, and a norm after each; the queries
    /// and the keys each normed across every head at once; Qwen3's NEOX rotation and SiLU FFN; a KV head a query head.
    Olmo2,
}

/// Every block format a checkpoint's matrices may come in besides Q8_0: the K-quants, the legacy formats, MXFP4 and the
/// IQ4 formats — the formats each lane lock runs, in the order a checkpoint mixing them takes them by turns.
pub const BLOCK_FORMATS: [GgmlType; 12] = [
    GgmlType::Q4K,
    GgmlType::Q5_0,
    GgmlType::Q6K,
    GgmlType::Mxfp4,
    GgmlType::Q2K,
    GgmlType::Q4_0,
    GgmlType::Q3K,
    GgmlType::Q5K,
    GgmlType::Q4_1,
    GgmlType::Q5_1,
    GgmlType::Iq4Nl,
    GgmlType::Iq4Xs,
];

pub const ALL: &[Tiny] = &[
    Tiny::Qwen3,
    Tiny::Llama,
    Tiny::LlamaTied,
    Tiny::Qwen3Moe,
    Tiny::Qwen35,
    Tiny::Gemma3,
    Tiny::GptOss,
    Tiny::DeepSeek2,
    Tiny::DeepSeek2Split,
    Tiny::MiniCpm,
    Tiny::Arcee,
    Tiny::Granite,
    Tiny::Gemma2,
];

/// The window of a tiny Gemma's local layers: shorter than every run the locks make (prompt and continuation), so the
/// window binds within each.
pub const WINDOW: usize = 4;

pub const D: usize = 64;
pub const VOCAB: usize = 96;

impl Tiny {
    pub fn name(self) -> &'static str {
        match self {
            Tiny::Qwen3 => "qwen3",
            Tiny::Llama => "llama",
            Tiny::LlamaTied => "llama-tied",
            Tiny::Qwen3Moe => "qwen3moe",
            Tiny::Qwen35 => "qwen35",
            Tiny::Gemma3 => "gemma3",
            Tiny::GptOss => "gpt-oss",
            Tiny::DeepSeek2 => "deepseek2",
            Tiny::DeepSeek2Split => "deepseek2-split",
            Tiny::Granite => "granite",
            Tiny::MiniCpm => "minicpm",
            Tiny::Arcee => "arcee",
            Tiny::Gemma2 => "gemma2",
            Tiny::Olmo2 => "olmo2",
        }
    }

    /// The architecture at `D` wide.
    pub fn spec(self, ctx: usize) -> ModelSpec {
        self.sized(ctx, 1)
    }

    /// The architecture at four times `D` wide, every matrix row a whole number of 256-weight super-blocks — the size
    /// a checkpoint in a K-quant format needs.
    pub fn wide(self, ctx: usize) -> ModelSpec {
        self.sized(ctx, 4)
    }

    /// The architecture at 2048 wide, every matrix row a whole number of super-blocks: a dense FFN of 2048 and experts
    /// of 1024, and a latent attention with DeepSeek-V2-Lite's own heads (16 of 128 unrotated and 64 rotated, a latent
    /// of 512, values of 128) — each FFN matrix, the latent's query projection and the latent output projection the
    /// shape a GEMV takes four rows a warp at.
    pub fn medium(self, ctx: usize) -> ModelSpec {
        let mut spec = self.wide(ctx);
        spec.d = 2048;
        for layer in &mut spec.layers {
            match &mut layer.ffn {
                Ffn::Dense { dff, .. } | Ffn::ChannelMix { dff } => *dff = 2048,
                Ffn::Moe(m) => {
                    m.expert_dff = 1024;
                    m.shared_dff = 1024 * m.shared_experts;
                }
                Ffn::DenseAndMoe { dff, moe, .. } => {
                    *dff = 2048;
                    moe.expert_dff = 1024;
                    moe.shared_dff = 1024 * moe.shared_experts;
                }
                Ffn::None => {}
            }
            if let Mixer::Mla(m) = &mut layer.mixer {
                (m.heads, m.qk_nope, m.qk_rope, m.v_dim, m.kv_lora_rank) = (16, 128, 64, 128, 512);
                if let Some(r) = &mut m.rope {
                    r.dims = 64;
                }
                let mscale = 1.0 + 0.0707 * 40f64.ln();
                m.scale = mscale * mscale / ((m.qk_nope + m.qk_rope) as f64).sqrt();
            }
        }
        spec
    }

    fn sized(self, ctx: usize, w: usize) -> ModelSpec {
        let neox = !matches!(self, Tiny::Llama | Tiny::LlamaTied | Tiny::Granite | Tiny::MiniCpm | Tiny::Arcee);
        let (d, hd) = (D * w, 16 * w);
        let (dff, expert_dff) = if w == 1 { (96, 32) } else { (256, 256) };
        let attention = Attention {
            heads: 4,
            kv_heads: 2,
            head_dim: hd,
            value_dim: hd,
            rope: Some(Rope { dims: hd, theta: 10000.0, neox, scaling: RopeScaling::None, sections: None }),
            qk_norm: neox,
            output_gate: false,
            fused_qkv: false,
            bias: false,
            output_bias: false,
            window: None,
            sinks: false,
            softcap: None,
            scale: 1.0 / (hd as f64).sqrt(),
            temperature: None,
            qk_unit_norm: false,
            qk_norm_whole: false,
        };
        let dense = Ffn::Dense { dff, act: Act::Silu };
        let moe = Ffn::Moe(Moe {
            experts: 4,
            top_k: 2,
            expert_dff,
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
        let gated = Attention {
            heads: 2,
            kv_heads: 1,
            head_dim: 32 * w,
            value_dim: 32 * w,
            rope: Some(Rope {
                dims: 24 * w,
                theta: 10000.0,
                neox: true,
                scaling: RopeScaling::None,
                sections: Some([4 * w, 4 * w, 0, 4 * w]),
            }),
            qk_norm: true,
            output_gate: true,
            fused_qkv: false,
            bias: false,
            output_bias: false,
            window: None,
            sinks: false,
            softcap: None,
            scale: 1.0 / ((32 * w) as f64).sqrt(),
            temperature: None,
            qk_unit_norm: false,
            qk_norm_whole: false,
        };
        let gdn = GatedDelta { k_heads: 2, v_heads: 4, head_dim: 32 * w, conv_taps: 4, gate: GateActivation::Silu };
        let global = Attention {
            rope: Some(Rope { dims: hd, theta: 1_000_000.0, neox: true, scaling: RopeScaling::None, sections: None }),
            ..attention.clone()
        };
        let local = Attention { window: Some(Window::Sliding(WINDOW)), ..attention.clone() };
        let gemma = matches!(self, Tiny::Gemma3 | Tiny::Gemma2);
        let olmo = self == Tiny::Olmo2;
        let whole = Attention { kv_heads: 4, qk_norm: true, qk_norm_whole: true, ..attention.clone() };
        let capped = Attention { softcap: Some(50.0), qk_norm: false, ..attention.clone() };
        let oss = Attention {
            rope: Some(Rope {
                dims: hd,
                theta: 150_000.0,
                neox: true,
                scaling: RopeScaling::Yarn { factor: 32.0, original_ctx: 16, beta_fast: 32.0, beta_slow: 1.0, magnitude: 1.25 },
                sections: None,
            }),
            qk_norm: false,
            bias: true,
            output_bias: true,
            sinks: true,
            ..attention.clone()
        };
        let oss_moe = Ffn::Moe(Moe {
            experts: 4,
            top_k: 2,
            expert_dff,
            act: Act::SwigluOai { alpha: 1.702, limit: 7.0 },
            gating: Gating::SoftmaxOfTopK,
            norm_top_k: false,
            weights_scale: 1.0,
            shared_experts: 0,
            shared_dff: 0,
            shared_gate: false,
            biases: true,
            groups: None,
            latent: None,
        });
        // DeepSeek-V2's YaRN: the cosines and sines unscaled, the scores by the magnitude `1 + 0.1·m·ln(factor)` squared.
        let deepseek = matches!(self, Tiny::DeepSeek2 | Tiny::DeepSeek2Split);
        let (nope, rope) = (64 * w, 8 * w);
        let mscale = 1.0 + 0.0707 * 40f64.ln();
        let mla = Mla {
            heads: 4,
            q_lora_rank: (self == Tiny::DeepSeek2Split).then_some(64 * w),
            kv_lora_rank: 64 * w,
            qk_nope: nope,
            qk_rope: rope,
            v_dim: 32 * w,
            rope: Some(Rope {
                dims: rope,
                theta: 10000.0,
                neox: false,
                scaling: RopeScaling::Yarn { factor: 40.0, original_ctx: 16, beta_fast: 32.0, beta_slow: 1.0, magnitude: 1.0 },
                sections: None,
            }),
            scale: mscale * mscale / ((nope + rope) as f64).sqrt(),
            kv_b_split: self == Tiny::DeepSeek2Split,
            indexer: None,
            latent_norm_eps: MLA_LATENT_NORM_EPS,
        };
        let deepseek_moe = Ffn::Moe(Moe {
            experts: 4,
            top_k: 2,
            expert_dff,
            act: Act::Silu,
            gating: Gating::Softmax,
            norm_top_k: false,
            weights_scale: 1.0,
            shared_experts: 1,
            shared_dff: expert_dff,
            shared_gate: false,
            biases: false,
            groups: None,
            latent: None,
        });
        let layers: Vec<Layer> = (0..3)
            .map(|l| {
                let mixer = match self {
                    Tiny::DeepSeek2 | Tiny::DeepSeek2Split => Mixer::Mla(mla.clone()),
                    Tiny::Qwen35 if l < 2 => Mixer::GatedDelta(gdn.clone()),
                    Tiny::Qwen35 => Mixer::Attention(gated.clone()),
                    Tiny::Gemma3 if l < 2 => Mixer::Attention(local.clone()),
                    Tiny::Gemma3 => Mixer::Attention(global.clone()),
                    Tiny::GptOss => Mixer::Attention(Attention { window: (l % 2 == 0).then_some(Window::Sliding(WINDOW)), ..oss.clone() }),
                    Tiny::Gemma2 => Mixer::Attention(Attention { window: (l % 2 == 0).then_some(Window::Sliding(WINDOW)), ..capped.clone() }),
                    Tiny::Granite => Mixer::Attention(Attention { scale: 0.125, ..attention.clone() }),
                    Tiny::Olmo2 => Mixer::Attention(whole.clone()),
                    _ => Mixer::Attention(attention.clone()),
                };
                let ffn = match self {
                    _ if deepseek && l == 0 => dense.clone(),
                    _ if deepseek => deepseek_moe.clone(),
                    Tiny::Qwen3Moe => moe.clone(),
                    Tiny::GptOss => oss_moe.clone(),
                    Tiny::Gemma3 | Tiny::Gemma2 => Ffn::Dense { dff, act: Act::GeluTanh },
                    Tiny::Arcee => Ffn::Dense { dff, act: Act::ReluSquared },
                    _ => dense.clone(),
                };
                Layer { mixer, ffn, pre_norms: !olmo, post_mixer_norm: gemma || olmo, post_ffn_norm: gemma || olmo }
            })
            .collect();
        ModelSpec {
            arch: match self {
                Tiny::Qwen3 => "qwen3",
                Tiny::Llama | Tiny::LlamaTied => "llama",
                Tiny::Qwen3Moe => "qwen3moe",
                Tiny::Qwen35 => "qwen35",
                Tiny::Gemma3 => "gemma3",
                Tiny::GptOss => "gpt-oss",
                Tiny::DeepSeek2 | Tiny::DeepSeek2Split => "deepseek2",
                Tiny::Granite => "granite",
                Tiny::MiniCpm => "minicpm",
                Tiny::Arcee => "arcee",
                Tiny::Gemma2 => "gemma2",
                Tiny::Olmo2 => "olmo2",
            }
            .to_string(),
            d,
            vocab: VOCAB,
            ctx_len: ctx,
            rms_eps: 1e-6,
            embedding_scale: match self {
                _ if gemma => Some((d as f64).sqrt()),
                Tiny::Granite => Some(4.0),
                Tiny::MiniCpm => Some(8.0),
                _ => None,
            },
            tied_output: matches!(self, Tiny::LlamaTied | Tiny::Gemma3 | Tiny::Gemma2),
            final_logit_softcap: (self == Tiny::Gemma2).then_some(30.0),
            layers,
            mtp_blocks: 0,
            ffn_norm: if matches!(self, Tiny::Qwen35 | Tiny::GptOss) { "post_attention_norm" } else { "ffn_norm" },
            hyper: None,
            ple: None,
            indexer: None,
            residual_scale: match self {
                Tiny::Granite => Some(0.25),
                Tiny::MiniCpm => Some(0.5),
                _ => None,
            },
            logit_scale: match self {
                Tiny::Granite => Some(LogitScale::AfterHead(0.125)),
                Tiny::MiniCpm => Some(LogitScale::BeforeHead(0.25)),
                _ => None,
            },
            norm: Norm::Rms,
            head_bias: false,
            parts: Vec::new(),
        }
    }
}

/// A seeded integer in `[-63, 63]` for element `i` of tensor `name`.
fn level(name: &str, i: usize) -> i32 {
    let salt = name.bytes().fold(0u64, |h, b| h.wrapping_mul(131).wrapping_add(b as u64));
    let h = (i as u64).wrapping_mul(2_654_435_761).wrapping_add(salt.wrapping_mul(40_503)) ^ (salt >> 7);
    (h % 127) as i32 - 63
}

/// The f16 bits of 2⁻⁷, the scale of every Q8_0 block.
const SCALE_BITS: u16 = 0x2000;

/// Is `name` a projection or a table the author declares in Q8_0? (Every bias and norm stays `f32`.)
fn quantized(name: &str) -> bool {
    // A vector stays f32, as the converters keep every one-dimensional tensor.
    name.ends_with(".weight") && !name.contains("ffn_gate_inp_shexp") && [
        "token_embd", "output.weight", "attn_q.", "attn_k.", "attn_v.", "attn_output", "attn_qkv", "attn_gate",
        "ssm_out", "ffn_gate.", "ffn_up.", "ffn_down.", "ffn_gate_inp", "_exps", "attn_q_a.", "attn_q_b.",
        "attn_kv_a_mqa", "attn_kv_b", "attn_k_b", "attn_v_b", "_shexp",
    ]
    .iter()
    .any(|p| name.contains(p))
}

/// `spec` with every rotation scaled by the checkpoint's per-frequency factors (`rope_freqs.weight`), as Llama 3's is.
pub fn with_rope_factors(mut spec: ModelSpec) -> ModelSpec {
    for layer in &mut spec.layers {
        if let Mixer::Attention(a) = &mut layer.mixer {
            if let Some(rope) = &mut a.rope {
                rope.scaling = RopeScaling::Factors;
            }
        }
    }
    spec
}

/// Every tensor a checkpoint of `spec` holds, in GGUF order (`dims[0]` the row).
pub fn tensors(spec: &ModelSpec) -> Vec<(String, Vec<u64>)> {
    let mut t: Vec<(String, Vec<u64>)> = Vec::new();
    let mut add = |name: String, dims: &[u64]| t.push((name, dims.to_vec()));
    let d = spec.d as u64;
    add("token_embd.weight".into(), &[d, spec.vocab as u64]);
    add("output_norm.weight".into(), &[d]);
    // A biased LayerNorm's bias beside every block norm's weight (PhiMoE's).
    let norm_bias = spec.norm == (Norm::Layer { bias: true });
    if norm_bias {
        add("output_norm.bias".into(), &[d]);
    }
    if !spec.tied_output {
        add("output.weight".into(), &[d, spec.vocab as u64]);
    }
    if spec.head_bias {
        add("output.bias".into(), &[spec.vocab as u64]);
    }
    // One table of per-frequency factors serves every rotation that reads one, as Llama 3's checkpoints store it.
    let factored = spec.layers.iter().find_map(|l| match &l.mixer {
        Mixer::Attention(a) => a.rope.as_ref().filter(|r| r.scaling == RopeScaling::Factors).map(|r| r.dims),
        _ => None,
    });
    if let Some(dims) = factored {
        add("rope_freqs.weight".into(), &[dims as u64 / 2]);
    }
    // Gemma 4's layers past the shared point hold no keys or values, and its `attention_k_eq_v` layers no values.
    let shared_from = spec.parts.iter().find_map(|p| if let Part::SharedKv { from } = p { Some(*from) } else { None });
    let keyed: Vec<usize> = spec.parts.iter().find_map(|p| if let Part::KeyAsValue { layers } = p { Some(layers.clone()) } else { None }).unwrap_or_default();
    for (l, layer) in spec.layers.iter().enumerate() {
        let blk = |s: &str| format!("blk.{l}.{s}.weight");
        let (has_kv, has_v) = (shared_from.is_none_or(|f| l < f), !keyed.contains(&l));
        if layer.pre_norms {
            add(blk("attn_norm"), &[d]);
            if norm_bias {
                add(format!("blk.{l}.attn_norm.bias"), &[d]);
            }
        }
        match &layer.mixer {
            Mixer::Attention(a) => {
                let (aq, akv, hd) = ((a.heads * a.head_dim) as u64, (a.kv_heads * a.head_dim) as u64, a.head_dim as u64);
                let (avd, ao) = ((a.kv_heads * a.value_dim) as u64, (a.heads * a.value_dim) as u64);
                let bias = |s: &str| format!("blk.{l}.{s}.bias");
                // A fused projection holds each output row's queries, keys and values one after another, its bias likewise.
                if a.fused_qkv {
                    add(blk("attn_qkv"), &[d, aq + akv + avd]);
                    add(blk("attn_output"), &[ao, d]);
                    if a.bias {
                        add(bias("attn_qkv"), &[aq + akv + avd]);
                    }
                } else {
                    add(blk("attn_q"), &[d, if a.output_gate { 2 * aq } else { aq }]);
                    if has_kv {
                        add(blk("attn_k"), &[d, akv]);
                    }
                    if has_kv && has_v {
                        add(blk("attn_v"), &[d, avd]);
                    }
                    add(blk("attn_output"), &[ao, d]);
                    if a.bias {
                        add(bias("attn_q"), &[aq]);
                        add(bias("attn_k"), &[akv]);
                        add(bias("attn_v"), &[avd]);
                    }
                }
                if a.output_bias {
                    add(bias("attn_output"), &[d]);
                }
                if a.sinks {
                    add(blk("attn_sinks"), &[a.heads as u64]);
                }
                if a.qk_norm {
                    let (qw, kw) = if a.qk_norm_whole { (aq, akv) } else { (hd, hd) };
                    add(blk("attn_q_norm"), &[qw]);
                    if has_kv {
                        add(blk("attn_k_norm"), &[kw]);
                    }
                }
            }
            Mixer::GatedDelta(g) => {
                let (qk, vw, nv) = ((g.k_heads * g.head_dim) as u64, (g.v_heads * g.head_dim) as u64, g.v_heads as u64);
                add(blk("attn_qkv"), &[d, 2 * qk + vw]);
                add(blk("attn_gate"), &[d, vw]);
                add(blk("ssm_conv1d"), &[g.conv_taps as u64, 2 * qk + vw]);
                add(format!("blk.{l}.ssm_dt.bias"), &[nv]);
                add(blk("ssm_beta"), &[d, nv]);
                add(blk("ssm_alpha"), &[d, nv]);
                add(blk("ssm_norm"), &[g.head_dim as u64]);
                add(blk("ssm_out"), &[vw, d]);
                add(format!("blk.{l}.ssm_a"), &[nv]);
            }
            Mixer::Mla(m) => {
                let (h, qk, kw) = (m.heads as u64, (m.qk_nope + m.qk_rope) as u64, (m.kv_lora_rank + m.qk_rope) as u64);
                let (nope, lora, vd) = (m.qk_nope as u64, m.kv_lora_rank as u64, m.v_dim as u64);
                match m.q_lora_rank {
                    None => add(blk("attn_q"), &[d, h * qk]),
                    Some(r) => {
                        add(blk("attn_q_a"), &[d, r as u64]);
                        add(blk("attn_q_a_norm"), &[r as u64]);
                        add(blk("attn_q_b"), &[r as u64, h * qk]);
                    }
                }
                add(blk("attn_kv_a_mqa"), &[d, kw]);
                add(blk("attn_kv_a_norm"), &[lora]);
                if m.kv_b_split {
                    add(blk("attn_k_b"), &[nope, lora, h]);
                    add(blk("attn_v_b"), &[lora, vd, h]);
                } else {
                    add(blk("attn_kv_b"), &[lora, h * (nope + vd)]);
                }
                add(blk("attn_output"), &[h * vd, d]);
                if let (Some(ix), Some(r)) = (m.indexer.as_ref().filter(|ix| ix.full), m.q_lora_rank) {
                    let (ih, ihd) = (ix.heads as u64, ix.head_dim as u64);
                    add(blk("indexer.attn_q_b"), &[r as u64, ih * ihd]);
                    add(blk("indexer.attn_k"), &[d, ihd]);
                    add(blk("indexer.k_norm"), &[ihd]);
                    add(format!("blk.{l}.indexer.k_norm.bias"), &[ihd]);
                    add(blk("indexer.proj"), &[d, ih]);
                }
            }
            Mixer::Recurrent(r) => {
                for (name, dims) in r.tensors(spec.d) {
                    add(format!("blk.{l}.{name}"), &dims);
                }
            }
            Mixer::None => {}
        }
        if layer.post_mixer_norm {
            add(blk("post_attention_norm"), &[d]);
        }
        // A block with no mixer norms its FFN by its one norm; one with no FFN has no FFN norm.
        if layer.pre_norms && layer.mixer != Mixer::None && layer.ffn != Ffn::None {
            add(blk(spec.ffn_norm), &[d]);
            if norm_bias {
                add(format!("blk.{l}.{}.bias", spec.ffn_norm), &[d]);
            }
        }
        for (name, dims) in layer.ffn.tensors(spec.d) {
            add(format!("blk.{l}.{name}"), &dims);
        }
        if layer.post_ffn_norm {
            add(blk("post_ffw_norm"), &[d]);
        }
    }
    for part in &spec.parts {
        for (name, dims) in part.tensors(spec) {
            add(name, &dims);
        }
    }
    t
}

/// The value of element `i` of tensor `name`: a projection's `level / 128`; a norm weight `1 + level / 256`; the
/// recurrence's rate negative, so its memory decays; a frequency factor `1 + |level| / 8`, from 1 to about 9 as Llama
/// 3's run from 1 to 8.
fn value(name: &str, i: usize) -> f32 {
    let v = level(name, i) as f32;
    if name == "rope_freqs.weight" {
        1.0 + (v / 8.0).abs()
    } else if name.ends_with("norm.weight") {
        1.0 + v / 256.0
    } else if name.ends_with(".ssm_a") {
        -(v / 128.0).abs() - 0.25
    } else {
        v / 128.0
    }
}

/// Write the checkpoint of `spec` to `path`: projections and tables in Q8_0, norms, biases and the recurrence's small
/// tensors in `f32`, every weight exact in both.
pub fn write_checkpoint(spec: &ModelSpec, path: &Path) {
    // gpt-oss keeps its router in `f32`, as its checkpoints do.
    write_checkpoint_as(spec, path, &|name| {
        if quantized(name) && !(spec.arch == "gpt-oss" && name.contains("ffn_gate_inp")) {
            GgmlType::Q8_0
        } else {
            GgmlType::F32
        }
    });
}

/// The metadata a converter writes for `spec`: every key `ModelSpec::from_gguf` reads, so the checkpoint reads back
/// as `spec` — the tensors carrying what the reader takes from their presence (QK-norm, biases, a head of its own).
pub fn checkpoint_meta(spec: &ModelSpec) -> Vec<(String, MetaValue)> {
    let arch = spec.arch.as_str();
    let key = |s: &str| format!("{arch}.{s}");
    let int = |v: usize| MetaValue::U64(v as u64);
    let real = MetaValue::F64;
    let array = |ty: u32, items: Vec<MetaValue>| MetaValue::Array { ty, items };
    const U32: u32 = 4;
    const BOOL: u32 = 7;
    let mut m = vec![
        ("general.architecture".to_string(), MetaValue::Str(arch.as_bytes().to_vec())),
        (key("embedding_length"), int(spec.d)),
        (key("block_count"), int(spec.layers.len() + spec.mtp_blocks)),
        (key("context_length"), int(spec.ctx_len)),
        (key("attention.layer_norm_rms_epsilon"), real(spec.rms_eps)),
    ];
    if spec.mtp_blocks > 0 {
        m.push((key("nextn_predict_layers"), int(spec.mtp_blocks)));
    }
    if let Some(cap) = spec.final_logit_softcap {
        m.push((key("final_logit_softcapping"), real(cap)));
    }
    if let Some(cap) = spec.layers.iter().find_map(|l| match &l.mixer {
        Mixer::Attention(a) => a.softcap,
        _ => None,
    }) {
        m.push((key("attn_logit_softcapping"), real(cap)));
    }
    // Granite's and MiniCPM's multipliers as their converters write them: the logit scale is the divisor.
    if matches!(arch, "granite" | "minicpm") {
        if let Some(e) = spec.embedding_scale {
            m.push((key("embedding_scale"), real(e)));
        }
        if let Some(r) = spec.residual_scale {
            m.push((key("residual_scale"), real(r)));
        }
        if let Some(LogitScale::AfterHead(f) | LogitScale::BeforeHead(f)) = spec.logit_scale {
            m.push((key("logit_scale"), real(1.0 / f)));
        }
        if let (true, Some(Mixer::Attention(a))) = (arch == "granite", spec.layers.first().map(|l| &l.mixer)) {
            m.push((key("attention.scale"), real(a.scale)));
        }
    }
    // Mistral 3's attention temperature: its floor is the rotation's original context, which the rotation's keys write.
    if arch == "mistral3" {
        if let Some(t) = spec.layers.iter().find_map(|l| match &l.mixer {
            Mixer::Attention(a) => a.temperature,
            _ => None,
        }) {
            m.push((key("attention.temperature_scale"), real(t.scale)));
        }
    }
    let rope_keys = |m: &mut Vec<(String, MetaValue)>, rope: &Rope| {
        m.push((key("rope.dimension_count"), int(rope.dims)));
        m.push((key("rope.freq_base"), real(rope.theta)));
        if let Some(sections) = rope.sections {
            m.push((key("rope.dimension_sections"), array(U32, sections.iter().map(|&v| MetaValue::U32(v as u32)).collect())));
        }
        match rope.scaling {
            RopeScaling::None | RopeScaling::Factors => {}
            RopeScaling::Linear { factor } => {
                m.push((key("rope.scaling.type"), MetaValue::Str(b"linear".to_vec())));
                m.push((key("rope.scaling.factor"), real(factor)));
            }
            RopeScaling::Yarn { factor, original_ctx, beta_fast, beta_slow, magnitude } => {
                // With no log multiplier the reader's magnitude is `get_mscale(1) · attn_factor`.
                let mscale = if factor <= 1.0 { 1.0 } else { 0.1 * factor.ln() + 1.0 };
                m.push((key("rope.scaling.type"), MetaValue::Str(b"yarn".to_vec())));
                m.push((key("rope.scaling.factor"), real(factor)));
                m.push((key("rope.scaling.original_context_length"), int(original_ctx)));
                m.push((key("rope.scaling.yarn_beta_fast"), real(beta_fast)));
                m.push((key("rope.scaling.yarn_beta_slow"), real(beta_slow)));
                m.push((key("rope.scaling.attn_factor"), real(magnitude / mscale)));
            }
        }
    };
    // A global layer's rotation is the model's own; a windowed layer's base is written apart (`rope.freq_base_swa`).
    let attentions: Vec<&Attention> = spec
        .layers
        .iter()
        .filter_map(|l| match &l.mixer {
            Mixer::Attention(a) => Some(a),
            Mixer::Recurrent(Recurrent::Parallel(a, _)) => Some(&**a),
            _ => None,
        })
        .collect();
    let attention = attentions.iter().find(|a| a.window.is_none()).or(attentions.first()).copied();
    if let Some(a) = attention {
        m.push((key("attention.head_count"), int(a.heads)));
        m.push((key("attention.head_count_kv"), int(a.kv_heads)));
        m.push((key("attention.key_length"), int(a.head_dim)));
        m.push((key("attention.value_length"), int(a.value_dim)));
        if let Some(rope) = &a.rope {
            rope_keys(&mut m, rope);
        }
    }
    if let Some(mla) = spec.layers.iter().find_map(|l| match &l.mixer {
        Mixer::Mla(x) => Some(x),
        _ => None,
    }) {
        m.push((key("attention.head_count"), int(mla.heads)));
        m.push((key("attention.head_count_kv"), int(mla.heads)));
        m.push((key("attention.key_length_mla"), int(mla.qk_nope + mla.qk_rope)));
        m.push((key("attention.value_length_mla"), int(mla.v_dim)));
        m.push((key("attention.kv_lora_rank"), int(mla.kv_lora_rank)));
        if let Some(q) = mla.q_lora_rank {
            m.push((key("attention.q_lora_rank"), int(q)));
        }
        if let Some(rope) = &mla.rope {
            rope_keys(&mut m, rope);
        }
        // DeepSeek's sparse attention: its indexer's shape, and GLM-5's layers' kinds where one is shared.
        if let Some(ix) = &mla.indexer {
            m.push((key("attention.indexer.head_count"), int(ix.heads)));
            m.push((key("attention.indexer.key_length"), int(ix.head_dim)));
            m.push((key("attention.indexer.top_k"), int(ix.top_k)));
            let full: Vec<bool> = spec.layers.iter().map(|l| matches!(&l.mixer, Mixer::Mla(x) if x.indexer.as_ref().is_some_and(|i| i.full))).collect();
            if full.contains(&false) {
                m.push((key("attention.indexer.types"), array(BOOL, full.into_iter().map(MetaValue::Bool).collect())));
            }
        }
        // The reader scales latent scores by `mscale² / √key`, `mscale = magnitude · (1 + 0.1 · (L / 0.1) · ln factor)` for a
        // stored log multiplier `L` (and then reads the magnitude as `attn_factor` itself). A score scale other than
        // `magnitude² / √key` is written as the multiplier that gives it back to the bit.
        // DeepSeek-V3.2's and GLM-5's loaders take the stored tenth as `mscale_all_dim` for the rotation's magnitude too,
        // which is one only at V3.2's multiplier of 1: such a model is written with it.
        let sparse = matches!(arch, "deepseek32" | "glm-dsa");
        let scaling = mla.rope.as_ref().map(|r| r.scaling.clone());
        if let (true, Some(RopeScaling::Yarn { factor, magnitude, .. })) = (sparse, &scaling) {
            let mscale = magnitude * (1.0 + 0.1 * 1.0 * factor.ln());
            let scale = mscale * mscale / ((mla.qk_nope + mla.qk_rope) as f64).sqrt();
            assert_eq!(mla.scale, scale, "a sparse latent attention's score scale other than its log multiplier of 1 gives");
            m.retain(|(k, _)| k != &key("rope.scaling.attn_factor"));
            m.push((key("rope.scaling.attn_factor"), real(*magnitude)));
            m.push((key("rope.scaling.yarn_log_multiplier"), real(0.1)));
        }
        if let (false, Some(RopeScaling::Yarn { factor, magnitude, .. })) = (sparse, scaling) {
            let key_len = (mla.qk_nope + mla.qk_rope) as f64;
            let read = |stored: f64| {
                let mscale = magnitude * (1.0 + 0.1 * (stored / 0.1) * factor.ln());
                mscale * mscale / key_len.sqrt()
            };
            if mla.scale != read(0.0) {
                let mut stored = ((mla.scale * key_len.sqrt()).sqrt() / magnitude - 1.0) / factor.ln();
                for _ in 0..64 {
                    if read(stored) == mla.scale {
                        break;
                    }
                    stored = if read(stored) < mla.scale { stored.next_up() } else { stored.next_down() };
                }
                assert_eq!(read(stored), mla.scale, "a latent score scale no stored log multiplier gives back");
                m.retain(|(k, _)| k != &key("rope.scaling.attn_factor"));
                m.push((key("rope.scaling.attn_factor"), real(magnitude)));
                m.push((key("rope.scaling.yarn_log_multiplier"), real(stored)));
            }
        }
    }
    // LFM2's blocks: KV heads a block, 0 for a convolution block, and the convolution's taps.
    if let Some(c) = spec.layers.iter().find_map(|l| match &l.mixer {
        Mixer::Recurrent(Recurrent::ShortConv(c)) => Some(c),
        _ => None,
    }) {
        let kv = spec.layers.iter().map(|l| MetaValue::U32(match &l.mixer {
            Mixer::Attention(a) => a.kv_heads as u32,
            _ => 0,
        }));
        m.retain(|(k, _)| k != &key("attention.head_count_kv"));
        m.push((key("attention.head_count_kv"), array(U32, kv.collect())));
        m.push((key("shortconv.l_cache"), int(c.taps)));
    }
    // A model with no attention writes no heads, as the Mamba converters write them: zero.
    if !spec.layers.iter().any(|l| matches!(l.mixer, Mixer::Attention(_) | Mixer::Mla(_) | Mixer::Recurrent(Recurrent::Parallel(..)))) {
        m.push((key("attention.head_count"), int(0)));
    }
    // Nemotron-H's and Jamba's blocks: KV heads a block, 0 but for attention; Nemotron-H's FFN width a block, 0 but for an
    // FFN block.
    // Kimi-Linear's: one KV head an attention block, the delta attention's head width and taps, and the latent
    // attention's shared key dimensions, which do not rotate.
    if let Some(dk) = spec.layers.iter().find_map(|l| match &l.mixer {
        Mixer::Recurrent(Recurrent::DeltaAttention(d)) => Some(d),
        _ => None,
    }) {
        let kv = spec.layers.iter().map(|l| MetaValue::U32(u32::from(matches!(l.mixer, Mixer::Mla(_)))));
        m.retain(|(k, _)| k != &key("attention.head_count_kv"));
        m.push((key("attention.head_count_kv"), array(U32, kv.collect())));
        m.push((key("kda.head_dim"), int(dk.head_dim)));
        m.push((key("ssm.conv_kernel"), int(dk.conv_taps)));
        if !spec.layers.iter().any(|l| matches!(l.mixer, Mixer::Mla(_))) {
            m.push((key("attention.head_count"), int(dk.heads)));
        }
    }
    if let Some(mla) = spec.layers.iter().find_map(|l| match &l.mixer {
        Mixer::Mla(x) if x.rope.is_none() => Some(x),
        _ => None,
    }) {
        m.push((key("rope.dimension_count"), int(mla.qk_rope)));
    }
    if matches!(arch, "nemotron_h" | "jamba") {
        let kv = spec.layers.iter().map(|l| MetaValue::U32(match &l.mixer {
            Mixer::Attention(a) => a.kv_heads as u32,
            _ => 0,
        }));
        m.retain(|(k, _)| k != &key("attention.head_count_kv"));
        m.push((key("attention.head_count_kv"), array(U32, kv.collect())));
    }
    if arch == "nemotron_h" {
        let ff = spec.layers.iter().map(|l| MetaValue::U32(match &l.ffn {
            Ffn::Dense { dff, .. } => *dff as u32,
            _ => 0,
        }));
        m.push((key("feed_forward_length"), array(U32, ff.collect())));
    }
    // Mamba-1's selective scan as `JambaModel` writes it.
    if let Some(scan) = spec.layers.iter().find_map(|l| match &l.mixer {
        Mixer::Recurrent(Recurrent::Selective(s)) => Some(s),
        _ => None,
    }) {
        m.push((key("ssm.conv_kernel"), int(scan.conv_taps)));
        m.push((key("ssm.inner_size"), int(scan.inner)));
        m.push((key("ssm.state_size"), int(scan.state)));
        m.push((key("ssm.time_step_rank"), int(scan.dt_rank)));
    }
    // Mamba-2's scan as `Mamba2Model` writes it, alone or beside attention.
    if let Some(scan) = spec.layers.iter().find_map(|l| match &l.mixer {
        Mixer::Recurrent(Recurrent::Ssd(s) | Recurrent::Parallel(_, s)) => Some(s),
        _ => None,
    }) {
        m.push((key("ssm.conv_kernel"), int(scan.conv_taps)));
        m.push((key("ssm.inner_size"), int(scan.heads * scan.head_dim)));
        m.push((key("ssm.state_size"), int(scan.state)));
        m.push((key("ssm.time_step_rank"), int(scan.heads)));
        m.push((key("ssm.group_count"), int(scan.groups)));
    }
    if let Some(g) = spec.layers.iter().find_map(|l| match &l.mixer {
        Mixer::GatedDelta(g) => Some(g),
        _ => None,
    }) {
        m.push((key("ssm.group_count"), int(g.k_heads)));
        m.push((key("ssm.time_step_rank"), int(g.v_heads)));
        m.push((key("ssm.state_size"), int(g.head_dim)));
        m.push((key("ssm.conv_kernel"), int(g.conv_taps)));
        m.push((key("ssm.inner_size"), int(g.v_heads * g.head_dim)));
        let recurrent = spec.layers.iter().map(|l| MetaValue::Bool(matches!(l.mixer, Mixer::GatedDelta(_)))).collect();
        m.push((key("attention.recurrent_layers"), array(BOOL, recurrent)));
    }
    // MiniMax-01's: its lightning blocks by `attention.recurrent_layers`, their heads where no block attends, and the
    // residual's scale.
    if let Some(x) = spec.layers.iter().find_map(|l| match &l.mixer {
        Mixer::Recurrent(Recurrent::Lightning(x)) => Some(x),
        _ => None,
    }) {
        let recurrent = spec.layers.iter().map(|l| MetaValue::Bool(matches!(l.mixer, Mixer::Recurrent(Recurrent::Lightning(_))))).collect();
        m.push((key("attention.recurrent_layers"), array(BOOL, recurrent)));
        if attention.is_none() {
            m.retain(|(k, _)| k != &key("attention.head_count"));
            m.push((key("attention.head_count"), int(x.heads)));
            m.push((key("attention.key_length"), int(x.head_dim)));
            m.push((key("attention.value_length"), int(x.head_dim)));
        }
    }
    if let Some(scale) = spec.parts.iter().find_map(|p| if let Part::NormedResidual { scale } = p { Some(*scale) } else { None }) {
        m.push((key("residual_scale"), real(scale)));
    }
    // Gemma 4's: the windowed blocks' head width and rotated width, each block's KV heads and FFN width, the shared
    // blocks, the per-layer input width, and its mixture's keys.
    if arch == "gemma4" {
        let local = attentions.iter().find(|a| a.window.is_some()).copied();
        if let Some(a) = local {
            m.push((key("attention.key_length_swa"), int(a.head_dim)));
            m.push((key("attention.value_length_swa"), int(a.value_dim)));
            if let Some(r) = &a.rope {
                m.push((key("rope.dimension_count_swa"), int(r.dims)));
            }
        }
        let kv = spec.layers.iter().map(|l| MetaValue::U32(match &l.mixer {
            Mixer::Attention(a) => a.kv_heads as u32,
            _ => 0,
        }));
        m.retain(|(k, _)| k != &key("attention.head_count_kv"));
        m.push((key("attention.head_count_kv"), array(U32, kv.collect())));
        let ff = spec.layers.iter().map(|l| MetaValue::U32(match &l.ffn {
            Ffn::Dense { dff, .. } | Ffn::DenseAndMoe { dff, .. } => *dff as u32,
            _ => 0,
        }));
        m.push((key("feed_forward_length"), array(U32, ff.collect())));
        let shared = spec.parts.iter().find_map(|p| if let Part::SharedKv { from } = p { Some(spec.layers.len() - from) } else { None });
        m.push((key("attention.shared_kv_layers"), int(shared.unwrap_or(0))));
        let ple = spec.parts.iter().find_map(|p| if let Part::PerLayerInput { dim } = p { Some(*dim) } else { None });
        m.push((key("embedding_length_per_layer_input"), int(ple.unwrap_or(0))));
        if let Some(moe) = spec.layers.iter().find_map(|l| if let Ffn::DenseAndMoe { moe, .. } = &l.ffn { Some(moe) } else { None }) {
            m.push((key("expert_count"), int(moe.experts)));
            m.push((key("expert_used_count"), int(moe.top_k)));
            m.push((key("expert_feed_forward_length"), int(moe.expert_dff)));
        }
    }
    // RWKV-7's: its head width, its four low-rank pairs' ranks, and the LayerNorms' epsilon under its LayerNorm key.
    if let Some(r) = spec.layers.iter().find_map(|l| match &l.mixer {
        Mixer::Recurrent(Recurrent::Rwkv7(r)) => Some(r),
        _ => None,
    }) {
        m.push((key("wkv.head_size"), int(r.head_size)));
        m.push((key("attention.decay_lora_rank"), int(r.decay_lora)));
        m.push((key("attention.iclr_lora_rank"), int(r.iclr_lora)));
        m.push((key("attention.value_residual_mix_lora_rank"), int(r.value_res_lora)));
        m.push((key("attention.gate_lora_rank"), int(r.gate_lora)));
        m.retain(|(k, _)| k != &key("attention.layer_norm_rms_epsilon"));
        m.push((key("attention.layer_norm_epsilon"), real(spec.rms_eps)));
    }
    let windows: Vec<Option<usize>> = spec
        .layers
        .iter()
        .map(|l| match &l.mixer {
            Mixer::Attention(a) => a.window.map(Window::span),
            _ => None,
        })
        .collect();
    if let Some(window) = windows.iter().flatten().next() {
        m.push((key("attention.sliding_window"), int(*window)));
        if matches!(arch, "gemma3" | "gemma4") {
            m.push((key("attention.sliding_window_pattern"), array(BOOL, windows.iter().map(|w| MetaValue::Bool(w.is_some())).collect())));
            let local = spec.layers.iter().find_map(|l| match &l.mixer {
                Mixer::Attention(a) if a.window.is_some() => a.rope.as_ref().map(|r| r.theta),
                _ => None,
            });
            if let Some(theta) = local {
                m.push((key("rope.freq_base_swa"), real(theta)));
            }
        } else {
            // A window on all but every `pattern`-th layer.
            let pattern = windows.iter().position(|w| w.is_none()).map_or(windows.len(), |p| p + 1);
            m.push((key("attention.sliding_window_pattern"), int(pattern)));
        }
    }
    match spec.layers.iter().find_map(|l| match &l.ffn {
        Ffn::Dense { dff, .. } | Ffn::ChannelMix { dff } => Some(*dff),
        _ => None,
    }) {
        Some(_) if matches!(arch, "nemotron_h" | "gemma4") => {}
        Some(dff) => m.push((key("feed_forward_length"), int(dff))),
        // A model with no FFN writes a width of zero, as `Mamba2Model` does.
        None if spec.layers.iter().all(|l| l.ffn == Ffn::None) => m.push((key("feed_forward_length"), int(0))),
        None => {}
    }
    if let Some(moe) = spec.layers.iter().find_map(|l| match &l.ffn {
        Ffn::Moe(x) => Some(x),
        _ => None,
    }) {
        m.push((key("expert_count"), int(moe.experts)));
        m.push((key("expert_used_count"), int(moe.top_k)));
        // Mistral 3's, PhiMoE's and MiniMax-01's experts are as wide as the FFN: their arms read no width of the experts'
        // own.
        if matches!(arch, "mistral3" | "phimoe" | "minimax-01") {
            m.push((key("feed_forward_length"), int(moe.expert_dff)));
        }
        m.push((key("expert_feed_forward_length"), int(moe.expert_dff)));
        m.push((key("expert_weights_scale"), real(moe.weights_scale)));
        m.push((key("expert_shared_count"), int(moe.shared_experts)));
        if moe.shared_experts > 0 {
            m.push((key("expert_shared_feed_forward_length"), int(moe.shared_dff)));
        }
        m.push((key("expert_weights_norm"), MetaValue::Bool(moe.norm_top_k)));
        m.push((key("expert_gating_func"), int(if matches!(moe.gating, Gating::Sigmoid { .. }) { 2 } else { 1 })));
        if let Some((groups, used)) = moe.groups {
            m.push((key("expert_group_count"), int(groups)));
            m.push((key("expert_group_used_count"), int(used)));
        }
        let lead = spec.layers.iter().take_while(|l| matches!(l.ffn, Ffn::Dense { .. })).count();
        m.push((key("leading_dense_block_count"), int(lead)));
    }
    m
}

/// Write the checkpoint of `spec` to `path`, each tensor in the format `format_of` names. A Q8_0 block holds whole
/// multiples of its scale 2⁻⁷ (exact in `f32`); a block of another quantized format holds seeded bytes with its scales
/// set small and finite, so its values are the model's whatever the bits — the reference decodes the same bytes.
pub fn write_checkpoint_as(spec: &ModelSpec, path: &Path, format_of: &dyn Fn(&str) -> GgmlType) {
    write_tensors(spec, path, &checkpoint_meta(spec), format_of);
}

/// The hybrid models' gated delta layers, each case's gates projected in a format of its own, written under `dir`: Qwen3.5
/// with its gates in `f32` (the tiny checkpoint's) and in Q8_0 (the real Qwen3.5's), Qwen3.5 at 2048 wide with both,
/// Qwen3.8's three value heads a key head with its gates in `f32` (the real Qwen3.8's), and heads of 512 — a convolution
/// whose L2 norm takes more threads than a GEMV's eight warps. Every other matrix of whole 32-value rows in Q8_0. Each
/// case names itself, its spec, its checkpoint and whether its gates are projected in `f32`.
pub fn gated_delta_cases(dir: &Path, tag: &str, ctx: usize) -> Vec<(String, ModelSpec, std::path::PathBuf, bool)> {
    let write = |spec: ModelSpec, name: &str, f32_gates: bool| {
        let path = dir.join(format!("{name}-{tag}.gguf"));
        let blocked: Vec<String> = tensors(&spec).into_iter().filter(|(n, dims)| dims.len() == 2 && dims[0] % 32 == 0 && !n.contains("norm")).map(|(n, _)| n).collect();
        write_checkpoint_as(&spec, &path, &|t| {
            let gate = t.contains("ssm_alpha") || t.contains("ssm_beta");
            if blocked.iter().any(|b| b == t) && !(gate && f32_gates) {
                GgmlType::Q8_0
            } else {
                GgmlType::F32
            }
        });
        (name.to_string(), spec, path, f32_gates)
    };
    let reshaped = |mut spec: ModelSpec, heads: Option<(usize, usize)>, width: Option<usize>| {
        for layer in &mut spec.layers {
            if let Mixer::GatedDelta(g) = &mut layer.mixer {
                if let Some((nk, nv)) = heads {
                    (g.k_heads, g.v_heads) = (nk, nv);
                }
                if let Some(w) = width {
                    g.head_dim = w;
                }
            }
        }
        spec
    };
    vec![
        write(Tiny::Qwen35.spec(ctx), "qwen35-f32-gates", true),
        write(Tiny::Qwen35.spec(ctx), "qwen35-q8-gates", false),
        write(Tiny::Qwen35.medium(ctx), "qwen35-medium-q8-gates", false),
        write(Tiny::Qwen35.medium(ctx), "qwen35-medium-f32-gates", true),
        write(reshaped(Tiny::Qwen35.medium(ctx), Some((2, 6)), None), "qwen38-heads-f32-gates", true),
        write(reshaped(Tiny::Qwen35.spec(ctx), None, Some(512)), "qwen35-heads512-q8-gates", false),
    ]
}

/// Write the checkpoint of `spec` as llama.cpp's converter writes one in the 16-bit float format `ty` (F16 or BF16):
/// every two-dimensional matrix of whole 32-value rows in `ty` — a gated delta rule's gate projections too — and the
/// vectors, norms and convolution taps in `f32`, as the BF16 conversions of Qwen3.5-0.8B, Llama-3.2-1B and Gemma-3-1B
/// keep them. Every value is the `f32` checkpoint's.
pub fn write_checkpoint_in(spec: &ModelSpec, path: &Path, ty: GgmlType) {
    let matrices: Vec<String> =
        tensors(spec).into_iter().filter(|(n, dims)| dims.len() == 2 && dims[0] % 32 == 0 && !n.contains("norm")).map(|(n, _)| n).collect();
    write_checkpoint_as(spec, path, &|n| if matrices.iter().any(|m| m == n) { ty } else { GgmlType::F32 });
}

/// The checkpoint of `spec` with its tensors and no metadata at all — what a converter that writes no keys leaves, a
/// file the spec reader cannot describe.
pub fn write_bare_checkpoint(spec: &ModelSpec, path: &Path) {
    write_tensors(spec, path, &[], &|name| if quantized(name) { GgmlType::Q8_0 } else { GgmlType::F32 });
}

fn write_tensors(spec: &ModelSpec, path: &Path, meta: &[(String, MetaValue)], format_of: &dyn Fn(&str) -> GgmlType) {
    let list = tensors(spec);
    let outs: Vec<TensorOut> =
        list.iter().map(|(name, dims)| TensorOut { name: name.clone(), dims: dims.clone(), ty: format_of(name) }).collect();
    gguf::write(path, meta, &outs, |k| {
        let (name, dims) = &list[k];
        let n = dims.iter().product::<u64>() as usize;
        Ok(match outs[k].ty {
            GgmlType::F32 => (0..n).flat_map(|i| value(name, i).to_le_bytes()).collect(),
            // A matrix's weights are multiples of 2⁻⁷ within ±0.5, whole in a 16-bit float's mantissa: the file holds the
            // values the `f32` checkpoint holds, and a weight that would round is refused.
            GgmlType::F16 | GgmlType::Bf16 => (0..n)
                .flat_map(|i| {
                    use logicaffeine_compile::model::quant::{bf16_to_f32, f16_to_f32, f32_to_bf16, f32_to_f16};
                    let v = value(name, i);
                    let (bits, back) = if outs[k].ty == GgmlType::F16 { (f32_to_f16(v), f16_to_f32(f32_to_f16(v))) } else { (f32_to_bf16(v), bf16_to_f32(f32_to_bf16(v))) };
                    assert_eq!(back.to_bits(), v.to_bits(), "{name}[{i}] = {v} is not whole in {}", outs[k].ty.name());
                    bits.to_le_bytes()
                })
                .collect(),
            GgmlType::Q8_0 => {
                let mut bytes = Vec::with_capacity(n / 32 * 34);
                for b in 0..n / 32 {
                    bytes.extend_from_slice(&SCALE_BITS.to_le_bytes());
                    bytes.extend((0..32).map(|j| level(name, b * 32 + j) as i8 as u8));
                }
                bytes
            }
            ty => seeded_blocks(name, ty, n),
        })
    })
    .expect("the tiny checkpoint writes");
}

/// `n` weights of `name` as blocks of `ty`: seeded bytes, each block's scale fields overwritten so its values span
/// about ±0.5 like the Q8_0 weights' — Q2_K `d` 2⁻⁷ and minimum 2⁻⁶ (4-bit scales of 2-bit levels), Q3_K `d` 2⁻⁸ (a
/// 6-bit signed scale of a 3-bit level), Q4_K/Q5_K `d` 2⁻¹² with their minima at 2⁻⁹ (6-bit scales of 4-/5-bit levels),
/// Q6_K `d` 2⁻¹³ (an int8 scale of a 6-bit level), Q4_0/Q5_0 `d` 2⁻⁵/2⁻⁶ (a level about its middle), Q4_1/Q5_1 the
/// same with a negative minimum, MXFP4 an exponent of 2⁻⁵ over its ±12, IQ4_NL `d` 2⁻⁸ over its ±127 levels, IQ4_XS
/// `d` 2⁻¹³ (a 6-bit signed scale of the same levels).
pub fn seeded_blocks(name: &str, ty: GgmlType, n: usize) -> Vec<u8> {
    let (len, size) = (ty.block_len(), ty.block_bytes());
    let mut bytes: Vec<u8> = (0..n / len * size).map(|i| level(name, i) as u8).collect();
    let f16 = |v: u16| v.to_le_bytes();
    for block in bytes.chunks_exact_mut(size) {
        match ty {
            GgmlType::Q4K | GgmlType::Q5K => {
                block[0..2].copy_from_slice(&f16(0x0C00));
                block[2..4].copy_from_slice(&f16(0x1800));
            }
            GgmlType::Q6K => block[size - 2..].copy_from_slice(&f16(0x0800)),
            GgmlType::Q2K => {
                block[80..82].copy_from_slice(&f16(0x2000));
                block[82..84].copy_from_slice(&f16(0x2400));
            }
            GgmlType::Q3K => block[108..110].copy_from_slice(&f16(0x1C00)),
            GgmlType::Q4_0 => block[0..2].copy_from_slice(&f16(0x2800)),
            GgmlType::Q5_0 => block[0..2].copy_from_slice(&f16(0x2400)),
            GgmlType::Q4_1 => {
                block[0..2].copy_from_slice(&f16(0x2400));
                block[2..4].copy_from_slice(&f16(0xB000));
            }
            GgmlType::Q5_1 => {
                block[0..2].copy_from_slice(&f16(0x2000));
                block[2..4].copy_from_slice(&f16(0xB000));
            }
            GgmlType::Mxfp4 => block[0] = 123,
            GgmlType::Iq4Nl => block[0..2].copy_from_slice(&f16(0x1C00)),
            GgmlType::Iq4Xs => block[0..2].copy_from_slice(&f16(0x0800)),
            other => panic!("no seeded block for {}", other.name()),
        }
    }
    bytes
}
