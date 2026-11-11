//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! qwen4_exp_hyper_lock — Qwen3.8-Flash-Next's hyper-connected residual (QWEN4-2 of `benchmarks/PREPARE_FOR_SAMBA.md`),
//! written by the author and computed as the reference computes it.
//!
//! The residual is `lanes` copies of the token's embedding, `lanes·d` wide. A mix replaces each norm before a mixer and
//! before an FFN: the lanes normed each by its own row of the weight (`rmsnorm … in L lanes`), gated by
//! `σ(U·silu(D·x̂ / L))` (`hadamard`) and averaged (`the mean of … over L lanes`); the sublayer's output joins every lane
//! by the lane's share `2σ(J·x̂ / L)` (`spread over L lanes by`). A last mix, without shares, feeds the head.
//!
//!   • the author writes it over a dense attention model, an expert mixture, a Gated DeltaNet hybrid gated by sigmoids
//!     (Qwen3.8's), Gemma 3's sandwich of norms and a latent attention; a prompt's prefill and the decode step after it
//!     equal the reference's logits and tokens, at four lanes and at three;
//!   • the lanes move the logits: the model read through its mixes is not the model read through its norms;
//!   • every per-layer program reads and writes the lanes' width, and the tensor-parallel author refuses the lanes by
//!     name;
//!   • a decode step lowers onto the SN50 whole and computes the VM's logits: on an fp32 twin of the chip within 2⁻¹⁶ of
//!     the largest, on the chip within 2%.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test qwen4_exp_hyper_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

mod common;
#[path = "common/model_harness.rs"]
mod model_harness;
#[path = "common/spatial.rs"]
mod spatial_support;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::model::author;
use logicaffeine_compile::model::reference::{argmax, Reference};
use logicaffeine_compile::model::spec::{GateActivation, HyperConnections, Mixer, ModelSpec};
use logicaffeine_compile::spatial::eval::{eval_choreography_every_tensor, Value};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::spatial::target::{ComputeCap, SpatialTarget};
use model_harness::{assert_close, carried, checkpoint, filled, run, CTX, PROMPT};
use spatial_support::inputs_from_fills;
use std::collections::HashMap;
use tiny_models::Tiny;

/// `tiny`'s spec with a residual of `lanes` hyper-connection lanes mixed through a rank of 4.
fn hyper(tiny: Tiny, lanes: usize) -> ModelSpec {
    ModelSpec { hyper: Some(HyperConnections { lanes, rank: 4 }), ..tiny.spec(CTX) }
}

/// Qwen3.8's hybrid at the tiny width: Qwen 3.5's layers, each Gated DeltaNet gated by sigmoids, the residual in lanes.
fn qwen38_shaped(lanes: usize) -> ModelSpec {
    let mut spec = hyper(Tiny::Qwen35, lanes);
    for layer in &mut spec.layers {
        if let Mixer::GatedDelta(g) = &mut layer.mixer {
            g.gate = GateActivation::Sigmoid;
        }
    }
    spec
}

fn models() -> Vec<(&'static str, ModelSpec)> {
    vec![
        ("dense attention", hyper(Tiny::Qwen3, 4)),
        ("an expert mixture", hyper(Tiny::Qwen3Moe, 4)),
        ("Qwen3.8's hybrid", qwen38_shaped(4)),
        ("Gemma 3's sandwich of norms", hyper(Tiny::Gemma3, 4)),
        ("a latent attention", hyper(Tiny::DeepSeek2, 4)),
        ("dense attention in three lanes", hyper(Tiny::Qwen3, 3)),
    ]
}

/// The prompt's prefill and the decode step after it, written by the author and run on the VM, against the reference.
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
fn a_mixer_equals_the_reference() {
    for (what, spec) in models() {
        prefills_and_decodes(&spec, what);
    }
}

#[test]
fn the_lanes_move_the_logits() {
    // The checkpoint holds the norms too: read through them in place of the mixes, the same weights give other logits.
    for (what, spec) in models() {
        let ckpt = checkpoint(&spec);
        let logits = |s: &ModelSpec| Reference::new(s.clone(), ckpt.clone()).expect("reference").step(PROMPT).expect("prefill");
        let (laned, normed) = (logits(&spec), logits(&ModelSpec { hyper: None, ..spec.clone() }));
        let scale = laned.iter().fold(0.0f64, |m, x| m.max(x.abs()));
        assert!(laned.iter().zip(&normed).any(|(a, b)| (a - b).abs() > 1e-3 * scale), "{what}: the lanes move the logits");
    }
}

#[test]
fn every_layer_program_reads_and_writes_the_lanes() {
    let spec = qwen38_shaped(4);
    let width = 4 * spec.d;
    assert_eq!(author::hidden_width(&spec), width, "the hidden state is four lanes wide");
    let declares = |src: &str, name: &str, rows: usize| {
        let want = format!("Let {name} be a {rows} by {width} tensor of Real.");
        assert!(src.lines().any(|l| l == want), "the program declares `{want}`:\n{src}");
    };
    for l in 0..spec.layers.len() {
        declares(&author::layer(&spec, l, "/w").0, &author::layer_input(l), 1);
        declares(&author::decode_layer(&spec, l, "/w", CTX, 3).0, &author::layer_input(l), 1);
        declares(&author::stacked_layer(&spec, l, "/w", CTX, &[3, 4], author::RowState::Shared).0, &author::stacked_layer_input(l), 2);
    }
    declares(&author::head(&spec, "/w").0, &author::layer_input(spec.layers.len()), 1);
    declares(&author::prefill_head(&spec, "/w", 3).0, &author::stacked_layer_input(spec.layers.len()), 3);
    let refusal = author::check_tp(&spec, 2).expect_err("the tensor-parallel split has one residual of d");
    assert!(refusal.to_string().contains("hyper-connected"), "the refusal names the lanes: {refusal}");
}

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

fn on_the_chip(src: &str, target: &SpatialTarget) -> Vec<f64> {
    let prog = compile_program_to_spatial(src, target).expect("a tensor program").into_whole().unwrap_or_else(|why| panic!("{why}"));
    let out = eval_choreography_every_tensor(&prog, &inputs_from_fills(&prog)).expect("the program evaluates");
    let logits = prog.tensors.iter().find(|t| t.name == "logits").expect("the logits").id;
    match out.tensors.get(&logits).expect("the evaluation hands back the logits") {
        Value::Float(v) => v.clone(),
        Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
    }
}

#[test]
fn the_sn50_computes_a_hyper_connected_decode_step() {
    for (what, spec) in [("dense attention", hyper(Tiny::Qwen3, 4)), ("Qwen3.8's hybrid", qwen38_shaped(4))] {
        let ckpt = checkpoint(&spec);
        let mut reference = Reference::new(spec.clone(), ckpt.clone()).expect("reference");
        let prefill = reference.step(PROMPT).expect("reference prefill");
        let tok = argmax(&prefill);
        let caches = carried(&spec, &reference);
        let prog = author::decode_step(&spec, "/w", CTX, tok as usize + 1, PROMPT.len() + 1);
        let src = filled(&spec, &prog, &ckpt, &caches);
        let (vm, _) = run(&src, spec.vocab);
        let scale = vm.iter().fold(0.0f64, |m, x| m.max(x.abs())).max(1e-9);
        for (target, bound, form) in [(fp32_twin(), scale / 65536.0, "WIRING"), (SpatialTarget::sn50_rdu(), 0.02 * scale, "CHIP")] {
            let chip = on_the_chip(&src, &target);
            assert_eq!(chip.len(), vm.len(), "{what}: the logits' count");
            for (k, (c, v)) in chip.iter().zip(&vm).enumerate() {
                assert!((c - v).abs() <= bound, "{what}: {form} logit {k} is {c} on {}, the VM's {v}", target.name);
            }
        }
    }
}
