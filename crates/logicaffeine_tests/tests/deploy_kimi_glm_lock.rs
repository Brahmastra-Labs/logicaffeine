//! deploy_kimi_glm_lock — GLM-4.5 (`glm4moe`) and Kimi K2 (`deepseek2`), DEPLOY-KIMI-GLM of
//! `benchmarks/PREPARE_FOR_SAMBA.md`, on the SN50 through the generic path.
//!
//! GLM-4.5's tiny twin (biased projections with per-head QK norms under a partial NEOX rotation, a dense block then
//! DeepSeek's mixture under sigmoid scores, the FFN norm `model::layout` renames, and the draft block a published
//! checkpoint carries) and Kimi K2's (DeepSeek-V3's latent attention and mixture with one expert group, a 2.827 routed
//! scale and Kimi's YaRN ramp) bring up from their Hugging Face directories and compute what transformers, the f64
//! reference and the VM compute (`common/samba_deploy.rs`); and every size each publishes, at its real widths, lowers
//! for the SN50 one program per layer kind, each certified — Kimi K2's router row of 384 experts in blocks of the row.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test deploy_kimi_glm_lock`

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

/// GLM-4.5's sizes, as their `config.json` states them: GLM-4.5, GLM-4.6 and GLM-4.7 share one shape (355B, 32B
/// active, per-head QK norms); GLM-4.5-Air is the 106B without them.
const SIZES: [Size; 2] = [
    Size {
        name: "GLM-4.5, GLM-4.6, GLM-4.7",
        d: 5120,
        heads: 96,
        kv_heads: 8,
        head_dim: 128,
        dff: 12_288,
        vocab: 151_552,
        experts: Some(Experts { count: 160, top_k: 8, dff: 1536, shared_dff: 1536, groups: None }),
        qk_norm: Some(true),
        ..DENSE
    },
    Size {
        name: "GLM-4.5-Air",
        d: 4096,
        heads: 96,
        kv_heads: 8,
        head_dim: 128,
        dff: 10_944,
        vocab: 151_552,
        experts: Some(Experts { count: 128, top_k: 8, dff: 1408, shared_dff: 1408, groups: None }),
        qk_norm: Some(false),
        ..DENSE
    },
];

/// Kimi K2's size, as its `config.json` states it (Instruct, 0905 and Thinking alike): DeepSeek-V3's latent attention at
/// 64 heads, a dense first block, 384 experts of which a token takes 8 beside one shared, in one group.
const KIMI: [Size; 1] = [Size {
    name: "Kimi K2",
    d: 7168,
    heads: 64,
    kv_heads: 64,
    dff: 18_432,
    vocab: 163_840,
    experts: Some(Experts { count: 384, top_k: 8, dff: 2048, shared_dff: 2048, groups: None }),
    latent: Some(Latent { q_lora: Some(1536), kv_lora: 512, qk_nope: 128, qk_rope: 64, v_dim: 128 }),
    ..DENSE
}];

#[test]
fn the_tiny_bundle_equals_the_vm_and_the_references() {
    the_tiny_deploys("glm4moe");
}

#[test]
fn kimi_k2s_tiny_bundle_equals_the_vm_and_the_references() {
    the_tiny_deploys("kimi_k2");
}


#[test]
fn every_real_width_layer_kind_lowers_and_certifies() {
    if !common::require_tool("hf-twins") {
        return;
    }
    let (dir, _) = common::hf_twins::twin("glm4moe");
    let twin = ModelSpec::from_source(&*open_checkpoint(&dir).expect("the twin")).expect("GLM-4.5's spec");
    every(SIZES.iter().map(|s| (s.name.to_string(), s)), |size| every_layer_kind_lowers_and_certifies(&at_size(&twin, size), size.name));
}

/// Kimi K2 at its real widths: its twin's architecture at 384 experts, the router row of twelve tiles scored and routed
/// in blocks of the row (`spatial_moe_real_width_lock`).
#[test]
fn every_kimi_k2_layer_kind_lowers_and_certifies() {
    if !common::require_tool("hf-twins") {
        return;
    }
    let (dir, _) = common::hf_twins::twin("kimi_k2");
    let twin = ModelSpec::from_source(&*open_checkpoint(&dir).expect("the twin")).expect("Kimi K2's spec");
    every(KIMI.iter().map(|s| (s.name.to_string(), s)), |size| every_layer_kind_lowers_and_certifies(&at_size(&twin, size), size.name));
}
