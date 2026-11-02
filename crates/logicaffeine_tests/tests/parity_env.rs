//! Stdlib-parity north-star corpus — domain: env
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

/// Read an environment variable (native only).
/// provenance — c: stdlib.h.getenv · python: os.getenv · rust: env.var · go: os.Getenv
/// LOGOS surface: `get(key)`
#[test]
fn parity_env_get() {
    assert_all_tiers(
        "## Main
Let o be get(\"LOGOS_NOPE_XYZ_123\").
Inspect o:
    When Some (v):
        Show 1.
    When None:
        Show 0.
",
        "0",
    );
}
