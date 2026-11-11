//! spatial_disaggregation_lock — a model's work shared between chips: a prompt prefilled on one decodes on another, and a
//! model's layers split over two hand their hidden state on (SERVE-RDU-3 of `benchmarks/PREPARE_FOR_SAMBA.md`).
//!
//! SambaNova serves MiniMax with prefill on GPUs and decode on its RDUs, the cache handed over the network [HC26]. Here
//! both sides are ours, so the hand-off is agreed at compile time: the state a decode step carries — every attention
//! layer's key and value cache, every gated-delta layer's memory and conv rings — named and shaped as the author
//! declares it in the one program every backend lowers (`spatial::handoff::contract`). A prefill on any chip leaves
//! exactly those tensors; a decode on any chip reads exactly those; a bundle's manifest states them so a host on either
//! side knows what to send.
//!
//! The GPU here is the CUDA target's program in the spatial IR (`SpatialTarget::rtx3060`), run by the spatial
//! evaluator; the RDU is the SN50 bundle `largo emit rdu` writes, read back off disk and run as a host runs it. Each
//! decode's logits are held to the f64 reference's continuation of the same prompt within the bf16 datapath's bound
//! (`2%·√(d/64)` of the largest logit, as `rdu_bringup_lock`'s are).
//!
//! The longest prompt a decode can follow — `CTX − 1` tokens — prefills whole on the GPU target: its rotations and
//! per-head norms of 28 heads of 16 are past an RTX 3060 block's memory and inside its accumulator's page, so they take
//! a head, a tile or a block of the row an item (`spatial_site_memory_lock`), and every tensor the prefill hands on is
//! the whole-row forms' value for value.
//!
//! The decodes themselves — a prompt prefilled on the GPU target decoded on the RDU and the reverse, a prompt kept on the
//! RDU, a model split over the two — are `spatial_disaggregation_reds_lock`'s handed REDs until NUM-1: the SN50's bf16
//! elementwise datapath drifts past the reference's bound, and with fp32 elementwise every one of them holds.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test spatial_disaggregation_lock`

#![cfg(feature = "codegen")]

#[path = "cli_common/mod.rs"]
mod cli;
#[path = "common/tiny_models.rs"]
mod tiny_models;
#[path = "common/samba_handoff.rs"]
#[allow(dead_code)]
mod samba_handoff;

use logicaffeine_compile::model::program::Cut;
use logicaffeine_compile::spatial::handoff::{contract, Handoff, HandoffError};
use logicaffeine_compile::spatial::target::SpatialTarget;
use samba_handoff::{authored, emit_bundle, prefill_on, Model, CTX};
use std::collections::BTreeMap;
use tiny_models::Tiny;

/// The architectures a hand-off covers: attention alone, a hybrid with recurrent state, windowed attention with sinks
/// and an expert mixture.
const SHARED: [Tiny; 4] = [Tiny::Qwen3, Tiny::Qwen35, Tiny::Gemma3, Tiny::GptOss];

#[test]
fn the_decode_bundle_states_the_state_it_carries() {
    for t in SHARED {
        let m = Model::write(t.spec(CTX), &format!("{t:?}"));
        let (_, manifest) = emit_bundle(&m);
        let want = contract(&m.spec, CTX).expect("the contract");
        let stated = &manifest["handoff"];
        assert_eq!(stated["ctx"], CTX, "{t:?}: the hand-off's cache length");
        let carried: Vec<(String, usize, usize, usize)> = stated["carried"]
            .as_array()
            .unwrap_or_else(|| panic!("{t:?}: the manifest states its carried state: {manifest}"))
            .iter()
            .map(|c| {
                (
                    c["name"].as_str().unwrap().to_string(),
                    c["layer"].as_u64().unwrap() as usize,
                    c["rows"].as_u64().unwrap() as usize,
                    c["cols"].as_u64().unwrap() as usize,
                )
            })
            .collect();
        let expected: Vec<(String, usize, usize, usize)> = want.carried.iter().map(|c| (c.name.clone(), c.layer, c.rows, c.cols)).collect();
        assert_eq!(carried, expected, "{t:?}: the manifest's carried state is the contract");
        // Every tensor a kind program binds to a layer's state is in the contract, at the contract's size.
        for b in manifest["layers"].as_array().unwrap() {
            for e in b["tensors"].as_array().unwrap().iter().filter(|e| e["role"] == "state") {
                let bound = e["layer"].as_str().unwrap();
                assert!(want.carried.iter().any(|c| c.name == bound), "{t:?}: `{bound}` is carried state the contract names");
            }
        }
    }
}

#[test]
fn a_hand_off_that_breaks_the_contract_is_refused_by_name() {
    let spec = Tiny::Qwen35.spec(CTX);
    let c = contract(&spec, CTX).expect("the contract");
    let whole: BTreeMap<String, Vec<f64>> = c.carried.iter().map(|t| (t.name.clone(), vec![0.0; t.rows * t.cols])).collect();
    assert_eq!(c.check(&Handoff { position: 2, state: whole.clone() }), Ok(()));
    let first = c.carried[0].name.clone();
    let mut missing = whole.clone();
    missing.remove(&first);
    assert_eq!(c.check(&Handoff { position: 2, state: missing }), Err(HandoffError::Missing(first.clone())));
    let mut short = whole.clone();
    short.get_mut(&first).unwrap().pop();
    let n = c.carried[0].rows * c.carried[0].cols;
    assert_eq!(c.check(&Handoff { position: 2, state: short }), Err(HandoffError::Length { name: first.clone(), want: n, got: n - 1 }));
    let mut extra = whole.clone();
    extra.insert("Kc99".into(), vec![0.0]);
    assert_eq!(c.check(&Handoff { position: 2, state: extra }), Err(HandoffError::Unknown("Kc99".into())));
    // A decode after `position` tokens writes row `position + 1`: the cache must have it.
    assert_eq!(c.check(&Handoff { position: CTX, state: whole }), Err(HandoffError::Position { position: CTX, ctx: CTX }));
}

/// The GPU target with a site's memory past anything a tiny model's rows need, so every row op keeps its whole-row form:
/// the forms a site's memory forces on the RTX 3060 are held to it.
fn whole_rows() -> SpatialTarget {
    let mut t = SpatialTarget::rtx3060();
    t.site.local_mem_bytes = 64 << 20;
    t.site.local_mem_opt_in_bytes = None;
    t
}

#[test]
fn the_longest_prompt_a_decode_follows_prefills_whole_on_the_gpu_target() {
    // `CTX − 1` tokens: a block of 7 rows whose rotations and per-head norms are 28 heads of 16 — past an RTX 3060
    // block's memory, inside its accumulator's page — so those sections take a head, a tile or a block of the row an
    // item. They compute the whole-row forms' values: every carried tensor is the whole-row twin's, value for value.
    let (rtx, twin) = (SpatialTarget::rtx3060(), whole_rows());
    let cut = Cut::Prefill { rows: CTX - 1, ctx: CTX };
    for t in SHARED {
        let m = Model::write(t.spec(CTX), &format!("{t:?}"));
        let prompt: Vec<u32> = (0..CTX as u32 - 1).map(|i| (i * 5 + 3) % m.spec.vocab as u32).collect();
        let what = format!("{t:?}'s {}-token prompt on {}", prompt.len(), rtx.name);
        let (forced, whole) = (authored(&m, cut, &rtx), authored(&m, cut, &twin));
        assert!(
            forced.stages.len() > whole.stages.len(),
            "{what}: a site's memory moves some section off its whole-row form ({} stages, the twin's {})",
            forced.stages.len(),
            whole.stages.len()
        );
        let (handed, wanted) = (prefill_on(&m, &rtx, &prompt), prefill_on(&m, &twin, &prompt));
        let c = contract(&m.spec, CTX).expect("the contract");
        assert_eq!(handed.state.len(), c.carried.len(), "{what}: every carried tensor");
        for carried in &c.carried {
            let (got, want) = (&handed.state[&carried.name], &wanted.state[&carried.name]);
            assert!(want.iter().any(|v| *v != 0.0), "{what}: `{}` holds the prompt", carried.name);
            assert_eq!(got, want, "{what}: `{}` is the whole-row forms' value for value", carried.name);
        }
    }
}
