//! model_hf_config_reds_lock — published configurations of releases the Samba book deploys that the Hugging Face intake
//! still refuses, each a ticket's RED handed over frozen (`fixtures/frozen_reds.tsv`); the ticket that lands one moves
//! it into `model_hf_intake_lock::every_published_configuration_reads` unchanged.
//!
//! Run: `./scripts/run-handed-reds-tests.sh`

#![cfg(feature = "handed-reds")]
#![cfg(feature = "codegen")]

mod common;

use logicaffeine_compile::model::hf::HfCheckpoint;
use std::path::PathBuf;

fn reads(repo: &str, arch: &str) {
    if !common::require_tool("hf-configs") {
        return;
    }
    let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../external/hf-configs").join(repo);
    let (got, _) = HfCheckpoint::read_configuration(&dir).unwrap_or_else(|e| panic!("{repo}: {e}"));
    assert_eq!(got, arch, "{repo}: the architecture");
}

/// gpt-oss as OpenAI publishes it stores its experts in MXFP4 (`quantization_config` mxfp4), which the converter repacks
/// into GGUF's MXFP4 blocks. INTAKE-3 lands it.
#[test]
fn gpt_oss_published_configurations_read() {
    reads("openai/gpt-oss-20b", "gpt-oss");
    reads("openai/gpt-oss-120b", "gpt-oss");
}

/// Kimi K2 Thinking stores its experts as int4 with group scales (`quantization_config` compressed-tensors). INTAKE-3
/// lands it.
#[test]
fn kimi_k2_thinking_published_configuration_reads() {
    reads("moonshotai/Kimi-K2-Thinking", "deepseek2");
}
