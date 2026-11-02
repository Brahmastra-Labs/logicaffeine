//! deploy_reds_lock — the Samba book's DEPLOY REDs a ticket still owes, each handed over frozen
//! (`fixtures/frozen_reds.tsv`); the ticket that lands one moves it into its family's deploy lock unchanged.
//!
//! Run: `./scripts/run-handed-reds-tests.sh`

#![cfg(feature = "handed-reds")]
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

use samba_deploy::the_tiny_deploys;

/// Gemma 3's twin (local and global layers, QK-norm, sandwich norms, GELU, the √d embedding scale) from its directory:
/// transformers, the reference and the VM agree; on the SN50's bf16 datapath a logit lands 2.04% of the largest from the
/// reference's against its 2% bound (`logs/samba/deploy-gemma-1.log`), and with the SIMD path's fp32 witness the step
/// holds (`logs/samba/deploy-gemma-simd-fp32.log`). NUM-1 lands it.
#[test]
fn gemma_3s_tiny_bundle_equals_the_vm_and_the_references() {
    the_tiny_deploys("gemma3");
}

/// Qwen 3.5's mixture twin (Gated DeltaNet and gated attention, four experts a token's top two of, beside a shared expert
/// under its sigmoid gate) from its directory: transformers, the reference and the VM agree, and the bundle lowered with
/// fp32 in place of bf16 is the reference's within 1.4e-6 of the largest logit. Its routing is the reference's: the
/// closest call is decided by 0.036 on a score of 0.23, and the call flipped lands 1.5 from the bundle. On the SN50's
/// bf16 datapath a logit lands 2.54% of the largest from the reference's against its 2% bound, the error growing layer
/// by layer (`logs/samba/diag-qwen35moe-layers.log`); with the SIMD path's fp32 witness it lands 1.34%
/// (`logs/samba/diag-qwen35moe-simd-fp32.log`). NUM-1 lands it.
#[test]
fn qwen_3_5s_mixtures_tiny_bundle_equals_the_vm_and_the_references() {
    the_tiny_deploys("qwen35moe");
}
