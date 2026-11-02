//! deploy_phi_lock — Phi-4 (`phi3`, INTAKE-2 of `benchmarks/PREPARE_FOR_SAMBA.md`) on the SN50 through the generic path.
//!
//! The family's tiny twin (one query-key-value projection and one gate-and-up projection, as the converter writes them,
//! presented apart by `model::layout`; NEOX rotation; no window) brings up from its Hugging Face directory and computes
//! what transformers, the f64 reference and the VM compute (`common/samba_deploy.rs`); and Phi-4's shape, at its real
//! widths, lowers for the SN50 one program per layer kind, each certified.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test deploy_phi_lock`

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
use samba_deploy::{at_size, every_layer_kind_lowers_and_certifies, the_tiny_deploys, Size, DENSE};

/// Phi-4's shape, as its `config.json` states it: 40 heads over 10 key/value heads of 128.
const SIZES: [Size; 1] = [Size { name: "phi-4", d: 5120, heads: 40, kv_heads: 10, head_dim: 128, dff: 17_920, vocab: 100_352, ..DENSE }];

#[test]
fn the_tiny_bundle_equals_the_vm_and_the_references() {
    the_tiny_deploys("phi3");
}

#[test]
fn every_real_width_layer_kind_lowers_and_certifies() {
    if !common::require_tool("hf-twins") {
        return;
    }
    let (dir, _) = common::hf_twins::twin("phi3");
    let twin = ModelSpec::from_source(&*open_checkpoint(&dir).expect("the twin")).expect("Phi-4's spec");
    every(SIZES.iter().map(|s| (s.name.to_string(), s)), |size| every_layer_kind_lowers_and_certifies(&at_size(&twin, size), size.name));
}
