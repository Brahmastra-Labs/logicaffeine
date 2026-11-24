//! ★ LOCK — the test harness builds generated C as `largo build --release --emit c` does. ★
//!
//! Every C row of the suite compiles the C it checks through the harness (`common::build_c_release`), which keeps its
//! own copy of the flags so a test binary does not link the build tool. The copy is the tool's, flag for flag: the
//! dialect the generated C is written in and the release flags, so a row holds the C a user's release build runs.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test c_build_flags_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use logicaffeine_cli::project::build_c::{C_DIALECT_FLAGS, C_RELEASE_FLAGS};

#[test]
fn the_harness_builds_c_in_the_dialect_largo_states() {
    assert_eq!(common::C_DIALECT_FLAGS, C_DIALECT_FLAGS);
}

#[test]
fn the_harness_builds_c_with_largos_release_flags() {
    assert_eq!(common::C_RELEASE_FLAGS, C_RELEASE_FLAGS);
}
