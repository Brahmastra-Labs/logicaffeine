//! deploy_mixtral_lock — DEPLOY-MIXTRAL of `benchmarks/PREPARE_FOR_SAMBA.md`: Mixtral on the SN50 through the generic path.
//!
//! The family's tiny twin (Mistral's attention with its interleaved rotation, eight experts' worth of routing in four,
//! two a token, the kept softmax weights renormalized) brings up from its Hugging Face directory and computes what
//! transformers, the f64 reference and the VM compute (`common/samba_deploy.rs`); and each Mixtral size, at its real
//! widths, lowers for the SN50 one program per layer kind, each certified.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test deploy_mixtral_lock`

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

/// Each Mixtral size, as its `config.json` states it.
const SIZES: [Size; 2] = [
    Size {
        name: "Mixtral-8x7B",
        d: 4096,
        heads: 32,
        kv_heads: 8,
        head_dim: 128,
        vocab: 32_000,
        experts: Some(Experts { count: 8, top_k: 2, dff: 14_336, shared_dff: 0, groups: None }),
        ..DENSE
    },
    Size {
        name: "Mixtral-8x22B",
        d: 6144,
        heads: 48,
        kv_heads: 8,
        head_dim: 128,
        vocab: 32_768,
        experts: Some(Experts { count: 8, top_k: 2, dff: 16_384, shared_dff: 0, groups: None }),
        ..DENSE
    },
];

#[test]
fn the_tiny_bundle_equals_the_vm_and_the_references() {
    the_tiny_deploys("mixtral");
}

#[test]
fn every_real_width_layer_kind_lowers_and_certifies() {
    if !common::require_tool("hf-twins") {
        return;
    }
    let (dir, _) = common::hf_twins::twin("mixtral");
    let twin = ModelSpec::from_source(&*open_checkpoint(&dir).expect("the twin")).expect("Mixtral's spec");
    every(SIZES.iter().map(|s| (s.name.to_string(), s)), |size| every_layer_kind_lowers_and_certifies(&at_size(&twin, size), size.name));
}
