//! deploy_deepseek3_lock — DEPLOY-DEEPSEEK3 of `benchmarks/PREPARE_FOR_SAMBA.md`: DeepSeek-V3 and R1 on the SN50 through
//! the generic path.
//!
//! The family's tiny twin (latent attention with a query LoRA and YaRN, a dense first block, then sigmoid routing
//! limited to the top groups with a selection bias, a shared expert and a routed scale) brings up from its Hugging Face
//! directory and computes what transformers, the f64 reference and the VM compute (`common/samba_deploy.rs`); and V3's
//! shape, which R1 shares, at its real widths, lowers for the SN50 one program per layer kind, each certified.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test deploy_deepseek3_lock`

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
use samba_deploy::{at_size, every_layer_kind_lowers_and_certifies, the_tiny_deploys, Experts, Latent, Size, DENSE};

/// DeepSeek-V3's shape, as its `config.json` states it (R1 is the same model): a dense block `intermediate_size` wide,
/// each routed and shared expert `moe_intermediate_size`, eight groups of which a token's experts come from four.
const SIZES: [Size; 1] = [Size {
    name: "DeepSeek-V3",
    d: 7168,
    heads: 128,
    kv_heads: 128,
    dff: 18_432,
    vocab: 129_280,
    experts: Some(Experts { count: 256, top_k: 8, dff: 2048, shared_dff: 2048, groups: Some((8, 4)) }),
    latent: Some(Latent { q_lora: Some(1536), kv_lora: 512, qk_nope: 128, qk_rope: 64, v_dim: 128 }),
    ..DENSE
}];

#[test]
fn the_tiny_bundle_equals_the_vm_and_the_references() {
    the_tiny_deploys("deepseek2");
}

#[test]
fn every_real_width_layer_kind_lowers_and_certifies() {
    if !common::require_tool("hf-twins") {
        return;
    }
    let (dir, _) = common::hf_twins::twin("deepseek2");
    let twin = ModelSpec::from_source(&*open_checkpoint(&dir).expect("the twin")).expect("DeepSeek-V3's spec");
    every(SIZES.iter().map(|s| (s.name.to_string(), s)), |size| every_layer_kind_lowers_and_certifies(&at_size(&twin, size), size.name));
}
