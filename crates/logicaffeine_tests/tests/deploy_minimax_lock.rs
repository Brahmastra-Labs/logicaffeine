//! deploy_minimax_lock — DEPLOY-MINIMAX of `benchmarks/PREPARE_FOR_SAMBA.md`: MiniMax-M2 on the SN50 through the generic
//! path.
//!
//! The family's tiny twin (its query and key each normed across every head at once, half of each head rotated, and in
//! every block experts scored by sigmoids, chosen by score plus a routing bias, the kept weights renormalized) brings up
//! from its Hugging Face directory and computes what transformers, the f64 reference and the VM compute
//! (`common/samba_deploy.rs`); and MiniMax-M2's shape, at its real widths, lowers for the SN50 one program per layer kind,
//! each certified.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test deploy_minimax_lock`

#![cfg(feature = "codegen")]

mod common;
#[path = "cli_common/mod.rs"]
mod cli;
#[path = "common/tiny_models.rs"]
mod tiny_models;
#[path = "common/samba_bringup.rs"]
mod samba_bringup;
#[path = "common/model_harness.rs"]
mod model_harness;
#[path = "common/samba_deploy.rs"]
mod samba_deploy;

use logicaffeine_compile::model::open_checkpoint;
use logicaffeine_compile::model::spec::ModelSpec;
use samba_bringup::every;
use samba_deploy::{at_size, every_layer_kind_lowers_and_certifies, the_tiny_deploys, Experts, Size, DENSE};

/// MiniMax-M2's shape, as its `config.json` states it: 256 experts of 1,536, eight a token, and 64 of each head's 128
/// dimensions rotated.
const SIZES: [Size; 1] = [Size {
    name: "MiniMax-M2",
    d: 3072,
    heads: 48,
    kv_heads: 8,
    head_dim: 128,
    vocab: 200_064,
    experts: Some(Experts { count: 256, top_k: 8, dff: 1536, shared_dff: 0, groups: None }),
    ..DENSE
}];

#[test]
fn the_tiny_bundle_equals_the_vm_and_the_references() {
    the_tiny_deploys("minimax_m2");
}

#[test]
fn every_real_width_layer_kind_lowers_and_certifies() {
    if !common::require_tool("hf-twins") {
        return;
    }
    let (dir, _) = common::hf_twins::twin("minimax_m2");
    let twin = ModelSpec::from_source(&*open_checkpoint(&dir).expect("the twin")).expect("MiniMax-M2's spec");
    every(SIZES.iter().map(|s| (s.name.to_string(), s)), |size| every_layer_kind_lowers_and_certifies(&at_size(&twin, size), size.name));
}
