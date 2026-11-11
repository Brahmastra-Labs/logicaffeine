//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! qwen4_exp_qsa_lock — Qwen sparse attention within its dense limit (QWEN4-6 of `benchmarks/PREPARE_FOR_SAMBA.md`).
//!
//! QSA attends, at a query's position, every position while the query's complete blocks fit the budget, and past that
//! the best `budget / block` blocks by its indexer's scores and the tail after them. A cache of up to
//! `(budget / block + 1)·block − 1` positions (`author::qsa_dense_limit`: 2,051 for Qwen3.8-Flash-Next's 2,048 in blocks
//! of 4) is therefore attended densely, and there a QSA layer is the attention it carries, exactly.
//!
//!   • the dense limit is that bound, and the f64 reference's QSA attention equals its dense attention up to it and
//!     not one block past it, so the selection the bound guards is real;
//!   • within the limit the author writes a QSA layer as its dense attention (`author::check_at`): a prompt's prefill and
//!     the decode step after it equal the reference's logits and tokens, on a dense attention model and on Qwen 3.5's
//!     hybrid, and the decode step lowers onto the SN50 whole, its wiring the VM's on an fp32 twin of the chip;
//!   • past the limit, and where no cache length is known, the selection is refused by name.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test qwen4_exp_qsa_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(feature = "codegen")]

mod common;
#[path = "common/model_harness.rs"]
mod model_harness;
#[path = "common/spatial.rs"]
mod spatial_support;
#[path = "common/tiny_models.rs"]
mod tiny_models;

use logicaffeine_compile::model::author;
use logicaffeine_compile::model::reference::{argmax, Reference};
use logicaffeine_compile::model::spec::{Indexer, Mixer, ModelSpec};
use logicaffeine_compile::spatial::eval::{eval_choreography_every_tensor, Value};
use logicaffeine_compile::spatial::pipeline::compile_program_to_spatial;
use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::spatial::target::{ComputeCap, SpatialTarget};
use model_harness::{assert_close, carried, checkpoint, filled, run, CTX, PROMPT};
use spatial_support::inputs_from_fills;
use std::collections::HashMap;
use tiny_models::Tiny;

/// `tiny`'s spec with QSA on every attention layer: two index heads as wide as the attention's heads, `budget`
/// positions in blocks of `block`.
fn with_qsa(tiny: Tiny, budget: usize, block: usize) -> ModelSpec {
    let mut spec = tiny.spec(CTX);
    let (mut layers, mut head_dim) = (Vec::new(), 0);
    for (l, layer) in spec.layers.iter().enumerate() {
        if let Mixer::Attention(a) = &layer.mixer {
            layers.push(l);
            head_dim = a.head_dim;
        }
    }
    assert!(!layers.is_empty(), "{tiny:?} has attention layers");
    spec.indexer = Some(Indexer { heads: 2, head_dim, budget, block, layers });
    spec
}

#[test]
fn the_dense_limit_is_where_the_complete_blocks_fill_the_budget() {
    let ix = |budget, block| Indexer { heads: 4, head_dim: 128, budget, block, layers: vec![3] };
    assert_eq!(author::qsa_dense_limit(&ix(2048, 4)), 2051, "Qwen3.8-Flash-Next's");
    assert_eq!(author::qsa_dense_limit(&ix(8, 4)), 11, "the tiny twin's");
    assert_eq!(author::qsa_dense_limit(&ix(12, 4)), 15);
    // The reference's own QSA: dense up to the limit, not one block past it.
    let spec = with_qsa(Tiny::Qwen3, 4, 2);
    let limit = author::qsa_dense_limit(spec.indexer.as_ref().unwrap());
    assert_eq!(limit, 5);
    let ckpt = checkpoint(&spec);
    let dense = ModelSpec { indexer: None, ..spec.clone() };
    let prompt: Vec<u32> = (0..CTX as u32).map(|i| (i * 7 + 3) % spec.vocab as u32).collect();
    let logits = |s: &ModelSpec, n: usize| Reference::new(s.clone(), ckpt.clone()).expect("reference").step(&prompt[..n]).expect("prefill");
    let (a, b) = (logits(&spec, limit), logits(&dense, limit));
    assert_close(&a, &b, "QSA within its dense limit is its dense attention");
    let past = limit + 3;
    let (a, b) = (logits(&spec, past), logits(&dense, past));
    let scale = b.iter().fold(0.0f64, |m, x| m.max(x.abs()));
    assert!(a.iter().zip(&b).any(|(x, y)| (x - y).abs() > 1e-6 * scale), "past the limit QSA drops positions the dense attention reads");
}

/// The prompt's prefill and the decode step after it, written by the author for a cache of `CTX` positions and run on
/// the VM, against the reference.
fn prefills_and_decodes(spec: &ModelSpec, what: &str) {
    author::check_at(spec, CTX).unwrap_or_else(|e| panic!("{what}: {e}"));
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
fn within_its_dense_limit_a_qsa_layer_is_written_dense_and_equals_the_reference() {
    for (what, tiny) in [("dense attention", Tiny::Qwen3), ("Qwen 3.5's hybrid", Tiny::Qwen35)] {
        let spec = with_qsa(tiny, 12, 4);
        assert!(CTX <= author::qsa_dense_limit(spec.indexer.as_ref().unwrap()), "{what}: the cache is within the limit");
        prefills_and_decodes(&spec, what);
    }
}

#[test]
fn past_its_dense_limit_or_with_no_cache_length_qsa_is_refused_by_name() {
    let spec = with_qsa(Tiny::Qwen3, 8, 4);
    assert!(CTX > author::qsa_dense_limit(spec.indexer.as_ref().unwrap()), "the cache passes the limit");
    let past = author::check_at(&spec, CTX).expect_err("a cache past the dense limit");
    assert!(past.to_string().contains("past its dense limit"), "the refusal names the limit: {past}");
    let unknown = author::check(&spec).expect_err("no cache length");
    assert!(unknown.to_string().contains("QSA"), "the refusal names QSA: {unknown}");
    author::check_at(&spec, author::qsa_dense_limit(spec.indexer.as_ref().unwrap())).expect("at the limit it is dense");
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

fn logits_on(src: &str, target: &SpatialTarget) -> Vec<f64> {
    let p = compile_program_to_spatial(src, target).expect("a tensor program").into_whole().unwrap_or_else(|why| panic!("{why}"));
    let out = eval_choreography_every_tensor(&p, &inputs_from_fills(&p)).expect("the program evaluates");
    let id = p.tensors.iter().find(|t| t.name == "logits").expect("the logits").id;
    match out.tensors.get(&id).expect("logits") {
        Value::Float(v) => v.clone(),
        Value::Int(v) => v.iter().map(|x| *x as f64).collect(),
    }
}

/// The decode step lowers onto the SN50 whole, its wiring the VM's: on an fp32 twin of the chip, within 2⁻¹⁶ of the
/// largest logit, on dense attention and on Qwen 3.5's hybrid. What the chip's bf16 adds is the numerics contract's
/// (NUM-1), held by the exam's numerics rows.
#[test]
fn the_sn50_computes_a_qsa_decode_step_within_its_dense_limit() {
    for (what, tiny) in [("dense attention", Tiny::Qwen3), ("Qwen 3.5's hybrid", Tiny::Qwen35)] {
        let spec = with_qsa(tiny, 12, 4);
        let ckpt = checkpoint(&spec);
        let mut reference = Reference::new(spec.clone(), ckpt.clone()).expect("reference");
        let tok = argmax(&reference.step(PROMPT).expect("prefill"));
        let caches = carried(&spec, &reference);
        let prog = author::decode_step(&spec, "/w", CTX, tok as usize + 1, PROMPT.len() + 1);
        let src = filled(&spec, &prog, &ckpt, &caches);
        let (vm, _) = run(&src, spec.vocab);
        let scale = vm.iter().fold(0.0f64, |m, x| m.max(x.abs())).max(1e-9);
        let got = logits_on(&src, &fp32_twin());
        assert_eq!(got.len(), vm.len(), "{what}: the logits' count");
        for (k, (c, v)) in got.iter().zip(&vm).enumerate() {
            assert!((c - v).abs() <= scale / 65536.0, "{what}: logit {k} is {c} on the SN50's fp32 twin, the VM's {v}");
        }
    }
}
