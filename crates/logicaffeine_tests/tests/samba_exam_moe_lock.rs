//! samba_exam_moe_lock — the rows of the Samba book's exam E6 (section 10.6 of `benchmarks/PREPARE_FOR_SAMBA.md`,
//! adversarial expert routing).
//!
//! A row runs the SN50 bundle `largo emit rdu` writes, read back off disk, certified and run through the spatial
//! evaluator as a host runs it (`common/samba_handoff.rs`, `common/samba_exam.rs`), against the f64 reference over the
//! same checkpoint. Each row writes the checkpoint's router biases, selection biases or columns so the routing it is
//! about is made on purpose:
//!
//!   • E6.1 — the tiny gpt-oss (four experts a mixture, two a token, the kept logits softmaxed, a router bias each
//!     expert), every token's first choice one expert, sixty-four sequences decoded at once, each after its own prompt:
//!     each sequence the GPU's own bf16 datapath's bit for bit, and within its reach of the reference (Tristen,
//!     2026-11-12: the bf16 products' error, which every bf16 chip shares, passes the 2% bound on this model);
//!   • E6.4 — the tiny DeepSeek-V2 routed as DeepSeek-V3 routes (sigmoid scores, each expert chosen by its score plus a
//!     selection bias, the top two kept, renormalized and scaled by 2.5), two experts tied at the second place: the
//!     first of them is kept, as the reference keeps it;
//!   • E6.5 — the same routing with a group limit: the best expert in a group the limit leaves out is not kept;
//!   • E6.6 — the tiny gpt-oss with a NaN among a token's router scores, and a token whose whole router row is NaN beside
//!     three that are not, each part the GPU's own bf16 datapath's bit for bit (the reference ranks a NaN below every
//!     number, as the chip does).
//!
//! Each row also shows its call matters: deciding it the other way moves the reference's logits past the datapath.
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

use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::reference::{Reference, RoutingCall};
use logicaffeine_compile::model::spec::{Ffn, Gating, ModelSpec};
use logicaffeine_compile::spatial::handoff::Handoff;
use logicaffeine_compile::spatial::target::SpatialTarget;
use samba_exam::{decode_batch_on, decode_batch_on_bundle, poison_f32, poison_q8, reference_after, within_the_datapath, within_the_gpu_path};
use samba_handoff::{decode_on_bundle, decode_on_layers, prefill_on, Model, BF16_LOGIT_REL, CTX, NEXT};
use std::collections::BTreeMap;
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

/// Write `biases` as every mixture layer's router bias.
fn router_bias(m: &Model, biases: &[f32]) {
    for l in mixtures(m) {
        for (e, b) in biases.iter().enumerate() {
            poison_f32(m, &format!("blk.{l}.ffn_gate_inp.bias"), e, *b);
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

/// The reference's logits for the first token, every mixture's call at it decided the other way: the first expert it
/// passes over kept in place of the last it keeps.
fn reference_flipped(m: &Model) -> Vec<f64> {
    let mut r = Reference::new(m.spec.clone(), Gguf::open(&m.path).expect("the checkpoint")).expect("the reference reads it");
    for layer in mixtures(m) {
        r.flip_routing(RoutingCall { layer, position: 0, margin: 0.0, score: 0.0 });
    }
    r.step(&[NEXT]).expect("the reference steps")
}

#[test]
fn router_ties_break_in_the_references_order() {
    // Experts 1 and 2 read one router column and carry one selection bias, so their selection scores are equal on every
    // tier; expert 0 leads them by two and expert 3 trails them by two. Choosing two, the tie is at the second place:
    // expert 1 is kept, as the reference keeps the first of a tie.
    let m = Model::write(routed(None), "deepseek-v3-routing-tied");
    let d = m.spec.d;
    assert_eq!(d % 32, 0, "a router column is whole Q8_0 blocks");
    for l in mixtures(&m) {
        // `ffn_gate_inp` is `d` by `E`, Q8_0: expert `e`'s column is elements `e·d ..`, `d/32` blocks, each set to one
        // weight of 0.25 at its first element and zeros — the same column for both.
        for e in [1, 2] {
            for b in 0..d / 32 {
                poison_q8(&m, &format!("blk.{l}.ffn_gate_inp.weight"), e * d + 32 * b, 0.25);
            }
        }
    }
    bias(&m, &[2.0, 0.0, 0.0, -2.0]);
    let want = reference_after(&m, &[], NEXT);
    let got = decode_on_bundle(&m, None);
    within_the_datapath(&m, &got, &want, "a tie at the second place");
    // Not vacuous: keeping expert 2 instead moves the logits past the datapath's bound.
    let flipped = reference_flipped(&m);
    let moved = flipped.iter().zip(&want).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
    assert!(moved > 2.0 * datapath_bound(&m, &want), "breaking the tie the other way moves the logits by only {moved}");
}

#[test]
fn a_batch_routed_to_one_expert_decodes() {
    // Expert 0's router bias sits 64 past every logit: every token of every sequence chooses it first, and its weight
    // among the kept two is 1 to the last bit. Sixty-four sequences, each after a prompt of its own, are decoded at once:
    // the one expert takes the whole batch, and each sequence's logits are the GPU's own bf16 datapath's, bit for bit,
    // and its reference's within the datapath's bound or that path's own distance from it.
    let m = Model::write(Tiny::GptOss.spec(CTX), "gpt-oss-one-hot-expert");
    router_bias(&m, &[64.0, 0.0, 0.0, 0.0]);
    let vocab = m.spec.vocab as u32;
    let prompts: Vec<Vec<u32>> = (0..64u32).map(|b| vec![(b * 5 + 3) % vocab, (b * 11 + 1) % vocab]).collect();
    let handed: Vec<Handoff> = prompts.iter().map(|p| prefill_on(&m, &SpatialTarget::rtx3060(), p)).collect();
    let toks: Vec<u32> = (0..64u32).map(|b| (b * 3 + 1) % vocab).collect();
    let got = decode_batch_on_bundle(&m, &handed, &toks);
    let gpu = decode_batch_on(&m, &SpatialTarget::rtx3060(), &handed, &toks);
    for (b, (prompt, tok)) in prompts.iter().zip(&toks).enumerate() {
        within_the_gpu_path(&m, &got[b], &gpu[b], &reference_after(&m, prompt, *tok), &format!("sequence {b}"));
    }
    // Not vacuous: routed by their own logits, the first sequence's experts change and its logits move past the
    // datapath's bound.
    let plain = Model::write(Tiny::GptOss.spec(CTX), "gpt-oss-own-routing");
    let (want, free) = (reference_after(&m, &prompts[0], toks[0]), reference_after(&plain, &prompts[0], toks[0]));
    let moved = free.iter().zip(&want).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
    assert!(moved > 2.0 * datapath_bound(&m, &want), "one hot expert moves the logits by only {moved}");
}

#[test]
fn a_nan_router_row_is_handled_as_the_reference() {
    // Expert 3's router bias NaN: every token's router scores hold a NaN. The reference ranks a NaN below every number,
    // as the chip's routing does, so each token keeps its two best numbered experts and its logits are finite; the
    // bundle's are the GPU's own bf16 datapath's, bit for bit, and the reference's within that datapath.
    let m = Model::write(Tiny::GptOss.spec(CTX), "gpt-oss-nan-router-score");
    router_bias(&m, &[0.0, 0.0, 0.0, f32::NAN]);
    let want = reference_after(&m, &[], NEXT);
    assert!(want.iter().all(|v| v.is_finite()), "the reference keeps a NaN-scored expert out");
    let first = Handoff { position: 0, state: BTreeMap::new() };
    let gpu = decode_on_layers(&m, &SpatialTarget::rtx3060(), &first);
    within_the_gpu_path(&m, &decode_on_bundle(&m, None), &gpu, &want, "a NaN router score");

    // A token whose whole router row is NaN — its sequence's carried state NaN — beside three whose are not: its logits
    // are NaN and the other three's are the GPU path's and their references', as each sequence's arithmetic is its own.
    let m = Model::write(Tiny::GptOss.spec(CTX), "gpt-oss-nan-router-row");
    let vocab = m.spec.vocab as u32;
    let prompts: Vec<Vec<u32>> = (0..4u32).map(|b| vec![(b * 7 + 2) % vocab]).collect();
    let mut handed: Vec<Handoff> = prompts.iter().map(|p| prefill_on(&m, &SpatialTarget::rtx3060(), p)).collect();
    handed[2].state.values_mut().for_each(|v| v.iter_mut().for_each(|x| *x = f64::NAN));
    let toks: Vec<u32> = (0..4u32).map(|b| (b * 3 + 1) % vocab).collect();
    let got = decode_batch_on_bundle(&m, &handed, &toks);
    let gpu = decode_batch_on(&m, &SpatialTarget::rtx3060(), &handed, &toks);
    assert!(got[2].iter().all(|v| v.is_nan()), "the NaN sequence's logits are NaN");
    for b in [0, 1, 3] {
        within_the_gpu_path(&m, &got[b], &gpu[b], &reference_after(&m, &prompts[b], toks[b]), &format!("sequence {b} beside a NaN row"));
    }
}
