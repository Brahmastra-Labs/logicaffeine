//! caravel_signoff_lock — every Caravel project's `user_project_wrapper` hardened at the fixed 2920×3520
//! user area with the project's macro placed as `mprj`, and EVERY physical gate it is measured against,
//! passing or not.
//!
//! Source of truth: each project's wrapper ledger (`codegen_sva::caravel_project`; the DRAM command
//! player's is `benchmarks/silicon/caravel_signoff.json`), written by
//! `scripts/record-caravel-signoff.py <run_dir> <golden_drc.xml> <macro> <out.json>` from the
//! wrapper's OpenLane run plus a golden `sky130A_mr` KLayout report
//! (`scripts/run-golden-precheck-drc.sh`). Fail-closed: a missing field trips the lock. A project whose
//! wrapper has not been recorded yet contributes nothing; the DRAM project's always must.
//!
//! WHY THIS FILE WAS REWRITTEN. The record used to carry only the gates that passed — Magic DRC,
//! Netgen LVS, golden DRC, self-XOR — and stamped `signoff_clean: true`. The same run also had
//! `flow_status = flow failed`, a −0.83 ns hold violation, and 14 antenna violators on the Wishbone
//! pins into `mprj`. None of that was recorded, so this lock was green on an incomplete record: an
//! omitted gate is indistinguishable from a passing one. Every gate is now recorded and asserted.
//!
//! There is exactly ONE state a gate can be in: MET, asserted here unconditionally. No waiver field
//! exists in this schema, and none may be added — a field that can excuse a nonzero gate is an
//! omission with extra steps.
//!
//! WHY THE HOLD WAIVER IS GONE. The record used to carry `hold_waived_to_chip_integration` with a
//! prose reason: the 97 violating paths were Caravel harness passthroughs (harness pin → harness
//! pin, no user logic between), so they would close at full-chip integration. That basis was
//! checked against the min-corner report and is FALSE — all 97 run port → flop INSIDE `mprj`.
//! The real cause was common-path pessimism: efabless's template declares the single-source `clk`
//! as a `set_clock_latency -source` min/max PAIR (4.65 / 5.57), which races the earliest launch
//! against the latest capture across a path both edges share, while CRPR credits back 0.00 because
//! a lumped source latency exposes no common tree to walk. Declaring one source latency — launch
//! and capture variation still covered by `set_clock_uncertainty` and the derate — took hold from
//! −0.57 ns over 97 paths to +0.36 ns over none. The gate is MET, not excused.
//!
//! Antenna is likewise CLOSED, not waived (`mpw_precheck` gates on it): the wrapper routes 0/0
//! after the macro's pins were re-placed to mirror the wrapper DEF (see
//! `the_caravel_wrapper_is_shuttle_submittable`).

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::codegen_sva::caravel_project::{
    CaravelProject, CARAVEL_PROJECTS, DRAM_PROJECT,
};
use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// One project's recorded wrapper: the project, its ledger's path, and the `user_project_wrapper` record.
struct Wrapper {
    project: &'static CaravelProject,
    ledger: &'static str,
    w: serde_json::Value,
}

/// Every project whose wrapper is recorded. The DRAM project's must be: it is the one precheck-passed.
fn wrappers() -> Vec<Wrapper> {
    let mut out = Vec::new();
    for project in CARAVEL_PROJECTS {
        let p = root().join(project.wrapper_ledger);
        let raw = match fs::read_to_string(&p) {
            Ok(raw) => raw,
            Err(e) => {
                assert!(
                    project.key != DRAM_PROJECT.key,
                    "cannot read {}: {e} — regenerate via scripts/run-golden-precheck-drc.sh + \
                     scripts/record-caravel-signoff.py",
                    p.display()
                );
                continue;
            }
        };
        let d: serde_json::Value = serde_json::from_str(&raw)
            .unwrap_or_else(|e| panic!("bad JSON in {}: {e}", project.wrapper_ledger));
        out.push(Wrapper {
            project,
            ledger: project.wrapper_ledger,
            w: d["user_project_wrapper"].clone(),
        });
    }
    out
}

impl Wrapper {
    fn i(&self, k: &str) -> i64 {
        self.w[k]
            .as_i64()
            .unwrap_or_else(|| panic!("{}/user_project_wrapper: `{k}` missing/null", self.ledger))
    }
    fn f(&self, k: &str) -> f64 {
        self.w[k]
            .as_f64()
            .unwrap_or_else(|| panic!("{}/user_project_wrapper: `{k}` missing/null", self.ledger))
    }
    fn b(&self, k: &str) -> bool {
        self.w[k]
            .as_bool()
            .unwrap_or_else(|| panic!("{}/user_project_wrapper: `{k}` missing/null", self.ledger))
    }
}

/// The wrapper is hardened at the EXACT Caravel user-area die and streamed a GDSII.
#[test]
fn the_caravel_wrapper_hardened_at_the_fixed_die() {
    for wr in wrappers() {
        let (w, led) = (&wr.w, wr.ledger);
        assert_eq!(
            w["die_area"].as_str().unwrap_or(""),
            "0 0 2920 3520",
            "{led}: the wrapper must be hardened at the fixed 2920x3520 Caravel user area"
        );
        assert!(
            wr.i("gds_bytes") > 0,
            "{led}: no wrapper GDSII was streamed"
        );
        assert!(
            w["macro"].as_str() == Some(wr.project.macro_module),
            "{led}: the placed macro must be the project's full-interface user macro"
        );
    }
}

/// The gates the wrapper DOES meet: geometry agrees two ways, the layout is the netlist, the golden
/// shuttle deck passes, the route is complete, and setup is met with positive slack.
#[test]
fn the_caravel_wrapper_meets_its_geometry_and_setup_gates() {
    for wr in wrappers() {
        let led = wr.ledger;
        assert_eq!(
            wr.i("magic_drc_violations"),
            0,
            "{led}: wrapper Magic DRC is not clean"
        );
        assert_eq!(
            wr.i("lvs_errors"),
            0,
            "{led}: wrapper LVS error counter nonzero"
        );
        assert!(
            wr.b("lvs_clean"),
            "{led}: wrapper LVS is not a unique clean match (layout ≢ netlist)"
        );
        assert_eq!(
            wr.i("golden_drc_violations"),
            0,
            "{led}: wrapper fails the golden sky130A_mr DRC"
        );
        assert_eq!(
            wr.i("xor_self"),
            0,
            "{led}: KLayout↔Magic GDS XOR on the wrapper is nonzero"
        );
        assert_eq!(
            wr.i("routing_violations"),
            0,
            "{led}: wrapper detailed routing is not clean"
        );

        let setup = wr.f("setup_worst_slack_ns");
        assert!(
            setup > 0.0,
            "{led}: wrapper setup worst slack {setup} ns is not positive"
        );
        assert_eq!(
            wr.i("setup_violated_paths"),
            0,
            "{led}: wrapper has VIOLATED setup paths"
        );
        assert!(
            wr.b("multi_corner"),
            "{led}: wrapper STA was not multi-corner"
        );
    }
}

/// The electrical gates — max-slew and max-cap — asserted against their MEASURED values, not merely
/// recorded. `signoff_clean` already folds them in, but a single boolean computed by a script is not
/// evidence: if a future recorder edit drops them from its own gate, this test still fails.
///
/// Max-fanout is recorded and deliberately NOT gated, and the reason is specific rather than
/// convenient. All 96 flags on this wrapper are CTS clock buffers inside the macro driving 26–40
/// sinks against an SDC `set_max_fanout` of 10, on a one-cell shell whose slew and cap are both
/// zero. OpenLane has no `-max_fanout` repair anywhere — `repair_design` takes `-max_wire_length`,
/// `-slew_margin` and `-cap_margin` only — so it is not a P&R-repairable gate; `MAX_FANOUT_CONSTRAINT`
/// reaches abc's `buffer -c -N` at synthesis and `set_max_fanout` for reporting, nothing else.
/// Fanout matters only through the slew and cap it causes, and both are measured at zero here.
/// Gating on it would fail a manufacturable wrapper for a misapplied constraint.
#[test]
fn the_wrapper_electrical_gates_are_met() {
    for wr in wrappers() {
        let (w, led) = (&wr.w, wr.ledger);
        assert_eq!(
            wr.i("max_slew_violations"),
            0,
            "{led}: wrapper has max-SLEW violations — a transition slower than the library was \
         characterized for makes every downstream delay in the record a number the silicon will \
         not honour"
        );
        assert_eq!(
            wr.i("max_cap_violations"),
            0,
            "{led}: wrapper has max-CAPACITANCE violations — a driver loaded past its characterized \
         range is outside the timing model, not merely slow"
        );
        assert!(
            wr.f("max_transition_constraint") > 0.0 && wr.f("max_cap_constraint") > 0.0,
            "{led}: the slew/cap counts are meaningless without the constraints they were measured \
         against — record both"
        );
        assert_eq!(
            (wr.i("sta_netlist_missing_cells"), wr.i("sta_netlist_extra_cells")),
            (0, 0),
            "{led}: signoff STA read a netlist that is not the wrapper's layout — every slack and slew \
         above was computed on cells that were not drawn, or without cells that were"
        );
        assert!(
            w.get("max_fanout_violations").and_then(|v| v.as_i64()).is_some(),
            "{led}: max-fanout is not gated (see this test's docs) but it MUST stay recorded, so the \
         decision not to gate it is visible rather than invisible"
        );
    }
}

/// Hold must be MET and SAID OUT LOUD — never silence, and never an excuse.
///
/// This is the regression guard for the exact failure that motivated the rewrite: a gate that is not
/// recorded reads, to every consumer, identically to a gate that passed. So the measured slack must
/// be present, it must be positive, and the two waiver fields that once let a −0.57 ns wrapper call
/// itself clean must be ABSENT — asserted, so re-adding one turns this red instead of green.
#[test]
fn the_wrapper_hold_status_is_explicit_never_omitted() {
    for wr in wrappers() {
        let (w, led) = (&wr.w, wr.ledger);
        assert!(
            w.get("hold_worst_slack_ns").is_some(),
            "{led}: hold slack is ABSENT from the record — an omitted gate is indistinguishable from a \
         passing one; record the measured value even when it is negative"
        );
        let hold = wr.f("hold_worst_slack_ns");
        assert!(
            hold > 0.0,
            "{led}: hold must be MET — worst slack {hold} ns (a hold violation is dead silicon, and it \
         cannot be waived: see the header for why the passthrough basis was withdrawn)"
        );
        assert_eq!(
            wr.i("hold_violated_paths"),
            0,
            "{led}: wrapper has VIOLATED hold paths"
        );

        for k in ["hold_waived_to_chip_integration", "hold_waiver_reason"] {
            assert!(
                w.get(k).is_none(),
                "{led}: `{k}` is back in the record. No field may exist that excuses a nonzero gate — \
             close hold, do not re-describe it"
            );
        }
    }
}

/// The full shuttle-submittable verdict: every `mpw_precheck` gate at zero, ANTENNA INCLUDED.
///
/// CLOSED, and worth recording HOW, because the fix was geometric rather than a flow knob. The route
/// used to leave **14 antenna violators** on met3/met4/met5, every one a Wishbone pin into `mprj`.
/// No antenna lever touched them: the jog ECO moved 14 → 11 while pushing Magic DRC 0 → 42, and
/// diodes cannot fix an upper-metal area violation on a net that is simply too long.
///
/// The cause was pin placement. The fixed Caravel DEF puts all 104 Wishbone pins on the wrapper's
/// SOUTH edge (x 14.5..623.5 um), but the macro had been hardened with NO pin-order config, so
/// OpenLane auto-placed `wbs_dat_i[24]` on the macro's RIGHT edge at x~2858 — every Wishbone net ran
/// ~2.4 mm across the die. `scripts/gen-caravel-macro-pinorder.py` now mirrors the wrapper DEF's pin
/// order onto the macro (S=494 pins, same order), which moved that pin to (497.35, 0.0) on the
/// macro's BOTTOM edge, ~4 um in x from the wrapper pin it connects to. Nets went ~2.4 mm → ~880 um
/// and antenna went 14 → 0 with the jog OFF.
#[test]
fn the_caravel_wrapper_is_shuttle_submittable() {
    for wr in wrappers() {
        let (w, led) = (&wr.w, wr.ledger);
        assert_eq!(
            wr.i("antenna_violations"),
            0,
            "{led}: wrapper antenna summary nonzero"
        );
        assert_eq!(
            wr.i("antenna_violators"),
            0,
            "{led}: the wrapper antenna-violator list is non-empty"
        );
        assert_eq!(
            w["flow_status"].as_str().unwrap_or(""),
            "flow completed",
            "{led}: the wrapper flow did not complete"
        );
        assert!(
            wr.b("signoff_clean"),
            "{led}: wrapper not signoff-clean (a recorded gate above is nonzero)"
        );
    }
}

/// The record must carry EVERY gate, so no future edit can quietly go back to recording only the
/// passing ones. This asserts the shape of the evidence, independent of the values.
#[test]
fn the_wrapper_record_carries_every_gate() {
    for wr in wrappers() {
        let (w, led) = (&wr.w, wr.ledger);
        for k in [
            "die_area",
            "gds_bytes",
            "macro",
            "flow_status",
            "magic_drc_violations",
            "lvs_errors",
            "lvs_clean",
            "golden_drc_violations",
            "xor_self",
            "routing_violations",
            "antenna_violations",
            "antenna_violators",
            "setup_worst_slack_ns",
            "hold_worst_slack_ns",
            "setup_violated_paths",
            "hold_violated_paths",
            "multi_corner",
            "signoff_clean",
            "max_slew_violations",
            "max_cap_violations",
            "max_fanout_violations",
            "max_transition_constraint",
            "max_cap_constraint",
            "max_fanout_constraint",
            "sta_netlist_missing_cells",
            "sta_netlist_extra_cells",
            "pdk",
            "std_cell_library",
        ] {
            assert!(
                w.get(k).map(|v| !v.is_null()).unwrap_or(false),
                "{led}: `{k}` is missing from the wrapper record — every gate must be recorded, passing \
             or not, so an omission can never again read as a pass"
            );
        }
    }
}
