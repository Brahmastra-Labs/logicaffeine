//! sky130_deep_signoff_lock — the "foundry-grade" signoff lock. The top-line gates
//! (DRC/LVS/antenna/routing = 0) are necessary but NOT sufficient to call a GDSII genuinely
//! signed off. This lock pins the DEEP evidence a skeptical reviewer — or a golden signoff deck —
//! actually checks, for EVERY taped-out block, distilled by `scripts/record-sky130-signoff.py`
//! from each OpenLane run's real reports into `benchmarks/silicon/sky130_signoff.json`:
//!
//!   • setup AND hold worst slack are POSITIVE (met), with ZERO VIOLATED timing paths — a WNS of 0
//!     alone hides hold violations, which are the classic "clean layout, dead silicon" trap;
//!   • the KLayout↔Magic GDS XOR is ZERO — the two independent streamers agree, so no geometry was
//!     silently dropped or added;
//!   • Netgen reports a UNIQUE LVS match ("no mismatches"), not merely an error counter at 0;
//!   • the antenna VIOLATORS list is empty (not just the summary counter);
//!   • worst VPWR IR-drop is within budget (< 90 mV ≈ 5% of the 1.8 V rail);
//!   • timing was signed off MULTI-CORNER, not at a single typical corner.
//!
//! Every field is fail-closed at the recorder: a missing report becomes a sentinel that trips a
//! gate here. If any block regresses on any gate, this goes RED until the flow is re-run clean.
//!
//! SCOPE (locked below, so no prose can overclaim): this is a signoff-clean HARDENED MACRO under
//! the OPEN OpenLane/Sky130 flow — manufacturable, NOT fabricated silicon. A foundry tapeout still
//! needs a pad/ESD ring, a golden (Calibre) deck pass, DFT/scan, and a shuttle harness.

#![cfg(not(target_arch = "wasm32"))]

use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

const SIG: &str = "benchmarks/silicon/sky130_signoff.json";

fn doc() -> serde_json::Value {
    let p = root().join(SIG);
    let raw = fs::read_to_string(&p).unwrap_or_else(|e| {
        panic!(
            "cannot read {}: {e} — regenerate via scripts/openlane-run.sh",
            p.display()
        )
    });
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("bad JSON in {SIG}: {e}"))
}

fn blocks() -> serde_json::Map<String, serde_json::Value> {
    doc()["blocks"]
        .as_object()
        .cloned()
        .unwrap_or_else(|| panic!("{SIG}: `blocks` must be an object"))
}

/// The blocks that MUST be present and deeply-clean. Two of them ARE Infinichip (its command player
/// and the integrated subsystem) — so this lock covers the product itself, not just demo cores.
const REQUIRED: &[&str] = &[
    "soft_core_scalar",
    "rev_alu_core",
    "cgra_sumto",
    "nested_tri",
    "dram_executor", // Infinichip's command player
    "infinichip",    // the integrated memory subsystem
];

fn f64_field(b: &serde_json::Value, k: &str, name: &str) -> f64 {
    b[k].as_f64().unwrap_or_else(|| {
        panic!("{SIG}/{name}: `{k}` missing or null — re-run record-sky130-signoff.py")
    })
}
fn i64_field(b: &serde_json::Value, k: &str, name: &str) -> i64 {
    b[k].as_i64().unwrap_or_else(|| {
        panic!("{SIG}/{name}: `{k}` missing or null — re-run record-sky130-signoff.py")
    })
}
fn bool_field(b: &serde_json::Value, k: &str, name: &str) -> bool {
    b[k].as_bool().unwrap_or_else(|| {
        panic!("{SIG}/{name}: `{k}` missing or null — re-run record-sky130-signoff.py")
    })
}

/// Every required block must be present, and every OTHER block recorded must also pass — no block
/// gets a pass by being absent.
#[test]
fn every_required_block_is_recorded() {
    let b = blocks();
    for name in REQUIRED {
        assert!(
            b.contains_key(*name),
            "{SIG}: required block `{name}` is missing — tape it out"
        );
    }
    assert!(
        b.len() >= REQUIRED.len(),
        "{SIG}: fewer blocks than required"
    );
}

/// THE battery: every block passes every deep gate. One assertion per gate per block, so a failure
/// names exactly which gate on which block regressed.
#[test]
fn every_block_passes_every_deep_signoff_gate() {
    for (name, b) in blocks() {
        // Flow + artifact.
        assert_eq!(
            b["flow_status"].as_str().unwrap_or(""),
            "flow completed",
            "{name}: flow incomplete"
        );
        assert!(
            i64_field(&b, "gds_bytes", &name) > 0,
            "{name}: no GDSII streamed"
        );
        assert!(
            i64_field(&b, "std_cells", &name) > 0,
            "{name}: zero std cells (a stub, not a design)"
        );
        // Physical-verification gates.
        assert_eq!(
            i64_field(&b, "drc_violations", &name),
            0,
            "{name}: Magic DRC not clean"
        );
        assert_eq!(
            i64_field(&b, "lvs_errors", &name),
            0,
            "{name}: LVS error counter nonzero"
        );
        assert!(
            bool_field(&b, "lvs_clean", &name),
            "{name}: LVS is not a unique clean match"
        );
        assert_eq!(
            i64_field(&b, "antenna_violations", &name),
            0,
            "{name}: antenna summary nonzero"
        );
        assert_eq!(
            i64_field(&b, "antenna_violators", &name),
            0,
            "{name}: antenna violators list non-empty"
        );
        assert_eq!(
            i64_field(&b, "routing_violations", &name),
            0,
            "{name}: detailed routing not clean"
        );
        assert_eq!(
            i64_field(&b, "xor_differences", &name),
            0,
            "{name}: KLayout↔Magic GDS XOR nonzero"
        );
        // Timing: setup AND hold met, no violated paths.
        assert!(
            f64_field(&b, "setup_worst_slack_ns", &name) > 0.0,
            "{name}: setup slack not met"
        );
        assert!(
            f64_field(&b, "hold_worst_slack_ns", &name) > 0.0,
            "{name}: HOLD slack not met"
        );
        assert_eq!(
            i64_field(&b, "setup_violated_paths", &name),
            0,
            "{name}: setup has VIOLATED paths"
        );
        assert_eq!(
            i64_field(&b, "hold_violated_paths", &name),
            0,
            "{name}: hold has VIOLATED paths"
        );
        // Power integrity + corner coverage.
        let ir = f64_field(&b, "ir_drop_worst_mv", &name);
        assert!(
            (0.0..90.0).contains(&ir),
            "{name}: worst IR-drop {ir} mV out of budget (< 90 mV)"
        );
        assert!(
            bool_field(&b, "multi_corner", &name),
            "{name}: STA was not multi-corner"
        );
        // The recorder's own conjunction must agree.
        assert!(
            bool_field(&b, "signoff_clean", &name),
            "{name}: recorder marked NOT signoff-clean"
        );
    }
}

/// Dedicated: setup AND hold are BOTH met with positive slack for every block (the hold check is the
/// one a WNS=0 headline silently omits).
#[test]
fn timing_is_signed_off_for_both_setup_and_hold() {
    for (name, b) in blocks() {
        let setup = f64_field(&b, "setup_worst_slack_ns", &name);
        let hold = f64_field(&b, "hold_worst_slack_ns", &name);
        println!("{name}: setup {setup:+.3} ns, hold {hold:+.3} ns");
        assert!(
            setup > 0.0,
            "{name}: setup slack {setup} ns is not positive"
        );
        assert!(
            hold > 0.0,
            "{name}: hold slack {hold} ns is not positive (hold violation = dead silicon)"
        );
    }
}

/// Dedicated: the two independent GDS streamers (Magic, KLayout) produce byte-for-shape identical
/// geometry — XOR is exactly zero everywhere.
#[test]
fn gds_geometry_is_cross_checked_by_xor() {
    for (name, b) in blocks() {
        assert_eq!(
            i64_field(&b, "xor_differences", &name),
            0,
            "{name}: GDS XOR reveals a geometry mismatch"
        );
    }
}

/// Dedicated: LVS is a UNIQUE, clean layout-vs-schematic match for every block — the layout
/// implements exactly the synthesized netlist (which our iverilog + certified SAT prove ≡ the VM).
#[test]
fn lvs_is_a_unique_clean_match() {
    for (name, b) in blocks() {
        assert!(
            bool_field(&b, "lvs_clean", &name),
            "{name}: LVS did not report a unique clean match"
        );
        assert_eq!(
            i64_field(&b, "lvs_errors", &name),
            0,
            "{name}: LVS error counter nonzero"
        );
    }
}

/// Dedicated: worst-case VPWR IR-drop is comfortably within a 5%-of-rail (90 mV) budget for every
/// block — the power grid actually delivers voltage, it is not a floating promise.
#[test]
fn power_grid_ir_drop_is_within_budget() {
    for (name, b) in blocks() {
        let ir = f64_field(&b, "ir_drop_worst_mv", &name);
        println!("{name}: worst VPWR IR-drop {ir:.2} mV");
        assert!(
            (0.0..90.0).contains(&ir),
            "{name}: IR-drop {ir} mV exceeds the 90 mV budget"
        );
    }
}

/// Both Infinichip blocks specifically — the command player AND the integrated subsystem — clear
/// every deep gate. This is the product itself as a signoff-clean hardened macro.
#[test]
fn both_infinichip_blocks_are_deeply_clean() {
    let b = blocks();
    for name in ["dram_executor", "infinichip"] {
        let blk = b
            .get(name)
            .unwrap_or_else(|| panic!("{SIG}: {name} missing"));
        assert!(
            bool_field(blk, "signoff_clean", name),
            "{name}: not deeply signoff-clean"
        );
        assert!(
            f64_field(blk, "hold_worst_slack_ns", name) > 0.0,
            "{name}: hold not met"
        );
        assert_eq!(
            i64_field(blk, "xor_differences", name),
            0,
            "{name}: XOR nonzero"
        );
        assert!(
            bool_field(blk, "multi_corner", name),
            "{name}: not multi-corner"
        );
    }
    // The integrated subsystem is a real integrated design, not a stub.
    assert!(
        i64_field(&b["infinichip"], "std_cells", "infinichip") >= 5_000,
        "infinichip has too few cells to be the integrated subsystem"
    );
}
