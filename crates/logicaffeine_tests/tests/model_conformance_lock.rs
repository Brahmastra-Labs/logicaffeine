//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! THE AUTHORED PROGRAM COMPUTES WHAT THE MODEL COMPUTES — for every architecture, with no checkpoint.
//!
//! A model is brought up as data: its spec is read from its file, `model::author` writes the spec as a LOGOS
//! program, `model::bind` fills that program's weights from the checkpoint's tensors, and `model::reference` is what
//! the spec computes, in f64. This lock closes the loop on small models of every architecture the spec reads, built
//! in memory with weights that are exact in f64 (multiples of 2⁻¹⁰):
//!
//!   • a PREFILL CHUNK of the prompt, run on the VM, ends in the logits the reference computes for the prompt;
//!   • a DECODE STEP, its KV caches holding the reference's keys and values for the prompt, ends in the logits the
//!     reference computes for the next token — the cache read, the append and the masked attention included.
//!
//! The models differ where real checkpoints differ: QK-norm or not, a NEOX rotation or Llama's interleaved one (bound
//! through a per-head permutation of the query and key rows), a dense SiLU FFN or a softmax top-k mixture of experts,
//! an output head of its own or tied to the embedding — and Qwen3.5's hybrid: Gated DeltaNet blocks (more value
//! heads than key heads, a four-tap convolution, a delta-rule memory carried with the rings into the decode step)
//! beside gated attention with a partial, multi-section rotation. Each norm runs at the model's epsilon: 1e-6, the
//! sugar's own, or Llama 3's 1e-5, which the program states (`with epsilon`) on every norm.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

use std::collections::HashMap;

#[path = "common/model_harness.rs"]
mod model_harness;

use logicaffeine_compile::model::author;
use logicaffeine_compile::model::bind::{computed_tables, HostTable};
use logicaffeine_compile::model::reference::{argmax, Reference, WeightSource};
use logicaffeine_compile::model::spec::{
    Act, Attention, Ffn, GateActivation, Gating, Layer, LogitScale, Mixer, Mla, ModelSpec, Moe, Norm, Rope, RopeScaling, Temperature,
    MLA_LATENT_NORM_EPS,
};
use model_harness::{
    assert_close, carried, checkpoint, deepseek_v3_spec, filled, filled_split, llama4_spec, qwen2_moe_spec, qwen35_moe_spec,
    qwen35_spec, run, CTX, PROMPT,
};

#[derive(Clone, Copy)]
struct Shape {
    arch: &'static str,
    neox: bool,
    qk_norm: bool,
    moe: bool,
    tied: bool,
    hybrid: bool,
    eps: f64,
}

const SHAPES: &[Shape] = &[
    Shape { arch: "qwen3", neox: true, qk_norm: true, moe: false, tied: false, hybrid: false, eps: 1e-6 },
    Shape { arch: "qwen3moe", neox: true, qk_norm: true, moe: true, tied: false, hybrid: false, eps: 1e-6 },
    Shape { arch: "llama", neox: false, qk_norm: false, moe: false, tied: false, hybrid: false, eps: 1e-6 },
    Shape { arch: "llama", neox: false, qk_norm: false, moe: false, tied: true, hybrid: false, eps: 1e-6 },
    Shape { arch: "llama", neox: false, qk_norm: false, moe: false, tied: true, hybrid: false, eps: 1e-5 },
    Shape { arch: "qwen35", neox: true, qk_norm: true, moe: false, tied: false, hybrid: true, eps: 1e-6 },
    Shape { arch: "qwen35", neox: true, qk_norm: true, moe: false, tied: false, hybrid: true, eps: 1e-5 },
];

fn spec(s: Shape) -> ModelSpec {
    if s.hybrid {
        return ModelSpec { rms_eps: s.eps, ..qwen35_spec() };
    }
    let attention = Attention {
        heads: 4,
        kv_heads: 2,
        head_dim: 4,
        value_dim: 4,
        rope: Some(Rope { dims: 4, theta: 10000.0, neox: s.neox, scaling: RopeScaling::None, sections: None }),
        qk_norm: s.qk_norm,
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
    let ffn = if s.moe {
        Ffn::Moe(Moe {
            experts: 4,
            top_k: 2,
            expert_dff: 8,
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
        })
    } else {
        Ffn::Dense { dff: 24, act: Act::Silu }
    };
    ModelSpec {
        arch: s.arch.to_string(),
        d: 16,
        vocab: 40,
        ctx_len: CTX,
        rms_eps: s.eps,
        embedding_scale: None,
        tied_output: s.tied,
        final_logit_softcap: None,
        layers: (0..2)
            .map(|_| Layer {
                mixer: Mixer::Attention(attention.clone()),
                ffn: ffn.clone(),
                pre_norms: true,
                post_mixer_norm: false,
                post_ffn_norm: false,
            })
            .collect(),
        mtp_blocks: 0,
        ffn_norm: "ffn_norm",
        hyper: None,
        ple: None,
        indexer: None,
        residual_scale: None,
        logit_scale: None,
        norm: logicaffeine_compile::model::spec::Norm::Rms,
        head_bias: false,
        parts: Vec::new(),
    }
}

fn name(s: Shape) -> String {
    format!("{}{} at epsilon {:e}", s.arch, if s.tied { " (tied)" } else { "" }, s.eps)
}

#[test]
fn every_architecture_prefills_to_the_references_logits() {
    for &s in SHAPES {
        let spec = spec(s);
        author::check(&spec).unwrap_or_else(|e| panic!("{}: {e}", name(s)));
        let ckpt = checkpoint(&spec);
        let mut reference = Reference::new(spec.clone(), ckpt.clone()).expect("reference");
        let want = reference.step(PROMPT).expect("reference prefill");
        let toks: Vec<usize> = PROMPT.iter().map(|&t| t as usize + 1).collect();
        let prog = author::prefill(&spec, "/w", CTX, &toks, 1);
        let (got, next) = run(&filled(&spec, &prog, &ckpt, &HashMap::new()), spec.vocab);
        assert_close(&got, &want, &format!("{} prefill", name(s)));
        assert_eq!(next, argmax(&want), "{}: the prefill's next token", name(s));
    }
}

#[test]
fn every_architecture_decodes_a_step_to_the_references_logits() {
    for &s in SHAPES {
        let spec = spec(s);
        let ckpt = checkpoint(&spec);
        let mut reference = Reference::new(spec.clone(), ckpt.clone()).expect("reference");
        let prefill = reference.step(PROMPT).expect("reference prefill");
        let tok = argmax(&prefill);
        let caches = carried(&spec, &reference);
        let want = reference.step(&[tok]).expect("reference step");
        let prog = author::decode_step(&spec, "/w", CTX, tok as usize + 1, PROMPT.len() + 1);
        let (got, next) = run(&filled(&spec, &prog, &ckpt, &caches), spec.vocab);
        assert_close(&got, &want, &format!("{} decode step", name(s)));
        assert_eq!(next, argmax(&want), "{}: the step's next token", name(s));
    }
}

/// A mixture scored by sigmoids: its kept weights each scaling its expert's input (`SigmoidScaledInput`, Llama 4's) or
/// its output (`Sigmoid` without a selection bias), not renormalized and renormalized.
fn sigmoid_routed(gating: Gating, norm_top_k: bool) -> ModelSpec {
    let mut spec = spec(SHAPES[1]);
    for layer in &mut spec.layers {
        if let Ffn::Moe(m) = &mut layer.ffn {
            m.gating = gating.clone();
            m.norm_top_k = norm_top_k;
        }
    }
    spec
}

const SIGMOID_ROUTINGS: [(Gating, bool); 4] = [
    (Gating::SigmoidScaledInput, false),
    (Gating::SigmoidScaledInput, true),
    (Gating::Sigmoid { bias: false }, false),
    (Gating::Sigmoid { bias: false }, true),
];

#[test]
fn a_mixture_scored_by_sigmoids_prefills_and_decodes_to_the_references_logits() {
    for (gating, norm_top_k) in SIGMOID_ROUTINGS {
        let what = format!("{gating:?} (renormalized: {norm_top_k})");
        let spec = sigmoid_routed(gating.clone(), norm_top_k);
        author::check(&spec).unwrap_or_else(|e| panic!("{what}: {e}"));
        let ckpt = checkpoint(&spec);
        let mut reference = Reference::new(spec.clone(), ckpt.clone()).expect("reference");
        let want = reference.step(PROMPT).expect("reference prefill");
        let toks: Vec<usize> = PROMPT.iter().map(|&t| t as usize + 1).collect();
        let prog = author::prefill(&spec, "/w", CTX, &toks, 1);
        let (got, next) = run(&filled(&spec, &prog, &ckpt, &HashMap::new()), spec.vocab);
        assert_close(&got, &want, &format!("{what}: prefill"));
        assert_eq!(next, argmax(&want), "{what}: the prefill's next token");

        let tok = argmax(&want);
        let caches = carried(&spec, &reference);
        let step = reference.step(&[tok]).expect("reference step");
        let prog = author::decode_step(&spec, "/w", CTX, tok as usize + 1, PROMPT.len() + 1);
        let (got, next) = run(&filled(&spec, &prog, &ckpt, &caches), spec.vocab);
        assert_close(&got, &step, &format!("{what}: decode step"));
        assert_eq!(next, argmax(&step), "{what}: the step's next token");
    }
}

#[test]
fn each_sigmoid_routing_moves_the_logits() {
    // Not vacuous: on these weights every sigmoid routing differs from the softmax mixture's, and from each other.
    let logits = |spec: &ModelSpec| Reference::new(spec.clone(), checkpoint(spec)).unwrap().step(PROMPT).unwrap();
    let mut seen = vec![logits(&spec(SHAPES[1]))];
    for (gating, norm_top_k) in SIGMOID_ROUTINGS {
        let routed = logits(&sigmoid_routed(gating.clone(), norm_top_k));
        for other in &seen {
            let moved = routed.iter().zip(other).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
            assert!(moved > 1e-3, "{gating:?} (renormalized: {norm_top_k}): the logits move by only {moved}");
        }
        seen.push(routed);
    }
}

/// DeepSeek-V3's routing on the mixture of `SHAPES[1]` (four experts, top two): sigmoid scores, each expert chosen by its
/// score plus a selection bias where `bias`, among the experts of the better of two groups of two where `groups`, the
/// kept scores renormalized and scaled by `scale`.
fn deepseek_routed(bias: bool, groups: bool, scale: f64) -> ModelSpec {
    let mut spec = spec(SHAPES[1]);
    for layer in &mut spec.layers {
        if let Ffn::Moe(m) = &mut layer.ffn {
            m.gating = Gating::Sigmoid { bias };
            m.groups = groups.then_some((2, 1));
            m.weights_scale = scale;
            m.norm_top_k = true;
        }
    }
    spec
}

/// Each of DeepSeek-V3's clauses alone — the selection bias, the group limit, the routed scale — then all three.
const DEEPSEEK_ROUTINGS: [(bool, bool, f64); 4] = [(true, false, 1.0), (false, true, 1.0), (false, false, 2.5), (true, true, 2.5)];

#[test]
fn deepseek_v3s_routing_prefills_and_decodes_to_the_references_logits() {
    for (bias, groups, scale) in DEEPSEEK_ROUTINGS {
        let what = format!("biased {bias}, grouped {groups}, scaled by {scale}");
        let spec = deepseek_routed(bias, groups, scale);
        author::check(&spec).unwrap_or_else(|e| panic!("{what}: {e}"));
        let ckpt = checkpoint(&spec);
        let mut reference = Reference::new(spec.clone(), ckpt.clone()).expect("reference");
        let want = reference.step(PROMPT).expect("reference prefill");
        let toks: Vec<usize> = PROMPT.iter().map(|&t| t as usize + 1).collect();
        let prog = author::prefill(&spec, "/w", CTX, &toks, 1);
        let (got, next) = run(&filled(&spec, &prog, &ckpt, &HashMap::new()), spec.vocab);
        assert_close(&got, &want, &format!("{what}: prefill"));
        assert_eq!(next, argmax(&want), "{what}: the prefill's next token");

        let tok = argmax(&want);
        let caches = carried(&spec, &reference);
        let step = reference.step(&[tok]).expect("reference step");
        let prog = author::decode_step(&spec, "/w", CTX, tok as usize + 1, PROMPT.len() + 1);
        let (got, next) = run(&filled(&spec, &prog, &ckpt, &caches), spec.vocab);
        assert_close(&got, &step, &format!("{what}: decode step"));
        assert_eq!(next, argmax(&step), "{what}: the step's next token");
    }
}

#[test]
fn each_of_deepseek_v3s_clauses_moves_the_logits() {
    // Not vacuous: on these weights each clause changes the prefill's logits from the plain renormalized sigmoid
    // routing's, and the routings differ from each other.
    let logits = |spec: &ModelSpec| Reference::new(spec.clone(), checkpoint(spec)).unwrap().step(PROMPT).unwrap();
    let mut seen = vec![logits(&deepseek_routed(false, false, 1.0))];
    for (bias, groups, scale) in DEEPSEEK_ROUTINGS {
        let routed = logits(&deepseek_routed(bias, groups, scale));
        for other in &seen {
            let moved = routed.iter().zip(other).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
            assert!(moved > 1e-3, "biased {bias}, grouped {groups}, scaled by {scale}: the logits move by only {moved}");
        }
        seen.push(routed);
    }
}

#[test]
fn deepseek_v3s_routed_model_prefills_and_decodes_to_the_references_logits() {
    let spec = deepseek_v3_spec();
    author::check(&spec).unwrap_or_else(|e| panic!("deepseek-v3 routing: {e}"));
    let ckpt = checkpoint(&spec);
    let mut reference = Reference::new(spec.clone(), ckpt.clone()).expect("reference");
    let want = reference.step(PROMPT).expect("reference prefill");
    let toks: Vec<usize> = PROMPT.iter().map(|&t| t as usize + 1).collect();
    let prog = author::prefill(&spec, "/w", CTX, &toks, 1);
    let (got, next) = run(&filled(&spec, &prog, &ckpt, &HashMap::new()), spec.vocab);
    assert_close(&got, &want, "deepseek-v3 routing prefill");
    assert_eq!(next, argmax(&want), "deepseek-v3 routing: the prefill's next token");

    let tok = argmax(&want);
    let caches = carried(&spec, &reference);
    let step = reference.step(&[tok]).expect("reference step");
    let prog = author::decode_step(&spec, "/w", CTX, tok as usize + 1, PROMPT.len() + 1);
    let (got, next) = run(&filled(&spec, &prog, &ckpt, &caches), spec.vocab);
    assert_close(&got, &step, "deepseek-v3 routing decode step");
    assert_eq!(next, argmax(&step), "deepseek-v3 routing: the step's next token");
}

#[test]
fn a_group_limit_the_experts_cannot_hold_is_refused() {
    // Four experts: three groups do not split them evenly, and one group of two cannot hold the top two plus a third.
    for (groups, top_k) in [((3, 1), 2), ((2, 1), 3), ((2, 3), 2), ((0, 0), 2)] {
        let mut spec = deepseek_routed(true, true, 2.5);
        for layer in &mut spec.layers {
            if let Ffn::Moe(m) = &mut layer.ffn {
                m.groups = Some(groups);
                m.top_k = top_k;
            }
        }
        let refused = author::check(&spec).expect_err(&format!("{groups:?} choosing {top_k}: refused"));
        assert!(refused.to_string().contains("group limit"), "{groups:?} choosing {top_k}: {refused}");
    }
    // A group limit over a softmax of the kept logits would rank its groups by logits where the sugar ranks scores.
    let mut spec = deepseek_routed(false, true, 1.0);
    for layer in &mut spec.layers {
        if let Ffn::Moe(m) = &mut layer.ffn {
            m.gating = Gating::SoftmaxOfTopK;
        }
    }
    assert!(author::check(&spec).expect_err("a softmax of the kept logits limited to groups").to_string().contains("group limit"));
}

/// `SHAPES[1]`'s mixture beside a shared expert twice a routed one's width, scaled by its sigmoid gate when `gate`.
fn shared_gated(gate: bool) -> ModelSpec {
    let mut spec = spec(SHAPES[1]);
    for layer in &mut spec.layers {
        if let Ffn::Moe(m) = &mut layer.ffn {
            m.shared_experts = 1;
            m.shared_dff = 16;
            m.shared_gate = gate;
        }
    }
    spec
}

/// `spec` prefilled and stepped once on the VM against the reference.
fn prefills_and_decodes(spec: &ModelSpec, what: &str) {
    author::check(spec).unwrap_or_else(|e| panic!("{what}: {e}"));
    let ckpt = checkpoint(spec);
    let mut reference = Reference::new(spec.clone(), ckpt.clone()).expect("reference");
    let want = reference.step(PROMPT).expect("reference prefill");
    let toks: Vec<usize> = PROMPT.iter().map(|&t| t as usize + 1).collect();
    let prog = author::prefill(spec, "/w", CTX, &toks, 1);
    let (got, next) = run(&filled(spec, &prog, &ckpt, &HashMap::new()), spec.vocab);
    assert_close(&got, &want, &format!("{what}: prefill"));
    assert_eq!(next, argmax(&want), "{what}: the prefill's next token");
    let tok = argmax(&want);
    let caches = carried(spec, &reference);
    let step = reference.step(&[tok]).expect("reference step");
    let prog = author::decode_step(spec, "/w", CTX, tok as usize + 1, PROMPT.len() + 1);
    let (got, next) = run(&filled(spec, &prog, &ckpt, &caches), spec.vocab);
    assert_close(&got, &step, &format!("{what}: decode step"));
    assert_eq!(next, argmax(&step), "{what}: the step's next token");
}

#[test]
fn a_sigmoid_gated_deltanet_prefills_and_decodes_to_the_reference() {
    // Qwen3.8-Flash-Next's Gated DeltaNet gates its normed output by σ(z) where Qwen 3.5's gates by silu(z) (QWEN4-8):
    // written `with sigmoid gate`, the prompt and a step equal the reference, and the gate moves the logits.
    let silu = qwen35_spec();
    let mut spec = silu.clone();
    let mut gated = 0;
    for layer in &mut spec.layers {
        if let Mixer::GatedDelta(g) = &mut layer.mixer {
            g.gate = GateActivation::Sigmoid;
            gated += 1;
        }
    }
    assert!(gated > 0, "the hybrid has Gated DeltaNet layers");
    prefills_and_decodes(&spec, "a sigmoid-gated DeltaNet");
    let ckpt = checkpoint(&spec);
    let logits = |s: &ModelSpec| Reference::new(s.clone(), ckpt.clone()).expect("reference").step(PROMPT).expect("reference prefill");
    let (a, b) = (logits(&spec), logits(&silu));
    let scale = a.iter().fold(0.0f64, |m, x| m.max(x.abs()));
    assert!(a.iter().zip(&b).any(|(x, y)| (x - y).abs() > 1e-3 * scale), "the gate moves the logits");
}

#[test]
fn query_key_and_value_biases_without_an_output_bias_match_the_reference() {
    // Qwen2's attention: its query, key and value projections biased, its output projection not, as its checkpoints
    // carry them; written with no output bias to bind.
    let mut spec = qwen2_moe_spec();
    for layer in &mut spec.layers {
        layer.ffn = Ffn::Dense { dff: 24, act: Act::Silu };
    }
    let ckpt = checkpoint(&spec);
    assert!(
        ckpt.has("blk.0.attn_q.bias") && !ckpt.has("blk.0.attn_output.bias"),
        "the checkpoint carries query biases and no output bias"
    );
    prefills_and_decodes(&spec, "Qwen2's biased attention");
}

#[test]
fn a_scaled_attention_equals_the_reference() {
    // A score scale other than 1/√hd, as Gemma 3 27B's 1/√168 over heads of 128: the attention written `scaled by S`,
    // every score `q·k·S`, on the dense Qwen3 shape and on the Llama shape with its interleaved rotation.
    for (what, shape) in [("Qwen3", SHAPES[0]), ("Llama", SHAPES[2])] {
        let mut spec = spec(shape);
        for layer in &mut spec.layers {
            if let Mixer::Attention(a) = &mut layer.mixer {
                a.scale = 1.0 / (a.head_dim as f64 * 1.3125).sqrt();
            }
        }
        prefills_and_decodes(&spec, &format!("{what} with a score scale of 1/√(1.3125·hd)"));
    }
}

/// The dense Qwen3 shape (NEOX rotation, QK-norm) and the Llama shape (interleaved rotation), each attention changed by
/// `change`.
fn with_attention(change: impl Fn(&mut Attention)) -> Vec<(&'static str, ModelSpec)> {
    [("Qwen3", SHAPES[0]), ("Llama", SHAPES[2])]
        .map(|(what, shape)| {
            let mut spec = spec(shape);
            for layer in &mut spec.layers {
                if let Mixer::Attention(a) = &mut layer.mixer {
                    change(a);
                }
            }
            (what, spec)
        })
        .into()
}

#[test]
fn a_value_width_unlike_the_key_width_equals_the_reference() {
    // Values narrower and wider than the keys: the value projection, the value cache and each head's attended row
    // `value_dim` wide, the output projection reading `heads · value_dim`.
    for value_dim in [2, 8] {
        for (what, spec) in with_attention(|a| a.value_dim = value_dim) {
            prefills_and_decodes(&spec, &format!("{what} with values {value_dim} wide over keys 4 wide"));
        }
    }
}

#[test]
fn fused_qkv_equals_the_reference() {
    // Phi-3's one projection: each output row the queries, then the keys, then the values, its bias likewise; under the
    // NEOX rotation and under Llama's interleaved one, whose query and key rows the program reads in halves.
    for bias in [false, true] {
        for (what, spec) in with_attention(|a| (a.fused_qkv, a.bias) = (true, bias)) {
            prefills_and_decodes(&spec, &format!("{what} with a fused projection{}", if bias { " and its bias" } else { "" }));
        }
    }
    let (_, wide) = with_attention(|a| (a.fused_qkv, a.value_dim) = (true, 8)).remove(0);
    prefills_and_decodes(&wide, "Qwen3 with a fused projection over values 8 wide");
}

#[test]
fn a_qk_norm_across_every_head_equals_the_reference() {
    // MiniMax-M2's query and key, each normed across every head at once before the rotation: one segment as wide as the
    // projection, its weight as wide. Split over chips that each hold whole KV heads it is legal — each chip's part of
    // the row is in the norm once, its sum of squares all-reduced; over more chips than KV heads it is refused by name.
    for (what, spec) in with_attention(|a| (a.qk_norm, a.qk_norm_whole) = (true, true)) {
        prefills_and_decodes(&spec, &format!("{what} with its QK norm across every head"));
        let kv = spec
            .layers
            .iter()
            .find_map(|l| match &l.mixer {
                Mixer::Attention(a) => Some(a.kv_heads),
                _ => None,
            })
            .expect("an attention");
        for tp in 1..=kv {
            author::check_tp(&spec, tp).unwrap_or_else(|e| panic!("{what}: a QK norm across every head over {tp} of {kv} KV heads' chips: {e}"));
        }
        let refused = author::check_tp(&spec, kv + 1).expect_err("a QK norm across every head over more chips than KV heads");
        assert!(refused.0.contains("across every head"), "{what}: {refused}");
    }
}

#[test]
fn a_softcapped_attention_equals_the_reference() {
    // Gemma 2's scores, each `C·tanh(s/C)` before its softmax: a cap of 0.5 binds on these weights' scores, which reach
    // past it, and a cap of 50 (Gemma 2's own) leaves them all but unmoved.
    for cap in [0.5, 50.0] {
        for (what, spec) in with_attention(|a| a.softcap = Some(cap)) {
            prefills_and_decodes(&spec, &format!("{what} with its scores capped at {cap}"));
        }
    }
}

#[test]
fn a_final_logit_softcap_equals_the_reference() {
    // Gemma 2's logits, each `C·tanh(z/C)`: a cap of 0.25 binds on these weights' logits, 30 (Gemma 2's own) does not.
    for cap in [0.25, 30.0] {
        for (what, mut spec) in with_attention(|_| ()) {
            spec.final_logit_softcap = Some(cap);
            prefills_and_decodes(&spec, &format!("{what} with its logits capped at {cap}"));
        }
    }
}

/// `spec`'s decode step split over `tp` chips, Megatron's way, on the VM against the reference's step: each chip's
/// slices filled from the full tensors through the manifest, the caches holding the reference's prompt.
fn split_decodes(spec: &ModelSpec, tp: usize, what: &str) {
    author::check_tp(spec, tp).unwrap_or_else(|e| panic!("{what} over {tp} chips: {e}"));
    let ckpt = checkpoint(spec);
    let mut reference = Reference::new(spec.clone(), ckpt.clone()).expect("reference");
    let tok = argmax(&reference.step(PROMPT).expect("reference prefill"));
    let caches = carried(spec, &reference);
    let step = reference.step(&[tok]).expect("reference step");
    let prog = author::tp_decode_step(spec, "/w", CTX, tok as usize + 1, PROMPT.len() + 1, tp)
        .unwrap_or_else(|e| panic!("{what} over {tp} chips: {e}"));
    let (got, next) = run(&filled_split(spec, &prog, &ckpt, &caches), spec.vocab);
    assert_close(&got, &step, &format!("{what}: the decode step over {tp} chips"));
    assert_eq!(next, argmax(&step), "{what}: the step's next token over {tp} chips");
}

#[test]
fn each_attention_the_author_writes_splits_over_chips_as_the_reference_computes() {
    // Two chips take a KV head each; three split the query heads of the KV heads unevenly. Each chip's value columns,
    // value-cache columns and output-projection rows are its heads' `value_dim` apiece; a fused projection's parts, the
    // scale and the caps ride in each chip's copy of the clauses.
    let variants: [(&str, fn(&mut Attention)); 6] = [
        ("values 2 wide", |a| a.value_dim = 2),
        ("values 8 wide", |a| a.value_dim = 8),
        ("a fused projection", |a| a.fused_qkv = true),
        ("a fused projection over values 8 wide", |a| (a.fused_qkv, a.value_dim) = (true, 8)),
        ("a scale of 1/√(1.3125·hd)", |a| a.scale = 1.0 / (a.head_dim as f64 * 1.3125).sqrt()),
        ("its scores capped at 0.5", |a| a.softcap = Some(0.5)),
    ];
    for tp in [2, 3] {
        for (feature, change) in variants {
            for (what, spec) in with_attention(change) {
                split_decodes(&spec, tp, &format!("{what} with {feature}"));
            }
        }
        for (what, mut spec) in with_attention(|_| ()) {
            spec.final_logit_softcap = Some(0.25);
            split_decodes(&spec, tp, &format!("{what} with its logits capped at 0.25"));
        }
    }
}

#[test]
fn qwen35_moe_prefills_and_decodes_to_the_references_logits() {
    // Qwen3.5-MoE: Gated DeltaNet and gated attention with Qwen's mixture in every block, the kept softmax weights
    // renormalized, a shared expert scaled by its gate.
    prefills_and_decodes(&qwen35_moe_spec(), "Qwen3.5-MoE's shape");
}

#[test]
fn a_gated_shared_expert_prefills_and_decodes_to_the_references_logits() {
    for (what, spec) in [("a gated shared expert", shared_gated(true)), ("a Qwen2-MoE-shaped model", qwen2_moe_spec())] {
        author::check(&spec).unwrap_or_else(|e| panic!("{what}: {e}"));
        let ckpt = checkpoint(&spec);
        let mut reference = Reference::new(spec.clone(), ckpt.clone()).expect("reference");
        let want = reference.step(PROMPT).expect("reference prefill");
        let toks: Vec<usize> = PROMPT.iter().map(|&t| t as usize + 1).collect();
        let prog = author::prefill(&spec, "/w", CTX, &toks, 1);
        let (got, next) = run(&filled(&spec, &prog, &ckpt, &HashMap::new()), spec.vocab);
        assert_close(&got, &want, &format!("{what}: prefill"));
        assert_eq!(next, argmax(&want), "{what}: the prefill's next token");

        let tok = argmax(&want);
        let caches = carried(&spec, &reference);
        let step = reference.step(&[tok]).expect("reference step");
        let prog = author::decode_step(&spec, "/w", CTX, tok as usize + 1, PROMPT.len() + 1);
        let (got, next) = run(&filled(&spec, &prog, &ckpt, &caches), spec.vocab);
        assert_close(&got, &step, &format!("{what}: decode step"));
        assert_eq!(next, argmax(&step), "{what}: the step's next token");
    }
}

#[test]
fn the_shared_gate_moves_the_logits() {
    // Not vacuous: the gate scales the shared expert by a factor in (0, 1) that moves with the token.
    let logits = |spec: &ModelSpec| Reference::new(spec.clone(), checkpoint(spec)).unwrap().step(PROMPT).unwrap();
    let moved = logits(&shared_gated(true)).iter().zip(logits(&shared_gated(false))).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
    assert!(moved > 1e-3, "the shared gate moves the logits by only {moved}");
}

#[test]
fn every_host_table_a_step_declares_is_one_the_host_computes() {
    // The RDU bundle writes `bind::computed_tables` beside the rotary tables: a table a step declares that the list
    // misses would reach the chip unwritten.
    let models = [
        ("a gated shared expert", shared_gated(true)),
        ("Qwen2-MoE's shape", qwen2_moe_spec()),
        ("Llama 4", llama4_spec()),
        ("DeepSeek-V3", deepseek_v3_spec()),
    ];
    for (what, spec) in models {
        let computed = computed_tables(&spec);
        let (_, f32s, _) = author::decode_step(&spec, "/w", CTX, 1, 1);
        for table in f32s.keys().filter_map(|n| HostTable::named(n)).filter(|t| !matches!(t, HostTable::Rotary(_))) {
            assert!(computed.contains(&table), "{what}: `{}` is declared and not computed: {computed:?}", table.name());
        }
    }
    // Not vacuous: the gated step declares the row of ones that spreads its gate across the row.
    let spec = shared_gated(true);
    let (_, f32s, _) = author::decode_step(&spec, "/w", CTX, 1, 1);
    assert!(f32s.contains_key(&HostTable::Ones(spec.d).name()), "the gate's spread is a host table");
}

#[test]
fn llama4_prefills_and_decodes_to_the_references_logits() {
    let spec = llama4_spec();
    author::check(&spec).unwrap_or_else(|e| panic!("llama4: {e}"));
    let ckpt = checkpoint(&spec);
    let mut reference = Reference::new(spec.clone(), ckpt.clone()).expect("reference");
    let want = reference.step(PROMPT).expect("reference prefill");
    let toks: Vec<usize> = PROMPT.iter().map(|&t| t as usize + 1).collect();
    let prog = author::prefill(&spec, "/w", CTX, &toks, 1);
    let (got, next) = run(&filled(&spec, &prog, &ckpt, &HashMap::new()), spec.vocab);
    assert_close(&got, &want, "llama4 prefill");
    assert_eq!(next, argmax(&want), "llama4: the prefill's next token");

    let tok = argmax(&want);
    let caches = carried(&spec, &reference);
    let step = reference.step(&[tok]).expect("reference step");
    let prog = author::decode_step(&spec, "/w", CTX, tok as usize + 1, PROMPT.len() + 1);
    let (got, next) = run(&filled(&spec, &prog, &ckpt, &caches), spec.vocab);
    assert_close(&got, &step, "llama4 decode step");
    assert_eq!(next, argmax(&step), "llama4: the step's next token");
}

#[test]
fn llama4s_chunks_norm_and_temperature_each_move_the_logits() {
    // Not vacuous: on these weights and this prompt (positions 0 to 4, across the chunk boundary at 3) each Llama 4
    // feature changes the reference's logits.
    let logits = |spec: &ModelSpec| {
        let mut r = Reference::new(spec.clone(), checkpoint(spec)).unwrap();
        r.step(PROMPT).unwrap();
        r.step(&[7]).unwrap()
    };
    let full = logits(&llama4_spec());
    let without = |f: &dyn Fn(&mut Attention)| {
        let mut spec = llama4_spec();
        for layer in &mut spec.layers {
            if let Mixer::Attention(a) = &mut layer.mixer {
                f(a);
            }
        }
        logits(&spec)
    };
    for (what, other) in [
        ("the chunks", without(&|a| a.window = None)),
        ("the weightless QK norm", without(&|a| a.qk_unit_norm = false)),
        ("the temperature", without(&|a| a.temperature = None)),
    ] {
        let moved = full.iter().zip(&other).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
        assert!(moved > 1e-3, "{what} moves the logits by only {moved}");
    }
}

/// Ministral 3's attention (`models/mistral3.cpp`): the Llama shape under YaRN, every layer's rotated queries scaled by
/// the attention temperature, its floors of two positions counted from zero.
fn ministral3_spec(offset: f64) -> ModelSpec {
    let mut spec = spec(SHAPES[2]);
    spec.arch = "mistral3".to_string();
    for layer in &mut spec.layers {
        if let Mixer::Attention(a) = &mut layer.mixer {
            if let Some(r) = a.rope.as_mut() {
                let magnitude = 1.0 + 0.1 * 16f64.ln();
                r.scaling = RopeScaling::Yarn { factor: 16.0, original_ctx: 2, beta_fast: 32.0, beta_slow: 1.0, magnitude };
            }
            a.temperature = Some(Temperature { floor: 2.0, scale: 0.5, offset });
        }
    }
    spec
}

#[test]
fn ministral3s_temperature_on_every_layer_prefills_and_decodes_to_the_references_logits() {
    prefills_and_decodes(&ministral3_spec(0.0), "ministral3");
}

#[test]
fn ministral3s_temperature_and_where_it_counts_from_each_move_the_logits() {
    // Not vacuous: the prompt's positions 0 to 3 fall in floors 0, 0, 1, 1 counted from zero and 0, 1, 1, 2 counted from
    // one (Llama 4's), so its last query, at 3, takes another factor either way; and the temperature scales every rotated
    // layer's queries.
    let logits = |spec: &ModelSpec| Reference::new(spec.clone(), checkpoint(spec)).unwrap().step(PROMPT).unwrap();
    let ministral = logits(&ministral3_spec(0.0));
    let mut untempered = ministral3_spec(0.0);
    for layer in &mut untempered.layers {
        if let Mixer::Attention(a) = &mut layer.mixer {
            a.temperature = None;
        }
    }
    for (what, other) in [("the temperature", logits(&untempered)), ("counting positions from one", logits(&ministral3_spec(1.0)))] {
        let moved = ministral.iter().zip(&other).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
        assert!(moved > 1e-3, "{what} moves the logits by only {moved}");
    }
}

/// GLM-5's latent attention at its epsilon of 10⁻⁵ on two blocks: a low-rank query, the latent's expansion split, an
/// interleaved rotation; its latent norms at 10⁻⁶, as GLM-5's code builds them, every other norm at the model's.
fn latent_attention_spec() -> ModelSpec {
    let mla = Mla {
        heads: 4,
        q_lora_rank: Some(8),
        kv_lora_rank: 8,
        qk_nope: 4,
        qk_rope: 4,
        v_dim: 4,
        rope: Some(Rope { dims: 4, theta: 10000.0, neox: false, scaling: RopeScaling::None, sections: None }),
        scale: 1.0 / 8f64.sqrt(),
        kv_b_split: true,
        indexer: None,
        latent_norm_eps: MLA_LATENT_NORM_EPS,
    };
    ModelSpec {
        arch: "deepseek2".into(),
        d: 16,
        vocab: 40,
        ctx_len: CTX,
        rms_eps: 1e-5,
        embedding_scale: None,
        tied_output: false,
        final_logit_softcap: None,
        layers: (0..2)
            .map(|_| Layer {
                mixer: Mixer::Mla(mla.clone()),
                ffn: Ffn::Dense { dff: 24, act: Act::Silu },
                pre_norms: true,
                post_mixer_norm: false,
                post_ffn_norm: false,
            })
            .collect(),
        mtp_blocks: 0,
        ffn_norm: "ffn_norm",
        hyper: None,
        ple: None,
        indexer: None,
        residual_scale: None,
        logit_scale: None,
        norm: Norm::Rms,
        head_bias: false,
        parts: Vec::new(),
    }
}

#[test]
fn a_latent_attention_whose_latent_norms_take_their_own_epsilon_prefills_and_decodes_to_the_references_logits() {
    // The program states the latent norms' 10⁻⁶ on their own statements and the model's 10⁻⁵ on every other norm.
    let spec = latent_attention_spec();
    let (prog, _, _) = author::prefill(&spec, "/w", CTX, &[1, 2], 1);
    let latent: Vec<&str> = prog.lines().filter(|l| l.contains("with weight Qan") || l.contains("with weight Kan")).collect();
    assert_eq!(latent.len(), 4, "a query norm and a latent norm a block: {latent:?}");
    assert!(latent.iter().all(|l| l.ends_with("with epsilon 1e-6.")), "the latent norms at 10⁻⁶: {latent:?}");
    let model: Vec<&str> = prog.lines().filter(|l| l.contains(" be rmsnorm of ") && !latent.contains(l)).collect();
    assert!(!model.is_empty() && model.iter().all(|l| l.ends_with("with epsilon 1e-5.")), "every other norm at 10⁻⁵: {model:?}");
    prefills_and_decodes(&spec, "GLM-5's latent attention at 10⁻⁵");
}

#[test]
fn a_latent_attentions_own_epsilon_moves_the_logits() {
    // Not vacuous: the latent norms at the model's 10⁻⁵, as llama.cpp's latent-attention graphs take them, move the
    // logits far past the bound the program and the reference agree within.
    let spec = latent_attention_spec();
    let mut at_the_model = spec.clone();
    for layer in &mut at_the_model.layers {
        if let Mixer::Mla(m) = &mut layer.mixer {
            m.latent_norm_eps = spec.rms_eps;
        }
    }
    let logits = |s: &ModelSpec| Reference::new(s.clone(), checkpoint(s)).unwrap().step(PROMPT).unwrap();
    let (own, model) = (logits(&spec), logits(&at_the_model));
    let moved = own.iter().zip(&model).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
    assert!(moved > 1e-7, "the latent norms' epsilon moves the logits by only {moved}");
}

/// The Llama shape with Granite's and MiniCPM's multipliers: the embedding scaled by 12, each sublayer's output scaled by
/// `residual` before it joins the residual, and the logit scale `logit` where its model applies it.
fn multiplied(residual: Option<f64>, logit: Option<LogitScale>) -> ModelSpec {
    ModelSpec { embedding_scale: Some(12.0), residual_scale: residual, logit_scale: logit, ..spec(SHAPES[2]) }
}

#[test]
fn granites_multipliers_prefill_and_decode_to_the_references_logits() {
    // Granite: the logits divided by `logits_scaling` after the head, each sublayer's output times `residual_multiplier`,
    // the embedding times `embedding_multiplier`, every score times `attention_multiplier` in place of 1/√hd.
    let mut spec = multiplied(Some(0.21875), Some(LogitScale::AfterHead(0.125)));
    for layer in &mut spec.layers {
        if let Mixer::Attention(a) = &mut layer.mixer {
            a.scale = 0.0078125;
        }
    }
    prefills_and_decodes(&spec, "Granite's multipliers");
}

#[test]
fn minicpms_scaled_head_prefills_and_decodes_to_the_references_logits() {
    // MiniCPM: the output norm's result times `dim_model_base / d` before the head, each sublayer's output times
    // `scale_depth / √layers`.
    let spec = multiplied(Some(1.4 / 2f64.sqrt()), Some(LogitScale::BeforeHead(0.125)));
    prefills_and_decodes(&spec, "MiniCPM's scaled head");
}

#[test]
fn each_multiplier_moves_the_logits() {
    // Not vacuous: the reference applies each multiplier, where its model applies it.
    let logits = |spec: &ModelSpec| Reference::new(spec.clone(), checkpoint(spec)).unwrap().step(PROMPT).unwrap();
    let plain = logits(&spec(SHAPES[2]));
    let cases = [
        ("a residual scale", ModelSpec { residual_scale: Some(0.25), ..spec(SHAPES[2]) }),
        ("a logit scale after the head", ModelSpec { logit_scale: Some(LogitScale::AfterHead(0.25)), ..spec(SHAPES[2]) }),
        ("a logit scale before the head", ModelSpec { logit_scale: Some(LogitScale::BeforeHead(0.25)), ..spec(SHAPES[2]) }),
    ];
    for (what, spec) in cases {
        let scaled = logits(&spec);
        let moved = scaled.iter().zip(&plain).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
        assert!(moved > 1e-3, "{what}: the logits move by only {moved}");
    }
}

#[test]
fn an_ungated_relu_squared_ffn_prefills_and_decodes_to_the_references_logits() {
    // Arcee's FFN (`models/arcee.cpp`): relu of the up projection, squared, then the down projection, with no gate.
    let mut spec = spec(SHAPES[2]);
    for layer in &mut spec.layers {
        layer.ffn = Ffn::Dense { dff: 24, act: Act::ReluSquared };
    }
    prefills_and_decodes(&spec, "a relu² FFN");
}
