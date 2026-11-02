//! deploy_llama4_lock — DEPLOY-LLAMA4 of `benchmarks/PREPARE_FOR_SAMBA.md`: Llama 4 on the SN50 through the generic path.
//!
//! The family's tiny twin (Scout's text model: rotated layers in chunks with a weightless QK norm, a rotation-free layer
//! whose queries take the attention temperature, experts on alternate layers, one a token, its input scaled by its
//! router's sigmoid, beside a shared expert; Llama 3's frequency factors) brings up from its Hugging Face directory and
//! computes what transformers, the f64 reference and the VM compute (`common/samba_deploy.rs`); and both Llama 4 sizes,
//! at their real widths, lower for the SN50 one program per layer kind, each certified.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test deploy_llama4_lock`

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

/// Both Llama 4 sizes, as their `config.json` states them: a dense layer `intermediate_size_mlp` wide, each expert and
/// the shared one `intermediate_size`.
const SIZES: [Size; 2] = [
    Size {
        name: "Llama-4-Scout-17B-16E",
        d: 5120,
        heads: 40,
        kv_heads: 8,
        head_dim: 128,
        dff: 16_384,
        vocab: 202_048,
        experts: Some(Experts { count: 16, top_k: 1, dff: 8192, shared_dff: 8192, groups: None }),
        ..DENSE
    },
    Size {
        name: "Llama-4-Maverick-17B-128E",
        d: 5120,
        heads: 40,
        kv_heads: 8,
        head_dim: 128,
        dff: 16_384,
        vocab: 202_048,
        experts: Some(Experts { count: 128, top_k: 1, dff: 8192, shared_dff: 8192, groups: None }),
        ..DENSE
    },
];

#[test]
fn the_tiny_bundle_equals_the_vm_and_the_references() {
    the_tiny_deploys("llama4");
}

#[test]
fn every_real_width_layer_kind_lowers_and_certifies() {
    if !common::require_tool("hf-twins") {
        return;
    }
    let (dir, _) = common::hf_twins::twin("llama4");
    let twin = ModelSpec::from_source(&*open_checkpoint(&dir).expect("the twin")).expect("Llama 4's spec");
    every(SIZES.iter().map(|s| (s.name.to_string(), s)), |size| every_layer_kind_lowers_and_certifies(&at_size(&twin, size), size.name));
}
