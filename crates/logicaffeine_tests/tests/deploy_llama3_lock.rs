//! deploy_llama3_lock — DEPLOY-LLAMA3 of `benchmarks/PREPARE_FOR_SAMBA.md`: Llama 3.x on the SN50 through the generic path.
//!
//! The family's tiny twin (Llama's attention with its interleaved rotation and Llama 3's frequency scaling; a tied
//! head) brings up from its Hugging Face directory and computes what transformers, the f64 reference and the VM compute
//! (`common/samba_deploy.rs`); and every size of Llama 3.1, 3.2 and 3.3, at its real widths, lowers for the SN50 one
//! program per layer kind, each certified.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test deploy_llama3_lock`

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

/// Every size of Llama 3.1, 3.2 and 3.3, as each one's `config.json` states it (3.3 70B is 3.1 70B's shape).
const SIZES: [Size; 5] = [
    Size { name: "Llama-3.2-1B", d: 2048, heads: 32, kv_heads: 8, head_dim: 64, dff: 8192, vocab: 128_256, tied: true, ..DENSE },
    Size { name: "Llama-3.2-3B", d: 3072, heads: 24, kv_heads: 8, head_dim: 128, dff: 8192, vocab: 128_256, tied: true, ..DENSE },
    Size { name: "Llama-3.1-8B", d: 4096, heads: 32, kv_heads: 8, head_dim: 128, dff: 14_336, vocab: 128_256, tied: false, ..DENSE },
    Size { name: "Llama-3.1-70B", d: 8192, heads: 64, kv_heads: 8, head_dim: 128, dff: 28_672, vocab: 128_256, tied: false, ..DENSE },
    Size { name: "Llama-3.1-405B", d: 16_384, heads: 128, kv_heads: 8, head_dim: 128, dff: 53_248, vocab: 128_256, tied: false, ..DENSE },
];

#[test]
fn the_tiny_bundle_equals_the_vm_and_the_references() {
    the_tiny_deploys("llama");
}

#[test]
fn every_real_width_layer_kind_lowers_and_certifies() {
    if !common::require_tool("hf-twins") {
        return;
    }
    let (dir, _) = common::hf_twins::twin("llama");
    let twin = ModelSpec::from_source(&*open_checkpoint(&dir).expect("the twin")).expect("Llama's spec");
    every(SIZES.iter().map(|s| (s.name.to_string(), s)), |size| every_layer_kind_lowers_and_certifies(&at_size(&twin, size), size.name));
}
