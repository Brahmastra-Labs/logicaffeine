//! Stdlib-parity north-star corpus — domain: error
//!
//! Auto-scaffolded from benchmarks/parity/catalog/superset.json by
//! scripts/bench/gen-parity-corpus.py. Each test mirrors one deduplicated capability of the
//! C/Python/Rust/Go (+Java) superset. `#[ignore]` = parity-pending (not yet implemented);
//! the tree stays green. To LAND a capability: implement it, replace the body with a real
//! `assert_all_tiers` behavioral spec, DELETE the `#[ignore]` line, flip the catalog entry
//! to `status: green`, and bump GREEN_FLOOR in parity_catalog_lock.rs.
#![allow(unused_imports, dead_code)]

mod common;
use common::assert_all_tiers;

/// Construct a failure Result carrying an error.
/// provenance — rust: Result.unwrap_err
/// LOGOS surface: `Err(e)`
#[test]
fn parity_error_result_err() {
    assert_all_tiers(
        "## Main
Let r be a new Err with error \"boom\".
Inspect r:
    When Ok (v):
        Show 1.
    When Err (e):
        Show e.
",
        "boom",
    );
}

/// Construct a success Result carrying a value.
/// provenance — rust: Result.unwrap
/// LOGOS surface: `Ok(v)`
#[test]
fn parity_error_result_ok() {
    assert_all_tiers(
        "## Main
Let r be a new Ok with value 42.
Inspect r:
    When Ok (v):
        Show v.
    When Err (e):
        Show 0.
",
        "42",
    );
}

/// Extract a value or fall back to a default.
/// provenance — python: builtins.dict.get · rust: Result.unwrap_or · java: java.util.Optional.orElse
/// LOGOS surface: `r or else default`
#[test]
fn parity_error_unwrap_or() {
    assert_all_tiers(
        "## Main
Let r be a new Ok with value 7.
Show unwrapOr(r, 99).
Let r2 be a new Err with error \"x\".
Show unwrapOr(r2, 99).
",
        "7
99",
    );
}
