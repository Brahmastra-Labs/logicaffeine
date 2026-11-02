//! sky130_signoff_lock — the "we taped out on Sky130" claim is pinned to a real OpenLane
//! RTL-to-GDSII signoff artifact, so no prose can claim a tape-out the flow did not actually
//! produce clean.
//!
//! Source of truth: `benchmarks/silicon/sky130_signoff.json`, written by
//! `scripts/openlane-run.sh <block> <clk> <period> [rtl.v ...]` — which runs the block through
//! OpenLane v1 (yosys → OpenROAD → Magic → Netgen → KLayout) on the open Sky130A PDK and parses
//! the run's `reports/metrics.csv`. A block is SIGNOFF-CLEAN only when the flow completed AND a
//! GDSII exists AND every physical-verification gate is zero: Magic DRC, Netgen LVS, antenna, and
//! detailed-routing violations. The same PDK-agnostic Verilog that fits the Nexys A7 tapes out
//! here — no ECP5/Xilinx primitives on the standard-cell path.
//!
//! Why this exists: a "tape-out" is only real if it is DRC/LVS clean with a streamed GDSII. If a
//! re-run ever regresses any gate above zero, `signoff_clean` flips and this goes RED; when the
//! artifact is refreshed the DOC is what changes to match it.

#![cfg(not(target_arch = "wasm32"))]

use std::fs;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn json(rel: &str) -> serde_json::Value {
    let p = root().join(rel);
    let raw = fs::read_to_string(&p).unwrap_or_else(|e| {
        panic!(
            "cannot read {}: {e} — regenerate via scripts/openlane-run.sh",
            p.display()
        )
    });
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("bad JSON in {rel}: {e}"))
}

const SIG: &str = "benchmarks/silicon/sky130_signoff.json";

/// The PDKs this project's flow supports. A ledger may name any of them; it may not name
/// something unrecognised, which would mean the recorder failed to read the run's own config.tcl.
/// Adding a PDK is a one-line change HERE and a `PdkTarget` — never a change to a gate.
const KNOWN_PDKS: &[&str] = &["sky130A", "sky130B", "gf180mcuC", "gf180mcuD", "ihp-sg13g2"];

fn assert_known_pdk(pdk: &str, where_: &str) {
    assert!(
        KNOWN_PDKS.iter().any(|k| pdk.contains(k)),
        "{where_}: meta.pdk = {pdk:?} names no PDK this flow supports ({KNOWN_PDKS:?}). \
         The recorder derives this from the run's own config.tcl, so an unrecognised value means \
         it could not read it — not that a new PDK was used."
    );
}

/// Every block recorded in the artifact must be a genuine, physically-clean tape-out: flow
/// completed, a GDSII streamed, and DRC / LVS / antenna / routing all zero.
#[test]
fn every_sky130_block_is_signoff_clean() {
    let d = json(SIG);
    let blocks = d["blocks"]
        .as_object()
        .expect("sky130_signoff.json: `blocks` must be an object");
    assert!(
        !blocks.is_empty(),
        "{SIG}: no blocks recorded — run scripts/openlane-run.sh"
    );

    let g = |v: &serde_json::Value, k: &str| -> i64 {
        v[k].as_i64()
            .unwrap_or_else(|| panic!("{SIG}: block missing/!int `{k}`"))
    };

    for (name, b) in blocks {
        assert_eq!(
            b["flow_status"].as_str().unwrap_or(""),
            "flow completed",
            "{SIG}/{name}: flow did not complete"
        );
        assert!(
            g(b, "gds_bytes") > 0,
            "{SIG}/{name}: no GDSII was streamed out"
        );
        assert_eq!(
            g(b, "drc_violations"),
            0,
            "{SIG}/{name}: Magic DRC is not clean"
        );
        assert_eq!(
            g(b, "lvs_errors"),
            0,
            "{SIG}/{name}: Netgen LVS does not match"
        );
        assert_eq!(
            g(b, "antenna_violations"),
            0,
            "{SIG}/{name}: antenna rule violations remain"
        );
        assert_eq!(
            g(b, "routing_violations"),
            0,
            "{SIG}/{name}: detailed routing is not clean"
        );
        // Independently gate TIMING — don't just trust `signoff_clean`. A signed-off block must MEET
        // setup with positive worst-corner slack and zero violated setup/hold paths. This is a second
        // line of defense: it catches a run recorded with WNS < 0 even if `signoff_clean` were wrongly
        // true (the class of bug where a tight-clock re-sign that failed setup was stamped clean).
        let setup_slack = b["setup_worst_slack_ns"]
            .as_f64()
            .unwrap_or_else(|| panic!("{SIG}/{name}: missing/!float `setup_worst_slack_ns`"));
        assert!(
            setup_slack > 0.0,
            "{SIG}/{name}: setup worst slack {setup_slack}ns is not positive — the design fails timing"
        );
        assert_eq!(
            g(b, "setup_violated_paths"),
            0,
            "{SIG}/{name}: has VIOLATED setup timing paths"
        );
        assert_eq!(
            g(b, "hold_violated_paths"),
            0,
            "{SIG}/{name}: has VIOLATED hold timing paths"
        );
        // Independently gate the remaining DEEP signoff evidence too — the lock trusts `signoff_clean`
        // for NOTHING. A complete independent verifier: XOR-clean GDS, the FULL antenna-violator list
        // (not just the summary counter), Netgen unique-match, IR-drop within budget, multi-corner STA.
        assert_eq!(
            g(b, "xor_differences"),
            0,
            "{SIG}/{name}: KLayout-vs-Magic GDS XOR is not clean"
        );
        assert_eq!(
            g(b, "antenna_violators"),
            0,
            "{SIG}/{name}: the antenna-violator list is non-empty"
        );
        assert!(
            b["lvs_clean"].as_bool().unwrap_or(false),
            "{SIG}/{name}: Netgen LVS is not a clean unique match"
        );
        assert!(
            b["multi_corner"].as_bool().unwrap_or(false),
            "{SIG}/{name}: multi-corner STA did not run"
        );
        let ir = b["ir_drop_worst_mv"]
            .as_f64()
            .unwrap_or_else(|| panic!("{SIG}/{name}: missing/!float `ir_drop_worst_mv`"));
        assert!(
            (0.0..90.0).contains(&ir),
            "{SIG}/{name}: worst IR-drop {ir}mV is outside the [0, 90) budget"
        );
        assert!(
            b["signoff_clean"].as_bool().unwrap_or(false),
            "{SIG}/{name}: signoff_clean=false — a gate above is nonzero"
        );
        // Not an empty netlist that "passes" by producing nothing.
        assert!(
            g(b, "std_cells") > 0,
            "{SIG}/{name}: zero std cells — did the flow synth anything?"
        );
    }
}

/// The two headline blocks — the reversible ALU core and the scalar LOGOS CPU — must both be
/// present, and the CPU must be a real design (thousands of cells), not a stub.
#[test]
fn the_scalar_cpu_and_reversible_core_both_taped_out() {
    let d = json(SIG);
    let blocks = &d["blocks"];

    let rev = &blocks["rev_alu_core"];
    assert!(
        rev.is_object(),
        "{SIG}: rev_alu_core missing — the reversible ALU tape-out is the first block"
    );
    assert!(
        rev["signoff_clean"].as_bool().unwrap_or(false),
        "{SIG}: rev_alu_core not signoff-clean"
    );

    let cpu = &blocks["soft_core_scalar"];
    assert!(
        cpu.is_object(),
        "{SIG}: soft_core_scalar missing — the flagship CPU tape-out"
    );
    assert!(
        cpu["signoff_clean"].as_bool().unwrap_or(false),
        "{SIG}: soft_core_scalar not signoff-clean"
    );
    assert!(
        cpu["std_cells"].as_i64().unwrap_or(0) >= 1_000,
        "{SIG}: soft_core_scalar has too few cells to be the real scalar CPU"
    );
    assert!(
        cpu["gds_bytes"].as_i64().unwrap_or(0) > 0
            && cpu["die_area_mm2"].as_f64().unwrap_or(0.0) > 0.0,
        "{SIG}: soft_core_scalar must have a streamed GDSII with a nonzero die area"
    );

    // The Futamura counted-loop CGRA takes control flow all the way to silicon — it too tapes out.
    let cgra = &blocks["cgra_sumto"];
    assert!(
        cgra.is_object(),
        "{SIG}: cgra_sumto missing — the counted-loop CGRA tape-out"
    );
    assert!(
        cgra["signoff_clean"].as_bool().unwrap_or(false),
        "{SIG}: cgra_sumto not signoff-clean"
    );
}

/// Infinichip's command player — the deterministic DRAM command-stream executor (`dram_executor`,
/// the rung-3 in-order countdown FSM whose decode + timing are certified by our own SAT in
/// `codegen_sva::dram`) — takes the product itself all the way to signoff-clean Sky130 GDSII. This
/// upgrades Infinichip from "FPGA-proven, not GDSII" to a real manufacturable die (still NOT
/// fabricated silicon — that is the funded bring-up milestone).
#[test]
fn the_infinichip_command_player_is_signoff_clean() {
    let d = json(SIG);
    let cp = &d["blocks"]["dram_executor"];
    assert!(
        cp.is_object(),
        "{SIG}: dram_executor missing — run scripts/openlane-run.sh dram_executor clk <period> \
         dram_executor.v to tape out Infinichip's command player"
    );
    assert_eq!(
        cp["flow_status"].as_str().unwrap_or(""),
        "flow completed",
        "{SIG}: dram_executor flow did not complete"
    );
    assert!(
        cp["gds_bytes"].as_i64().unwrap_or(0) > 0,
        "{SIG}: dram_executor streamed no GDSII"
    );
    assert!(
        cp["signoff_clean"].as_bool().unwrap_or(false),
        "{SIG}: dram_executor not signoff-clean — a DRC/LVS/antenna/routing gate is nonzero"
    );
    assert!(
        cp["std_cells"].as_i64().unwrap_or(0) > 0,
        "{SIG}: dram_executor has zero std cells — did the flow synth the FSM?"
    );
}

/// The integrated **Infinichip** subsystem — the command player + SECDED(72,64) ECC + double-buffer
/// composed into one `infinichip` top — takes the whole deterministic memory subsystem to a
/// signoff-clean Sky130 GDSII. This is the product itself as manufacturable silicon (still NOT
/// fabricated — a foundry/MPW run is the funded next step). A real integrated design, not a stub.
#[test]
fn the_infinichip_subsystem_is_signoff_clean() {
    let d = json(SIG);
    let ic = &d["blocks"]["infinichip"];
    assert!(
        ic.is_object(),
        "{SIG}: infinichip missing — run scripts/openlane-run.sh infinichip clk <period> \
         infinichip.v to tape out the integrated memory subsystem"
    );
    assert_eq!(
        ic["flow_status"].as_str().unwrap_or(""),
        "flow completed",
        "{SIG}: infinichip flow did not complete"
    );
    assert!(
        ic["gds_bytes"].as_i64().unwrap_or(0) > 0,
        "{SIG}: infinichip streamed no GDSII"
    );
    assert!(
        ic["signoff_clean"].as_bool().unwrap_or(false),
        "{SIG}: infinichip not signoff-clean — a DRC/LVS/antenna/routing gate is nonzero"
    );
    assert!(
        ic["std_cells"].as_i64().unwrap_or(0) >= 1_000,
        "{SIG}: infinichip has too few cells to be the real integrated subsystem"
    );
}

/// The PDK / flow provenance must be recorded, so the artifact says WHAT taped it out — the open
/// Sky130A PDK through OpenLane, not a hand-typed claim.
#[test]
fn the_signoff_records_the_open_pdk_and_flow() {
    let d = json(SIG);
    let pdk = d["meta"]["pdk"].as_str().unwrap_or("");
    let flow = d["meta"]["flow"].as_str().unwrap_or("");
    assert_known_pdk(pdk, SIG);
    assert!(
        flow.to_lowercase().contains("openlane"),
        "{SIG}: meta.flow must name OpenLane (got {flow:?})"
    );
}
