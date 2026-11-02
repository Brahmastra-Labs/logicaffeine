//! fabric_grids_lock — `benchmarks/silicon/fabric_grids.json`, the recorded tile arrays of the spatial
//! compute fabric, is the compiler's own placer output and nothing else: it MUST equal the live
//! `fabric_soc::fabric_grids_json()`, so a placement the compiler no longer produces cannot stay recorded.
//!
//! When RED: regenerate — `cargo run -q -p logicaffeine-compile --example dump_fabric_grid >
//! benchmarks/silicon/fabric_grids.json`.

#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};

use logicaffeine_compile::codegen_sva::fabric_soc::fabric_grids_json;
const GRIDS: &str = "benchmarks/silicon/fabric_grids.json";

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn read(rel: &str) -> String {
    let p = root().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("cannot read {}: {e}", p.display()))
}

fn json(rel: &str) -> serde_json::Value {
    serde_json::from_str(&read(rel)).unwrap_or_else(|e| panic!("bad JSON in {rel}: {e}"))
}

#[test]
fn the_grid_data_behind_the_drawings_is_the_placers_own_output() {
    let live = fabric_grids_json();
    let checked_in = json(GRIDS);
    assert_eq!(
        checked_in, live,
        "{GRIDS} is not what the placer produces today — regenerate with \
         `cargo run -q -p logicaffeine-compile --example dump_fabric_grid > {GRIDS}` and re-render the drawings"
    );
    // The arrays the drawings show are the ones on silicon: five islands on the common 8×3 config array.
    let cf = &live["config_fabric"];
    assert_eq!(
        cf["islands"].as_array().map(Vec::len),
        Some(5),
        "five datapaths on the config array"
    );
    assert_eq!(
        (cf["rows"].as_u64(), cf["cols"].as_u64()),
        (Some(8), Some(3)),
        "the 8×3 array of nexys_fabric_soc"
    );
    let h = &live["horner"];
    let (dr, lr) = (
        h["diagonal"]["rows"].as_u64().unwrap(),
        h["leveled"]["rows"].as_u64().unwrap(),
    );
    assert!(
        lr < dr,
        "the area-minimizing placer must use fewer rows than the diagonal placer ({lr} vs {dr})"
    );
}
