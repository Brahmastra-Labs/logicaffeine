//! PIT-OF-SUCCESS — reading a missing map key with the raw `item k of m` TRAPS on every executing
//! tier, never silently returns a default. This is the confirmed LOGOS spec (Tristen, 2026-08-31):
//! `item k of m` requires `k` present; to get a default, guard with `m contains k` or use the stdlib
//! `counterGet`. The C tier was the only one locked before (the `e2e_c_map_*` tests + `assert_c_traps`);
//! this file locks the trap on the tree-walker and AOT-Rust too, so no backend can silently regress to
//! returning `0`/`""` — the exact divergence that would break the cross-tier byte-identity invariant.
//!
//! The trap MESSAGE is tier-specific (the tree-walker/AOT-Rust runtime says "not found in map", the C
//! runtime says "missing map key") — what is locked is that each tier FAILS LOUD, never defaults.

mod common;
use common::{assert_c_traps, assert_interpreter_fails, assert_panics};

/// A `Map of Int to Int` with key 1 present; reading absent key 999 must trap, not yield 0.
const MISSING_INT_KEY: &str =
    "## Main\nLet mutable m be a new Map of Int to Int.\nSet item 1 of m to 42.\nShow item 999 of m.\n";

/// A `Map of Text to Int`; reading an absent Text key must trap, not yield 0.
const MISSING_TEXT_KEY: &str =
    "## Main\nLet mutable m be a new Map of Text to Int.\nSet item \"x\" of m to 42.\nShow item \"missing\" of m.\n";

#[test]
fn missing_int_key_traps_on_the_tree_walker() {
    assert_interpreter_fails(MISSING_INT_KEY, "not found in map");
}

#[test]
fn missing_text_key_traps_on_the_tree_walker() {
    assert_interpreter_fails(MISSING_TEXT_KEY, "not found in map");
}

#[test]
fn missing_int_key_traps_on_aot_rust() {
    assert_panics(MISSING_INT_KEY, "not found in map");
}

#[test]
fn missing_int_key_traps_on_the_c_backend() {
    assert_c_traps(MISSING_INT_KEY, "missing map key");
}

#[test]
fn missing_text_key_traps_on_the_c_backend() {
    assert_c_traps(MISSING_TEXT_KEY, "missing map key");
}

/// A `Map of Text to Seq of Int` — a map the C tier holds in a generic family, not a fixed one; reading an absent key
/// must trap as the fixed families do, not yield an empty list.
const MISSING_KEY_OF_A_SEQ_MAP: &str =
    "## Main\nLet mutable m be a new Map of Text to Seq of Int.\nSet item \"a\" of m to [1, 2].\nShow item \"missing\" of m.\n";

#[test]
fn missing_key_of_a_generic_map_traps_on_the_tree_walker() {
    assert_interpreter_fails(MISSING_KEY_OF_A_SEQ_MAP, "not found in map");
}

#[test]
fn missing_key_of_a_generic_map_traps_on_the_c_backend() {
    assert_c_traps(MISSING_KEY_OF_A_SEQ_MAP, "missing map key");
}
