//! File-IO natives are FIRST-CLASS on the executing tiers — file.lg's `read`/
//! `write` and the `readBytes` builtin run on the tree-walker AND the VM with
//! identical results (and on AOT via `map_native_function`, locked separately by
//! `parity_io.rs` / the HTTP differential). Before this lock, each primitive
//! worked on only ONE tier: `read`/`write` were AOT-only (the tree-walker/VM ran
//! the empty native body and returned Nothing — an `Inspect` then died with
//! "no arm"), and `readBytes` was tree-walker/VM-only (unresolved in generated
//! Rust). A server that is green on one tier must not lose file IO on another.
//!
//! YOU DO NOT GET TO WEAKEN THIS LOCK TO MAKE IT PASS. A divergence here means
//! file IO drifted between tiers — fix the TIER, never relax the assert.
#![allow(unused_imports, dead_code)]
mod common;
use common::assert_all_tiers;

/// `write` then `read` round-trip through `Inspect` — both tiers, same output.
#[test]
fn write_then_read_round_trips_on_both_tiers() {
    assert_all_tiers(
        "## Main\nLet w be write(\"/tmp/logos_xtier_rw.txt\", \"tier parity\").\nInspect w:\n    When Ok (u):\n        Show \"written\".\n    When Err (e):\n        Show e.\nLet r be read(\"/tmp/logos_xtier_rw.txt\").\nInspect r:\n    When Ok (v):\n        Show v.\n    When Err (e):\n        Show e.\n",
        "written\ntier parity",
    );
}

/// A failed `read` surfaces as `Err` (never Nothing, never a trap) — both tiers.
#[test]
fn read_of_a_missing_file_is_err_on_both_tiers() {
    assert_all_tiers(
        "## Main\nLet r be read(\"/tmp/logos_xtier_missing_g8.txt\").\nInspect r:\n    When Ok (v):\n        Show \"unexpectedly ok\".\n    When Err (e):\n        Show \"err\".\n",
        "err",
    );
}

/// `readBytes` returns the file's raw bytes as a `Seq of Int` — both tiers.
#[test]
fn read_bytes_returns_raw_bytes_on_both_tiers() {
    assert_all_tiers(
        "## Main\nLet w be write(\"/tmp/logos_xtier_bytes.txt\", \"AB\").\nInspect w:\n    When Ok (u):\n        Show readBytes(\"/tmp/logos_xtier_bytes.txt\").\n    When Err (e):\n        Show e.\n",
        "[65, 66]",
    );
}

/// Declarer-wins: a program DEFINING its own `read` keeps its definition — the
/// native seam must never shadow a user function with a real body.
#[test]
fn a_user_defined_read_wins_over_the_native_seam() {
    assert_all_tiers(
        "## To read (path: Text) -> Text:\n    Return \"mine\".\n\n## Main\nShow read(\"/tmp/whatever\").\n",
        "mine",
    );
}
