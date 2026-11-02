//! samba_exam_moe_reds_lock — the rows of the Samba book's exam E6 (section 10.6 of `benchmarks/PREPARE_FOR_SAMBA.md`,
//! adversarial expert routing) still open, each a handed RED frozen in `fixtures/frozen_reds.tsv`.
//!
//! A row runs the SN50 bundle `largo emit rdu` writes, read back off disk, certified and run through the spatial
//! evaluator as a host runs it (`common/samba_handoff.rs`, `common/samba_exam.rs`), against the f64 reference over the
//! same checkpoint. A row writes the checkpoint's router biases or columns so the routing it is about is made on
//! purpose:
//!
//!   • E6.1 — the tiny gpt-oss (four experts a mixture, two a token, the kept logits softmaxed, a router bias each
//!     expert), every token's first choice one expert, sixty-four sequences decoded at once, each after its own prompt
//!     (NUM-1: the batch decodes each sequence as it decodes alone and routes it the reference's way; the bf16 datapath's
//!     error after a prompt reaches 0.22 against a bound of 0.12);
//!   • E6.4 — the tiny DeepSeek-V2 routed as DeepSeek-V3 routes, two experts tied at the second place: the first of them
//!     is kept, as the reference keeps it, and the decode is the reference's within the datapath (NUM-1: the tie is
//!     broken the reference's way — the bundle's logits lie 0.115 from the reference's and 6.7 from the other order's —
//!     and the bf16 datapath's error on this checkpoint is 2.6% of its largest logit, past the 2% the exam allows);
//!   • E6.6 — the tiny gpt-oss with a NaN among a token's router scores, and a token whose whole router row is NaN
//!     beside three that are not (NUM-1: the reference ranks a NaN below every number, as the chip does, and both parts
//!     route as it does; a sequence beside the NaN one lies 0.114 from its reference after its prompt, past 0.109).
//!
//! A ticket that lands a row moves its test, unchanged, into `samba_exam_moe_lock`.
//!
//! Run on purpose: `scripts/run-handed-reds-tests.sh`, or
//! `cargo nextest run -p logicaffeine-tests --features handed-reds --cargo-profile test-opt --test samba_exam_moe_reds_lock`.

#![cfg(feature = "codegen")]
#![cfg(feature = "handed-reds")]

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
use samba_exam::{decode_batch_on_bundle, poison_f32, poison_q8, reference_after, within_the_datapath};
use samba_handoff::{decode_on_bundle, prefill_on, Model, BF16_LOGIT_REL, CTX, NEXT};
use tiny_models::Tiny;

/// Every mixture layer of `m`.
fn mixtures(m: &Model) -> Vec<usize> {
    m.spec.layers.iter().enumerate().filter(|(_, l)| matches!(l.ffn, Ffn::Moe(_))).map(|(l, _)| l).collect()
}

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

/// Write `biases` as every mixture layer's selection bias.
fn bias(m: &Model, biases: &[f32]) {
    for l in mixtures(m) {
        for (e, b) in biases.iter().enumerate() {
            poison_f32(m, &format!("blk.{l}.exp_probs_b.bias"), e, *b);
        }
    }
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
fn a_batch_routed_to_one_expert_decodes() {
    // Expert 0's router bias sits 64 past every logit: every token of every sequence chooses it first, and its weight
    // among the kept two is 1 to the last bit. Sixty-four sequences, each after a prompt of its own, are decoded at once:
    // the one expert takes the whole batch, and each sequence's logits are its own reference's within the datapath.
    let m = Model::write(Tiny::GptOss.spec(CTX), "gpt-oss-one-hot-expert");
    router_bias(&m, &[64.0, 0.0, 0.0, 0.0]);
    let vocab = m.spec.vocab as u32;
    let prompts: Vec<Vec<u32>> = (0..64u32).map(|b| vec![(b * 5 + 3) % vocab, (b * 11 + 1) % vocab]).collect();
    let handed: Vec<Handoff> = prompts.iter().map(|p| prefill_on(&m, &SpatialTarget::rtx3060(), p)).collect();
    let toks: Vec<u32> = (0..64u32).map(|b| (b * 3 + 1) % vocab).collect();
    let got = decode_batch_on_bundle(&m, &handed, &toks);
    for (b, (prompt, tok)) in prompts.iter().zip(&toks).enumerate() {
        within_the_datapath(&m, &got[b], &reference_after(&m, prompt, *tok), &format!("sequence {b}"));
    }
    // Not vacuous: routed by their own logits, the first sequence's experts change and its logits move past the
    // datapath's bound.
    let plain = Model::write(Tiny::GptOss.spec(CTX), "gpt-oss-own-routing");
    let (want, free) = (reference_after(&m, &prompts[0], toks[0]), reference_after(&plain, &prompts[0], toks[0]));
    let moved = free.iter().zip(&want).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
    assert!(moved > 2.0 * datapath_bound(&m, &want), "one hot expert moves the logits by only {moved}");
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
fn a_nan_router_row_is_handled_as_the_reference() {
    // Expert 3's router bias NaN: every token's router scores hold a NaN. The reference ranks a NaN below every number,
    // as the chip's routing does, so each token keeps its two best numbered experts and its logits are finite; the
    // bundle's are the reference's within the datapath.
    let m = Model::write(Tiny::GptOss.spec(CTX), "gpt-oss-nan-router-score");
    router_bias(&m, &[0.0, 0.0, 0.0, f32::NAN]);
    let want = reference_after(&m, &[], NEXT);
    assert!(want.iter().all(|v| v.is_finite()), "the reference keeps a NaN-scored expert out");
    within_the_datapath(&m, &decode_on_bundle(&m, None), &want, "a NaN router score");

    // A token whose whole router row is NaN — its sequence's carried state NaN — beside three whose are not: its logits
    // are NaN and the other three's are their references', as each sequence's arithmetic is its own.
    let m = Model::write(Tiny::GptOss.spec(CTX), "gpt-oss-nan-router-row");
    let vocab = m.spec.vocab as u32;
    let prompts: Vec<Vec<u32>> = (0..4u32).map(|b| vec![(b * 7 + 2) % vocab]).collect();
    let mut handed: Vec<Handoff> = prompts.iter().map(|p| prefill_on(&m, &SpatialTarget::rtx3060(), p)).collect();
    handed[2].state.values_mut().for_each(|v| v.iter_mut().for_each(|x| *x = f64::NAN));
    let toks: Vec<u32> = (0..4u32).map(|b| (b * 3 + 1) % vocab).collect();
    let got = decode_batch_on_bundle(&m, &handed, &toks);
    assert!(got[2].iter().all(|v| v.is_nan()), "the NaN sequence's logits are NaN");
    for b in [0, 1, 3] {
        within_the_datapath(&m, &got[b], &reference_after(&m, &prompts[b], toks[b]), &format!("sequence {b} beside a NaN row"));
    }
}
