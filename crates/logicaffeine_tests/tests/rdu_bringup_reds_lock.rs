//! rdu_bringup_reds_lock — every tiny architecture brings up on the SN50 from its checkpoint alone: the handed REDs of
//! INTAKE-6 (`benchmarks/PREPARE_FOR_SAMBA.md`), frozen in `fixtures/frozen_reds.tsv`.
//!
//! A row is brought up as `rdu_bringup_lock`'s are (`common/samba_bringup.rs`, `brings_up_close`): the GGUF checkpoint
//! handed to `largo emit rdu --chip sn50 --model <checkpoint> --ctx 4`, the bundle read back off disk, re-certified and
//! run for one decode step with every tensor filled from the checkpoint, its logits the f64 reference's within the
//! bf16 datapath's bound. A row names every architecture that fails, and why. Every architecture brings up at its own
//! width (`rdu_bringup_lock::every_tiny_architecture_brings_up_on_the_sn50`, moved there green); four times as wide,
//! the expert mixture's drift is open.
//!
//! Run on purpose: `scripts/run-handed-reds-tests.sh`, or
//! `cargo nextest run -p logicaffeine-tests --features handed-reds --cargo-profile test-opt --test rdu_bringup_reds_lock`.

#![cfg(feature = "codegen")]
#![cfg(feature = "handed-reds")]

#[path = "cli_common/mod.rs"]
mod cli;
#[path = "common/tiny_models.rs"]
mod tiny_models;
#[path = "common/samba_bringup.rs"]
mod samba_bringup;

use samba_bringup::{brings_up_close, every, CTX};
use tiny_models::ALL;

#[test]
fn every_tiny_architecture_four_times_as_wide_brings_up_on_the_sn50() {
    every(ALL.iter().map(|&t| (format!("{t:?} four times as wide"), t)), |t| {
        brings_up_close(t.wide(CTX), &format!("{t:?} four times as wide"))
    });
}
