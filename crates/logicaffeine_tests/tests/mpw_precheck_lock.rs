//! mpw_precheck_lock — the SHUTTLE ACCEPTANCE gate, pinned.
//!
//! `mpw_precheck` is the 19-check battery ChipFoundry/efabless runs to decide whether a submission
//! is accepted at all. Passing OpenLane's own signoff is necessary but NOT sufficient: the precheck
//! additionally runs the golden Magic DRC, the KLayout FEOL / BEOL / offgrid / metal-density /
//! pin-label / zeroarea decks, and DEVICE-LEVEL LVS against the extracted GDS — plus license, SPDX,
//! Makefile, defaults, documentation, netlist-consistency, GPIO-defines, XOR-vs-golden, spike,
//! illegal-cellname and OEB checks on the submission REPOSITORY, not just the layout.
//!
//! Every one of those is a real way to be rejected after paying for a slot. Three of them failed the
//! first time we ran it and none were silicon problems: the GPIO config was still the scaffold's
//! `13'hXXXX` placeholders, the README was byte-identical to the template, and 12 files lacked SPDX
//! headers. A fourth — device-level LVS — failed because the scaffold's `lvs_config.json` still
//! named the TEMPLATE macro, so our macro extracted as an empty black box ("contains no devices")
//! while the structural check happily reported "Circuits match uniquely".
//!
//! Source of truth: each Caravel project's precheck ledger (`codegen_sva::caravel_project`; the DRAM
//! command player's is `benchmarks/silicon/mpw_precheck.json`), written by `scripts/record-mpw-precheck.py`
//! from a real precheck log. The recorder is fail-closed — a check the log does not explicitly report
//! as PASSED is never recorded as passed. A project not yet prechecked contributes nothing; the DRAM
//! project's record always must exist.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::caravel_project::{
    CaravelProject, CARAVEL_PROJECTS, DRAM_PROJECT,
};
use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// The PDKs this project's flow supports; see sky130_signoff_lock for the rationale.
const KNOWN_PDKS: &[&str] = &["sky130A", "sky130B", "gf180mcuC", "gf180mcuD", "ihp-sg13g2"];

/// Every recorded precheck: the project, its ledger's path and the record.
fn docs() -> Vec<(&'static CaravelProject, &'static str, serde_json::Value)> {
    let mut out = Vec::new();
    for project in CARAVEL_PROJECTS {
        let mp = project.precheck_ledger;
        let raw = match fs::read_to_string(root().join(mp)) {
            Ok(raw) => raw,
            Err(e) => {
                assert!(
                    project.key != DRAM_PROJECT.key,
                    "cannot read {mp}: {e} — regenerate with scripts/caravel-submission.sh then \
                     scripts/run-mpw-precheck.sh"
                );
                continue;
            }
        };
        out.push((
            project,
            mp,
            serde_json::from_str(&raw).unwrap_or_else(|e| panic!("bad JSON in {mp}: {e}")),
        ));
    }
    out
}

/// The submission is ACCEPTED: every executed check passed and none failed.
#[test]
fn the_submission_passes_every_mpw_precheck() {
    for (_, mp, d) in docs() {
        let failed = d["checks_failed"].as_array().expect("checks_failed array");
        assert!(
            failed.is_empty(),
            "{mp}: mpw_precheck reports FAILED check(s) {failed:?} — the shuttle would reject this \
         submission. Fix them in scripts/caravel-submission.sh so the fix is reproducible, never by \
         hand-editing the record."
        );
        assert!(
            d["all_checks_passed"].as_bool().unwrap_or(false),
            "{mp}: all_checks_passed is false — the precheck log did not end in {{{{SUCCESS}}}}"
        );
    }
}

/// The whole battery actually ran. A precheck that executed two checks and passed both is not a
/// shuttle gate; this pins the count so a truncated or crashed run can never read as acceptance.
#[test]
fn the_whole_precheck_battery_ran() {
    for (_, mp, d) in docs() {
        let total = d["checks_total"].as_i64().unwrap_or(0);
        let executed = d["checks_executed"].as_i64().unwrap_or(0);
        let passed = d["checks_passed"].as_i64().unwrap_or(0);

        assert!(
            total >= 19,
            "{mp}: the precheck reported only {total} checks in the battery (expected 19+)"
        );
        assert_eq!(
            executed, total,
            "{mp}: only {executed} of {total} checks executed — the run was cut short"
        );
        assert_eq!(
            passed, executed,
            "{mp}: {passed} of {executed} executed checks are recorded as passed"
        );
    }
}

/// The gates that a reviewer actually asks about must each be named and passed — so a future
/// recorder change can never quietly drop one and still report acceptance.
#[test]
fn every_named_signoff_gate_is_present_and_passed() {
    for (_, mp, d) in docs() {
        let checks = d["checks"].as_object().expect("checks object");
        for needle in [
            "License",
            "Makefile",
            "Default",
            "Documentation",
            "Top Cell",
            "Consistency",
            "GPIO-Defines",
            "XOR",
            "Magic DRC",
            "Klayout FEOL",
            "Klayout BEOL",
            "Klayout Offgrid",
            "ZeroArea",
            "Spike",
            "Illegal Cellname",
            "OEB",
            "LVS",
        ] {
            let hit = checks.iter().find(|(k, _)| k.contains(needle));
            let (name, verdict) = hit.unwrap_or_else(|| {
                panic!(
                    "{mp}: no precheck named `{needle}` was recorded — the battery is incomplete"
                )
            });
            assert_eq!(
                verdict.as_str().unwrap_or(""),
                "passed",
                "{mp}: precheck `{name}` is not recorded as passed"
            );
        }
    }
}

/// The record must point at a real streamed GDS, so acceptance can never be claimed for an empty
/// or absent layout.
#[test]
fn the_accepted_submission_carries_a_real_gds() {
    for (_, mp, d) in docs() {
        let bytes = d["submitted_gds_bytes"].as_i64().unwrap_or(0);
        assert!(
            bytes > 1_000_000,
            "{mp}: the submitted GDS is {bytes} bytes — that is not a hardened Caravel user area"
        );
        let meta = &d["meta"];
        let pdk = meta["pdk"].as_str().unwrap_or("");
        assert!(
            KNOWN_PDKS.iter().any(|k| pdk.contains(k)),
            "{mp}: meta.pdk = {pdk:?} names no PDK this flow supports ({KNOWN_PDKS:?}) — the precheck \
         is per-PDK (it schedules FEWER checks on gf180 than on sky130), so the record must say \
         which one it ran against"
        );
        assert!(
            meta["tool"].as_str().unwrap_or("").contains("mpw_precheck"),
            "{mp}: meta.tool must name mpw_precheck"
        );
    }
}

/// The precheck must have run on the SAME wrapper the signoff ledger vouches for.
///
/// WHY THIS EXISTS, and it is not hypothetical. `the_accepted_submission_carries_a_real_gds`
/// asserts only that the submitted GDS is larger than a megabyte, which says nothing about WHICH
/// layout was accepted. So when the wrapper was re-hardened to close its 97 hold-violating paths,
/// `caravel_signoff.json` advanced to the new 58,152,238-byte GDS while the assembled submission
/// still held the previous 57,183,284-byte one — and the recorded "19/19 PASSED" went on
/// describing a wrapper that no longer existed. Every check in that battery was green, every
/// number in it was true of something, and the thing it was true of was not the artifact we would
/// have shipped.
///
/// A shuttle acceptance claim is a claim about ONE layout. Binding the two records by byte count
/// makes a re-harden turn this red until the precheck is re-run, which is the only order that
/// keeps the badge honest.
#[test]
fn the_precheck_ran_on_the_wrapper_the_signoff_ledger_vouches_for() {
    for (project, _, d) in docs() {
        let key = project.key;
        let accepted = d["submitted_gds_bytes"].as_i64().unwrap_or(0);

        let cs = root().join(project.wrapper_ledger);
        let raw =
            fs::read_to_string(&cs).unwrap_or_else(|e| panic!("cannot read {}: {e}", cs.display()));
        let signed: serde_json::Value = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("bad JSON in {}: {e}", project.wrapper_ledger));
        let vouched = signed["user_project_wrapper"]["gds_bytes"]
            .as_i64()
            .unwrap_or_else(|| {
                panic!("{} records the wrapper's gds_bytes", project.wrapper_ledger)
            });

        assert_eq!(
        accepted, vouched,
        "the precheck accepted a {accepted}-byte user_project_wrapper.gds, but the signoff ledger \
         vouches for a {vouched}-byte one (project {key}). The 19/19 result describes a DIFFERENT layout than the \
         one that is signed off — re-assemble with scripts/caravel-submission.sh (it resolves the \
         run by this byte count), re-run the precheck, and re-record it."
    );
    }
}
