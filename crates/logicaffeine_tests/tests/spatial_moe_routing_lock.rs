//! spatial_moe_routing_lock — DeepSeek-V3's routing in a whole decode step on a spatial chip: experts scored by sigmoids,
//! each chosen by its score plus a selection bias among the top groups, the kept scores renormalized over a sum clamped
//! at 2⁻¹⁴ and scaled, beside a shared expert.
//!
//! The model is `model_harness::deepseek_v3_spec` — a dense first layer, then two mixtures of sixteen experts in eight
//! groups of two, the top four groups kept (DeepSeek-V3's grouping), the top two experts chosen, scaled by 2.5. Its
//! decode step after a four-token prompt, authored with every weight, selection bias and cache written into the program,
//! is lowered whole and run in the spatial evaluator:
//!
//!   • the authored step is DeepSeek-V3's: the VM's logits are the f64 reference's, so the chip is held to the model's
//!     own routing, not to whatever routing the program happens to write;
//!   • WIRING — on an fp32 twin of the SN50 (fp32 in place of its bf16) the logits are the VM's within a relative 2⁻¹⁶;
//!   • CHIP PRECISION — on the SN50 they are the VM's within 2% of the largest, every routing call of the step decided
//!     by more than 2⁻⁶ of its score, past what the chip's bf16 router inputs can move;
//!   • the selection bias chooses and never weighs: the same bias added to every expert leaves the twin's logits
//!     bit-identical, and the step's own bias moves the VM's logits, so the agreement is not vacuous.
//!
//! And Qwen2-MoE's shape (`model_harness::qwen2_moe_spec`: eight experts by a softmax beside a shared expert twice a
//! routed one's width, scaled by its sigmoid gate) and Qwen3.5-MoE's (`model_harness::qwen35_moe_spec`: the same mixture
//! renormalized, behind Gated DeltaNet and gated attention) are held to the same rule.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test spatial_moe_routing_lock`

#![cfg(feature = "codegen")]

#[path = "common/model_harness.rs"]
mod model_harness;
#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::model::author;
use logicaffeine_compile::model::reference::{argmax, Reference, RoutingCall};
use logicaffeine_compile::model::spec::{Ffn, Gating, ModelSpec};
use logicaffeine_compile::spatial::eval::{eval_choreography_every_tensor, Inputs, Value};
use logicaffeine_compile::spatial::ir::SpatialProgram;
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::{ComputeCap, SpatialTarget};
use model_harness::{assert_close, carried, checkpoint, deepseek_v3_spec, filled, qwen2_moe_spec, qwen35_moe_spec, run, CTX, PROMPT};
use spatial_support::inputs_from_fills;

/// A chip with fp32 in place of bf16 on the matmul, pointwise and row units: every value of the step is fp32.
fn fp32_twin() -> SpatialTarget {
    let mut t = SpatialTarget::sn50_rdu();
    for c in &mut t.compute {
        if let ComputeCap::MatmulTile { precisions, .. } | ComputeCap::Elementwise { precisions } | ComputeCap::RowReduce { precisions } = c {
            if precisions.remove(&Precision::Bf16) {
                precisions.insert(Precision::Fp32);
            }
        }
    }
    t.precisions.remove(&Precision::Bf16);
    t.precisions.insert(Precision::Fp32);
    t
}

/// The decode step of `spec` after the prompt, every weight, selection bias and cache written into it, the VM's logits,
/// the reference's, and the closest routing call the reference made over the prompt and the step.
fn decode_step(spec: &ModelSpec) -> (String, Vec<f64>, Vec<f64>, RoutingCall) {
    let ckpt = checkpoint(spec);
    let mut reference = Reference::new(spec.clone(), ckpt.clone()).expect("reference");
    let prefill = reference.step(PROMPT).expect("reference prefill");
    let tok = argmax(&prefill);
    let caches = carried(spec, &reference);
    let want = reference.step(&[tok]).expect("reference step");
    let closest = reference.closest_routing().expect("a model with experts routes");
    let prog = author::decode_step(spec, "/w", CTX, tok as usize + 1, PROMPT.len() + 1);
    let src = filled(spec, &prog, &ckpt, &caches);
    let (vm, _) = run(&src, spec.vocab);
    (src, vm, want, closest)
}

/// The step lowered whole for `target`.
fn lowered(src: &str, target: &SpatialTarget) -> SpatialProgram {
    compile_program_to_spatial(src, target).expect("a tensor program").into_whole().unwrap_or_else(|why| panic!("{why}"))
}

/// The step's logits on `inputs`: the step argmaxes them, so they are a stored activation, read with every stored
/// tensor.
fn logits(prog: &SpatialProgram, inputs: &Inputs) -> Vec<f64> {
    let out = eval_choreography_every_tensor(prog, inputs).expect("the program evaluates");
    let id = prog.tensors.iter().find(|t| t.name == "logits").expect("the logits").id;
    match out.tensors.get(&id).expect("the evaluation hands back the logits") {
        Value::Float(v) => v.clone(),
        Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
    }
}

/// Every selection bias's tensor in the step: one a mixture layer.
fn selection_biases(prog: &SpatialProgram) -> Vec<String> {
    let names: Vec<String> = prog.tensors.iter().filter(|t| t.name.starts_with("Bs")).map(|t| t.name.clone()).collect();
    assert_eq!(names.len(), 2, "both mixture layers read their selection bias: {names:?}");
    names
}

/// `spec`'s decode step on the fp32 twin and on the SN50 against the VM, the VM against the reference, every routing
/// call decided past the chip's rounding.
fn agree(spec: &ModelSpec, what: &str) {
    let (src, vm, want, closest) = decode_step(spec);
    assert_close(&vm, &want, &format!("{what}: the authored step on the VM, against the reference"));
    assert!(closest.margin > closest.score.abs() / 64.0, "{what}: every routing call is decided past the chip's rounding: {closest:?}");
    let scale = vm.iter().fold(0.0f64, |m, x| m.max(x.abs())).max(1e-9);
    for (target, bound, form) in [(fp32_twin(), scale / 65536.0, "WIRING"), (SpatialTarget::sn50_rdu(), 0.02 * scale, "CHIP")] {
        let prog = lowered(&src, &target);
        let chip = logits(&prog, &inputs_from_fills(&prog));
        assert_eq!(chip.len(), vm.len(), "{what}: the logits' count");
        for (k, (c, v)) in chip.iter().zip(&vm).enumerate() {
            assert!((c - v).abs() <= bound, "{what}: {form} logit {k} is {c} on {}, the VM's {v}", target.name);
        }
    }
}

#[test]
fn a_gated_shared_expert_equals_the_vm() {
    let spec = qwen2_moe_spec();
    assert!(
        spec.layers.iter().all(|l| matches!(&l.ffn, Ffn::Moe(m) if m.shared_gate && m.shared_dff > m.expert_dff)),
        "every layer's shared expert is gated and wider than a routed one"
    );
    agree(&spec, "Qwen2-MoE's shape");
}

#[test]
fn a_qwen35_moe_step_equals_the_vm() {
    let spec = qwen35_moe_spec();
    assert!(
        spec.layers.iter().any(|l| matches!(l.mixer, logicaffeine_compile::model::spec::Mixer::GatedDelta(_)))
            && spec.layers.iter().all(|l| matches!(&l.ffn, Ffn::Moe(m) if m.shared_gate && m.norm_top_k)),
        "a hybrid whose every block is a renormalized mixture beside a gated shared expert"
    );
    agree(&spec, "Qwen3.5-MoE's shape");
}

#[test]
fn group_limited_sigmoid_routing_equals_the_vm() {
    let spec = deepseek_v3_spec();
    assert!(
        spec.layers.iter().skip(1).all(|l| matches!(&l.ffn, Ffn::Moe(m) if m.gating == (Gating::Sigmoid { bias: true }) && m.groups.is_some() && m.weights_scale != 1.0)),
        "every layer past the first routes as DeepSeek-V3 does"
    );
    agree(&spec, "DeepSeek-V3's routing");
}

#[test]
fn the_bias_selects_but_does_not_weight() {
    let spec = deepseek_v3_spec();
    let (src, _, _, _) = decode_step(&spec);
    let prog = lowered(&src, &fp32_twin());
    let inputs = inputs_from_fills(&prog);
    let plain = logits(&prog, &inputs);
    // The same amount added to every expert's selection bias moves every selection score and every group's score
    // alike: the same experts are chosen with the same weights, so the logits are bit for bit the same.
    let mut even = inputs.clone();
    for name in selection_biases(&prog) {
        let id = prog.tensors.iter().find(|t| t.name == name).expect("the selection bias").id;
        let Some(Value::Float(v)) = even.tensors.get_mut(&id) else { panic!("`{name}`: a float selection bias") };
        v.iter_mut().for_each(|b| *b += 0.375);
    }
    assert_eq!(logits(&prog, &even), plain, "an even selection bias changes nothing");
    // Not vacuous: the step's own selection bias chooses other experts than its scores alone would, and the logits move.
    let mut unbiased = spec.clone();
    for layer in &mut unbiased.layers {
        if let Ffn::Moe(m) = &mut layer.ffn {
            m.gating = Gating::Sigmoid { bias: false };
        }
    }
    let (_, biased_vm, _, _) = decode_step(&spec);
    let (_, unbiased_vm, _, _) = decode_step(&unbiased);
    let moved = biased_vm.iter().zip(&unbiased_vm).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
    assert!(moved > 1e-3, "the selection bias moves the step's logits by only {moved}");
}
