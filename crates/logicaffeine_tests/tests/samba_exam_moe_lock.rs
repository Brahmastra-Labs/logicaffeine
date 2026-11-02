//! samba_exam_moe_lock — the rows of the Samba book's exam E6 (section 10.6 of `benchmarks/PREPARE_FOR_SAMBA.md`,
//! mixtures of experts) that hold today.
//!
//! A row runs the SN50 bundle `largo emit rdu` writes, read back off disk, certified and run through the spatial
//! evaluator as a host runs it (`common/samba_handoff.rs`, `common/samba_exam.rs`), against the f64 reference over the
//! same checkpoint, decoding the first token: every routing call a row is about is made at that token. The model is the tiny DeepSeek-V2 — latent attention, a dense first layer, then mixtures of four
//! experts beside a shared expert — routed as DeepSeek-V3 routes: sigmoid scores, each expert chosen by its score plus a
//! selection bias, the top two kept, renormalized and scaled by 2.5. Each row writes the checkpoint's selection biases
//! so the call it is about is made on purpose, far from the chip's rounding:
//!
//!   • E6.5 — the best expert in a group the limit leaves out: its group's other expert sinks the group's score, and
//!     the expert is not kept.
//!
//! The row also shows the call matters: deciding it the other way moves the reference's logits past the datapath. The
//! rows still open are `samba_exam_moe_reds_lock`'s.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test samba_exam_moe_lock`

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

use logicaffeine_compile::model::spec::{Ffn, Gating, ModelSpec};
use samba_exam::{poison_f32, reference_after, within_the_datapath};
use samba_handoff::{decode_on_bundle, Model, BF16_LOGIT_REL, CTX, NEXT};
use tiny_models::Tiny;

/// The tiny DeepSeek-V2 routed as DeepSeek-V3 routes, with the group limit `groups` (`(count, kept)`).
fn routed(groups: Option<(usize, usize)>) -> ModelSpec {
    let mut spec = Tiny::DeepSeek2.spec(CTX);
    for layer in &mut spec.layers {
        if let Ffn::Moe(m) = &mut layer.ffn {
            m.gating = Gating::Sigmoid { bias: true };
            m.groups = groups;
            m.weights_scale = 2.5;
            m.norm_top_k = true;
        }
    }
    spec
}

/// Every mixture layer of `m`.
fn mixtures(m: &Model) -> Vec<usize> {
    m.spec.layers.iter().enumerate().filter(|(_, l)| matches!(l.ffn, Ffn::Moe(_))).map(|(l, _)| l).collect()
}

/// Write `biases` as every mixture layer's selection bias.
fn bias(m: &Model, biases: &[f32]) {
    for l in mixtures(m) {
        for (e, b) in biases.iter().enumerate() {
            poison_f32(m, &format!("blk.{l}.exp_probs_b.bias"), e, *b);
        }
    }
}

/// The bound `within_the_datapath` holds a decode to.
fn datapath_bound(m: &Model, want: &[f64]) -> f64 {
    let scale = want.iter().fold(0.0f64, |acc, v| acc.max(v.abs()));
    BF16_LOGIT_REL * (m.spec.d as f64 / 64.0).sqrt().max(1.0) * scale
}

#[test]
fn group_limits_exclude_the_best_experts() {
    // Two groups of two, one kept. Expert 0's selection bias makes it the best of all whatever the router's logits (its
    // score past two, every other's below), and its group partner's sinks their group's score (the sum of its two best)
    // below one, where the other group's is past two: experts 2 and 3 are kept, never expert 0.
    let m = Model::write(routed(Some((2, 1))), "deepseek-v3-routing-grouped");
    bias(&m, &[2.0, -3.0, 1.0, 1.0]);
    let want = reference_after(&m, &[], NEXT);
    let got = decode_on_bundle(&m, None);
    within_the_datapath(&m, &got, &want, "a group limit leaving the best expert out");
    // Not vacuous: without the group limit expert 0 is kept, and the logits move past the datapath's bound.
    let ungrouped = Model::write(routed(None), "deepseek-v3-routing-ungrouped");
    bias(&ungrouped, &[2.0, -3.0, 1.0, 1.0]);
    let free = reference_after(&ungrouped, &[], NEXT);
    let moved = free.iter().zip(&want).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
    assert!(moved > 2.0 * datapath_bound(&m, &want), "the group limit moves the logits by only {moved}");
}
