//! proof_receipts_lock — Demo 3's figures come from ledgers that say what box and what build produced them.
//!
//! PROOF-RECEIPTS-FRESH's RED tests (benchmarks/FUSIONS_WE_NEED.md, section 12.20). The certified-SAT ledger records
//! the rival solvers' versions but not the box it ran on or the build of ours it measured; a re-run on a quiet box
//! records the boot, the load average and the commit, as the GPU scoreboard does. (The Arena scoreboard already names
//! its leaderboard revision and its checker's digest.)

use std::path::{Path, PathBuf};

use serde_json::Value;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn json(path: &str) -> Value {
    serde_json::from_str(&std::fs::read_to_string(root().join(path)).unwrap_or_else(|e| panic!("{path}: {e}"))).unwrap()
}

#[test]
fn the_solver_ledger_records_the_box_and_the_build_it_ran_on() {
    let m = &json("benchmarks/results/solvers.json")["metadata"];
    for field in ["boot_id", "load_average_1m", "logos_commit"] {
        assert!(!m[field].is_null(), "benchmarks/results/solvers.json metadata has no `{field}`");
    }
    assert!(m["load_average_1m"].as_f64().is_some(), "the load average is a number: {}", m["load_average_1m"]);
}
