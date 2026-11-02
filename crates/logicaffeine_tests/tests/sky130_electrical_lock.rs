//! sky130_electrical_lock — the ELECTRICAL signoff gates, which until now no ledger recorded and
//! no lock could see.
//!
//! `sky130_signoff_lock` and `sky130_deep_signoff_lock` gate DRC / LVS / antenna / routing / XOR /
//! timing-slack / IR-drop. None of them gates `report_check_types` — the max-slew, max-fanout and
//! max-capacitance violators. That gap was not hypothetical: `fabric_soc` shipped recorded as
//! `signoff_clean: true` while carrying **4,929 max-fanout and 18 max-slew violations** at the
//! typical corner, and every lock stayed green because no field held the number. A net driving far
//! more load than its library cell is characterized for has transition times the timing model does
//! not cover, so a slack figure computed over it is optimistic by an unknown amount.
//!
//! A count is meaningless without the constraint it was measured against — `fabric_soc`'s 4,929 is
//! against an unusually tight `MAX_FANOUT_CONSTRAINT` of 6 — so the recorder captures both, from
//! the run's own `config.tcl`, and this lock requires both to be present.
//!
//! Recorded by `scripts/record-sky130-signoff.py` from `reports/signoff/*rcx_sta.checks.rpt`,
//! worst-case across every corner, and re-derived from the run directory by
//! `sky130_provenance_lock` like every other field.
//!
//! This lock requires the evidence to be RECORDED. The recorder's `signoff_clean` conjoins slew and
//! capacitance at zero (fanout is recorded but not gated — `record-sky130-signoff.py` says why), and
//! `signoff_ready_lock` holds that conjunction to the recorded fields in both directions.

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/sky130_ledgers.rs"]
mod sky130_ledgers;

use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// Every registered block ledger, of every class — the registry `sky130_ledger_registry_lock` holds equal to
/// the block ledgers on disk, so a ledger added for a second PDK cannot slip past this gate.
fn ledgers() -> Vec<String> {
    let mut out: Vec<String> = sky130_ledgers::registry(&root()).into_iter().map(|l| l.path).collect();
    out.sort();
    out
}

fn json(rel: &str) -> serde_json::Value {
    let p = root().join(rel);
    let raw = fs::read_to_string(&p).unwrap_or_else(|e| {
        panic!(
            "cannot read {}: {e} — regenerate with scripts/openlane-run.sh, or re-record from an \
             existing run with scripts/record-sky130-signoff.py <run_dir> <block> <ledger>",
            p.display()
        )
    });
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("bad JSON in {rel}: {e}"))
}

/// (ledger, block name, block record) for every recorded block across every ledger.
fn every_block() -> Vec<(String, String, serde_json::Value)> {
    let mut out = Vec::new();
    for led in ledgers() {
        let d = json(&led);
        // A ledger with no `blocks` object is a differently-shaped record (the Caravel wrapper has
        // its own lock and its own schema) — it contributes no blocks rather than failing here.
        let Some(blocks) = d["blocks"].as_object() else {
            continue;
        };
        for (name, rec) in blocks {
            out.push((led.clone(), name.clone(), rec.clone()));
        }
    }
    assert!(
        !out.is_empty(),
        "no blocks recorded in any ledger — the ledgers cannot all be empty"
    );
    out
}

/// The three `report_check_types` violation counters. Recording them is what makes them fixable:
/// a number nobody writes down is a number nobody drives to zero.
const COUNTS: &[&str] = &[
    "max_slew_violations",
    "max_fanout_violations",
    "max_cap_violations",
];

/// The constraints those counters were measured against. A count without its limit cannot be
/// compared across blocks, and a block that silently loosened its limit would otherwise look
/// improved.
const CONSTRAINTS: &[&str] = &[
    "max_transition_constraint",
    "max_fanout_constraint",
    "max_cap_constraint",
];

#[test]
fn every_block_records_its_electrical_check_evidence() {
    for (led, name, b) in every_block() {
        for k in COUNTS {
            let v = b.get(*k).unwrap_or_else(|| {
                panic!(
                    "{led}/{name}: no `{k}` — the electrical checks are unrecorded, so no lock can \
                     see them. Re-record with scripts/record-sky130-signoff.py."
                )
            });
            let n = v
                .as_i64()
                .unwrap_or_else(|| panic!("{led}/{name}: `{k}` is not an integer: {v}"));
            assert!(
                n >= 0,
                "{led}/{name}: `{k}` = {n} — negative is the recorder's fail-closed sentinel, \
                 meaning the checks report was missing or unparseable for that run"
            );
        }
        for k in CONSTRAINTS {
            let v = b.get(*k).unwrap_or_else(|| {
                panic!("{led}/{name}: no `{k}` — a violation count without its limit is uninterpretable")
            });
            let x = v
                .as_f64()
                .unwrap_or_else(|| panic!("{led}/{name}: `{k}` is not a number: {v}"));
            assert!(
                x > 0.0,
                "{led}/{name}: `{k}` = {x} — a non-positive limit is not a real constraint"
            );
        }
    }
}

#[test]
fn every_block_records_the_pdk_it_was_hardened_on() {
    for (led, name, b) in every_block() {
        let pdk = b.get("pdk").and_then(|v| v.as_str()).unwrap_or_else(|| {
            panic!(
                "{led}/{name}: no `pdk` field. The ledger keys blocks by NAME alone, so hardening \
                 this block on a second PDK would silently OVERWRITE this record. Every block must \
                 name the PDK it was built against."
            )
        });
        assert!(!pdk.trim().is_empty(), "{led}/{name}: `pdk` is empty");
    }
}

/// Every number the electrical and timing gates hold was computed by signoff STA on the netlist it read, so that
/// netlist must be the layout's: every component of the final DEF, and nothing else. The antenna ECO adds diode and
/// filler cells after detailed routing writes the netlist; until the flow wrote it again, signoff STA read the
/// pre-ECO one. Recorded by `scripts/record-sky130-signoff.py` as the layout's cells the STA netlist lacks and the
/// STA netlist's cells the layout lacks, worst over every signoff STA log; `signoff_clean` requires both 0.
#[test]
fn every_clean_block_was_timed_on_the_netlist_it_drew() {
    for (led, name, b) in every_block() {
        for k in ["sta_netlist_missing_cells", "sta_netlist_extra_cells"] {
            let v = b.get(k).unwrap_or_else(|| {
                panic!(
                    "{led}/{name}: no `{k}` — nothing records whether signoff STA read the layout's netlist. \
                     Re-record with scripts/record-sky130-signoff.py."
                )
            });
            let n = v
                .as_i64()
                .unwrap_or_else(|| panic!("{led}/{name}: `{k}` is not an integer: {v}"));
            if b["signoff_clean"] == serde_json::Value::Bool(true) {
                assert_eq!(n, 0, "{led}/{name}: recorded clean, but `{k}` = {n} — its timing was signed off on another netlist");
            }
        }
    }
}
