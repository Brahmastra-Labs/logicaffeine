//! spatial_nope_attention_lock — Llama 4's attention on a spatial chip: rotated layers in chunks with the weightless QK
//! norm after the rotation, and a rotation-free layer whose queries take the attention temperature (a rotation whose
//! cosine is the position's factor and whose sine is zero, read from the host's tables as the rotary tables are).
//!
//! The model is `model_harness::llama4_spec` — its temperature floor 2, not llama.cpp's 8192, so the factor moves within
//! a short context. Its decode step after a four-token prompt, authored with every weight, host table and cache written
//! into the program, is lowered whole and run in the spatial evaluator:
//!
//!   • WIRING — on an fp32 twin of the SN50 (fp32 in place of its bf16) the logits are the VM's within a relative 2⁻¹⁶;
//!   • CHIP PRECISION — on the SN50 they are the VM's within 2% of the largest;
//!   • and the temperature moves the VM's logits on this step, so the agreement is not vacuous.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test spatial_nope_attention_lock`

#![cfg(feature = "codegen")]

#[path = "common/model_harness.rs"]
mod model_harness;
#[path = "common/spatial.rs"]
mod spatial_support;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::model::author;
use logicaffeine_compile::model::reference::{argmax, Reference};
use logicaffeine_compile::model::spec::{Mixer, ModelSpec, Temperature};
use logicaffeine_compile::spatial::eval::{eval_choreography_every_tensor, Value};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::{ComputeCap, SpatialTarget};
use model_harness::{carried, checkpoint, filled, llama4_spec, run, CTX, PROMPT};
use spatial_support::inputs_from_fills;

/// A chip with fp32 in place of bf16 on the matmul, pointwise and row units. A matmul takes the narrowest operand its
/// engine offers, so fp32 BESIDE bf16 would still round every activation a matmul reads to bf16; in its place, every
/// value of the step is fp32.
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

/// The decode step of `spec` after the prompt, every weight, table and cache written into it, and the VM's logits.
fn decode_step(spec: &ModelSpec) -> (String, Vec<f64>) {
    let ckpt = checkpoint(spec);
    let mut reference = Reference::new(spec.clone(), ckpt.clone()).expect("reference");
    let prefill = reference.step(PROMPT).expect("reference prefill");
    let tok = argmax(&prefill);
    let caches = carried(spec, &reference);
    let prog = author::decode_step(spec, "/w", CTX, tok as usize + 1, PROMPT.len() + 1);
    let src = filled(spec, &prog, &ckpt, &caches);
    let (vm, _) = run(&src, spec.vocab);
    (src, vm)
}

/// The step's logits as the spatial evaluator computes the program lowered whole for `target`. The step argmaxes its
/// logits, so they are a stored activation, read with every stored tensor rather than with the program's outputs.
fn on_the_chip(src: &str, target: &SpatialTarget) -> Vec<f64> {
    let prog = compile_program_to_spatial(src, target).expect("a tensor program").into_whole().unwrap_or_else(|why| panic!("{why}"));
    let out = eval_choreography_every_tensor(&prog, &inputs_from_fills(&prog)).expect("the program evaluates");
    let logits = prog.tensors.iter().find(|t| t.name == "logits").expect("the logits").id;
    match out.tensors.get(&logits).expect("the evaluation hands back the logits") {
        Value::Float(v) => v.clone(),
        Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
    }
}

fn agree(spec: &ModelSpec, what: &str) {
    let (src, vm) = decode_step(spec);
    let scale = vm.iter().fold(0.0f64, |m, x| m.max(x.abs())).max(1e-9);
    for (target, bound, form) in [(fp32_twin(), scale / 65536.0, "WIRING"), (SpatialTarget::sn50_rdu(), 0.02 * scale, "CHIP")] {
        let chip = on_the_chip(&src, &target);
        assert_eq!(chip.len(), vm.len(), "{what}: the logits' count");
        for (k, (c, v)) in chip.iter().zip(&vm).enumerate() {
            assert!((c - v).abs() <= bound, "{what}: {form} logit {k} is {c} on {}, the VM's {v}", target.name);
        }
    }
}

#[test]
fn a_rotation_free_layer_equals_the_vm() {
    let spec = llama4_spec();
    assert!(
        matches!(&spec.layers[3].mixer, Mixer::Attention(a) if a.rope.is_none() && a.temperature.is_some()),
        "the last layer turns by no rotation and takes the temperature"
    );
    agree(&spec, "Llama 4's decode step");
}

#[test]
fn temperature_tuning_equals_the_vm() {
    // At the step's position (4) the factor is `1 + 0.5·ln(⌊5/2⌋ + 1)`, not one: without it the VM's logits move.
    let spec = llama4_spec();
    let mut untuned = spec.clone();
    for layer in &mut untuned.layers {
        if let Mixer::Attention(a) = &mut layer.mixer {
            a.temperature = None;
        }
    }
    let (_, tuned_vm) = decode_step(&spec);
    let (_, untuned_vm) = decode_step(&untuned);
    let moved = tuned_vm.iter().zip(&untuned_vm).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
    assert!(moved > 1e-3, "the temperature moves the step's logits by only {moved}");
    agree(&spec, "the tuned step");
}

#[test]
fn the_temperature_is_one_below_its_floor() {
    // llama.cpp's constants: every position before the 8192nd scales by exactly one, the 8192nd (index 8191) by more.
    let t = Temperature { floor: 8192.0, scale: 0.1 };
    assert_eq!(t.at(0), 1.0, "position 0");
    assert_eq!(t.at(8190), 1.0, "the last position below the floor");
    assert!(t.at(8191) > 1.0, "the floor's own position: {}", t.at(8191));
    assert_eq!(t.at(8191), 1.0 + 0.1 * 2f64.ln(), "one floor past: 1 + 0.1·ln 2");
}
