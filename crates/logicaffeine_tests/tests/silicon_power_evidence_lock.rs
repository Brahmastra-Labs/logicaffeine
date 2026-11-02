//! silicon_power_evidence_lock — a power figure is measured on the netlist its signoff ledger SHIPS,
//! at the clock it is signed off at, and every figure derived from it is the arithmetic of it.
//!
//! WHY. The power ledgers carried no identity of the netlist they measured. When the 2026-09-23
//! all-corner re-harden gave every block a new netlist, `sky130_power.json` still described a July
//! soft-core (65,432 cells, 30.8 mW; the shipped core is 69,433 cells), `systolic_power.json` a
//! 74,664-cell tile (it ships 42,228), and every lock stayed green. The measurements also toggled a
//! fixed 100 MHz testbench clock whatever the block's signoff clock was, so an energy per MAC could
//! divide a power measured at one toggle rate by a different clock. And a Fmax quoted beside them was
//! the typical-corner best case the part does not guarantee.
//!
//! So each power record names the GDS it measured and the clock it measured at, both must be the
//! signoff ledger's, and each quoted Fmax is the frequency every corner meets on that netlist:
//! 1000 / (signoff period - worst setup slack across every corner).

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/sky130_ledgers.rs"]
mod sky130_ledgers;

use std::path::{Path, PathBuf};

#[path = "common/prose_figures.rs"]
mod prose_figures;
use prose_figures::{figures, numbers, printed_from, prose};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn json(rel: &str) -> serde_json::Value {
    let raw =
        std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"));
    serde_json::from_str(&raw).unwrap_or_else(|e| panic!("bad JSON in {rel}: {e}"))
}

/// The block's record in whichever registered ledger holds it, of either class.
fn signoff(block: &str) -> serde_json::Value {
    for l in sky130_ledgers::registry(&root()) {
        let d = json(&l.path);
        if !d["blocks"][block].is_null() {
            return d["blocks"][block].clone();
        }
    }
    panic!("{block} is in no registered block ledger");
}

fn num(v: &serde_json::Value, what: &str) -> f64 {
    v.as_f64()
        .unwrap_or_else(|| panic!("{what} is missing or not a number: {v}"))
}

/// (where, block, the power record) for every power measurement the silicon ledgers carry.
fn records() -> Vec<(&'static str, &'static str, serde_json::Value)> {
    let dfp = json("benchmarks/silicon/decision_fabric_power.json");
    vec![
        (
            "sky130_power.json",
            "soft_core_scalar",
            json("benchmarks/silicon/sky130_power.json"),
        ),
        (
            "decision_fabric_power.json#signoff_clean",
            "decision_hft_bist",
            dfp["signoff_clean"].clone(),
        ),
        (
            "decision_fabric_power.json#vcd_validation",
            "decision_hft_realbist",
            dfp["vcd_validation"].clone(),
        ),
        (
            "decision_fabric_power.json#cpu_twin",
            "soft_core_realbist",
            dfp["cpu_twin"].clone(),
        ),
        (
            "systolic_power.json",
            "systolic_tile_realbist",
            json("benchmarks/silicon/systolic_power.json"),
        ),
        (
            "systolic_pipelined_power.json",
            "systolic_pipe8_realbist",
            json("benchmarks/silicon/systolic_pipelined_power.json"),
        ),
        (
            "gemm_fabric.json#energy.signoff_clean_tape",
            "gemm_mac_realbist",
            json("benchmarks/silicon/gemm_fabric.json")["energy"]["signoff_clean_tape"].clone(),
        ),
    ]
}

#[test]
fn every_power_record_measured_the_gds_its_ledger_ships_at_its_signoff_clock() {
    let mut wrong = Vec::new();
    for (at, block, r) in records() {
        let led = signoff(block);
        if r["gds_bytes"].as_i64() != led["gds_bytes"].as_i64() {
            wrong.push(format!(
                "  {at}: measured GDS {} bytes, {block} ships {}",
                r["gds_bytes"], led["gds_bytes"]
            ));
        }
        if r["clock_period_ns"].as_f64() != led["clock_period_ns"].as_f64() {
            wrong.push(format!(
                "  {at}: measured at {} ns, {block} is signed off at {} ns",
                r["clock_period_ns"], led["clock_period_ns"]
            ));
        }
        let mhz = r["clock_mhz"].as_f64().unwrap_or(f64::NAN);
        let want = 1000.0 / num(&led["clock_period_ns"], "ledger clock");
        if (mhz - want).abs() > 0.01 {
            wrong.push(format!(
                "  {at}: clock {mhz} MHz, the signoff clock is {want:.3} MHz"
            ));
        }
    }
    assert!(wrong.is_empty(),
        "power evidence not bound to the shipped netlist — re-run scripts/measure-silicon-power.py <block> \
         and scripts/record-silicon-power.py:\n{}", wrong.join("\n"));
}

/// The switching activity a power figure rests on is the design's only if the netlist computes the
/// design over the simulated window: the RTL it was synthesized from, run over the same window, must
/// follow the same signature trajectory — every settled move at the same time. And that agreement
/// proves something only if the signature moved — a MISR fed a decision that is always zero sits at
/// its reset value however wrong the logic behind it is.
#[test]
fn every_power_record_is_witnessed_by_its_rtl_and_every_vcd_figure_by_a_live_signature() {
    let mut wrong = Vec::new();
    for (at, _, r) in records() {
        if r["simulated_cycles"].as_u64().unwrap_or(0) == 0 {
            wrong.push(format!("  {at}: no simulated window recorded"));
            continue;
        }
        let (gate, rtl) = (r["signature"].as_str(), r["rtl_signature"].as_str());
        if gate.is_none() || gate != rtl {
            wrong.push(format!(
                "  {at}: the gate window ends at {gate:?}, its RTL at {rtl:?}"
            ));
        }
        let (gt, rt) = (
            r["signature_trajectory_sha256"].as_str(),
            r["rtl_signature_trajectory_sha256"].as_str(),
        );
        if gt.map_or(true, |t| t.len() != 64) || gt != rt {
            wrong.push(format!(
                "  {at}: the gate trajectory {gt:?} is not the one its RTL follows, {rt:?}"
            ));
        }
        let vcd_figure = r["power_mw"]
            .as_object()
            .is_some_and(|o| o.keys().any(|k| k.contains("vcd")));
        if vcd_figure && r["signature_changes"].as_u64().unwrap_or(0) == 0 {
            wrong.push(format!(
                "  {at}: a VCD-measured power on a window whose signature never moved"
            ));
        }
    }
    assert!(wrong.is_empty(),
        "power activity not witnessed by the design it claims to be — re-run scripts/measure-silicon-power.py \
         <block> and scripts/record-silicon-power.py:\n{}", wrong.join("\n"));
}

#[test]
fn every_quoted_fmax_is_the_frequency_every_corner_meets() {
    let mut wrong = Vec::new();
    for (at, block, r) in records() {
        let Some(f) = r["fmax_mhz"].as_f64() else {
            continue;
        };
        let led = signoff(block);
        let every = 1000.0
            / (num(&led["clock_period_ns"], "period") - num(&led["setup_worst_slack_ns"], "slack"));
        if (f - every).abs() > 0.05 {
            wrong.push(format!(
                "  {at}: Fmax {f} MHz, every corner meets {every:.1} MHz on the shipped netlist"
            ));
        }
    }
    let hc = &json("benchmarks/silicon/decision_fabric_power.json")["honest_comparison"];
    for (side, block) in [
        ("soft_core", "soft_core_scalar"),
        ("fabric", "decision_hft_bist"),
    ] {
        let led = signoff(block);
        let every = 1000.0
            / (num(&led["clock_period_ns"], "period") - num(&led["setup_worst_slack_ns"], "slack"));
        let f = num(&hc[side]["fmax_mhz"], "honest_comparison fmax");
        if (f - every).abs() > 0.05 {
            wrong.push(format!(
                "  honest_comparison.{side}: Fmax {f} MHz, every corner meets {every:.1} MHz"
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "typical-corner Fmax quoted as the part's:\n{}",
        wrong.join("\n")
    );
}

#[test]
fn every_derived_energy_is_the_arithmetic_of_its_measurement() {
    let near = |a: f64, b: f64| (a - b).abs() <= 0.01 * b.abs() + 0.006;
    let mut wrong = Vec::new();
    for rel in [
        "benchmarks/silicon/systolic_power.json",
        "benchmarks/silicon/systolic_pipelined_power.json",
    ] {
        let p = json(rel);
        let period = num(&p["clock_period_ns"], "period");
        let macs = num(&p["macs_per_clock"], "macs");
        for act in ["default_activity", "vcd_measured"] {
            let want = num(&p["power_mw"][act], "power") * period / macs;
            let got = num(&p["energy_per_mac_pJ"][act], "energy/MAC");
            if !near(got, want) {
                wrong.push(format!(
                    "  {rel} {act}: {got} pJ/MAC, power x period / MACs = {want:.3}"
                ));
            }
        }
    }
    let g = &json("benchmarks/silicon/gemm_fabric.json")["energy"]["signoff_clean_tape"];
    let want = num(&g["power_mw"]["vcd_measured"], "gemm power")
        * num(&g["clock_period_ns"], "gemm period")
        / 8.0;
    if !near(
        num(&g["energy_per_mac_pJ"]["vcd_measured"], "gemm pJ/MAC"),
        want,
    ) {
        wrong.push(format!(
            "  gemm_fabric.json: pJ/MAC != power x period / 8 = {want:.3}"
        ));
    }
    let dfp = json("benchmarks/silicon/decision_fabric_power.json");
    let sc = &dfp["signoff_clean"];
    let period = num(&sc["clock_period_ns"], "fabric period");
    for (e, p) in [
        ("decision", "decision_selectdag_comb"),
        ("block", "block_total"),
    ] {
        let want = num(&sc["power_mw"][p], "fabric power") * period;
        if !near(num(&sc["energy_per_trade_pJ"][e], "fabric energy"), want) {
            wrong.push(format!(
                "  decision_fabric_power.json signoff_clean.{e}: != power x period = {want:.2} pJ"
            ));
        }
    }
    let sky = json("benchmarks/silicon/sky130_power.json");
    let soft = &dfp["honest_comparison"]["soft_core"];
    let want = num(&sky["power_w"]["total"], "soft W")
        * 1e3
        * num(&soft["decision_kernel_cycles"], "cycles")
        * num(&sky["clock_period_ns"], "soft period")
        / 1000.0;
    if !near(num(&soft["energy_per_op_nJ"], "soft nJ"), want) {
        wrong.push(format!(
            "  honest_comparison.soft_core: energy/op != power x cycles x period = {want:.2} nJ"
        ));
    }
    assert!(
        wrong.is_empty(),
        "derived energies that are not their measurement's arithmetic:\n{}",
        wrong.join("\n")
    );
}

/// Every ledger that states a power, an energy or a clock — in its numbers or in its prose.
const POWER_LEDGERS: &[&str] = &[
    "sky130_power",
    "decision_fabric_power",
    "systolic_power",
    "systolic_pipelined_power",
    "gemm_fabric",
    "data_dependent_control",
    "energy_efficiency",
];

#[test]
fn every_power_figure_a_ledger_states_in_prose_is_one_it_records() {
    let mut wrong = Vec::new();
    for name in POWER_LEDGERS {
        let d = json(&format!("benchmarks/silicon/{name}.json"));
        let recorded: Vec<f64> = numbers(&d)
            .into_iter()
            .flat_map(|u| [u, u * 1e3, u * 1e-3])
            .collect();
        for (path, s) in prose(&d) {
            for unit in ["mW", "µW", "W", "pJ", "nJ", "µJ", "MHz"] {
                for (v, digits) in figures(&s, unit) {
                    if !printed_from(v, digits, &recorded) {
                        wrong.push(format!(
                            "  {name}.json{path}: `{v} {unit}` is recorded nowhere in the ledger"
                        ));
                    }
                }
            }
        }
    }
    assert!(wrong.is_empty(),
        "prose quoting a power, energy or clock the ledger does not record — a figure typed once and left \
         behind when the measurement moved:\n{}", wrong.join("\n"));
}

#[test]
fn every_board_run_took_its_cycles_at_its_recorded_clock() {
    let board = &json("benchmarks/silicon/gemm_fabric.json")["measured_on_real_silicon"];
    let mhz = num(&board["core_clock_mhz"], "the board's core clock");
    let runs = board["runs"].as_array().expect("board runs");
    assert!(!runs.is_empty(), "no board runs recorded");
    for r in runs {
        let cycles = num(&r["on_chip_cycles"], "on-chip cycles");
        let wall = num(&r["wall_us"], "wall time");
        assert!(
            (cycles / mhz - wall).abs() <= 0.01,
            "{}: {cycles} cycles at {mhz} MHz take {:.2} µs, the record says {wall} µs",
            r["kernel"],
            cycles / mhz
        );
    }
}

#[test]
fn the_figure_scanner_reads_figures_and_only_figures() {
    assert_eq!(
        figures("draws 0.707 mW at 57.143 MHz", "mW"),
        vec![(0.707, 3)]
    );
    assert_eq!(figures("draws 0.707 mW at 57.143 MHz", "W"), vec![]);
    assert_eq!(
        figures("26.6mW; 1,261-2,039x LESS", "x"),
        vec![(2039.0, 0), (1261.0, 0)]
    );
    assert_eq!(figures("energy = power x time, v2 x", "x"), vec![]);
    assert_eq!(figures("on 0x1f and 4.9x", "x"), vec![(4.9, 1)]);
    assert_eq!(figures("a Wormhole b0", "W"), vec![]);
    assert!(printed_from(38.0, 0, &[37.62]) && !printed_from(35.0, 0, &[37.62]));
    assert!(printed_from(0.707, 3, &[0.707]) && !printed_from(0.886, 3, &[0.707, 0.812]));
}
