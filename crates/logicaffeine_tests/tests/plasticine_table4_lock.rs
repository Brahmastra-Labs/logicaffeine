//! plasticine_table4_lock — the Spatial PLDI'18 Table 4 credibility gate is kept as it was stated.
//!
//! `benchmarks/silicon/plasticine_table4.json` is Table 4 itself (with Table 2's data sizes and Table 3's tuned VU9P
//! runtimes), the one hand-typed file of the lane; `benchmarks/silicon/plasticine_table4_gate.json` is written only
//! by `scripts/plasticine/table4-ledger.py` from the public Plasticine chain's own outputs. The lock holds:
//!
//! A. the transcription agrees with itself: each Table 4 speedup is Table 3's tuned VU9P runtime over the Plasticine
//!    runtime, to the rounding of the published digits;
//! B. every point of the search pre-registered in `scripts/plasticine/table4/Table4.scala` has a mapping row or a
//!    refusal saying which pass produced nothing — no point is dropped after the fact;
//! C. every row's comparison with Table 4 (utilization in points, time as a ratio, inside the stated tolerances or
//!    not) is recomputed from its resources and cycles;
//! D. for each app, the mapped point closest to Table 4's utilization is simulated, on the generated DDR4 memory and
//!    on the paper's one DDR3-1600 channel;
//! E. every Tungsten run of a simulated point that finished — the routed network's and the ideal network's, on
//!    either memory — is compared with Table 4 by its recomputed time ratio, and a simulation refused names the
//!    stage that stopped it (Tungsten's own exception where the routed network stopped).
//!
//! Whether the gate passes is the ledger's data, printed by the report; this lock keeps the bookkeeping honest.

use serde_json::Value;
use std::collections::BTreeMap;
use std::path::PathBuf;

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn json(rel: &str) -> Value {
    let text = std::fs::read_to_string(root().join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

fn reference() -> Value {
    json("benchmarks/silicon/plasticine_table4.json")
}

fn gate() -> Value {
    json("benchmarks/silicon/plasticine_table4_gate.json")
}

fn rows(v: &Value) -> &Vec<Value> {
    v["rows"].as_array().expect("rows")
}

fn table4_row<'a>(reference: &'a Value, family: &str) -> &'a Value {
    rows(reference)
        .iter()
        .find(|r| r["abbrev"] == family || r["app"] == family)
        .unwrap_or_else(|| panic!("no Table 4 row for {family}"))
}

fn search() -> Vec<String> {
    let text = std::fs::read_to_string(root().join("scripts/plasticine/table4/Table4.scala")).expect("the search");
    text.lines()
        .filter_map(|l| l.strip_prefix("class "))
        .filter_map(|l| l.split_whitespace().next())
        .filter(|n| n.starts_with("Table4_"))
        .map(str::to_string)
        .collect()
}

fn family(app: &str) -> &str {
    app.trim_start_matches("Table4_").split('_').next().unwrap()
}

#[test]
fn the_transcription_agrees_with_table_3() {
    let r = reference();
    assert_eq!(r["machine"]["pcus"], 64);
    assert_eq!(r["machine"]["pmus"], 64);
    assert_eq!(rows(&r).len(), 5, "Table 4 has five rows");
    for row in rows(&r) {
        let (t, vu9p, x) = (
            row["time_ms"].as_f64().unwrap(),
            row["vu9p_tuned_ms"].as_f64().unwrap(),
            row["speedup_over_vu9p"].as_f64().unwrap(),
        );
        // Each published value is rounded to its last printed digit: half a unit either way.
        let half = |digits: i32| 0.5 * 10f64.powi(-digits);
        let (t_lo, t_hi) = (t - half(2), t + half(2));
        let (v_lo, v_hi) = (vu9p - half(1), vu9p + half(1));
        let (lo, hi) = (v_lo / t_hi - 0.05, v_hi / t_lo + 0.05);
        assert!(lo <= x && x <= hi, "{}: Table 4 says {x}x, Table 3 / Table 4 give {lo:.2}..{hi:.2}", row["app"]);
    }
}

#[test]
fn every_pre_registered_point_has_a_row_or_a_refusal() {
    let g = gate();
    let points = search();
    assert!(points.len() >= 5, "the search covers every Table 4 app");
    for fam in ["GEMM", "BS", "GDA", "KMeans", "TQ6"] {
        assert!(points.iter().any(|p| family(p) == fam), "the search has {fam} points");
    }
    for p in &points {
        let row = rows(&g)
            .iter()
            .find(|r| r["app"] == p.as_str() && r["backend"] == "PIR")
            .unwrap_or_else(|| panic!("{p}: no mapping row and no refusal"));
        if let Some(why) = row["refused"].as_str() {
            assert!(!why.is_empty() && row["refusal_tail"].is_string(), "{p}: a refusal says what produced nothing");
        } else {
            assert!(row["resources"]["PCU"]["total"] == 64 && row["resources"]["PMU"]["total"] == 64, "{p}: mapped onto 16 x 8");
        }
    }
    for r in rows(&g) {
        assert!(points.iter().any(|p| r["app"] == p.as_str()), "{}: a row for a point the search never named", r["key"]);
    }
}

#[test]
fn every_comparison_is_recomputed_from_resources_and_cycles() {
    let (g, refx) = (gate(), reference());
    let tol_u = g["tolerance"]["utilization_points"].as_f64().unwrap();
    let tol_t = g["tolerance"]["time_ratio"].as_f64().unwrap();
    assert_eq!((tol_u, tol_t), (5.0, 0.15), "the tolerances stated before any run");
    for r in rows(&g).iter().filter(|r| r["refused"].is_null()) {
        let t4 = table4_row(&refx, r["family"].as_str().unwrap());
        let res = &r["resources"];
        let points = [
            ("pcu_points", res["PCU"]["usage_pct"].as_f64().unwrap() - t4["pcu_pct"].as_f64().unwrap()),
            ("pmu_points", res["PMU"]["usage_pct"].as_f64().unwrap() - t4["pmu_pct"].as_f64().unwrap()),
            ("ag_points", res["DAG"]["usage_pct"].as_f64().unwrap() - t4["ag_pct"].as_f64().unwrap()),
        ];
        for (k, v) in points {
            assert!((r["table4"][k].as_f64().unwrap() - v).abs() < 0.006, "{}: {k}", r["key"]);
        }
        let within = points.iter().all(|(_, v)| v.abs() <= tol_u);
        assert_eq!(r["table4"]["utilization_within"], within, "{}", r["key"]);
        for mem in ["ddr4", "ddr3"] {
            if let Some(c) = r[format!("sim_{mem}")]["cycles"].as_f64() {
                let ratio = c / 1e6 / t4["time_ms"].as_f64().unwrap();
                assert!((r["table4"][format!("time_ratio_{mem}")].as_f64().unwrap() - ratio).abs() < 1e-4, "{} {mem}", r["key"]);
                assert_eq!(r["table4"][format!("time_within_{mem}")], (ratio - 1.0).abs() <= tol_t, "{} {mem}", r["key"]);
            }
        }
    }
}

#[test]
fn the_closest_mapped_point_of_each_app_is_simulated_on_both_memories() {
    let g = gate();
    let mut best: BTreeMap<String, (f64, String)> = BTreeMap::new();
    for r in rows(&g).iter().filter(|r| r["backend"] == "PIR" && r["refused"].is_null()) {
        let d: f64 = ["pcu_points", "pmu_points", "ag_points"].iter().map(|k| r["table4"][*k].as_f64().unwrap().abs()).sum();
        let fam = r["family"].as_str().unwrap().to_string();
        let app = r["app"].as_str().unwrap().to_string();
        if best.get(&fam).map_or(true, |(b, _)| d < *b) {
            best.insert(fam, (d, app));
        }
    }
    for (fam, (_, app)) in best {
        let sim = rows(&g)
            .iter()
            .find(|r| r["app"] == app.as_str() && r["backend"] == "Tst")
            .unwrap_or_else(|| panic!("{fam}: the closest point {app} is not simulated"));
        if sim["refused"].is_null() {
            assert!(sim["sim_ddr4"]["cycles"].is_u64() && sim["sim_ddr3"]["cycles"].is_u64(), "{app}: both memories");
        }
    }
}

#[test]
fn every_finished_tungsten_run_is_compared_and_a_stopped_one_names_its_stage() {
    let (g, refx) = (gate(), reference());
    let simulated: Vec<&Value> = rows(&g).iter().filter(|r| r["backend"] == "Tst").collect();
    for r in &simulated {
        let t4 = table4_row(&refx, r["family"].as_str().unwrap());
        if let Some(why) = r["refused"].as_str() {
            assert!(
                why.contains(": the routed network stopped on cycle ")
                    || why.contains(" produced no ")
                    || (why.contains(": the simulation finished on cycle ") && why.contains("the app's own check failed")),
                "{}: a refusal names the stage that stopped it: {why}",
                r["key"]
            );
        }
        for key in ["sim_ddr4", "sim_ddr3", "sim_ddr4_ideal", "sim_ddr3_ideal"] {
            let run = &r[key];
            let suffix = &key["sim_".len()..];
            if run["pass"] == true && run["cycles"].is_u64() {
                let ratio = run["cycles"].as_f64().unwrap() / 1e6 / t4["time_ms"].as_f64().unwrap();
                let printed = r["table4"][format!("time_ratio_{suffix}")].as_f64().unwrap_or_else(|| panic!("{}: {key} compared", r["key"]));
                assert!((printed - ratio).abs() < 1e-4, "{} {key}", r["key"]);
                let tol = g["tolerance"]["time_ratio"].as_f64().unwrap();
                assert_eq!(r["table4"][format!("time_within_{suffix}")], (ratio - 1.0).abs() <= tol, "{} {key}", r["key"]);
            } else {
                assert!(r["table4"][format!("time_ratio_{suffix}")].is_null(), "{} {key}: no ratio for a run that did not finish", r["key"]);
            }
        }
    }
}
