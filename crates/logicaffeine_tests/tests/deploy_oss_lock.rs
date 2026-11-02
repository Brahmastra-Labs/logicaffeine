//! deploy_oss_lock — DEPLOY-OSS of `benchmarks/PREPARE_FOR_SAMBA.md`: gpt-oss on the SN50 through the generic path.
//!
//! The family's tiny twin (biased attention with sinks, a 128-token window on alternate layers, YaRN, biased experts
//! under the clamped SwiGLU) brings up from its Hugging Face directory and computes what transformers, the f64 reference
//! and the VM compute (`common/samba_deploy.rs`); and both gpt-oss sizes, at their real widths, lower for the SN50 one
//! program per layer kind, each certified.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test deploy_oss_lock`

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

/// Both gpt-oss sizes, as their `config.json` states them.
const SIZES: [Size; 2] = [
    Size {
        name: "gpt-oss-20b",
        d: 2880,
        heads: 64,
        kv_heads: 8,
        head_dim: 64,
        vocab: 201_088,
        experts: Some(Experts { count: 32, top_k: 4, dff: 2880, shared_dff: 0, groups: None }),
        ..DENSE
    },
    Size {
        name: "gpt-oss-120b",
        d: 2880,
        heads: 64,
        kv_heads: 8,
        head_dim: 64,
        vocab: 201_088,
        experts: Some(Experts { count: 128, top_k: 4, dff: 2880, shared_dff: 0, groups: None }),
        ..DENSE
    },
];

#[test]
fn the_tiny_bundle_equals_the_vm_and_the_references() {
    the_tiny_deploys("gpt-oss");
}

#[test]
fn every_real_width_layer_kind_lowers_and_certifies() {
    if !common::require_tool("hf-twins") {
        return;
    }
    let (dir, _) = common::hf_twins::twin("gpt-oss");
    let twin = ModelSpec::from_source(&*open_checkpoint(&dir).expect("the twin")).expect("gpt-oss's spec");
    every(SIZES.iter().map(|s| (s.name.to_string(), s)), |size| every_layer_kind_lowers_and_certifies(&at_size(&twin, size), size.name));
}
