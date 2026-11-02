//! deploy_glm_lock — GLM-4 (`glm4`, INTAKE-2 of `benchmarks/PREPARE_FOR_SAMBA.md`) on the SN50 through the generic path.
//!
//! The family's tiny twin (half of each head rotated by interleaved pairs, biased projections, sandwich norms, and one
//! gate-and-up projection presented apart by `model::layout`) brings up from its Hugging Face directory and computes what
//! transformers, the f64 reference and the VM compute (`common/samba_deploy.rs`); and both GLM-4 0414 sizes, at their
//! real widths, lower for the SN50 one program per layer kind, each certified.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test deploy_glm_lock`

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

/// Both GLM-4 0414 sizes, as their `config.json` states them: the 9B's projections biased, the 32B's not.
const SIZES: [Size; 2] = [
    Size { name: "GLM-4-9B-0414", d: 4096, heads: 32, kv_heads: 2, head_dim: 128, dff: 13_696, vocab: 151_552, bias: Some(true), ..DENSE },
    Size { name: "GLM-4-32B-0414", d: 6144, heads: 48, kv_heads: 2, head_dim: 128, dff: 23_040, vocab: 151_552, bias: Some(false), ..DENSE },
];

#[test]
fn the_tiny_bundle_equals_the_vm_and_the_references() {
    the_tiny_deploys("glm4");
}

#[test]
fn every_real_width_layer_kind_lowers_and_certifies() {
    if !common::require_tool("hf-twins") {
        return;
    }
    let (dir, _) = common::hf_twins::twin("glm4");
    let twin = ModelSpec::from_source(&*open_checkpoint(&dir).expect("the twin")).expect("GLM-4's spec");
    every(SIZES.iter().map(|s| (s.name.to_string(), s)), |size| every_layer_kind_lowers_and_certifies(&at_size(&twin, size), size.name));
}
