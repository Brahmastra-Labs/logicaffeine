//! The Sky130 block ledgers — every JSON `scripts/record-sky130-signoff.py` writes — read from their one
//! registry, `benchmarks/silicon/sky130_ledgers.json`, with their class. Locks include this file by path and
//! read ledgers through it; none finds a ledger by its file name (`sky130_ledger_registry_lock`).

#![allow(dead_code)]

use std::path::{Path, PathBuf};

/// The registry, relative to the repository root.
pub const REGISTRY: &str = "benchmarks/silicon/sky130_ledgers.json";

/// `signoff`: LOGOS blocks hardened as deliverables, each registered in `codegen_sva::signoff_blocks`.
/// `comparative`: the comparative-synthesis bench's candidates, at the period the bench sets.
pub const CLASSES: &[&str] = &["signoff", "comparative"];

/// Where a ledger's OpenLane run directories are when its entry names none.
pub const DEFAULT_RUN_ROOT: &str = "tools/OpenLane/designs";

/// One registered ledger: its path relative to the repository root, its class, and where its runs are (relative to
/// the repository root).
#[derive(Debug, Clone)]
pub struct Ledger {
    pub path: String,
    pub class: String,
    pub run_root: String,
}

/// Every registered ledger, in the registry's order.
pub fn registry(root: &Path) -> Vec<Ledger> {
    let text = std::fs::read_to_string(root.join(REGISTRY)).unwrap_or_else(|e| panic!("{REGISTRY}: {e}"));
    let doc: serde_json::Value = serde_json::from_str(&text).unwrap_or_else(|e| panic!("{REGISTRY}: {e}"));
    doc["ledgers"]
        .as_array()
        .unwrap_or_else(|| panic!("{REGISTRY}: no `ledgers` list"))
        .iter()
        .map(|l| Ledger {
            path: l["path"].as_str().expect("a ledger's path").to_string(),
            class: l["class"].as_str().expect("a ledger's class").to_string(),
            run_root: l["run_root"].as_str().unwrap_or(DEFAULT_RUN_ROOT).to_string(),
        })
        .collect()
}

/// The registered ledgers of `class` (every class when `None`), as absolute paths.
pub fn ledgers(root: &Path, class: Option<&str>) -> Vec<PathBuf> {
    registry(root)
        .into_iter()
        .filter(|l| class.map_or(true, |c| l.class == c))
        .map(|l| root.join(l.path))
        .collect()
}
