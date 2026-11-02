//! comparative_synthesis_lock — every number in the comparative-synthesis ledger comes from a raw ledger.
//!
//! `benchmarks/silicon/comparative_synthesis.json` is written only by `scripts/qor/assemble.py`, which joins the
//! functional gate (`qor_function.json`), the Artix-7 open flow (`qor_artix7.json`) and the candidate manifest.
//! This lock re-derives the join on its own: every candidate the bench knows has a row (each LOGOS candidate,
//! each third-party candidate a tool built, each one a tool refused, each K7 controller); a row passes its
//! functional gate exactly when both stimulus modes folded the golden signature; every median, every count of
//! routed and refused seeds, every cell count and every derived throughput equals what the raw rows give; every
//! Sky130 attempt is what its stage row staged beside what its run recorded (`qor_sky130_stage.json`,
//! `qor_sky130.json`), a candidate's own minimum the shortest period it closed at and its kernel's iso period the
//! longest own minimum, every attempt of a kernel through one flow (the overlay's SHA3-256 the joined ledger names),
//! and every control on stock OpenLane (`qor_sky130_control.json`) beside the attempt at its period and never among
//! the attempts; every energy per operation is its candidate's measured power block (`qor_power.json`), re-derived here
//! from the block's class powers, window and clock sinks, at its kernel's iso period unless the block there missed;
//! every K7 traffic summary is its runs in `qor_dram.json`, each controller passing every stream and every gate seeing one
//! RTL per controller; every evidence tier is one of the closed set; and every ledger a bench script names is public, so
//! a clone re-derives what this lock re-derives.

#[path = "common/public_manifest.rs"]
mod public_manifest;

use logicaffeine_compile::codegen_sva::ddr2_traffic::{k7_ddr2_ctrl_verilog, k7_dfi_ctrl_verilog, K7_CONTROLLERS};
use logicaffeine_compile::codegen_sva::qor_harness::{kernels, logos_candidates, read_manifest};
use logicaffeine_system::keccak::sha3_256_bytes;
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn json(rel: &str) -> Value {
    let text = std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn joined() -> Value {
    json("benchmarks/silicon/comparative_synthesis.json")
}

fn rows(v: &Value) -> &Vec<Value> {
    v["rows"].as_array().expect("rows")
}

const TIERS: [&str; 9] = [
    "SIM",
    "SIMULATED_TUNGSTEN",
    "MODELED",
    "SYN",
    "PNR-open",
    "GDS",
    "GDS-VCD",
    "GDS-default-activity",
    "BOARD",
];

fn median(mut v: Vec<f64>) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-9 * a.abs().max(b.abs()).max(1.0)
}

#[test]
fn the_ledger_is_the_assemblers_and_its_tiers_are_the_closed_set() {
    let j = joined();
    assert_eq!(j["_generated_by"], "scripts/qor/assemble.py (never hand-edited)");
    assert_eq!(j["schema"], "logos.qor.comparative/1");
    let tiers: Vec<&str> = j["evidence_tiers"].as_array().unwrap().iter().map(|t| t.as_str().unwrap()).collect();
    assert_eq!(tiers, TIERS);
    for r in rows(&j) {
        for tier in [&r["function"]["tier"], &r["artix7"]["fabric"]["tier"], &r["artix7"]["native"]["tier"],
                     &r["power"]["measured"]["evidence_tier"]] {
            if let Some(t) = tier.as_str() {
                assert!(TIERS.contains(&t), "{}: tier {t}", r["key"]);
            }
        }
    }
    let keys: Vec<&str> = rows(&j).iter().map(|r| r["key"].as_str().unwrap()).collect();
    let mut sorted = keys.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(keys, sorted, "one row per key, in key order");
}

#[test]
fn every_candidate_the_bench_knows_has_a_row() {
    let j = joined();
    let have: BTreeMap<String, &Value> = rows(&j).iter().map(|r| (r["candidate"].as_str().unwrap().to_string(), r)).collect();
    for k in kernels() {
        for (c, _) in logos_candidates(k.id).unwrap_or_default() {
            let r = have.get(&c.name).unwrap_or_else(|| panic!("LOGOS candidate {} has no row", c.name));
            assert_eq!(r["tool"], "logos", "{}", c.name);
        }
    }
    let m = read_manifest(&root()).unwrap();
    for e in &m.candidates {
        let r = have.get(&e.candidate.name).unwrap_or_else(|| panic!("{} has no row", e.candidate.name));
        assert_eq!(r["tool"], e.tool.as_str(), "{}", e.candidate.name);
        assert_eq!(r["generate"], e.generate.as_str(), "{}: the command that built it", e.candidate.name);
    }
    for refusal in &m.refusals {
        let r = have.get(&refusal.name).unwrap_or_else(|| panic!("the refused {} has no row", refusal.name));
        assert_eq!(r["refusal"]["reason"], refusal.reason.as_str(), "{}: the refusal is printed", refusal.name);
        assert!(r["function"].is_null() && r["derived"].is_null(), "{}: a refused candidate has no numbers", refusal.name);
    }
    let k7 = json("benchmarks/silicon/qor/kernels/K7/kernel.json");
    for (name, _) in k7["designs"].as_object().unwrap() {
        assert!(have.contains_key(name), "the K7 controller {name} has no row");
    }
}

#[test]
fn a_functional_verdict_is_both_modes_folding_the_golden() {
    let j = joined();
    let raw = json("benchmarks/silicon/qor_function.json");
    let mut by: BTreeMap<String, Vec<&Value>> = BTreeMap::new();
    for r in rows(&raw) {
        by.entry(r["candidate"].as_str().unwrap().to_string()).or_default().push(r);
    }
    for r in rows(&j) {
        let cand = r["candidate"].as_str().unwrap();
        let Some(raw_rows) = by.get(cand) else {
            assert!(r["function"].is_null(), "{cand}: a functional verdict with no simulation behind it");
            continue;
        };
        let modes: BTreeSet<&str> = raw_rows.iter().map(|x| x["mode"].as_str().unwrap()).collect();
        let want = modes == BTreeSet::from(["edge", "shaped"])
            && raw_rows.iter().all(|x| {
                x["pass"] == true && x["signature"] == x["golden_signature"] && x["ops"] == x["ops_requested"]
            });
        assert_eq!(r["function"]["pass"], want, "{cand}");
        if want {
            let shaped = raw_rows.iter().find(|x| x["mode"] == "shaped").unwrap();
            assert_eq!(r["function"]["steady_cycles_per_op"], shaped["steady_cycles_per_op"], "{cand}");
        } else {
            assert!(r["function"]["steady_cycles_per_op"].is_null(), "{cand}: no throughput for a failed gate");
        }
    }
}

#[test]
fn every_artix7_summary_and_derived_throughput_is_recomputed_from_its_seeds() {
    let j = joined();
    let raw = json("benchmarks/silicon/qor_artix7.json");
    for r in rows(&j) {
        let cand = r["candidate"].as_str().unwrap();
        for mapping in ["fabric", "native"] {
            let seeds: Vec<&Value> = rows(&raw).iter().filter(|x| x["candidate"] == cand && x["mapping"] == mapping).collect();
            let s = &r["artix7"][mapping];
            if seeds.is_empty() {
                assert!(s.is_null(), "{cand} {mapping}: a summary with no seeds");
                continue;
            }
            let routed: Vec<f64> = seeds
                .iter()
                .filter(|x| x["verdict"] == "complete" && !x["fmax_mhz"].is_null())
                .map(|x| x["fmax_mhz"].as_f64().unwrap())
                .collect();
            assert_eq!(s["routed"].as_u64().unwrap() as usize, routed.len(), "{cand} {mapping}");
            assert_eq!(s["refused"].as_array().unwrap().len(), seeds.iter().filter(|x| x["verdict"] != "complete").count(), "{cand} {mapping}");
            if routed.is_empty() {
                assert!(s["fmax_mhz"].is_null(), "{cand} {mapping}");
            } else {
                assert!(close(s["fmax_mhz"]["median"].as_f64().unwrap(), median(routed.clone())), "{cand} {mapping}: median");
                assert!(close(s["fmax_mhz"]["min"].as_f64().unwrap(), routed.iter().cloned().fold(f64::MAX, f64::min)), "{cand}");
                assert!(close(s["fmax_mhz"]["max"].as_f64().unwrap(), routed.iter().cloned().fold(f64::MIN, f64::max)), "{cand}");
            }
            let cells: Vec<&Value> = seeds.iter().map(|x| &x["candidate_cells"]).filter(|c| !c.is_null()).collect();
            if !cells.is_empty() && cells.iter().all(|c| *c == cells[0]) {
                assert_eq!(&s["cells"], cells[0], "{cand} {mapping}: the candidate's cells");
            }
        }
        let fab = &r["artix7"]["fabric"];
        let steady = r["function"]["steady_cycles_per_op"].as_f64();
        match (steady, fab["fmax_mhz"]["median"].as_f64()) {
            (Some(c), Some(f)) if r["function"]["pass"] == true => {
                let mops = f / c;
                let got = r["derived"]["fabric_mresults_per_s"].as_f64().unwrap();
                assert!((got - mops).abs() < 1e-3, "{cand}: {got} results/us, the seeds give {mops}");
            }
            _ => assert!(r["derived"].is_null(), "{cand}: a throughput without a passing gate and a routed fabric seed"),
        }
    }
}

#[test]
fn every_sky130_attempt_own_minimum_and_iso_period_is_recomputed_from_its_runs() {
    let j = joined();
    let stage_path = root().join("benchmarks/silicon/qor_sky130_stage.json");
    let stage: Vec<Value> = if stage_path.exists() {
        json("benchmarks/silicon/qor_sky130_stage.json")["rows"].as_array().cloned().unwrap_or_default()
    } else {
        Vec::new()
    };
    let ledger = |p: &str| if root().join(p).exists() { json(p)["blocks"].clone() } else { Value::Null };
    let (blocks, control_blocks) = (ledger("benchmarks/silicon/qor_sky130.json"), ledger("benchmarks/silicon/qor_sky130_control.json"));
    let role = |s: &Value, r: &str| s["role"] == r;
    let mut own_by: BTreeMap<String, f64> = BTreeMap::new();
    let mut flows_by: BTreeMap<String, std::collections::BTreeSet<String>> = BTreeMap::new();
    let check = |a: &Value, s: &Value, blocks: &Value| {
        assert_eq!(a["period_ns"], s["period_ns"], "{}", a["block"]);
        assert_eq!(a["flow"], s["flow"], "{}: the flow its stage row records", a["block"]);
        assert_eq!(a["floorplan"], s["floorplan"], "{}: the floorplan its run used", a["block"]);
        assert_eq!(a["loosened"], s["loosened"], "{}: each floorplan left before its run's, and why", a["block"]);
        for k in ["flow_wall_s", "flow_peak_mem_gb"] {
            assert_eq!(a[k], s[k], "{}: {k}, what its run cost", a["block"]);
        }
        let rec = &blocks[a["block"].as_str().unwrap()];
        if rec.is_null() || !s["refused"].is_null() {
            assert!(
                a["signoff_clean"].is_null() && a["refused"] == s["refused"] && !a["refused"].is_null(),
                "{}: its flow recorded nothing, so it is refused, whatever an older run of the block left in the ledger",
                a["block"]
            );
            return;
        }
        for k in ["signoff_clean", "setup_worst_slack_ns", "hold_worst_slack_ns", "std_cells", "std_cell_area_um2", "synth_area_um2", "synth_findings"] {
            assert_eq!(a[k], rec[k], "{} {k}", a["block"]);
        }
        assert_eq!(a["candidate_area_um2"], rec["synth_module_areas_um2"][s["candidate_top"].as_str().unwrap()], "{}", a["block"]);
    };
    // The kernel's flow is the one the joined ledger names; an attempt staged through another is superseded.
    let current = |s: &Value| {
        let k = s["kernel"].as_str().unwrap();
        j["flows"]["sky130"][k].as_array().is_some_and(|fs| {
            fs.iter().any(|f| serde_json::from_str::<Value>(f.as_str().unwrap()).ok().as_ref() == Some(&s["flow"]))
        })
    };
    for r in rows(&j) {
        let cand = r["candidate"].as_str().unwrap();
        // A row only staged has run no flow (no `flow_rc`), and is no attempt yet.
        let ran = |s: &&Value| s.get("flow_rc").is_some();
        let mine: Vec<&Value> =
            stage.iter().filter(|s| s["candidate"] == cand && role(s, "attempt") && current(s)).filter(ran).collect();
        let controls: Vec<&Value> = stage
            .iter()
            .filter(|s| s["candidate"] == cand && role(s, "control") && s["flow"]["openlane"].is_string())
            .filter(ran)
            .filter(|s| {
                // A control is current when its flow is the kernel's joined flow on OpenLane as released: the same
                // identity, the overlay left out — what the assembler joins, an older control superseded with it.
                let k = s["kernel"].as_str().unwrap();
                j["flows"]["sky130"][k].as_array().is_some_and(|fs| {
                    fs.iter().any(|f| {
                        serde_json::from_str::<Value>(f.as_str().unwrap()).is_ok_and(|mut f| {
                            f["overlay_sha3"] = Value::Null;
                            f == s["flow"]
                        })
                    })
                })
            })
            .collect();
        if mine.is_empty() && controls.is_empty() {
            assert!(r["sky130"].is_null(), "{cand}: no attempt staged, so no Sky130 summary");
            continue;
        }
        let attempts = r["sky130"]["attempts"].as_array().unwrap_or_else(|| panic!("{cand}: its attempts"));
        assert_eq!(attempts.len(), mine.len(), "{cand}: every staged attempt is joined, and no control among them");
        for a in attempts {
            let s = mine.iter().find(|s| s["block"] == a["block"]).unwrap_or_else(|| panic!("{}: staged", a["block"]));
            check(a, s, &blocks);
            assert!(s["flow"]["overlay_sha3"].is_string(), "{}: an attempt runs the overlay", a["block"]);
            flows_by.entry(r["kernel"].as_str().unwrap().to_string()).or_default().insert(
                serde_json::to_string(&s["flow"]).unwrap(),
            );
        }
        let beside = r["sky130"]["controls"].as_array().unwrap_or_else(|| panic!("{cand}: its controls"));
        assert_eq!(beside.len(), controls.len(), "{cand}: every staged control is joined");
        for b in beside {
            let c = &b["control"];
            let s = controls.iter().find(|s| s["block"] == c["block"]).unwrap_or_else(|| panic!("{}: staged", c["block"]));
            check(c, s, &control_blocks);
            assert!(s["flow"]["overlay_sha3"].is_null(), "{}: a control runs OpenLane as released", c["block"]);
            let o = &b["overlay"];
            let same = attempts.iter().find(|a| a["period_ns"] == c["period_ns"]);
            assert_eq!(o["block"], same.map_or(Value::Null, |a| a["block"].clone()), "{}: beside the attempt at its period", c["block"]);
            if let Some(a) = same {
                assert_eq!(s["flow"]["openlane"], a["flow"]["openlane"], "{}: the overlay's own base", c["block"]);
            }
        }
        let own = attempts.iter().filter(|a| a["signoff_clean"] == true).filter_map(|a| a["period_ns"].as_f64()).fold(None, |m: Option<f64>, p| Some(m.map_or(p, |m| m.min(p))));
        assert_eq!(r["sky130"]["own_minimum_ns"].as_f64(), own, "{cand}: the shortest period it closed at");
        if let Some(p) = own {
            let k = r["kernel"].as_str().unwrap().to_string();
            own_by.insert(k.clone(), own_by.get(&k).copied().unwrap_or(0.0).max(p));
        }
    }
    for (k, flows) in &flows_by {
        assert_eq!(flows.len(), 1, "{k}: every attempt of a kernel runs one flow, so its candidates compare: {flows:?}");
        let listed: std::collections::BTreeSet<String> = j["flows"]["sky130"][k]
            .as_array()
            .unwrap_or_else(|| panic!("{k}: flows.sky130"))
            .iter()
            .map(|f| serde_json::to_string(&serde_json::from_str::<Value>(f.as_str().unwrap()).unwrap()).unwrap())
            .collect();
        assert_eq!(&listed, flows, "{k}: the flow the joined ledger names");
    }
    for r in rows(&j).iter().filter(|r| !r["sky130"].is_null()) {
        let iso = own_by.get(r["kernel"].as_str().unwrap()).copied();
        assert_eq!(r["sky130"]["iso_period_ns"].as_f64(), iso, "{}: the kernel's longest own minimum", r["key"]);
        let steady = if r["function"]["pass"] == true { r["function"]["steady_cycles_per_op"].as_f64() } else { None };
        for at in ["at_own_minimum", "at_iso"] {
            let a = &r["sky130"][at];
            if a.is_null() {
                continue;
            }
            let want = match (a["signoff_clean"] == true, steady, a["std_cell_area_um2"].as_f64()) {
                (true, Some(st), Some(area)) if area > 0.0 => {
                    Some(1000.0 / a["period_ns"].as_f64().unwrap() / st / (area / 1e6))
                }
                _ => None,
            };
            match (a["mresults_per_s_per_mm2"].as_f64(), want) {
                (Some(have), Some(want)) => {
                    assert!(close(have, (want * 1e4).round() / 1e4), "{} {at}: {have} vs {want}", r["key"])
                }
                (None, None) => {}
                other => panic!("{} {at}: results per second per mm2 {other:?}", r["key"]),
            }
        }
    }
}

/// The assembler joins an attempt whose last flow recorded nothing as refused, whatever its block's ledger still holds
/// from an earlier flow of the block, and an attempt whose flow recorded with its run and the floorplan it ran in.
/// A row only staged has run no flow and is no attempt. Driven on a fixture: three stage rows of one candidate, two
/// blocks in the ledger, one row refused, one only staged.
#[test]
fn an_attempt_whose_flow_recorded_nothing_joins_as_refused_whatever_its_ledger_holds() {
    let work = tempfile::tempdir().unwrap();
    let out = std::process::Command::new("python3")
        .arg("-c")
        .arg(
            r#"
import importlib.util, json, os, sys
spec = importlib.util.spec_from_file_location("assemble", sys.argv[1])
a = importlib.util.module_from_spec(spec)
spec.loader.exec_module(a)
a.qorlib.flow_settings = lambda: "5" * 64
flow = a.qorlib.flow_identity(False)
fp = [dict(f) for f in a.qorlib.SKY130_FLOORPLANS]
def stage_row(block, period, refused, floorplan, loosened):
    return {"block": block, "candidate": "k1_x", "candidate_top": "decide", "role": "attempt", "period_ns": period,
            "flow": flow, "floorplan": floorplan, "loosened": loosened, "refused": refused, "flow_wall_s": 412.5,
            "flow_peak_mem_gb": 6.25}
rows = [dict(stage_row("qorsyn_k1_x_p10", 10.0, None, fp[1], [dict(fp[0], why="antenna")]), flow_rc=1),
        dict(stage_row("qorsyn_k1_x_p12", 12.0, "OpenLane exited 2 without a recorded run", fp[0], []), flow_rc=2),
        stage_row("qorsyn_k1_x_p14", 14.0, None, None, None)]
a.qorlib.load = lambda kind, default: {"rows": rows} if kind == "sky130_stage" else default
rec = {"signoff_clean": True, "setup_worst_slack_ns": 0.4, "hold_worst_slack_ns": 0.2, "std_cells": 900,
       "std_cell_area_um2": 9000.0, "synth_area_um2": 8000.0, "die_area_mm2": 0.04, "synth_findings": {},
       "synth_module_areas_um2": {"decide": 7000.0}}
ledger = os.path.join(sys.argv[2], "ledger.json")
json.dump({"blocks": {"qorsyn_k1_x_p10": rec, "qorsyn_k1_x_p12": rec}}, open(ledger, "w"))
a.SKY130_OF["attempt"] = ledger
print(json.dumps([[t["block"], t["refused"], t.get("signoff_clean"), t["floorplan"]["fp_core_util"],
                   [[c["fp_core_util"], c["why"]] for c in t["loosened"]], t["flow_peak_mem_gb"]] for t in a.sky130_attempts("k1_x")]))
"#,
        )
        .arg(root().join("scripts/qor/assemble.py"))
        .arg(work.path())
        .output()
        .expect("python3 runs");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        r#"[["qorsyn_k1_x_p10", null, true, 25, [[35, "antenna"]], 6.25], ["qorsyn_k1_x_p12", "OpenLane exited 2 without a recorded run", null, 35, [], 6.25]]"#,
        "the run at 10 ns in the floorplan congestion moved it to; the one at 12 ns refused, its block's older record unread"
    );
}

/// The joined ledger prints the flow settings every Sky130 run of the bench goes through — the config the compiler emits
/// for a bench block, the design's own fields left out — and they are the settings each current run's flow identity
/// hashes, so the report's account of the flow is the flow the runs went through.
#[test]
fn the_joined_ledger_prints_the_flow_settings_its_runs_went_through() {
    let j = joined();
    let settings = &j["flows"]["sky130_settings"];
    assert!(settings.as_object().is_some_and(|o| !o.is_empty()), "flows.sky130_settings: the emitted config");
    for k in ["DESIGN_NAME", "CLOCK_PORT", "CLOCK_PERIOD"] {
        assert!(settings[k].is_null(), "{k} is the design's own, not the flow's");
    }
    let sha = hex(&sha3_256_bytes(canonical(settings).as_bytes()));
    for (k, flows) in j["flows"]["sky130"].as_object().unwrap() {
        for f in flows.as_array().unwrap() {
            let f: Value = serde_json::from_str(f.as_str().unwrap()).unwrap();
            assert_eq!(f["flow_settings_sha3"].as_str(), Some(sha.as_str()), "{k}: its runs went through the printed settings");
        }
    }
}

/// A JSON value as Python's `json.dumps(v, sort_keys=True)` prints it (ASCII text, as the flow's config is).
fn canonical(v: &Value) -> String {
    match v {
        Value::Object(o) => {
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort();
            let fields: Vec<String> =
                keys.iter().map(|k| format!("{}: {}", serde_json::to_string(k).unwrap(), canonical(&o[*k]))).collect();
            format!("{{{}}}", fields.join(", "))
        }
        Value::Array(a) => format!("[{}]", a.iter().map(canonical).collect::<Vec<_>>().join(", ")),
        other => serde_json::to_string(other).unwrap(),
    }
}

/// The candidate's energy per operation from its power row's own figures, in pJ: its cells' and its buffers' power, and
/// the clock tree's in the share of clock sinks that are its own, over the window's length and operations.
fn energy_per_op(row: &Value, corner: &str) -> (f64, f64) {
    let c = &row["corners"][corner];
    let w = |k: &str| c["classes"][k]["total"].as_f64().unwrap_or_else(|| panic!("{} {corner}: class {k}", row["block"]));
    let (dut, harness) = (row["clock_sinks"]["dut"].as_f64().unwrap(), row["clock_sinks"]["harness"].as_f64().unwrap());
    let share = if dut + harness > 0.0 { dut / (dut + harness) } else { 0.0 };
    let per = row["window"]["span_ns"].as_f64().unwrap() * 1e3 / row["window"]["ops"].as_f64().unwrap();
    ((w("dut") + w("dut_buffers") + share * w("clock")) * per, c["total"]["total"].as_f64().unwrap() * per)
}

#[test]
fn every_energy_per_operation_is_its_measured_power_block() {
    let j = joined();
    let present = |p: &str| root().join(p).exists();
    let stage: Vec<Value> = if present("benchmarks/silicon/qor_sky130_stage.json") {
        json("benchmarks/silicon/qor_sky130_stage.json")["rows"].as_array().cloned().unwrap_or_default()
    } else {
        Vec::new()
    };
    let raw: Vec<Value> = if present("benchmarks/silicon/qor_power.json") {
        json("benchmarks/silicon/qor_power.json")["rows"].as_array().cloned().unwrap_or_default()
    } else {
        Vec::new()
    };
    let blocks = if present("benchmarks/silicon/qor_sky130_power.json") {
        json("benchmarks/silicon/qor_sky130_power.json")["blocks"].clone()
    } else {
        Value::Null
    };
    for r in rows(&j) {
        let cand = r["candidate"].as_str().unwrap();
        let staged: Vec<&Value> = stage.iter().filter(|s| s["candidate"] == cand && s["role"] == "power").collect();
        let measured: Vec<&Value> = raw.iter().filter(|p| p["candidate"] == cand).collect();
        if staged.is_empty() && measured.is_empty() {
            assert!(r["power"].is_null(), "{cand}: no power block, so no energy");
            continue;
        }
        assert_eq!(r["power"]["blocks"].as_array().map_or(0, |b| b.len()), staged.len(), "{cand}: every power block staged");
        let Some(p) = measured.iter().min_by(|a, b| a["period_ns"].as_f64().partial_cmp(&b["period_ns"].as_f64()).unwrap())
        else {
            assert!(r["power"]["measured"].is_null(), "{cand}: nothing measured");
            continue;
        };
        let m = &r["power"]["measured"];
        assert_eq!(m["block"], p["block"], "{cand}: the measured block at the shortest period");
        assert_eq!(m["evidence_tier"], "GDS-VCD");
        let s = staged.iter().find(|s| s["block"] == p["block"]).unwrap_or_else(|| panic!("{}: staged as power", p["block"]));
        assert_eq!(s["period_ns"], p["period_ns"], "{}: measured at the period it was hardened at", p["block"]);
        assert_eq!(blocks[p["block"].as_str().unwrap()]["signoff_clean"], true, "{}: measured only where it closes", p["block"]);
        let iso = r["sky130"]["iso_period_ns"].as_f64();
        let period = p["period_ns"].as_f64().unwrap();
        if iso != Some(period) {
            let at_iso = staged.iter().find(|s| s["period_ns"].as_f64() == iso);
            let missed = at_iso.is_some_and(|s| blocks[s["block"].as_str().unwrap()]["signoff_clean"] != true);
            assert!(missed && iso.is_some_and(|i| period > i), "{}: off the iso period {iso:?} only after a miss there", p["block"]);
        }
        for (corner, figures) in m["corners"].as_object().expect("corners") {
            let (dut, block) = energy_per_op(p, corner);
            for (k, want) in [("dut_pj", dut), ("block_pj", block)] {
                let have = figures[k].as_f64().unwrap();
                assert!((have - want).abs() <= 1e-6 * want.abs().max(1e-6), "{} {corner} {k}: {have} vs {want}", p["block"]);
            }
            assert_eq!(figures["dut_pj"], p["corners"][corner.as_str()]["energy"]["dut_pj"]);
        }
        assert_eq!(m["corners"].as_object().unwrap().len(), 3, "{cand}: every corner");
        assert!(j["flows"]["power"]["opensta"].as_array().unwrap().contains(&p["opensta"]), "{cand}: the OpenSTA it ran");
    }
}

#[test]
fn every_text_in_the_joined_ledger_is_printable() {
    fn walk(v: &Value, at: &str, out: &mut Vec<String>) {
        match v {
            Value::String(s) => {
                if s.chars().any(|c| (c.is_control() && c != '\n' && c != '\t') || c == '\u{7f}') {
                    out.push(format!("{at}: {s:?}"));
                }
            }
            Value::Array(a) => a.iter().enumerate().for_each(|(i, x)| walk(x, &format!("{at}[{i}]"), out)),
            Value::Object(o) => o.iter().for_each(|(k, x)| walk(x, &format!("{at}.{k}"), out)),
            _ => {}
        }
    }
    let mut bad = Vec::new();
    walk(&joined(), "comparative_synthesis.json", &mut bad);
    assert!(bad.is_empty(), "control characters the report would print: {bad:#?}");
}


fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// `a` is `b` rounded to `digits` decimals, as the assembler rounds.
fn rounded(a: &Value, b: f64, digits: i32) -> bool {
    a.as_f64().is_some_and(|a| (a - b).abs() <= 0.5 * 10f64.powi(-digits) + 1e-12)
}

const STREAMS: [&str; 3] = ["sequential", "scattered", "probe"];

/// The K7 traffic gate in the joined ledger. Every controller it drives passes every stream in its candidate
/// configuration, LiteDRAM's upstream-module runs standing beside as disclosure; each run's verdict is its refusal's
/// absence, and each records its operating point's CAS latency and the probe's read spacing (zero for a stream run as
/// fast as the controller takes it); every gate saw one RTL per controller (the `ddr2_ctrl` and `ddr2_dfi_ctrl` the traffic gate simulated are
/// `k7_ddr2_ctrl_verilog()` and `k7_dfi_ctrl_verilog()`, and each controller's Artix-7 and Sky130 runs staged the RTL
/// its traffic runs did); and
/// every K7 row's traffic summary is its runs re-derived.
#[test]
fn every_k7_traffic_summary_is_its_runs_and_one_rtl_per_controller() {
    let dram = json("benchmarks/silicon/qor_dram.json");
    let runs = rows(&dram);
    for r in runs {
        assert_eq!(r["pass"].as_bool(), Some(r["refusal"].is_null()), "{}: a verdict is its refusal's absence", r["key"]);
        if r["pass"] == true {
            for m in ["stream_cycles", "payload_cycles", "bus_span", "commands", "read_service", "read_bus"] {
                assert!(!r[m].is_null(), "{}: a passing run has its {m}", r["key"]);
            }
        }
        for f in ["cl", "every"] {
            assert!(r[f].is_u64(), "{}: the run records its {f}", r["key"]);
        }
        assert_eq!(r["every"].as_u64() == Some(0), r["pattern"] != "probe", "{}: only the probe spaces its reads", r["key"]);
    }
    for c in K7_CONTROLLERS {
        for stream in STREAMS {
            let run = runs
                .iter()
                .find(|r| r["candidate"] == c.name && r["pattern"] == stream && r["module"] != "upstream")
                .unwrap_or_else(|| panic!("{}: no {stream} run", c.name));
            assert_eq!(run["pass"], true, "{} {stream}: {}", c.name, run["refusal"]);
            assert_eq!(run["bl"].as_u64(), Some(c.bl), "{} {stream}", c.name);
        }
    }
    for stream in STREAMS {
        assert!(
            runs.iter().any(|r| r["candidate"] == "k7_litedram_ctrl" && r["module"] == "upstream" && r["pattern"] == stream),
            "LiteDRAM's upstream-module {stream} run stands beside"
        );
    }

    let mut rtl: BTreeMap<String, String> = BTreeMap::new();
    rtl.insert("k7_logos_ddr2_ctrl".into(), hex(&sha3_256_bytes(k7_ddr2_ctrl_verilog().as_bytes())));
    rtl.insert("k7_logos_dfi_ctrl".into(), hex(&sha3_256_bytes(k7_dfi_ctrl_verilog().as_bytes())));
    for r in runs.iter().filter(|r| r["module"] != "upstream") {
        let name = r["candidate"].as_str().unwrap().to_string();
        let sha = r["rtl_sha3_256"].as_str().unwrap().to_string();
        let want = rtl.entry(name.clone()).or_insert_with(|| sha.clone());
        assert_eq!(*want, sha, "{}: the traffic gate drove one RTL of {name}", r["key"]);
    }
    let mut staged: Vec<Value> = rows(&json("benchmarks/silicon/qor_artix7.json")).clone();
    if root().join("benchmarks/silicon/qor_sky130_stage.json").exists() {
        staged.extend(rows(&json("benchmarks/silicon/qor_sky130_stage.json")).iter().cloned());
    }
    for r in staged.iter().filter(|r| r["kernel"] == "K7") {
        if let Some(want) = rtl.get(r["candidate"].as_str().unwrap()) {
            assert_eq!(r["rtl_sha3_256"].as_str(), Some(want.as_str()), "{}: staged another RTL than the traffic gate drove", r["key"]);
        }
    }

    let joined = joined();
    for row in rows(&joined).iter().filter(|r| r["kernel"] == "K7") {
        let mine: Vec<&Value> = runs.iter().filter(|d| d["candidate"] == row["candidate"]).collect();
        if mine.is_empty() {
            assert!(row["dram"].is_null(), "{}: a summary without runs", row["key"]);
            continue;
        }
        assert_eq!(row["functional_gate"], "SIM: the K7 traffic gate", "{}", row["key"]);
        let summary = row["dram"].as_object().unwrap_or_else(|| panic!("{}: runs without a summary", row["key"]));
        assert_eq!(summary.len(), mine.len(), "{}: one summary entry a run", row["key"]);
        for d in mine {
            let key = format!("{}{}", d["pattern"].as_str().unwrap(), if d["module"] == "upstream" { "+upstream" } else { "" });
            let s = &summary[&key];
            for f in ["pass", "words", "part", "tck_ps", "bl", "cl", "every", "reproduce"] {
                assert_eq!(s[f], d[f], "{} {key}: {f}", row["key"]);
            }
            assert_eq!(s["evidence_tier"], "SIM");
            if d["pass"] != true {
                assert_eq!(s["refusal"], d["refusal"], "{} {key}", row["key"]);
                continue;
            }
            for f in ["stream_cycles", "commands", "write_latency", "read_latency", "read_service", "read_bus"] {
                assert_eq!(s[f], d[f], "{} {key}: {f}", row["key"]);
            }
            let cycles = d["stream_cycles"].as_f64().unwrap();
            let ns = cycles * d["tck_ps"].as_f64().unwrap() / 1000.0;
            let words = d["words"].as_f64().unwrap();
            let cas = d["commands"]["read"].as_f64().unwrap() + d["commands"]["write"].as_f64().unwrap();
            assert!(rounded(&s["stream_us"], ns / 1000.0, 3), "{} {key}: stream_us", row["key"]);
            assert!(rounded(&s["payload_share"], d["payload_cycles"].as_f64().unwrap() / cycles, 4), "{} {key}", row["key"]);
            assert!(rounded(&s["mbytes_per_s"], 2.0 * words * 8.0 / ns * 1000.0, 2), "{} {key}: mbytes_per_s", row["key"]);
            assert!(rounded(&s["row_hit_share"], d["row_hits"].as_f64().unwrap() / cas, 4), "{} {key}", row["key"]);
            match row["artix7"]["fabric"]["cells"]["lut"].as_f64() {
                Some(lut) => assert!(
                    rounded(&s["mbytes_per_s_per_klut"], s["mbytes_per_s"].as_f64().unwrap() / (lut / 1000.0), 2),
                    "{} {key}: MB/s per kLUT",
                    row["key"]
                ),
                None => assert!(s.get("mbytes_per_s_per_klut").is_none(), "{} {key}: no LUTs, no per-LUT figure", row["key"]),
            }
        }
    }
}

/// The joined ledger names the place-and-route tool its Artix-7 runs went through as exactly as the runs record it:
/// every nextpnr version string, every patch set, every binary (its SHA3-256, recorded since the binary was rebuilt
/// in place) and how many runs predate that record — so a binary rebuilt under an unchanged version string is never
/// hidden behind it.
#[test]
fn the_joined_ledger_names_every_nextpnr_binary_its_artix_runs_used() {
    let raw = json("benchmarks/silicon/qor_artix7.json");
    let runs = rows(&raw);
    let set = |field: &str| -> Vec<String> {
        let mut v: Vec<String> = runs.iter().filter_map(|r| r[field].as_str().map(str::to_string)).collect();
        v.sort();
        v.dedup();
        v
    };
    let flows = &joined()["flows"]["artix7"];
    for field in ["nextpnr", "nextpnr_patches_sha3", "nextpnr_sha3"] {
        let got: Vec<String> = flows[field].as_array().unwrap_or_else(|| panic!("flows.artix7.{field}")).iter().map(|v| v.as_str().unwrap().to_string()).collect();
        assert_eq!(got, set(field), "flows.artix7.{field}");
    }
    let unrecorded = runs.iter().filter(|r| r["nextpnr_sha3"].is_null()).count();
    assert_eq!(flows["nextpnr_sha3_unrecorded"].as_u64(), Some(unrecorded as u64), "runs that predate the binary's record");
}

/// The Artix-7 flow record, on runs that record their binary and runs that predate the record: every binary named
/// once, the runs without one counted, the version strings and patch sets as the runs give them; a run records its
/// RTL file by file or, a fold design's, as one SHA3-256.
#[test]
fn the_artix7_flow_record_counts_the_runs_that_predate_the_binary_record() {
    let out = std::process::Command::new("python3")
        .arg("-c")
        .arg(
            r#"
import importlib.util, json, sys
spec = importlib.util.spec_from_file_location("assemble", sys.argv[1])
a = importlib.util.module_from_spec(spec)
spec.loader.exec_module(a)
run = lambda version, binary, rtl=True: dict({"part": "xc7a100t", "yosys": "Yosys 0.33", "nextpnr": version, "nextpnr_patches_sha3": "p"},
                                   **({"nextpnr_sha3": binary} if binary else {}), **({"rtl_files": []} if rtl else {}))
runs = [run("v1", None, False), run("v1", "bb"), run("v1", "aa"), run("v2", "bb", False), run("v1", None, False),
        dict(run("v1", "aa", False), rtl_sha3_256="f")]
print(json.dumps(a.artix7_flow(runs), sort_keys=True))
"#,
        )
        .arg(root().join("scripts/qor/assemble.py"))
        .output()
        .expect("python3 runs");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        r#"{"nextpnr": ["v1", "v2"], "nextpnr_patches_sha3": ["p"], "nextpnr_sha3": ["aa", "bb"], "nextpnr_sha3_unrecorded": 2, "part": ["xc7a100t"], "rtl_unrecorded": 3, "yosys": ["Yosys 0.33"]}"#
    );
}

/// A K7 summary carries each run's operating point — the part, its clock, burst and CAS latency, the probe's read
/// spacing — keyed by stream, the upstream-module run beside; a refused run keeps it with its refusal.
#[test]
fn a_k7_summary_carries_each_runs_operating_point() {
    let out = std::process::Command::new("python3")
        .arg("-c")
        .arg(
            r#"
import importlib.util, json, sys
spec = importlib.util.spec_from_file_location("assemble", sys.argv[1])
a = importlib.util.module_from_spec(spec)
spec.loader.exec_module(a)
lat = {"min": 1, "max": 2}
def run(pattern, every, module=None, ok=True):
    r = {"key": "c/%s" % pattern + ("+u" if module else ""), "pattern": pattern, "module": module, "pass": ok, "words": 4,
         "part": "MT47H64M16-25", "tck_ps": 5000, "bl": 4, "cl": 3, "every": every, "reproduce": "r"}
    if not ok:
        return dict(r, refusal="tRP")
    return dict(r, stream_cycles=100, payload_cycles=16, row_hits=1, commands={"read": 4, "write": 4},
                write_latency=lat, read_latency=lat, read_service=lat, read_bus=lat)
s = a.dram_summary([run("sequential", 0), run("probe", 128), run("scattered", 0, "upstream", False)])
print(json.dumps({k: [v["cl"], v["every"], v["tck_ps"], v["bl"], v["part"], v["pass"]] for k, v in s.items()}, sort_keys=True))
"#,
        )
        .arg(root().join("scripts/qor/assemble.py"))
        .output()
        .expect("python3 runs");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout).trim(),
        r#"{"probe": [3, 128, 5000, 4, "MT47H64M16-25", true], "scattered+upstream": [3, 0, 5000, 4, "MT47H64M16-25", false], "sequential": [3, 0, 5000, 4, "MT47H64M16-25", true]}"#
    );
}

/// The router ladder held on every Artix-7 run that names its router: no run stands refused because the default router
/// (router2) could not finish routing — such a run was run again with the older router (router1), and that row says
/// what router2 did. Runs recorded before the ladder name no router.
#[test]
fn no_artix_run_stands_on_a_routing_timeout_of_the_default_router() {
    let raw = json("benchmarks/silicon/qor_artix7.json");
    let mut laddered = 0;
    for r in rows(&raw) {
        let verdict = r["verdict"].as_str().unwrap();
        match r["router"].as_str() {
            None => {}
            Some("router2") => {
                laddered += 1;
                assert!(!verdict.starts_with("refused: routing did not finish"), "{}: router2's routing timeout stands", r["key"]);
                assert!(r["router_ladder"].is_null(), "{}: a first run has no ladder", r["key"]);
            }
            Some("router1") => {
                laddered += 1;
                let first = &r["router_ladder"][0];
                assert_eq!(first["router"], "router2", "{}", r["key"]);
                assert!(first["verdict"].as_str().unwrap().starts_with("refused: routing did not finish"), "{}: router1 runs only after router2's routing timeout", r["key"]);
                assert!(r["flow"].as_str().unwrap().ends_with("--router router1"), "{}: the flow names its router", r["key"]);
                assert!(r["excerpts"]["pnr-router2.txt"].is_string(), "{}: router2's log kept", r["key"]);
            }
            Some(other) => panic!("{}: router {other}", r["key"]),
        }
    }
    assert!(laddered > 0, "runs recorded on the ladder");
}

/// Every `benchmarks/silicon/….json` the bench's scripts name, as a literal path or joined onto `SIL`: the raw ledgers
/// each recorder is the only writer of, the joined ledger, the manifest and the kernels' records. A name built at run
/// time (`qor_%s.json`) is named again in its writer's docstring.
fn ledgers_the_bench_names() -> BTreeSet<String> {
    let scripts = root().join("scripts/qor");
    let mut names = BTreeSet::new();
    for e in std::fs::read_dir(&scripts).unwrap().flatten() {
        if e.path().extension().map_or(true, |x| x != "py") {
            continue;
        }
        let text = std::fs::read_to_string(e.path()).unwrap();
        for (at, _) in text.match_indices("benchmarks/silicon/") {
            let path: String = text[at..]
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric() || "_./-%{}*".contains(*c))
                .collect();
            names.insert(path.trim_end_matches('.').to_string());
        }
        for (at, _) in text.match_indices("SIL, ") {
            let args = &text[at + "SIL, ".len()..];
            let args = &args[..args.find(')').unwrap_or(args.len())];
            let parts: Vec<&str> = args.split(',').map(str::trim).collect();
            if parts.iter().all(|p| p.len() >= 2 && p.starts_with('"') && p.ends_with('"')) {
                let rel: Vec<&str> = parts.iter().map(|p| &p[1..p.len() - 1]).collect();
                names.insert(format!("benchmarks/silicon/{}", rel.join("/")));
            }
        }
    }
    names.retain(|n| n.ends_with(".json") && !n.contains(['%', '{', '}', '*']));
    names
}

/// The bench's ledgers are public: every one a bench script names, written yet or not, is one a clone carries, so what
/// this lock re-derives is re-derivable from a clone and a ledger's first run lands in a tracked file.
#[test]
fn every_ledger_the_bench_names_is_public() {
    let names: Vec<String> = ledgers_the_bench_names().into_iter().collect();
    for known in ["comparative_synthesis", "qor_function", "qor_artix7", "qor_dram", "qor_sky130", "qor_sky130_power"] {
        assert!(names.contains(&format!("benchmarks/silicon/{known}.json")), "the census finds {known}.json: {names:?}");
    }
    for joined in ["qor/candidates.json", "qor/kernels/K7/kernel.json"] {
        assert!(names.contains(&format!("benchmarks/silicon/{joined}")), "the census reads paths joined onto SIL: {names:?}");
    }
    let ignored: Vec<&String> =
        names.iter().zip(public_manifest::git_ignored(&names)).filter(|(_, i)| *i).map(|(n, _)| n).collect();
    assert!(ignored.is_empty(), "bench ledgers a clone would not carry (negate them in .gitignore): {ignored:?}");
}

/// Each candidate's RTL file by file, as the bench builds it now: a LOGOS candidate's as the compiler emits it, a
/// third-party candidate's as the manifest holds it; name to SHA3-256.
fn rtl_the_bench_builds() -> BTreeMap<String, BTreeMap<String, String>> {
    let mut want = BTreeMap::new();
    for k in kernels().into_iter().filter(|k| k.id != "K7") {
        for (c, files) in logos_candidates(k.id).unwrap() {
            want.insert(c.name, files.iter().map(|(n, v)| (n.clone(), hex(&sha3_256_bytes(v.as_bytes())))).collect());
        }
    }
    for e in read_manifest(&root()).unwrap().candidates {
        let files = e.files.iter().map(|f| (f.path.rsplit('/').next().unwrap().to_string(), f.sha3_256.clone())).collect();
        want.insert(e.candidate.name, files);
    }
    want
}

/// SHA3-256 of nothing: what a stage's stand-in for an init file no tool emitted holds.
const EMPTY_SHA3: &str = "a7ffc6f8bf1ed76651c14756a061d662f580ff4de43b49fa82d80a4b80f8434a";

/// `files` as name to SHA3-256, each entry's name read from `name` (a run's record) or `from` (a Sky130 stage row,
/// whose staged name may differ), the stage's own wrapper left out, and its stand-ins (empty files for init files a
/// template defaults to and no tool emitted), each held to be empty.
fn rtl_named(files: &Value, field: &str, wrapper: Option<&str>) -> BTreeMap<String, String> {
    files
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| wrapper.map_or(true, |w| f["name"].as_str() != Some(w)))
        .filter(|f| {
            if f["stand_in"].is_string() {
                assert_eq!(f["sha3_256"].as_str(), Some(EMPTY_SHA3), "{}: a stand-in is empty", f["name"]);
                return false;
            }
            true
        })
        .map(|f| (f[field].as_str().unwrap().to_string(), f["sha3_256"].as_str().unwrap().to_string()))
        .collect()
}

/// Every run measured the RTL the bench builds now: each functional run's candidate files, each Artix-7 run's that
/// records them, and each Sky130 stage row through a flow the joined ledger names, by name and SHA3-256 — so no figure
/// stands for RTL the compiler or the tools no longer produce. Every functional run records them; the Artix-7 runs that
/// predate the record are the joined flow record's count.
#[test]
fn every_run_measured_the_rtl_the_bench_builds() {
    let want = rtl_the_bench_builds();
    let expect = |kind: &str, key: &str, candidate: &str, got: BTreeMap<String, String>| {
        let w = want.get(candidate).unwrap_or_else(|| panic!("{kind} {key}: {candidate} is no candidate the bench builds"));
        assert_eq!(&got, w, "{kind} {key}: measured other RTL than the bench builds");
    };
    let mut bound = 0;
    for r in rows(&json("benchmarks/silicon/qor_function.json")).iter().filter(|r| r["kernel"] != "K7") {
        let key = r["key"].as_str().unwrap();
        let files = r.get("rtl_files").unwrap_or_else(|| panic!("function {key}: no record of the RTL it ran"));
        expect("function", key, r["candidate"].as_str().unwrap(), rtl_named(files, "name", None));
        bound += 1;
    }
    let mut unrecorded = 0;
    for r in rows(&json("benchmarks/silicon/qor_artix7.json")).iter().filter(|r| r["kernel"] != "K7") {
        match r.get("rtl_files") {
            Some(files) => {
                expect("artix7", r["key"].as_str().unwrap(), r["candidate"].as_str().unwrap(), rtl_named(files, "name", None));
                bound += 1;
            }
            None => unrecorded += 1,
        }
    }
    assert_eq!(
        Some(unrecorded as u64),
        joined()["flows"]["artix7"]["rtl_unrecorded"].as_u64(),
        "the flow record counts the Artix-7 runs that predate the RTL record"
    );
    // A stage row through a flow the joined ledger no longer names is superseded: history, not a figure.
    let j = joined();
    let current = |s: &Value| {
        j["flows"]["sky130"][s["kernel"].as_str().unwrap()].as_array().is_some_and(|fs| {
            fs.iter().any(|f| serde_json::from_str::<Value>(f.as_str().unwrap()).ok().as_ref() == Some(&s["flow"]))
        })
    };
    for r in rows(&json("benchmarks/silicon/qor_sky130_stage.json")).iter().filter(|r| r["kernel"] != "K7" && current(r)) {
        let block = r["block"].as_str().unwrap();
        expect("sky130 stage", block, r["candidate"].as_str().unwrap(), rtl_named(&r["files"], "from", Some(&format!("{block}.v"))));
        bound += 1;
    }
    assert!(bound > 0, "no run records its RTL");
}
