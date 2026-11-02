//! signoff_ready_lock — a block recorded `signoff_clean: true` is READY TO MANUFACTURE, in every
//! ledger, and the claim is checked against the record itself rather than trusted.
//!
//! WHY. The per-gate locks (`sky130_signoff_lock`, `sky130_deep_signoff_lock`) read ONE ledger,
//! `sky130_signoff.json`. Six more ledgers carry blocks — `fabric_soc`, `fabric_horner_seq`,
//! `gemm_mac_realbist`, the systolic family, both decision blocks — and for those the only gate on a
//! `signoff_clean: true` was `sky130_provenance_lock`, which re-derives from run directories and so
//! skips on every machine that did not do the tape-out. A hand-edited `true` over an antenna count of
//! 10 would have passed CI. The 2026-09-24 audit that found every block's readiness was a scratch
//! script; this is that audit, lifted into the suite so it runs everywhere, forever.
//!
//! READY means all three of:
//!   1. every gate the recorder conjoins into `signoff_clean` is at its passing value — restated
//!      here independently, so the flag and the evidence it summarizes cannot disagree;
//!   2. the block was built at the period `codegen_sva::signoff_blocks` holds for it, the one derived
//!      from its measured slow corner — not some other period that happened to close;
//!   3. the golden `sky130A_mr` deck (the one `mpw_precheck` accepts a shuttle on) passed on the very
//!      GDS the ledger ships, byte for byte.

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/sky130_ledgers.rs"]
mod sky130_ledgers;

use logicaffeine_compile::codegen_sva::signoff_blocks::registered_period_ns;
use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn json(rel: &str) -> serde_json::Value {
    let raw =
        std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("bad JSON in {rel}: {e}"))
}

/// (ledger, block, record) for every block in every signoff-class ledger of the registry, which
/// `sky130_ledger_registry_lock` holds equal to the block ledgers on disk — so a new ledger cannot slip past.
fn every_block() -> Vec<(String, String, serde_json::Value)> {
    let mut leds: Vec<String> = sky130_ledgers::registry(&root())
        .into_iter()
        .filter(|l| l.class == "signoff")
        .map(|l| l.path)
        .collect();
    leds.sort();
    let mut out = Vec::new();
    for led in leds {
        let Some(blocks) = json(&led)["blocks"].as_object().cloned() else {
            continue;
        };
        for (name, b) in blocks {
            out.push((led.clone(), name, b));
        }
    }
    assert!(
        out.len() >= 19,
        "found only {} ledger blocks — discovery is broken",
        out.len()
    );
    out
}

fn clean_blocks() -> Vec<(String, String, serde_json::Value)> {
    let out: Vec<_> = every_block()
        .into_iter()
        .filter(|(_, _, b)| b["signoff_clean"] == serde_json::json!(true))
        .collect();
    assert!(!out.is_empty(), "no block is recorded signoff-clean");
    out
}

/// Every gate `record-sky130-signoff.py` conjoins into `signoff_clean`, with the reason it failed.
/// Missing and null fields fail: an absent gate reads exactly like a passing one otherwise.
fn failed_gates(b: &serde_json::Value) -> Vec<String> {
    let int = |k: &str| b[k].as_i64();
    let num = |k: &str| b[k].as_f64();
    let mut bad = Vec::new();
    let mut zero = |k: &str| {
        if int(k) != Some(0) {
            bad.push(format!("{k}={}", b[k]));
        }
    };
    for k in [
        "drc_violations",
        "lvs_errors",
        "antenna_violations",
        "antenna_violators",
        "routing_violations",
        "xor_differences",
        "setup_violated_paths",
        "hold_violated_paths",
        "max_slew_violations",
        "max_cap_violations",
        "sta_netlist_missing_cells",
        "sta_netlist_extra_cells",
    ] {
        zero(k);
    }
    let status = b["flow_status"].as_str().unwrap_or("");
    if !matches!(status, "flow completed" | "flow_completed") {
        bad.push(format!("flow_status={status:?}"));
    }
    if b["lvs_clean"] != serde_json::json!(true) {
        bad.push(format!("lvs_clean={}", b["lvs_clean"]));
    }
    if b["multi_corner"] != serde_json::json!(true) {
        bad.push(format!("multi_corner={}", b["multi_corner"]));
    }
    for k in ["setup_worst_slack_ns", "hold_worst_slack_ns"] {
        if !num(k).is_some_and(|s| s > 0.0) {
            bad.push(format!("{k}={}", b[k]));
        }
    }
    if !num("ir_drop_worst_mv").is_some_and(|mv| (0.0..90.0).contains(&mv)) {
        bad.push(format!(
            "ir_drop_worst_mv={} (budget 90)",
            b["ir_drop_worst_mv"]
        ));
    }
    for k in ["gds_bytes", "std_cells"] {
        if !int(k).is_some_and(|v| v > 0) {
            bad.push(format!("{k}={}", b[k]));
        }
    }
    bad
}

/// The flag is the conjunction of the gates, in both directions. A `true` over a failing gate is a
/// false claim; a `false` over all-passing gates is a record that no longer says what its run did.
#[test]
fn signoff_clean_is_exactly_the_conjunction_of_every_gate_in_every_ledger() {
    let mut wrong = Vec::new();
    for (led, name, b) in every_block() {
        let failed = failed_gates(&b);
        let claimed = b["signoff_clean"] == serde_json::json!(true);
        if claimed && !failed.is_empty() {
            wrong.push(format!(
                "  {led}/{name}: recorded CLEAN but fails {}",
                failed.join(", ")
            ));
        }
        if !claimed && failed.is_empty() {
            wrong.push(format!(
                "  {led}/{name}: recorded NOT clean but every gate passes — re-record it"
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "signoff_clean disagrees with the gates it summarizes:\n{}",
        wrong.join("\n")
    );
}

/// A clean block was built at exactly the period the compiler holds for it. Closing at some other
/// period is a different part: the registry is what `emit_flow_config` will build next time, and a
/// ledger entry from a period the registry no longer holds cannot be rebuilt from the tree.
#[test]
fn every_clean_block_was_built_at_its_registered_period() {
    let mut wrong = Vec::new();
    for (led, name, b) in clean_blocks() {
        let built = b["clock_period_ns"].as_f64();
        match (registered_period_ns(&name), built) {
            (Some(reg), Some(p)) if (reg - p).abs() < 1e-9 => {}
            (reg, p) => wrong.push(format!(
                "  {led}/{name}: built at {p:?} ns, registry holds {reg:?} ns"
            )),
        }
    }
    assert!(
        wrong.is_empty(),
        "clean blocks off their registered period:\n{}",
        wrong.join("\n")
    );
}

/// A clean block passed the golden `sky130A_mr` deck, and on the GDS the ledger ships. Magic DRC is
/// the flow's own deck; the golden one is what the shuttle accepts on, and it has caught geometry
/// Magic passed (same-net notches, minimum area, enclosure-to-wire spacing) on this very campaign.
#[test]
fn every_clean_block_is_golden_clean_on_the_gds_it_ships() {
    const GP: &str = "benchmarks/silicon/golden_precheck.json";
    let golden = json(GP);
    let mut wrong = Vec::new();
    for (led, name, b) in clean_blocks() {
        let g = &golden["blocks"][&name];
        if g.is_null() {
            wrong.push(format!("  {led}/{name}: no golden-deck verdict in {GP}"));
            continue;
        }
        if g["golden_clean"] != serde_json::json!(true)
            || g["golden_drc_violations"].as_i64() != Some(0)
            || g["zeroarea_violations"].as_i64() != Some(0)
        {
            wrong.push(format!(
                "  {name}: golden_clean={} golden_drc={} zeroarea={}",
                g["golden_clean"], g["golden_drc_violations"], g["zeroarea_violations"]
            ));
        }
        if g["gds_bytes"].as_i64() != b["gds_bytes"].as_i64() {
            wrong.push(format!(
                "  {name}: golden checked a {}-byte GDS, {led} ships {}",
                g["gds_bytes"], b["gds_bytes"]
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "clean blocks without a golden verdict on their shipped GDS:\n{}",
        wrong.join("\n")
    );
}

/// The gate check bites: take a real clean record and spoil one gate at a time — each spoil alone
/// must be caught. A gate list that silently lost an entry would otherwise pass every ledger.
#[test]
fn every_single_spoiled_gate_is_caught_on_its_own() {
    let (_, name, clean) = clean_blocks().into_iter().next().expect("a clean block");
    assert!(
        failed_gates(&clean).is_empty(),
        "{name}: the baseline record must pass"
    );
    let spoils: &[(&str, serde_json::Value)] = &[
        ("drc_violations", 1.into()),
        ("lvs_errors", 1.into()),
        ("antenna_violations", 1.into()),
        ("antenna_violators", 1.into()),
        ("routing_violations", 1.into()),
        ("xor_differences", 1.into()),
        ("setup_violated_paths", 1.into()),
        ("hold_violated_paths", 1.into()),
        ("max_slew_violations", 1.into()),
        ("max_cap_violations", 1.into()),
        ("flow_status", "flow failed".into()),
        ("lvs_clean", false.into()),
        ("multi_corner", false.into()),
        ("setup_worst_slack_ns", (-0.01).into()),
        ("setup_worst_slack_ns", 0.0.into()),
        ("hold_worst_slack_ns", (-0.01).into()),
        ("hold_worst_slack_ns", 0.0.into()),
        ("ir_drop_worst_mv", 90.0.into()),
        ("ir_drop_worst_mv", (-1.0).into()),
        ("gds_bytes", 0.into()),
        ("std_cells", 0.into()),
        ("drc_violations", serde_json::Value::Null),
        ("setup_worst_slack_ns", serde_json::Value::Null),
    ];
    for (k, v) in spoils {
        let mut b = clean.clone();
        b[*k] = v.clone();
        assert!(
            !failed_gates(&b).is_empty(),
            "{name} with {k}={v} passed the gate check"
        );
    }
    for k in [
        "max_slew_violations",
        "multi_corner",
        "ir_drop_worst_mv",
        "lvs_clean",
    ] {
        let mut b = clean.clone();
        b.as_object_mut().unwrap().remove(k);
        assert!(
            !failed_gates(&b).is_empty(),
            "{name} with `{k}` ABSENT passed the gate check"
        );
    }
}

/// A "what this runs at on the chip" projection runs at the signoff clock — the period the slow
/// corner meets — never at a typical-corner best case.
///
/// `scalar_timing.json` projected the demos at 90.3 MHz and `hft_latency.json` projected a tick at
/// 90 MHz / 9,622 ns: both from `soft_core_scalar`'s 11.07 ns typical critical path. The part
/// guarantees its registered 15 ns (66.7 MHz). Cycle counts are exact (the core is deterministic,
/// cycles == VM op count), so the projection is cycles × the registered period, and nothing else.
#[test]
fn every_tapeout_projection_runs_at_the_signoff_clock() {
    let period = registered_period_ns("soft_core_scalar").expect("soft_core_scalar is registered");
    let mhz = 1000.0 / period;
    let near = |a: f64, b: f64, tol: f64| (a - b).abs() <= tol;
    let mut wrong = Vec::new();

    let st = json("benchmarks/silicon/scalar_timing.json");
    let t_mhz = st["tapeout_mhz"].as_f64().unwrap_or(f64::NAN);
    if !near(t_mhz, mhz, 0.01) {
        wrong.push(format!(
            "  scalar_timing.json tapeout_mhz {t_mhz}, signoff clock {mhz:.2} MHz"
        ));
    }
    for d in st["demos"].as_array().expect("scalar_timing.json demos") {
        let cycles = d["cycles"].as_f64().expect("demo cycles");
        let ns = d["tapeout_ns"].as_f64().unwrap_or(f64::NAN);
        if !near(ns, cycles * period, 0.05) {
            wrong.push(format!(
                "  scalar_timing.json {}: {ns} ns, {cycles} cycles x {period} ns = {}",
                d["program"],
                cycles * period
            ));
        }
    }

    let hft = json("benchmarks/silicon/hft_latency.json");
    let cycles = hft["chip"]["per_message_cycles_median"]
        .as_f64()
        .expect("hft per-message cycles");
    let tier = &hft["chip"]["latency_by_tier"]["Sky130 tapeout"];
    let (t_mhz, t_ns) = (
        tier["mhz"].as_f64().unwrap_or(f64::NAN),
        tier["ns_per_msg"].as_f64().unwrap_or(f64::NAN),
    );
    if !near(t_mhz, mhz, 0.01) || !near(t_ns, cycles * period, 0.05) {
        wrong.push(format!("  hft_latency.json Sky130 tier {t_mhz} MHz / {t_ns} ns, signoff clock {mhz:.2} MHz x {cycles} cycles = {} ns", cycles * period));
    }
    assert!(
        wrong.is_empty(),
        "tapeout projections off the signoff clock:\n{}",
        wrong.join("\n")
    );
}
