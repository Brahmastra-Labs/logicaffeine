//! deploy_qwen25_lock — DEPLOY-QWEN25 of `benchmarks/PREPARE_FOR_SAMBA.md`: Qwen 2.5 on the SN50 through the generic path.
//!
//! The family's tiny twin (Qwen2's attention, biased on its query, key and value projections; a tied head) brings up
//! from its Hugging Face directory and computes what transformers, the f64 reference and the VM compute
//! (`common/samba_deploy.rs`); and every size Qwen 2.5 publishes, at its real widths, lowers for the SN50 one program
//! per layer kind, each certified.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test deploy_qwen25_lock`

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

/// Every size Qwen 2.5 publishes, as each one's `config.json` states it.
const SIZES: [Size; 7] = [
    Size { name: "Qwen2.5-0.5B", d: 896, heads: 14, kv_heads: 2, head_dim: 64, dff: 4864, vocab: 151_936, tied: true, ..DENSE },
    Size { name: "Qwen2.5-1.5B", d: 1536, heads: 12, kv_heads: 2, head_dim: 128, dff: 8960, vocab: 151_936, tied: true, ..DENSE },
    Size { name: "Qwen2.5-3B", d: 2048, heads: 16, kv_heads: 2, head_dim: 128, dff: 11_008, vocab: 151_936, tied: true, ..DENSE },
    Size { name: "Qwen2.5-7B", d: 3584, heads: 28, kv_heads: 4, head_dim: 128, dff: 18_944, vocab: 152_064, tied: false, ..DENSE },
    Size { name: "Qwen2.5-14B", d: 5120, heads: 40, kv_heads: 8, head_dim: 128, dff: 13_824, vocab: 152_064, tied: false, ..DENSE },
    Size { name: "Qwen2.5-32B", d: 5120, heads: 40, kv_heads: 8, head_dim: 128, dff: 27_648, vocab: 152_064, tied: false, ..DENSE },
    Size { name: "Qwen2.5-72B", d: 8192, heads: 64, kv_heads: 8, head_dim: 128, dff: 29_568, vocab: 152_064, tied: false, ..DENSE },
];

#[test]
fn the_tiny_bundle_equals_the_vm_and_the_references() {
    the_tiny_deploys("qwen2");
}

#[test]
fn every_real_width_layer_kind_lowers_and_certifies() {
    if !common::require_tool("hf-twins") {
        return;
    }
    let (dir, _) = common::hf_twins::twin("qwen2");
    let twin = ModelSpec::from_source(&*open_checkpoint(&dir).expect("the twin")).expect("Qwen2's spec");
    every(SIZES.iter().map(|s| (s.name.to_string(), s)), |size| every_layer_kind_lowers_and_certifies(&at_size(&twin, size), size.name));
}
