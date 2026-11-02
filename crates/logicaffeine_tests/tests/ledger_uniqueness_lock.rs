//! ledger_uniqueness_lock — one block, one signoff record.
//!
//! WHY THIS EXISTS. `scripts/openlane-run.sh` records through
//! `OUT_JSON="${OUT_JSON:-$SIL/sky130_signoff.json}"` — the DEFAULT, not the ledger the block
//! already lives in. So re-hardening a block that was first recorded in a campaign-specific
//! ledger writes a SECOND entry into the default one, and nothing noticed.
//!
//! MEASURED when this was written, after re-hardening two systolic blocks:
//!     systolic_os_pipe8_realbist   sky130_signoff.json: clean=TRUE   cells=406487
//!                                  systolic_pipelined_signoff.json: clean=FALSE cells=536104
//!     systolic_pipe16_realbist     present in both, different runs, different cell counts
//! The first is the dangerous shape: the SAME BLOCK read signoff-clean from one ledger and
//! not-clean from another, and which answer you got depended on which file you opened. A
//! tape-out decision made from the wrong one is made on evidence that a different file
//! contradicts.
//!
//! Resolved by REGENERATING each entry into its original ledger from the run's own reports
//! (never by copying JSON between files) and deleting the misplaced copies.

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/sky130_ledgers.rs"]
mod sky130_ledgers;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every registered block ledger, of every class: a block is one block whichever ledger records it.
fn ledgers() -> Vec<(String, serde_json::Value)> {
    let mut out = Vec::new();
    for p in sky130_ledgers::ledgers(&root(), None) {
        let name = p.file_name().and_then(|s| s.to_str()).unwrap_or("").to_string();
        let raw = std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        let v = serde_json::from_str::<serde_json::Value>(&raw).unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        out.push((name, v));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// A block may be recorded in exactly one ledger.
#[test]
fn no_block_is_recorded_in_two_ledgers() {
    let mut seen: BTreeMap<String, Vec<(String, Option<bool>)>> = BTreeMap::new();
    for (file, doc) in ledgers() {
        let Some(blocks) = doc["blocks"].as_object() else {
            continue;
        };
        for (b, v) in blocks {
            seen.entry(b.clone())
                .or_default()
                .push((file.clone(), v["signoff_clean"].as_bool()));
        }
    }
    assert!(
        seen.len() >= 15,
        "only {} blocks scanned across ledgers — the scan is not working",
        seen.len()
    );
    let dups: Vec<String> = seen
        .iter()
        .filter(|(_, e)| e.len() > 1)
        .map(|(b, e)| {
            let verdicts: std::collections::BTreeSet<_> = e.iter().map(|(_, c)| *c).collect();
            let conflict = if verdicts.len() > 1 {
                "  CONFLICTING VERDICTS"
            } else {
                ""
            };
            let detail: Vec<String> = e.iter().map(|(f, c)| format!("{f}(clean={c:?})")).collect();
            format!("  {b}: {}{conflict}", detail.join(" + "))
        })
        .collect();
    assert!(
        dups.is_empty(),
        "{} block(s) are recorded in more than one signoff ledger:\n{}\n\n\
         A block with two records can be read as clean from one file and not-clean from \
         another. Re-record the block into the ledger it already lives in (set OUT_JSON) and \
         delete the misplaced copy — regenerate from the run, never copy JSON between files.",
        dups.len(),
        dups.join("\n")
    );
}
