//! golden_precheck_lock — pins that our taped-out GDS passes the EFABLESS GOLDEN signoff DRC decks,
//! the ones `mpw_precheck` uses to ACCEPT a shuttle submission — not merely OpenLane's built-in
//! Magic DRC. This closes the honest "open deck ≠ foundry deck" gap for DRC.
//!
//! Source of truth: `benchmarks/silicon/golden_precheck.json`, written by
//! `scripts/run-golden-precheck-drc.sh <block> <gds>` — which runs KLayout with SkyWater/efabless
//! `sky130A_mr.drc` (the full maximal-rule deck, FEOL+BEOL) + the golden `zeroarea` geometry check
//! on the streamed GDS and counts violations exactly as the precheck does (`<item>` entries).
//!
//! A block is golden-clean only when BOTH decks report zero. The recorder is fail-closed: a missing
//! or unreadable report becomes -1, which trips this lock. Passing the golden KLayout signoff DRC is
//! a real, foundry-relevant result; it is NOT the whole shuttle precheck (Caravel-structure checks,
//! the golden Magic DRC pass, and LVS-at-precheck remain, alongside the pad/ESD ring + analog PHY +
//! the shuttle run itself).

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/sky130_ledgers.rs"]
mod sky130_ledgers;

use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

const GP: &str = "benchmarks/silicon/golden_precheck.json";

fn doc() -> serde_json::Value {
    let p = root().join(GP);
    let raw = fs::read_to_string(&p).unwrap_or_else(|e| {
        panic!(
            "cannot read {}: {e} — regenerate via scripts/run-golden-precheck-drc.sh",
            p.display()
        )
    });
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("bad JSON in {GP}: {e}"))
}

fn i(b: &serde_json::Value, k: &str, name: &str) -> i64 {
    b[k].as_i64()
        .unwrap_or_else(|| panic!("{GP}/{name}: `{k}` missing or null"))
}

/// Both Infinichip blocks — the command player and the integrated subsystem — pass the golden
/// SkyWater/efabless KLayout signoff DRC (`sky130A_mr`, FEOL+BEOL) AND the golden zeroarea check
/// with ZERO violations. This is the DRC deck a shuttle actually accepts against.
#[test]
fn both_infinichip_blocks_pass_the_golden_signoff_drc() {
    let d = doc();
    for name in ["dram_executor", "infinichip"] {
        let b = d["blocks"].get(name).unwrap_or_else(|| {
            panic!("{GP}: {name} missing — run scripts/run-golden-precheck-drc.sh {name} <gds>")
        });
        assert_eq!(
            i(b, "golden_drc_violations", name),
            0,
            "{name}: golden sky130A_mr signoff DRC is not clean"
        );
        assert_eq!(
            i(b, "zeroarea_violations", name),
            0,
            "{name}: golden zeroarea check not clean"
        );
        assert!(
            b["feol"].as_bool().unwrap_or(false),
            "{name}: FEOL rules were not run"
        );
        assert!(
            b["beol"].as_bool().unwrap_or(false),
            "{name}: BEOL rules were not run"
        );
        assert!(
            b["golden_clean"].as_bool().unwrap_or(false),
            "{name}: not golden-clean"
        );
    }
}

/// Every recorded block is golden-clean — no block sneaks in with a nonzero or sentinel count.
#[test]
fn every_recorded_block_is_golden_clean() {
    let d = doc();
    let blocks = d["blocks"].as_object().expect("`blocks` object");
    assert!(!blocks.is_empty(), "{GP}: no blocks recorded");
    for (name, b) in blocks {
        assert_eq!(
            i(b, "golden_drc_violations", name),
            0,
            "{name}: golden DRC nonzero/sentinel"
        );
        assert_eq!(
            i(b, "zeroarea_violations", name),
            0,
            "{name}: zeroarea nonzero/sentinel"
        );
    }
}

/// The artifact records WHICH golden deck was run, so the claim is anchored to the efabless
/// maximal-rule deck via mpw_precheck, not a hand-typed assertion.
#[test]
fn the_golden_deck_provenance_is_recorded() {
    let d = doc();
    let deck = d["meta"]["deck"].as_str().unwrap_or("");
    let src = d["meta"]["source"].as_str().unwrap_or("");
    // efabless ships one maximal-rule deck per PDK under a shared naming convention, so the gate
    // is "a golden _mr deck for a PDK we support", not "the sky130A one".
    const KNOWN_DECKS: &[&str] = &["sky130A_mr", "sky130B_mr", "gf180mcuC_mr", "gf180mcuD_mr"];
    assert!(
        KNOWN_DECKS.iter().any(|k| deck.contains(k)),
        "{GP}: meta.deck = {deck:?} names no golden maximal-rule deck this flow supports \
         ({KNOWN_DECKS:?})"
    );
    assert!(
        src.to_lowercase().contains("mpw_precheck"),
        "{GP}: meta.source must name mpw_precheck"
    );
}

/// A golden verdict is about ONE GDS, and must say which.
///
/// The entries carried no GDS identity, so a `golden_clean: true` outlived its file: on
/// 2026-09-23 `dram_executor`, `infinichip` and `dram_executor_wb` were re-hardened (new GDS,
/// honest slow-corner periods) while their golden entries still described the previous streams.
/// Every golden entry for a block that also has a signoff record must name the same GDS byte
/// count as that record.
#[test]
fn every_golden_verdict_is_bound_to_the_gds_the_signoff_ledger_ships() {
    let golden = doc();
    let mut signoff = std::collections::HashMap::new();
    for path in sky130_ledgers::ledgers(&root(), None) {
        let d: serde_json::Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        if let Some(bs) = d["blocks"].as_object() {
            for (name, b) in bs {
                if let Some(g) = b["gds_bytes"].as_i64() {
                    signoff.insert(name.clone(), g);
                }
            }
        }
    }
    for (name, b) in golden["blocks"].as_object().expect("blocks") {
        let Some(&shipped) = signoff.get(name) else {
            continue;
        };
        let checked = b["gds_bytes"].as_i64().unwrap_or_else(|| {
            panic!(
            "{GP}/{name}: no `gds_bytes` — the golden verdict does not say which GDS it checked")
        });
        assert_eq!(checked, shipped,
            "{GP}/{name}: golden deck checked a {checked}-byte GDS but the signoff ledger ships {shipped} bytes \
             — re-run scripts/run-golden-precheck-drc.sh on the shipped GDS");
    }
}
