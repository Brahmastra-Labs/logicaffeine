//! The harness the model locks share: a small model's checkpoint built in memory with weights exact in f64, the authored
//! program with its weights, host tables and carried state written into it as literals, the VM's run of it, and the
//! decode step's carried state taken from the reference. Llama 4's small configuration lives here too.

#![allow(dead_code)]

use std::collections::HashMap;

use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::model::author;
use logicaffeine_compile::model::bind::{bind, host_values, neox_order, rope_tables, values, Source};
use logicaffeine_compile::model::reference::{Reference, Tensors, WeightSource};
use logicaffeine_compile::model::spec::{
    Act, Attention, Ffn, GateActivation, GatedDelta, Gating, Layer, Mixer, ModelSpec, Moe, Rope, RopeScaling, Temperature,
    Window,
};

pub const CTX: usize = 12;
pub const PROMPT: &[u32] = &[5, 17, 2, 30];

/// A weight in `(-0.5, 0.5)`, exact in f64 and as a decimal literal: a multiple of 2⁻¹⁰.
pub fn weight(name: &str, i: usize) -> f32 {
    let salt = name.bytes().fold(0u64, |h, b| h.wrapping_mul(131).wrapping_add(b as u64));
    let h = (i as u64).wrapping_mul(2654435761).wrapping_add(salt.wrapping_mul(40503)) % 1023;
    (h as f32 - 511.0) / 1024.0
}

/// The model's tensors, named and laid out as a GGUF checkpoint holds them.
pub fn checkpoint(spec: &ModelSpec) -> Tensors {
    let mut t = Tensors::default();
    let mut add = |name: &str, dims: &[u64]| {
        let n = dims.iter().product::<u64>() as usize;
        let mut v: Vec<f32> = (0..n).map(|i| weight(name, i)).collect();
        if name.ends_with("norm.weight") {
            v.iter_mut().for_each(|w| *w += 1.0);
        }
        // `ssm_a` holds −exp(A_log): a negative rate, so the memory decays.
        if name.ends_with(".ssm_a") {
            v.iter_mut().for_each(|w| *w = -(*w + 0.75));
        }
        // A rotation's frequency factors divide its frequencies: each in `[1, 1.5)`, as Llama 3's are at least one.
        if name == "rope_freqs.weight" {
            v.iter_mut().for_each(|w| *w = 1.0 + w.abs());
        }
        t.f32(name, dims, &v);
    };
    let d = spec.d as u64;
    if let Some(dims) = factored(spec) {
        add("rope_freqs.weight", &[dims as u64 / 2]);
    }
    add("token_embd.weight", &[d, spec.vocab as u64]);
    add("output_norm.weight", &[d]);
    if !spec.tied_output {
        add("output.weight", &[d, spec.vocab as u64]);
    }
    for (l, layer) in spec.layers.iter().enumerate() {
        let blk = |s: &str| format!("blk.{l}.{s}.weight");
        add(&blk("attn_norm"), &[d]);
        match &layer.mixer {
            Mixer::Attention(a) => {
                let (aq, akv, hd) =
                    ((a.heads * a.head_dim) as u64, (a.kv_heads * a.head_dim) as u64, a.head_dim as u64);
                let (avd, ao) = ((a.kv_heads * a.value_dim) as u64, (a.heads * a.value_dim) as u64);
                // A fused projection holds the queries, keys and values one after another in each output row. A gated
                // query projection holds each head's gate beside its queries; its bias is the queries' alone.
                let parts: &[(&str, u64, u64)] = if a.fused_qkv {
                    &[("attn_qkv", aq + akv + avd, aq + akv + avd)]
                } else {
                    &[("attn_q", if a.output_gate { 2 * aq } else { aq }, aq), ("attn_k", akv, akv), ("attn_v", avd, avd)]
                };
                for &(part, width, biased) in parts {
                    add(&blk(part), &[d, width]);
                    if a.bias {
                        add(&format!("blk.{l}.{part}.bias"), &[biased]);
                    }
                }
                add(&blk("attn_output"), &[ao, d]);
                if a.output_bias {
                    add(&format!("blk.{l}.attn_output.bias"), &[d]);
                }
                if a.sinks {
                    add(&blk("attn_sinks"), &[a.heads as u64]);
                }
                if a.qk_norm {
                    let (qw, kw) = if a.qk_norm_whole { (aq, akv) } else { (hd, hd) };
                    add(&blk("attn_q_norm"), &[qw]);
                    add(&blk("attn_k_norm"), &[kw]);
                }
            }
            Mixer::GatedDelta(g) => {
                let (qk, vw, nv) = ((g.k_heads * g.head_dim) as u64, (g.v_heads * g.head_dim) as u64, g.v_heads as u64);
                add(&blk("attn_qkv"), &[d, 2 * qk + vw]);
                add(&blk("attn_gate"), &[d, vw]);
                add(&blk("ssm_conv1d"), &[g.conv_taps as u64, 2 * qk + vw]);
                add(&format!("blk.{l}.ssm_dt.bias"), &[nv]);
                add(&blk("ssm_beta"), &[d, nv]);
                add(&blk("ssm_alpha"), &[d, nv]);
                add(&blk("ssm_norm"), &[g.head_dim as u64]);
                add(&blk("ssm_out"), &[vw, d]);
                add(&format!("blk.{l}.ssm_a"), &[nv]);
            }
            // Latent attention: the query (or its low-rank halves and their norm), the latent with the shared rotated
            // key, the latent's norm, its expansion into each head's keys and values, and the output projection.
            Mixer::Mla(m) => {
                let (h, qk, kw) = (m.heads as u64, (m.qk_nope + m.qk_rope) as u64, (m.kv_lora_rank + m.qk_rope) as u64);
                let (nope, lora, vd) = (m.qk_nope as u64, m.kv_lora_rank as u64, m.v_dim as u64);
                match m.q_lora_rank {
                    None => add(&blk("attn_q"), &[d, h * qk]),
                    Some(r) => {
                        add(&blk("attn_q_a"), &[d, r as u64]);
                        add(&blk("attn_q_a_norm"), &[r as u64]);
                        add(&blk("attn_q_b"), &[r as u64, h * qk]);
                    }
                }
                add(&blk("attn_kv_a_mqa"), &[d, kw]);
                add(&blk("attn_kv_a_norm"), &[lora]);
                if m.kv_b_split {
                    add(&blk("attn_k_b"), &[nope, lora, h]);
                    add(&blk("attn_v_b"), &[lora, vd, h]);
                } else {
                    add(&blk("attn_kv_b"), &[lora, h * (nope + vd)]);
                }
                add(&blk("attn_output"), &[h * vd, d]);
            }
        }
        if layer.post_mixer_norm {
            add(&blk("post_attention_norm"), &[d]);
        }
        add(&blk(spec.ffn_norm), &[d]);
        match &layer.ffn {
            Ffn::Dense { dff, .. } => {
                let f = *dff as u64;
                add(&blk("ffn_gate"), &[d, f]);
                add(&blk("ffn_up"), &[d, f]);
                add(&blk("ffn_down"), &[f, d]);
            }
            Ffn::Moe(m) => {
                let (e, f) = (m.experts as u64, m.expert_dff as u64);
                add(&blk("ffn_gate_inp"), &[d, e]);
                add(&blk("ffn_gate_exps"), &[d, f, e]);
                add(&blk("ffn_up_exps"), &[d, f, e]);
                add(&blk("ffn_down_exps"), &[f, d, e]);
                if m.gating == (Gating::Sigmoid { bias: true }) {
                    add(&format!("blk.{l}.exp_probs_b.bias"), &[e]);
                }
                if m.biases {
                    let bias = |s: &str| format!("blk.{l}.{s}.bias");
                    add(&bias("ffn_gate_inp"), &[e]);
                    add(&bias("ffn_gate_exps"), &[f, e]);
                    add(&bias("ffn_up_exps"), &[f, e]);
                    add(&bias("ffn_down_exps"), &[d, e]);
                }
                if m.shared_gate {
                    add(&blk("ffn_gate_inp_shexp"), &[d]);
                }
                if m.shared_experts > 0 {
                    let sf = m.shared_dff as u64;
                    add(&blk("ffn_gate_shexp"), &[d, sf]);
                    add(&blk("ffn_up_shexp"), &[d, sf]);
                    add(&blk("ffn_down_shexp"), &[sf, d]);
                }
            }
        }
        if layer.post_ffn_norm {
            add(&blk("post_ffw_norm"), &[d]);
        }
    }
    t
}

pub fn lit(v: f64) -> String {
    format!("{v:.20}")
}

/// The rotated dimensions of a rotation whose frequency factors the checkpoint holds (`rope_freqs.weight`).
fn factored(spec: &ModelSpec) -> Option<usize> {
    spec.layers.iter().find_map(|l| match &l.mixer {
        Mixer::Attention(a) => a.rope.as_ref().filter(|r| r.scaling == RopeScaling::Factors).map(|r| r.dims),
        _ => None,
    })
}

/// The rotary tables from the spec, and any `extra` tensor (a KV cache) from the values given. A rotation scaled by
/// the checkpoint's frequency factors, and a latent attention's, take their tables from the host's own fill
/// ([`host_values`]), which reads the factors and every scaling.
fn given(spec: &ModelSpec, extra: &HashMap<String, Vec<f64>>) -> HashMap<String, Vec<f64>> {
    let rope = spec.layers.iter().find_map(|l| match &l.mixer {
        Mixer::Attention(a) => a.rope.clone(),
        _ => None,
    });
    let Some(rope) = rope.filter(|_| factored(spec).is_none()) else {
        return extra.clone();
    };
    let (cos, sin) = rope_tables(&rope, CTX, None);
    let half = rope.dims / 2;
    let mut fills: HashMap<String, Vec<f64>> = extra.clone();
    fills.insert("Rcos".into(), cos.clone());
    fills.insert("Rsin".into(), sin.clone());
    fills.insert("RC".into(), cos[..half].to_vec());
    fills.insert("RS".into(), sin[..half].to_vec());
    fills
}

/// The values a declared weight `var` is filled with: a host table the spec computes, or its checkpoint tensor through
/// its binding; `None` for carried state.
fn weight_values(spec: &ModelSpec, var: &str, source: &mut dyn WeightSource) -> Option<Vec<f64>> {
    if let Some(v) = host_values(spec, var, CTX, source).unwrap_or_else(|e| panic!("`{var}`: {e}")) {
        return Some(v);
    }
    match bind(spec, var).unwrap_or_else(|e| panic!("{var} is unbound: {e:?}")) {
        Source::Weight(b) => Some(values(&b, source).expect("bound tensor")),
        Source::State => None,
    }
}

/// `src` with each tensor in `fills` filled right after its declaration, then the logits and the token shown.
fn written(src: &str, fills: &HashMap<String, Vec<f64>>, vocab: usize) -> String {
    let mut out = String::new();
    for line in src.lines() {
        out.push_str(line);
        out.push('\n');
        let Some(var) = line.strip_prefix("Let ").and_then(|r| r.split_once(" be a ")).map(|(v, _)| v) else {
            continue;
        };
        if let Some(vals) = fills.get(var) {
            for (i, v) in vals.iter().enumerate() {
                if *v != 0.0 {
                    out.push_str(&format!("Set item {} of {var} to {}.\n", i + 1, lit(*v)));
                }
            }
        }
    }
    for k in 1..=vocab {
        out.push_str(&format!("Show item {k} of logits.\n"));
    }
    out.push_str("Show next.\n");
    out
}

/// The authored program with every declared weight filled from the checkpoint through its binding, the rotary
/// tables from the spec, and any `extra` tensor (a KV cache) from the values given — each fill right after its
/// declaration — then the logits shown.
pub fn filled(spec: &ModelSpec, prog: &author::Authored, ckpt: &Tensors, extra: &HashMap<String, Vec<f64>>) -> String {
    filled_from(spec, prog, &mut ckpt.clone(), extra)
}

/// [`filled`] with every weight read from `source`: a checkpoint on disk as a host opens it.
pub fn filled_from(spec: &ModelSpec, prog: &author::Authored, source: &mut dyn WeightSource, extra: &HashMap<String, Vec<f64>>) -> String {
    let mut fills = given(spec, extra);
    for var in prog.1.keys().chain(prog.2.keys()) {
        if !fills.contains_key(var) {
            if let Some(v) = weight_values(spec, var, source) {
                fills.insert(var.clone(), v);
            }
        }
    }
    written(&prog.0, &fills, spec.vocab)
}

/// A program split over chips, filled as [`filled`] fills one: each chip's slice from the full tensor its manifest
/// names, at the coordinates the slice maps it to, and each replicated weight whole.
pub fn filled_split(spec: &ModelSpec, prog: &author::TpAuthored, ckpt: &Tensors, extra: &HashMap<String, Vec<f64>>) -> String {
    let (src, f32w, q8w, manifest) = prog;
    let mut full = given(spec, extra);
    for var in f32w.keys().chain(q8w.keys()) {
        if !full.contains_key(var) {
            if let Some(v) = weight_values(spec, var, &mut ckpt.clone()) {
                full.insert(var.clone(), v);
            }
        }
    }
    let mut fills = full.clone();
    for (var, slice) in &manifest.slices {
        let whole = full.get(&slice.of).unwrap_or_else(|| panic!("`{var}` is a slice of `{}`, which nothing fills", slice.of));
        let (rows, cols) = slice.shape();
        fills.insert(var.clone(), (0..rows * cols).map(|i| whole[slice.full_index(i / cols, i % cols)]).collect());
    }
    written(src, &fills, spec.vocab)
}

/// The VM's logits and token (the token back to a 0-based id).
pub fn run(src: &str, vocab: usize) -> (Vec<f64>, u32) {
    let vm = vm_outcome(src);
    assert!(vm.error.is_none(), "VM error: {:?}", vm.error);
    let nums: Vec<f64> = vm.output.split_whitespace().map(|s| s.parse().unwrap()).collect();
    assert_eq!(nums.len(), vocab + 1, "the logits and the token");
    (nums[..vocab].to_vec(), nums[vocab] as u32 - 1)
}

pub fn assert_close(got: &[f64], want: &[f64], what: &str) {
    for (k, (g, w)) in got.iter().zip(want).enumerate() {
        assert!((g - w).abs() <= 1e-9 * w.abs().max(1.0), "{what}: logit {k} is {g}, the reference's {w}");
    }
}

/// The decode step's carried state as the program holds it after the prompt: each attention layer's `CTX`-row
/// caches (the prompt's positions filled, in the program's basis), and each Gated DeltaNet layer's memory and
/// convolution rings — a ring holds its last `taps` inputs, the oldest of which the step shifts out, so the
/// reference's `taps − 1` fill its newer rows.
pub fn carried(spec: &ModelSpec, reference: &Reference) -> HashMap<String, Vec<f64>> {
    let mut state = HashMap::new();
    for (l, layer) in spec.layers.iter().enumerate() {
        match &layer.mixer {
            Mixer::Attention(a) => {
                let (k, v) = reference.cached(l);
                let (akv, avd) = (a.kv_heads * a.head_dim, a.kv_heads * a.value_dim);
                let mut kc = vec![0f64; CTX * akv];
                let mut vc = vec![0f64; CTX * avd];
                for (p, (kr, vr)) in k.iter().zip(v).enumerate() {
                    let kr = match &a.rope {
                        Some(r) if !r.neox => neox_order(kr, a.head_dim, r.dims),
                        _ => kr.clone(),
                    };
                    kc[p * akv..(p + 1) * akv].copy_from_slice(&kr);
                    vc[p * avd..(p + 1) * avd].copy_from_slice(vr);
                }
                state.insert(format!("Kc{l}"), kc);
                state.insert(format!("Vc{l}"), vc);
            }
            Mixer::GatedDelta(g) => {
                let (qk, vw, taps) = (g.k_heads * g.head_dim, g.v_heads * g.head_dim, g.conv_taps);
                state.insert(format!("St{l}"), reference.memory(l).to_vec());
                for (var, first, width) in [("Csq", 0, qk), ("Csk", qk, qk), ("Csv", 2 * qk, vw)] {
                    let mut ring = vec![0f64; taps * width];
                    for (t, row) in reference.conv(l).iter().enumerate() {
                        ring[(t + 1) * width..(t + 2) * width].copy_from_slice(&row[first..first + width]);
                    }
                    state.insert(format!("{var}{l}"), ring);
                }
            }
            Mixer::Mla(_) => unreachable!("no shape here has latent attention"),
        }
    }
    state
}

/// Llama 4, small: three rotated layers (interleaved pairs) in chunks of 3 positions with the weightless QK norm after the
/// rotation, then a rotation-free layer whose queries take the attention temperature — its floor 2 here, not llama.cpp's
/// 8192, so the factor moves within a short context; a dense SiLU FFN on even layers and on odd ones a mixture scored by
/// sigmoids, one expert a token, its weight scaling the input, beside a shared expert.
/// A model routed as DeepSeek-V3 routes: a dense first layer, then two mixtures of sixteen experts scored by sigmoids,
/// each chosen by its score plus a selection bias among the top four of eight groups (DeepSeek-V3's grouping), the top
/// two kept, renormalized and scaled by 2.5, beside a shared expert.
pub fn deepseek_v3_spec() -> ModelSpec {
    let attention = Attention {
        heads: 4,
        kv_heads: 2,
        head_dim: 4,
        value_dim: 4,
        rope: Some(Rope { dims: 4, theta: 10000.0, neox: true, scaling: RopeScaling::None, sections: None }),
        qk_norm: false,
        output_gate: false,
        fused_qkv: false,
        bias: false,
        output_bias: false,
        window: None,
        sinks: false,
        softcap: None,
        scale: 0.5,
        temperature: None,
        qk_unit_norm: false,
        qk_norm_whole: false,
    };
    let moe = Ffn::Moe(Moe {
        experts: 16,
        top_k: 2,
        expert_dff: 8,
        act: Act::Silu,
        gating: Gating::Sigmoid { bias: true },
        norm_top_k: true,
        weights_scale: 2.5,
        shared_experts: 1,
        shared_dff: 8,
        shared_gate: false,
        biases: false,
        groups: Some((8, 4)),
    });
    ModelSpec {
        arch: "deepseek2".to_string(),
        d: 16,
        vocab: 40,
        ctx_len: CTX,
        rms_eps: 1e-6,
        embedding_scale: None,
        tied_output: false,
        final_logit_softcap: None,
        layers: (0..3)
            .map(|l| Layer {
                mixer: Mixer::Attention(attention.clone()),
                ffn: if l == 0 { Ffn::Dense { dff: 24, act: Act::Silu } } else { moe.clone() },
                post_mixer_norm: false,
                post_ffn_norm: false,
            })
            .collect(),
        mtp_blocks: 0,
        ffn_norm: "ffn_norm",
        hyper: None,
        ple: None,
        indexer: None,
    }
}

/// A model shaped as Qwen2-MoE: attention with biased projections under a NEOX rotation, and in every layer a mixture
/// of eight experts — the top two by a softmax, not renormalized — beside a shared expert twice a routed one's width,
/// scaled by its sigmoid gate.
pub fn qwen2_moe_spec() -> ModelSpec {
    let attention = Attention {
        heads: 4,
        kv_heads: 2,
        head_dim: 4,
        value_dim: 4,
        rope: Some(Rope { dims: 4, theta: 10000.0, neox: true, scaling: RopeScaling::None, sections: None }),
        qk_norm: false,
        output_gate: false,
        fused_qkv: false,
        bias: true,
        output_bias: false,
        window: None,
        sinks: false,
        softcap: None,
        scale: 0.5,
        temperature: None,
        qk_unit_norm: false,
        qk_norm_whole: false,
    };
    let moe = Ffn::Moe(Moe {
        experts: 8,
        top_k: 2,
        expert_dff: 8,
        act: Act::Silu,
        gating: Gating::Softmax,
        norm_top_k: false,
        weights_scale: 1.0,
        shared_experts: 1,
        shared_dff: 16,
        shared_gate: true,
        biases: false,
        groups: None,
    });
    ModelSpec {
        arch: "qwen2moe".to_string(),
        d: 16,
        vocab: 40,
        ctx_len: CTX,
        rms_eps: 1e-6,
        embedding_scale: None,
        tied_output: false,
        final_logit_softcap: None,
        layers: (0..2)
            .map(|_| Layer { mixer: Mixer::Attention(attention.clone()), ffn: moe.clone(), post_mixer_norm: false, post_ffn_norm: false })
            .collect(),
        mtp_blocks: 0,
        ffn_norm: "ffn_norm",
        hyper: None,
        ple: None,
        indexer: None,
    }
}

/// Qwen3.5's hybrid, small: two Gated DeltaNet blocks then gated attention. The attention rotates 6 of each head's
/// 8 dimensions, its three frequencies in sections `[1, 1, 0, 1]` — the third reads a text token's fourth position
/// stream and does not turn; the recurrence tiles 2 key heads over 4 value heads.
pub fn qwen35_spec() -> ModelSpec {
    let attention = Attention {
        heads: 2,
        kv_heads: 1,
        head_dim: 8,
        value_dim: 8,
        rope: Some(Rope {
            dims: 6,
            theta: 10000.0,
            neox: true,
            scaling: RopeScaling::None,
            sections: Some([1, 1, 0, 1]),
        }),
        qk_norm: true,
        output_gate: true,
        fused_qkv: false,
        bias: false,
        output_bias: false,
        window: None,
        sinks: false,
        softcap: None,
        scale: 1.0 / 8f64.sqrt(),
        temperature: None,
        qk_unit_norm: false,
        qk_norm_whole: false,
    };
    let gdn = GatedDelta { k_heads: 2, v_heads: 4, head_dim: 4, conv_taps: 4, gate: GateActivation::Silu };
    ModelSpec {
        arch: "qwen35".into(),
        d: 16,
        vocab: 40,
        ctx_len: CTX,
        rms_eps: 1e-6,
        embedding_scale: None,
        tied_output: false,
        final_logit_softcap: None,
        layers: (0..3)
            .map(|l| Layer {
                mixer: if l == 2 { Mixer::Attention(attention.clone()) } else { Mixer::GatedDelta(gdn.clone()) },
                ffn: Ffn::Dense { dff: 24, act: Act::Silu },
                post_mixer_norm: false,
                post_ffn_norm: false,
            })
            .collect(),
        mtp_blocks: 0,
        ffn_norm: "post_attention_norm",
        hyper: None,
        ple: None,
        indexer: None,
    }
}

/// Qwen3.5-MoE's shape: `qwen35_spec`'s hybrid with Qwen's mixture in every block — four experts, two a token, the kept
/// softmax weights renormalized — beside a shared expert twice a routed one's width, scaled by its sigmoid gate.
pub fn qwen35_moe_spec() -> ModelSpec {
    let moe = Ffn::Moe(Moe {
        experts: 4,
        top_k: 2,
        expert_dff: 8,
        act: Act::Silu,
        gating: Gating::Softmax,
        norm_top_k: true,
        weights_scale: 1.0,
        shared_experts: 1,
        shared_dff: 16,
        shared_gate: true,
        biases: false,
        groups: None,
    });
    let mut spec = qwen35_spec();
    spec.arch = "qwen35moe".to_string();
    for layer in &mut spec.layers {
        layer.ffn = moe.clone();
    }
    spec
}

pub fn llama4_spec() -> ModelSpec {
    let base = Attention {
        heads: 4,
        kv_heads: 2,
        head_dim: 4,
        value_dim: 4,
        rope: Some(Rope { dims: 4, theta: 10000.0, neox: false, scaling: RopeScaling::None, sections: None }),
        qk_norm: false,
        output_gate: false,
        fused_qkv: false,
        bias: false,
        output_bias: false,
        window: None,
        sinks: false,
        softcap: None,
        scale: 0.5,
        temperature: None,
        qk_unit_norm: false,
        qk_norm_whole: false,
    };
    let rotated = Attention { window: Some(Window::Chunked(3)), qk_unit_norm: true, ..base.clone() };
    let plain = Attention { rope: None, temperature: Some(Temperature { floor: 2.0, scale: 0.5 }), ..base };
    let moe = Ffn::Moe(Moe {
        experts: 4,
        top_k: 1,
        expert_dff: 8,
        act: Act::Silu,
        gating: Gating::SigmoidScaledInput,
        norm_top_k: false,
        weights_scale: 1.0,
        shared_experts: 1,
        shared_dff: 8,
        shared_gate: false,
        biases: false,
        groups: None,
    });
    ModelSpec {
        arch: "llama4".to_string(),
        d: 16,
        vocab: 40,
        ctx_len: CTX,
        rms_eps: 1e-5,
        embedding_scale: None,
        tied_output: false,
        final_logit_softcap: None,
        layers: (0..4)
            .map(|l| Layer {
                mixer: Mixer::Attention(if l == 3 { plain.clone() } else { rotated.clone() }),
                ffn: if l % 2 == 1 { moe.clone() } else { Ffn::Dense { dff: 24, act: Act::Silu } },
                post_mixer_norm: false,
                post_ffn_norm: false,
            })
            .collect(),
        mtp_blocks: 0,
        ffn_norm: "ffn_norm",
        hyper: None,
        ple: None,
        indexer: None,
    }
}
