//! deploy_gemma_lock — DEPLOY-GEMMA of `benchmarks/PREPARE_FOR_SAMBA.md`: Gemma, Gemma 2 and Gemma 3 on the SN50
//! through the generic path.
//!
//! Every size of the three, at its real widths and as its published configuration states them, lowers for the SN50 one
//! program per layer kind, each certified: Gemma 7B's heads wider than its width over its heads, Gemma 2's local and
//! global layers with their scores capped at 50 and its logits at 30. Gemma's and Gemma 2's tiny twins bring up from
//! their directories; Gemma 3's is a handed RED in `deploy_reds_lock` (NUM-1).
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test deploy_gemma_lock`

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

/// Every Gemma 3 size, as its `config.json` states it: 27B scores by `1/√query_pre_attn_scalar`, 1/√168 over heads of 128.
const SIZES: [Size; 4] = [
    Size { name: "gemma-3-1b", d: 1152, heads: 4, kv_heads: 1, head_dim: 256, dff: 6912, vocab: 262_144, tied: true, window: Some(512), ..DENSE },
    Size { name: "gemma-3-4b", d: 2560, heads: 8, kv_heads: 4, head_dim: 256, dff: 10_240, vocab: 262_208, tied: true, window: Some(1024), ..DENSE },
    Size { name: "gemma-3-12b", d: 3840, heads: 16, kv_heads: 8, head_dim: 256, dff: 15_360, vocab: 262_208, tied: true, window: Some(1024), ..DENSE },
    Size {
        name: "gemma-3-27b",
        d: 5376,
        heads: 32,
        kv_heads: 16,
        head_dim: 128,
        dff: 21_504,
        vocab: 262_208,
        tied: true,
        scale: Some(0.077_151_674_981_045_96),
        window: Some(1024),
        ..DENSE
    },
];

#[test]
fn every_real_width_layer_kind_lowers_and_certifies() {
    if !common::require_tool("hf-twins") {
        return;
    }
    let (dir, _) = common::hf_twins::twin("gemma3");
    let twin = ModelSpec::from_source(&*open_checkpoint(&dir).expect("the twin")).expect("Gemma 3's spec");
    every(SIZES.iter().map(|s| (s.name.to_string(), s)), |size| every_layer_kind_lowers_and_certifies(&at_size(&twin, size), size.name));
}

/// Every Gemma 2 size, as its `config.json` states it: each caps its scores at 50 and its logits at 30 over a
/// 4,096-token window; 27B scores by `1/√query_pre_attn_scalar`, 1/√144 over heads of 128.
const GEMMA2_SIZES: [Size; 3] = [
    Size { name: "gemma-2-2b", d: 2304, heads: 8, kv_heads: 4, head_dim: 256, dff: 9216, vocab: 256_000, tied: true, window: Some(4096), softcaps: Some((50.0, 30.0)), ..DENSE },
    Size { name: "gemma-2-9b", d: 3584, heads: 16, kv_heads: 8, head_dim: 256, dff: 14_336, vocab: 256_000, tied: true, window: Some(4096), softcaps: Some((50.0, 30.0)), ..DENSE },
    Size {
        name: "gemma-2-27b",
        d: 4608,
        heads: 32,
        kv_heads: 16,
        head_dim: 128,
        dff: 36_864,
        vocab: 256_000,
        tied: true,
        scale: Some(1.0 / 12.0),
        window: Some(4096),
        softcaps: Some((50.0, 30.0)),
        ..DENSE
    },
];

/// Every Gemma size, as its `config.json` states it: 2B's one key-value head, 7B's heads 256 wide over a width of 3072.
const GEMMA_SIZES: [Size; 2] = [
    Size { name: "gemma-2b", d: 2048, heads: 8, kv_heads: 1, head_dim: 256, dff: 16_384, vocab: 256_000, tied: true, ..DENSE },
    Size { name: "gemma-7b", d: 3072, heads: 16, kv_heads: 16, head_dim: 256, dff: 24_576, vocab: 256_000, tied: true, ..DENSE },
];

/// Gemma's twin (heads wider than its width over its heads, GELU under its legacy name, the √d embedding scale) from its
/// directory: transformers, the reference and the VM agree, and the bundle brings up on the SN50 within its datapath's
/// bound.
#[test]
fn gemmas_tiny_bundle_equals_the_vm_and_the_references() {
    the_tiny_deploys("gemma");
}

#[test]
fn every_gemma_size_lowers_and_certifies() {
    if !common::require_tool("hf-twins") {
        return;
    }
    let (dir, _) = common::hf_twins::twin("gemma");
    let twin = ModelSpec::from_source(&*open_checkpoint(&dir).expect("the twin")).expect("Gemma's spec");
    every(GEMMA_SIZES.iter().map(|s| (s.name.to_string(), s)), |size| every_layer_kind_lowers_and_certifies(&at_size(&twin, size), size.name));
}

/// Gemma 2's twin (alternating windows, scores and logits capped where the caps bind, sandwich norms, GELU, the √d
/// embedding scale) from its directory: transformers, the reference and the VM agree, and the bundle brings up on the
/// SN50 within its datapath's bound.
#[test]
fn gemma_2s_tiny_bundle_equals_the_vm_and_the_references() {
    the_tiny_deploys("gemma2");
}

#[test]
fn every_gemma2_size_lowers_and_certifies() {
    if !common::require_tool("hf-twins") {
        return;
    }
    let (dir, _) = common::hf_twins::twin("gemma2");
    let twin = ModelSpec::from_source(&*open_checkpoint(&dir).expect("the twin")).expect("Gemma 2's spec");
    every(GEMMA2_SIZES.iter().map(|s| (s.name.to_string(), s)), |size| {
        let spec = at_size(&twin, size);
        assert_eq!(spec.final_logit_softcap, Some(30.0), "the logits' cap");
        every_layer_kind_lowers_and_certifies(&spec, size.name)
    });
}

#[test]
fn every_size_is_its_published_configuration() {
    if !common::require_tool("hf-configs") {
        return;
    }
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../external/hf-configs/unsloth");
    let published = GEMMA_SIZES
        .iter()
        .zip(["gemma-2b-it", "gemma-7b-it"])
        .chain(GEMMA_SIZES.iter().zip(["gemma-1.1-2b-it", "gemma-1.1-7b-it"]))
        .chain(GEMMA2_SIZES.iter().zip(["gemma-2-2b-it", "gemma-2-9b-it", "gemma-2-27b-it"]))
        .chain(SIZES.iter().zip(["gemma-3-1b-it", "gemma-3-4b-it", "gemma-3-12b-it", "gemma-3-27b-it"]));
    every(published.map(|(s, repo)| (repo.to_string(), (s, repo))), |(size, repo)| {
        let path = root.join(repo).join("config.json");
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
        let top: serde_json::Value = serde_json::from_str(&text).expect("config.json parses");
        let c = top.get("text_config").unwrap_or(&top);
        let n = |k: &str| c[k].as_u64().unwrap_or_else(|| panic!("`{k}` is a number")) as usize;
        assert_eq!(
            (size.d, size.heads, size.kv_heads, size.head_dim, size.dff, size.vocab),
            (n("hidden_size"), n("num_attention_heads"), n("num_key_value_heads"), n("head_dim"), n("intermediate_size"), n("vocab_size")),
            "the width, the heads, the head width, the FFN and the vocabulary"
        );
        assert_eq!(size.window, c.get("sliding_window").and_then(serde_json::Value::as_u64).map(|w| w as usize), "the window");
        let scalar = c.get("query_pre_attn_scalar").and_then(serde_json::Value::as_u64).unwrap_or(size.head_dim as u64);
        let (ours, theirs) = (size.scale.unwrap_or(1.0 / (size.head_dim as f64).sqrt()), 1.0 / (scalar as f64).sqrt());
        assert!((ours - theirs).abs() <= theirs * 1e-15, "the score scale: {ours} against the configuration's {theirs}");
        let cap = |k: &str| c.get(k).and_then(serde_json::Value::as_f64);
        assert_eq!(size.softcaps, cap("attn_logit_softcapping").zip(cap("final_logit_softcapping")), "the caps");
        assert_eq!(size.tied, top["tie_word_embeddings"].as_bool().unwrap_or(true), "the tied head (Gemma's default)");
    });
}
