//! samba_exam_correctness_lock — the rows of the Samba book's exam E2 (section 10.2 of
//! `benchmarks/PREPARE_FOR_SAMBA.md`, correctness against the references) that hold today.
//!
//! A row runs the SN50 bundle `largo emit rdu` writes, read back off disk, certified and run through the spatial
//! evaluator as a host runs it (`common/samba_handoff.rs`, `common/samba_exam.rs`), against the f64 reference over the
//! same checkpoint. A prompt's state comes from the authored prefill on the GPU target and is handed to the bundle by
//! the decode's contract (`spatial::handoff`).
//!
//! The rows still open are handed REDs in `samba_exam_correctness_reds_lock`; a ticket that lands one moves its test
//! here unchanged (its `frozen_reds_lock` row keeps its digest and takes the new path) and points the book's row at it.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test samba_exam_correctness_lock`

#![cfg(feature = "codegen")]

#[path = "cli_common/mod.rs"]
mod cli;
#[path = "common/tiny_models.rs"]
mod tiny_models;
#[path = "common/samba_handoff.rs"]
#[allow(dead_code)]
mod samba_handoff;
#[path = "common/samba_exam.rs"]
#[allow(dead_code)]
mod samba_exam;

use logicaffeine_compile::spatial::target::SpatialTarget;
use samba_exam::{poison_f32, poison_q8, reference_after, within_the_datapath};
use samba_handoff::{decode_on_bundle, prefill_on, Model, CTX, NEXT, PROMPT};
use tiny_models::Tiny;

/// The architectures every row covers: attention alone, a hybrid with recurrent state, windowed attention, and an
/// expert mixture with sinks.
const SHARED: [Tiny; 4] = [Tiny::Qwen3, Tiny::Qwen35, Tiny::Gemma3, Tiny::GptOss];

fn bits(v: &[f64]) -> Vec<u64> {
    v.iter().map(|x| x.to_bits()).collect()
}

#[test]
fn two_runs_of_one_bundle_are_bit_identical() {
    for t in SHARED {
        let m = Model::write(t.spec(CTX), &format!("{t:?}"));
        let h = prefill_on(&m, &SpatialTarget::rtx3060(), &PROMPT);
        let (first, second) = (decode_on_bundle(&m, Some(&h)), decode_on_bundle(&m, Some(&h)));
        assert_eq!(bits(&first), bits(&second), "{t:?}: two runs of its bundle give the same logits, bit for bit");
    }
}

#[test]
fn an_outlier_channel_survives_bf16_sums() {
    // One channel of the token's embedding at about 1e4 (its Q8_0 block's scale 1e4/127, its level 127, the block's
    // other channels zero), as large models' outlier channels sit; every other entry is the tiny checkpoint's own.
    for t in SHARED {
        let m = Model::write(t.spec(CTX), &format!("{t:?}-outlier"));
        poison_q8(&m, "token_embd.weight", NEXT as usize * m.spec.d, 1e4);
        let logits = decode_on_bundle(&m, None);
        within_the_datapath(&m, &logits, &reference_after(&m, &[], NEXT), &format!("{t:?} with an outlier channel"));
    }
}

#[test]
fn nan_and_infinity_propagate_as_the_reference_propagates_them() {
    // A NaN, then an infinity, in one channel of the first layer's norm weight: every logit the reference makes NaN or
    // infinite is NaN or that infinity on the chip, and none the reference keeps finite is masked into a number. A
    // router meeting a NaN is the expert-routing row's (E6.6), so the architectures here are dense.
    for t in [Tiny::Qwen3, Tiny::Qwen35, Tiny::Gemma3] {
        for (what, v) in [("a NaN", f32::NAN), ("an infinity", f32::INFINITY)] {
            let m = Model::write(t.spec(CTX), &format!("{t:?}-poison"));
            poison_f32(&m, "blk.0.attn_norm.weight", 1, v);
            let (got, want) = (decode_on_bundle(&m, None), reference_after(&m, &[], NEXT));
            assert!(want.iter().any(|x| !x.is_finite()), "{t:?}, {what}: the reference's logits carry it");
            let class = |x: f64| {
                if x.is_nan() {
                    "NaN"
                } else if x == f64::INFINITY {
                    "+inf"
                } else if x == f64::NEG_INFINITY {
                    "-inf"
                } else {
                    "finite"
                }
            };
            for (i, (g, w)) in got.iter().zip(&want).enumerate() {
                assert_eq!(class(*g), class(*w), "{t:?}, {what}: logit {i} is {g} on the chip, {w} in the reference");
            }
        }
    }
}
