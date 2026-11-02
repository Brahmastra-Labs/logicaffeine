//! spatial_disaggregation_reds_lock — the decodes of a model's work shared between chips (SERVE-RDU-3 of
//! `benchmarks/PREPARE_FOR_SAMBA.md`), handed REDs frozen in `fixtures/frozen_reds.tsv` until NUM-1.
//!
//! A prompt prefilled on the GPU target decodes on the SN50 bundle and the reverse, a prompt kept on the RDU decodes as
//! it was prefilled, and a model split over the two hands its hidden state on — each held to the f64 reference's
//! continuation within the bf16 datapath's bound (`common/samba_handoff.rs`). The hand-off itself is exact: with fp32
//! elementwise on the SN50 (its fp32 twin) every case holds for every architecture here. What is left is the SN50's
//! bf16 elementwise datapath, whose rounding of the hidden state a prompt's positions and an expert router amplify
//! (Qwen3 0.143 against a bound of 0.100; gpt-oss 0.575 against 0.137): NUM-1's fp32 islands.
//!
//! Run on purpose: `scripts/run-handed-reds-tests.sh`, or
//! `cargo nextest run -p logicaffeine-tests --features handed-reds --cargo-profile test-opt --test spatial_disaggregation_reds_lock`.

#![cfg(feature = "codegen")]
#![cfg(feature = "handed-reds")]

#[path = "cli_common/mod.rs"]
mod cli;
#[path = "common/tiny_models.rs"]
mod tiny_models;
#[path = "common/samba_handoff.rs"]
mod samba_handoff;

use logicaffeine_compile::spatial::target::SpatialTarget;
use samba_handoff::{close_to_reference, decode_on_bundle, decode_on_layers, prefill_on, split_decode, Model, CTX, PROMPT};
use tiny_models::Tiny;

/// The architectures a hand-off covers: attention alone, a hybrid with recurrent state, windowed attention with sinks
/// and an expert mixture.
const SHARED: [Tiny; 4] = [Tiny::Qwen3, Tiny::Qwen35, Tiny::Gemma3, Tiny::GptOss];

#[test]
fn a_prompt_prefilled_on_a_gpu_decodes_on_the_rdu() {
    for t in SHARED {
        let m = Model::write(t.spec(CTX), &format!("{t:?}"));
        let handed = prefill_on(&m, &SpatialTarget::rtx3060(), &PROMPT);
        let logits = decode_on_bundle(&m, Some(&handed));
        close_to_reference(&m, &logits, &format!("{t:?}: prefilled on the GPU, decoded on the RDU"));
    }
}

#[test]
fn a_prompt_prefilled_on_the_rdu_decodes_on_a_gpu() {
    for t in SHARED {
        let m = Model::write(t.spec(CTX), &format!("{t:?}"));
        let handed = prefill_on(&m, &SpatialTarget::sn50_rdu(), &PROMPT);
        let logits = decode_on_layers(&m, &SpatialTarget::rtx3060(), &handed);
        close_to_reference(&m, &logits, &format!("{t:?}: prefilled on the RDU, decoded on the GPU"));
    }
}

#[test]
fn a_hand_off_decodes_as_a_prompt_kept_on_one_chip() {
    // The RDU's own prefill handed to its own decode: the hand-off is the identity on one chip.
    for t in SHARED {
        let m = Model::write(t.spec(CTX), &format!("{t:?}"));
        let handed = prefill_on(&m, &SpatialTarget::sn50_rdu(), &PROMPT);
        close_to_reference(&m, &decode_on_bundle(&m, Some(&handed)), &format!("{t:?}: prefilled and decoded on the RDU"));
    }
}

#[test]
fn a_model_split_over_two_chips_hands_its_hidden_state_on() {
    // The first half of the layers on the GPU, the rest on the RDU, each with its own share of the prefilled state.
    for t in SHARED {
        let m = Model::write(t.spec(CTX), &format!("{t:?}"));
        let handed = prefill_on(&m, &SpatialTarget::rtx3060(), &PROMPT);
        let cut = m.spec.layers.len() / 2;
        let logits = split_decode(&m, &handed, cut, (&SpatialTarget::rtx3060(), &SpatialTarget::sn50_rdu()));
        close_to_reference(&m, &logits, &format!("{t:?}: layers below {cut} on the GPU, the rest on the RDU"));
    }
}
