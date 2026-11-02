//! Small models of every architecture the spec reads, written as real GGUF checkpoints: the projections and tables
//! in Q8_0, the norms and the recurrence's small tensors in F32, every weight exact in both (a Q8_0 block's scale is
//! 2⁻⁷ and its values whole multiples of it), so a lane reading the file and the `f64` reference reading it see the
//! same numbers. Widths are multiples of 32, as Q8_0 rows need.

#![allow(dead_code)]

use std::path::Path;

use logicaffeine_compile::model::gguf::{self, MetaValue, TensorOut};
use logicaffeine_compile::model::quant::GgmlType;
use logicaffeine_compile::model::spec::{
    Act, Attention, Ffn, GateActivation, GatedDelta, Gating, Layer, Mixer, Mla, ModelSpec, Moe, Rope, RopeScaling,
    Window,
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
                Ffn::Dense { dff, .. } => *dff = 2048,
                Ffn::Moe(m) => {
                    m.expert_dff = 1024;
                    m.shared_dff = 1024 * m.shared_experts;
                }
            }
            if let Mixer::Mla(m) = &mut layer.mixer {
                (m.heads, m.qk_nope, m.qk_rope, m.v_dim, m.kv_lora_rank) = (16, 128, 64, 128, 512);
                m.rope.dims = 64;
                let mscale = 1.0 + 0.0707 * 40f64.ln();
                m.scale = mscale * mscale / ((m.qk_nope + m.qk_rope) as f64).sqrt();
            }
        }
        spec
    }

    fn sized(self, ctx: usize, w: usize) -> ModelSpec {
        let neox = !matches!(self, Tiny::Llama | Tiny::LlamaTied);
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
        let gemma = self == Tiny::Gemma3;
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
            rope: Rope {
                dims: rope,
                theta: 10000.0,
                neox: false,
                scaling: RopeScaling::Yarn { factor: 40.0, original_ctx: 16, beta_fast: 32.0, beta_slow: 1.0, magnitude: 1.0 },
                sections: None,
            },
            scale: mscale * mscale / ((nope + rope) as f64).sqrt(),
            kv_b_split: self == Tiny::DeepSeek2Split,
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
                    _ => Mixer::Attention(attention.clone()),
                };
                let ffn = match self {
                    _ if deepseek && l == 0 => dense.clone(),
                    _ if deepseek => deepseek_moe.clone(),
                    Tiny::Qwen3Moe => moe.clone(),
                    Tiny::GptOss => oss_moe.clone(),
                    Tiny::Gemma3 => Ffn::Dense { dff, act: Act::GeluTanh },
                    _ => dense.clone(),
                };
                Layer { mixer, ffn, post_mixer_norm: gemma, post_ffn_norm: gemma }
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
            }
            .to_string(),
            d,
            vocab: VOCAB,
            ctx_len: ctx,
            rms_eps: 1e-6,
            embedding_scale: gemma.then(|| (d as f64).sqrt()),
            tied_output: matches!(self, Tiny::LlamaTied | Tiny::Gemma3),
            final_logit_softcap: None,
            layers,
            mtp_blocks: 0,
            ffn_norm: if matches!(self, Tiny::Qwen35 | Tiny::GptOss) { "post_attention_norm" } else { "ffn_norm" },
            hyper: None,
            ple: None,
            indexer: None,
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
    if !spec.tied_output {
        add("output.weight".into(), &[d, spec.vocab as u64]);
    }
    // One table of per-frequency factors serves every rotation that reads one, as Llama 3's checkpoints store it.
    let factored = spec.layers.iter().find_map(|l| match &l.mixer {
        Mixer::Attention(a) => a.rope.as_ref().filter(|r| r.scaling == RopeScaling::Factors).map(|r| r.dims),
        _ => None,
    });
    if let Some(dims) = factored {
        add("rope_freqs.weight".into(), &[dims as u64 / 2]);
    }
    for (l, layer) in spec.layers.iter().enumerate() {
        let blk = |s: &str| format!("blk.{l}.{s}.weight");
        add(blk("attn_norm"), &[d]);
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
                    add(blk("attn_k"), &[d, akv]);
                    add(blk("attn_v"), &[d, avd]);
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
                    add(blk("attn_k_norm"), &[kw]);
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
            }
        }
        if layer.post_mixer_norm {
            add(blk("post_attention_norm"), &[d]);
        }
        add(blk(spec.ffn_norm), &[d]);
        match &layer.ffn {
            Ffn::Dense { dff, .. } => {
                let f = *dff as u64;
                add(blk("ffn_gate"), &[d, f]);
                add(blk("ffn_up"), &[d, f]);
                add(blk("ffn_down"), &[f, d]);
            }
            Ffn::Moe(m) => {
                let (e, f) = (m.experts as u64, m.expert_dff as u64);
                add(blk("ffn_gate_inp"), &[d, e]);
                add(blk("ffn_gate_exps"), &[d, f, e]);
                add(blk("ffn_up_exps"), &[d, f, e]);
                add(blk("ffn_down_exps"), &[f, d, e]);
                if m.gating == (Gating::Sigmoid { bias: true }) {
                    add(format!("blk.{l}.exp_probs_b.bias"), &[e]);
                }
                if m.shared_gate {
                    add(blk("ffn_gate_inp_shexp"), &[d]);
                }
                if m.shared_experts > 0 {
                    let s = m.shared_dff as u64;
                    add(blk("ffn_gate_shexp"), &[d, s]);
                    add(blk("ffn_up_shexp"), &[d, s]);
                    add(blk("ffn_down_shexp"), &[s, d]);
                }
                if m.biases {
                    let bias = |s: &str| format!("blk.{l}.{s}.bias");
                    add(bias("ffn_gate_inp"), &[e]);
                    add(bias("ffn_gate_exps"), &[f, e]);
                    add(bias("ffn_up_exps"), &[f, e]);
                    add(bias("ffn_down_exps"), &[d, e]);
                }
            }
        }
        if layer.post_ffn_norm {
            add(blk("post_ffw_norm"), &[d]);
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
        rope_keys(&mut m, &mla.rope);
        // The reader scales latent scores by `mscale² / √key`, `mscale = magnitude · (1 + 0.1 · (L / 0.1) · ln factor)` for a
        // stored log multiplier `L` (and then reads the magnitude as `attn_factor` itself). A score scale other than
        // `magnitude² / √key` is written as the multiplier that gives it back to the bit.
        if let RopeScaling::Yarn { factor, magnitude, .. } = mla.rope.scaling {
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
        if arch == "gemma3" {
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
    if let Some(dff) = spec.layers.iter().find_map(|l| match &l.ffn {
        Ffn::Dense { dff, .. } => Some(*dff),
        _ => None,
    }) {
        m.push((key("feed_forward_length"), int(dff)));
    }
    if let Some(moe) = spec.layers.iter().find_map(|l| match &l.ffn {
        Ffn::Moe(x) => Some(x),
        _ => None,
    }) {
        m.push((key("expert_count"), int(moe.experts)));
        m.push((key("expert_used_count"), int(moe.top_k)));
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
    let list = tensors(spec);
    let outs: Vec<TensorOut> =
        list.iter().map(|(name, dims)| TensorOut { name: name.clone(), dims: dims.clone(), ty: format_of(name) }).collect();
    let meta = checkpoint_meta(spec);
    gguf::write(path, &meta, &outs, |k| {
        let (name, dims) = &list[k];
        let n = dims.iter().product::<u64>() as usize;
        Ok(match outs[k].ty {
            GgmlType::F32 => (0..n).flat_map(|i| value(name, i).to_le_bytes()).collect(),
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
