//! ════════════════════════════════════════════════════════════════════════════
//! DIFFERENTIAL-BY-DEFAULT.
//!
//! Any program that runs must produce byte-identical output on EVERY execution tier
//! that can run it. This file proves the `run_all_tiers` / `assert_all_tiers` helpers
//! (common/mod.rs) fan a program across the always-available tiers (tree-walker + VM)
//! and assert agreement — so an ordinary behavioral test gains cross-tier coverage
//! for free instead of pinning a single tier.
//!
//! The WASM/AOT tiers are held to the same byte-identity contract by
//! `logicaffeine_compile/tests/wasm_aot_lock.rs` over its own corpus; the `enum Tier`
//! in `run_all_tiers` has NO wildcard arm, so a newly-added execution tier fails the
//! build until it joins this default differential.

mod common;
use common::{assert_all_tiers, run_all_tiers};

/// A spread of proven-valid sequential programs (arithmetic, precedence, collections,
/// the rational tower, a builtin) — each must run IDENTICALLY on tree-walker and VM.
/// Sourced from the existing interpreter-golden suites, now cross-tier by construction.
const AGREEING: &[(&str, &str)] = &[
    ("## Main\nLet a be 3.\nLet b be 4.\nShow a + b.", "7"),
    ("## Main\nLet m be (5 + 4) / 2.\nShow m.", "4"),
    ("## Main\nLet m be 17 / 5.\nShow m.", "3"),
    ("## Main\nLet items be [1, 2, 3].\nShow items.", "[1, 2, 3]"),
    (
        "## Main\nLet r: Rational be 1 / 2.\nLet a be 6 * r.\nShow a.",
        "3",
    ),
    (
        "## Main\nLet r: Rational be 1 / 2.\nLet s be r + 3.\nShow s.",
        "7/2",
    ),
    ("## Main\nLet r: Rational be 7 / 2.\nShow ceil(r).", "4"),
    ("## Main\nLet r: Rational be -7 / 2.\nShow abs(r).", "7/2"),
];

#[test]
fn behavioral_programs_agree_across_all_tiers() {
    for (src, expected) in AGREEING {
        // Runs on tree-walker AND VM, asserts byte-identical output, checks `expected`.
        assert_all_tiers(src, expected);
    }
}

/// `run_all_tiers` returns the agreed outcome (the value a test can then assert on),
/// and reports a clean (no-error) run for a well-formed program.
#[test]
fn run_all_tiers_returns_the_agreed_outcome() {
    let agreed = run_all_tiers("## Main\nLet a be 3.\nLet b be 4.\nShow a + b.");
    assert_eq!(
        agreed.error, None,
        "well-formed program should not error on any tier"
    );
    assert_eq!(agreed.output.trim(), "7");
}

/// A malformed program must FAIL THE SAME WAY on every tier — the error-parity half of
/// Invariant II. `run_all_tiers` asserts the error text matches across tiers internally;
/// here we confirm a broken program is reported as an error (not a silent success on one
/// tier and a failure on another).
#[test]
fn malformed_programs_fail_identically_across_tiers() {
    // Undefined variable: both tiers must surface an error, and the SAME error, which
    // `run_all_tiers` enforces internally (it would panic on divergence).
    let agreed = run_all_tiers("## Main\nShow undefined_variable_xyz.");
    assert!(
        agreed.error.is_some(),
        "an undefined variable must be an error on every tier, got clean output: {:?}",
        agreed.output
    );
}
