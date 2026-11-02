//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! RDU LOOP-LANE LOCK — a counted LOGOS loop lowered onto the SN50 descriptor through the Plasticine
//! lane: metapipeline II under the Tungsten-calibrated PCU model, checkerboard placement + static routing,
//! a re-checkable SAT certificate of the routed body, a VM anchor, and Spatial source for the public chain.
//!
//!  A. `a_counted_loop_lowers_onto_the_sn50_descriptor_with_a_certificate` — `lower_loop_lane` on the
//!     demo `dot.lg`: placed within 1040/1040 PCUs/PMUs, the II report `modeled`, the certificate rechecks
//!     (and a tampered copy does not), its digest binds the JSON summary, the VM anchor agrees three ways.
//!  B. `the_plasticine_target_for_sn50_is_derived_from_the_facts_and_names_its_modeled_fields` — every
//!     number of `PlasticineTarget::sn50_rdu()` carries a provenance row: published ones cite a public
//!     SN50 source, modeled ones say so; `pmu_kib == 432 MiB / 1040`; the DRAM is HBM2E at a modeled rate;
//!     the fp32 roofline is honestly the published bf16 peak. The three paper presets do not move.
//!  C. `the_seven_canonical_loops_lower_and_their_ii_matches_the_calibrated_pcu_rule` — the Plasticine
//!     lane's canonical counted loops on the SN50 descriptor: II = pipeline depth + 1 (7) with a carried
//!     register, 1 without — the rule Tungsten calibrated.
//!  D. `the_loop_lane_spatial_source_passes_pir_in_the_container` — `Logos_rdu_dot` through Spatial → PIR →
//!     plastiroute → Tungsten in the container (tool-gated: a counted `LOGOS_TOOL_SKIP` without docker or
//!     the fetched+built chain; serialized by the `plasticine-chain` nextest group).
//!
//! ★ MUTATION PROBES (each run once, 2026-09-18; the lock went red, the impl was restored byte-identical):
//!   - skip the SAT step (`certify_body` returns `(None, None, Some("skipped"))` before it routes) → A
//!     fails on `report["certificate"]["rechecked"]` (`an ALU-only body ships a re-checked certificate`);
//!   - type `pmu_kib: 256` into `PlasticineTarget::rdu` → B fails on `432 * 1024 / 1040` (256 ≠ 425);
//!   - swap the counted-loop lowering's unit model to `UnitModel::ideal_pipelined()` (register turnaround
//!     0, `plasticine_report::lower_counted_loop`) → C fails on `loop-sum` (II 1 ≠ 7).
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test rdu_loop_lane_lock`
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use std::collections::BTreeMap;
use std::path::PathBuf;

use logicaffeine_compile::codegen_sva::plasticine::PlasticineTarget;
use logicaffeine_compile::codegen_sva::plasticine_report::{
    canonical_workloads, lower_counted_loop, WorkloadKind, PROVENANCE_MODELED,
};
use logicaffeine_compile::compile::vm_outcome;
use logicaffeine_compile::spatial::lower::rdu_lanes::{
    anchor_params, lane_of, lower_loop_lane, RduLane,
};
use logicaffeine_compile::spatial::rdu_chip::{Provenance, RduChip, RduChipFacts};
use serde_json::Value;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn demo(name: &str) -> String {
    let p = repo_root().join("examples/rdu_demo").join(name);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("{}: {e}", p.display()))
}

fn json(s: &str) -> Value {
    serde_json::from_str(s).unwrap_or_else(|e| panic!("json: {e}\n{s}"))
}

fn vm_f(source: &str, fn_name: &str, params: &[i64]) -> i64 {
    let args = params
        .iter()
        .map(|p| {
            if *p < 0 {
                format!("(0 - {})", -p)
            } else {
                p.to_string()
            }
        })
        .collect::<Vec<_>>()
        .join(", ");
    let idx = source.find("## Main").expect("a Main");
    let prog = format!("{}## Main\n    Show {fn_name}({args}).\n", &source[..idx]);
    let out = vm_outcome(&prog);
    assert!(
        out.error.is_none(),
        "VM error for {fn_name}({args}): {:?}",
        out.error
    );
    out.output
        .split_whitespace()
        .next()
        .expect("an integer")
        .parse()
        .expect("i64")
}

// ───────────────────────────── A ─────────────────────────────

#[test]
fn a_counted_loop_lowers_onto_the_sn50_descriptor_with_a_certificate() {
    let src = demo("dot.lg");
    assert_eq!(lane_of(&src), RduLane::Loop);
    let art = lower_loop_lane(&src, RduChip::Sn50).expect("dot.lg lowers on the loop lane");
    assert_eq!(art.plasticine.fn_name, "dot");
    assert_eq!(art.plasticine.target, "sn50-rdu");

    let placement = json(&art.placement_json);
    assert_eq!(placement["target"], "sn50-rdu");
    assert_eq!(
        placement["pcu_total"], 1040,
        "the modeled SN50 mesh: 1040 PCUs"
    );
    assert_eq!(
        placement["pmu_total"], 1040,
        "the modeled SN50 mesh: 1040 PMUs"
    );
    assert_eq!(placement["routed_statically"], true);
    let pcus_used: u64 = placement["alloc"]["stages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["pcus"].as_u64().unwrap())
        .sum();
    assert!(
        pcus_used >= 1 && pcus_used <= 1040,
        "placed within the mesh: {pcus_used}"
    );

    let report = json(&art.report_json);
    assert_eq!(report["provenance"], PROVENANCE_MODELED);
    assert_eq!(report["kind"], "counted_loop");
    assert_eq!(report["target"], "sn50-rdu");
    assert_eq!(report["pcu_total"], 1040);
    assert_eq!(report["pmu_total"], 1040);
    assert_eq!(
        report["pipeline_ii"], 7,
        "`s` is loop-carried through the six-stage PCU: pipeline depth + 1 (Tungsten-calibrated)"
    );
    assert_eq!(
        report["certificate"]["rechecked"], true,
        "an ALU-only body ships a re-checked certificate"
    );
    assert!(report["certificate_omitted"].is_null());
    let anchor = &report["vm_anchor"];
    let params = anchor_params(1);
    assert_eq!(anchor["params"], serde_json::json!(params));
    assert_eq!(anchor["hazards_at_ii"], 0);
    assert_eq!(
        anchor["vm"], anchor["fabric"],
        "the VM's answer is the fabric's"
    );
    assert_eq!(
        anchor["vm"], anchor["pipelined_at_ii"],
        "…and the cycle-timed pipelined run's at the modeled II"
    );
    assert_eq!(
        anchor["vm"].as_i64().unwrap(),
        vm_f(&src, "dot", &params),
        "the anchor is the VM at the sample inputs"
    );

    let cert = json(&art.certificate_json);
    assert_eq!(cert["kind"], "sat-placement");
    assert_eq!(cert["rechecked"], true);
    assert!(cert["steps"].as_u64().unwrap() > 0 && cert["vars"].as_u64().unwrap() > 0);
    assert_eq!(
        cert["width"], 4,
        "the bounded fragment the routed body is proven at"
    );
    let pc = art
        .plasticine
        .certificate
        .as_ref()
        .expect("the artifact carries the certificate object");
    assert!(
        pc.recheck(),
        "the certificate rechecks independently of the placer"
    );
    assert!(
        !pc.with_last_step_removed().recheck(),
        "a tampered certificate is refused"
    );
    assert_eq!(
        cert["digest"],
        format!("{:016x}", pc.digest()),
        "the JSON summary is bound to the certificate by its digest"
    );
    assert_ne!(
        pc.digest(),
        pc.with_last_step_removed().digest(),
        "the digest sees a dropped step"
    );

    assert!(
        art.spatial_source
            .contains("@spatial object Logos_dot extends SpatialApp"),
        "Spatial source for the loop:\n{}",
        art.spatial_source
    );
    assert!(art
        .plasticine
        .spatial_test
        .contains("class Logos_dot extends SpatialTest"));
    for needle in [
        "lane: loop",
        "sn50-rdu",
        "II 7",
        "certificate",
        "1040",
        "dot",
    ] {
        assert!(
            art.pretty.contains(needle),
            "the pretty text names {needle:?}:\n{}",
            art.pretty
        );
    }

    // The SN40L column: the same loop, the same II, the SN40L's own descriptor.
    let art40 = lower_loop_lane(&src, RduChip::Sn40l).expect("dot.lg lowers on the SN40L too");
    assert_eq!(art40.plasticine.target, "sn40l-rdu");
    assert_eq!(json(&art40.report_json)["pipeline_ii"], 7);
    assert_eq!(json(&art40.placement_json)["pcu_total"], 1040);

    // A program that is not a counted loop is refused typed, never modeled wrong.
    let err =
        lower_loop_lane(&demo("fib.lg"), RduChip::Sn50).expect_err("fib.lg is not a counted loop");
    assert!(
        err.contains("counted range loop"),
        "the refusal names what the lane takes: {err}"
    );
}

// ───────────────────────────── B ─────────────────────────────

fn numeric_leaves(v: &Value, path: &mut Vec<String>, out: &mut BTreeMap<String, f64>) {
    match v {
        Value::Object(m) => {
            for (k, c) in m {
                path.push(k.clone());
                numeric_leaves(c, path, out);
                path.pop();
            }
        }
        Value::Array(a) => {
            for (i, c) in a.iter().enumerate() {
                path.push(i.to_string());
                numeric_leaves(c, path, out);
                path.pop();
            }
        }
        Value::Number(n) => {
            out.insert(path.join("."), n.as_f64().unwrap());
        }
        _ => {}
    }
}

#[test]
fn the_plasticine_target_for_sn50_is_derived_from_the_facts_and_names_its_modeled_fields() {
    let t = PlasticineTarget::sn50_rdu();
    let facts = RduChipFacts::sn50();
    let um = &facts.unit_model;
    assert_eq!(t.name, "sn50-rdu");
    assert_eq!(
        t.base,
        facts.fabric_target(),
        "the per-PCU envelope IS the facts' fabric target (sn50_shaped)"
    );
    assert_eq!(
        t.pcus(),
        facts.pcu_count.value,
        "the checkerboard holds the modeled PCU count"
    );
    assert_eq!(
        t.pmus(),
        facts.pmu_count.value,
        "…and the modeled PMU count"
    );
    assert_eq!(
        (t.pcu_rows, t.pcu_cols),
        (2 * facts.mesh().y, facts.mesh().x),
        "52x40 units: the 40x26 PCU mesh interleaved 1:1 with PMUs"
    );
    assert_eq!(
        (t.pcu_lanes, t.pcu_stages),
        (um.pcu_lanes.value, um.pcu_stages.value)
    );
    assert_eq!(
        (t.pmu_banks, t.pmu_stages),
        (um.pmu.value.banks, um.pmu.value.stages)
    );
    assert_eq!(t.pcu_counters, Some(um.counter_nest.value));
    assert_eq!(
        t.pmu_kib,
        432 * 1024 / 1040,
        "the published 432 MiB SRAM spread over the modeled 1040 PMUs"
    );
    assert_eq!(t.scratchpad_kib(), t.pmu_kib * 1040);
    assert_eq!(t.dram.kind, "HBM2E", "the published memory kind");
    assert!(t.dram.gbps > 0.0 && t.dram.channels >= 1);
    assert_eq!(
        t.peak_fp32_tflops, facts.peak_bf16_tflops.value as f64,
        "fp32 is not published: the bf16 peak stands as the roofline, and the cite says so"
    );
    assert_eq!(
        t.power_w, None,
        "power is not published — None, never a guess"
    );
    assert!(
        t.cite.contains("modeled") && t.cite.contains("sambanova.ai"),
        "the cite names the model AND the public source: {}",
        t.cite
    );

    // Reflection: every numeric field of the mesh descriptor (the wrapped envelope is pinned by
    // sn50_target_lock) has a provenance row whose value matches, and every row says which it is.
    let mut leaves = BTreeMap::new();
    numeric_leaves(
        &serde_json::to_value(&t).unwrap(),
        &mut Vec::new(),
        &mut leaves,
    );
    leaves.retain(|k, _| !k.starts_with("base."));
    let rows: BTreeMap<&str, _> = PlasticineTarget::rdu_provenance(RduChip::Sn50)
        .into_iter()
        .collect();
    for (path, value) in &leaves {
        let row = rows
            .get(path.as_str())
            .unwrap_or_else(|| panic!("{path} = {value} has no provenance row"));
        assert_eq!(
            row.value, *value,
            "{path}: the provenance row carries the descriptor's value"
        );
        match row.provenance {
            Provenance::Published => assert!(
                facts.cites.iter().any(|c| row.cite.contains(c)),
                "{path}: a published number cites a public SN50 source: {}",
                row.cite
            ),
            Provenance::Modeled => assert!(
                row.cite.contains("not public") || row.cite.contains("modeled"),
                "{path}: a modeled number says so: {}",
                row.cite
            ),
            Provenance::Derived | Provenance::Measured => panic!(
                "{path} is {:?}: every number on the RDU mesh descriptor is published or modeled",
                row.provenance
            ),
        }
    }
    for path in rows.keys() {
        assert!(
            leaves.contains_key(*path),
            "provenance row {path} names no numeric field of the descriptor"
        );
    }
    let modeled: Vec<&str> = rows
        .iter()
        .filter(|(_, r)| r.provenance == Provenance::Modeled)
        .map(|(k, _)| *k)
        .collect();
    for must in [
        "pcu_rows",
        "pcu_cols",
        "pcu_lanes",
        "pcu_stages",
        "pcu_regs_per_stage",
        "pmu_banks",
        "pmu_stages",
        "pmu_kib",
        "clock_ghz",
        "peak_fp32_tflops",
        "dram.gbps",
        "dram.channels",
        "area_mm2",
    ] {
        assert!(
            modeled.contains(&must),
            "{must} is not public and must be labeled modeled; modeled = {modeled:?}"
        );
    }
    assert!(
        rows["peak_fp32_tflops"].cite.contains("bf16"),
        "the fp32 roofline's cite says it is the bf16 peak: {}",
        rows["peak_fp32_tflops"].cite
    );

    // The SN40L twin reads the SN40L facts.
    let t40 = PlasticineTarget::sn40l_rdu();
    let f40 = RduChipFacts::sn40l();
    assert_eq!(t40.name, "sn40l-rdu");
    assert_eq!(t40.base, f40.fabric_target());
    assert_eq!(t40.pmu_kib, 520 * 1024 / 1040);
    assert_eq!(t40.peak_fp32_tflops, 638.0);
    assert_eq!(t40.pcus(), 1040);
    let rows40: BTreeMap<&str, _> = PlasticineTarget::rdu_provenance(RduChip::Sn40l)
        .into_iter()
        .collect();
    assert_eq!(
        rows40.keys().collect::<Vec<_>>(),
        rows.keys().collect::<Vec<_>>(),
        "both chips carry the same provenance ledger shape"
    );

    // The three paper presets do not move.
    let a = PlasticineTarget::isca17();
    assert_eq!(
        (a.pcu_rows, a.pcu_cols, a.pmu_kib, a.pcu_lanes, a.pcu_stages),
        (16, 8, 256, 16, 6)
    );
    assert!((a.dram.gbps - 51.2).abs() < 1e-9 && (a.peak_fp32_tflops - 12.3).abs() < 1e-9);
    let b = PlasticineTarget::sara_20x20();
    assert_eq!((b.pcu_rows, b.pcu_cols, b.pmu_stages), (20, 20, 10));
    assert!((b.dram.gbps - 1000.0).abs() < 1e-9);
    let c = PlasticineTarget::tungsten_20x20_ddr4();
    assert!((c.dram.gbps - 86.6).abs() < 1e-9 && c.name == "plasticine-tungsten-20x20-ddr4");
}

// ───────────────────────────── C ─────────────────────────────

#[test]
fn the_seven_canonical_loops_lower_and_their_ii_matches_the_calibrated_pcu_rule() {
    let t = PlasticineTarget::sn50_rdu();
    let mut seen = 0;
    for w in canonical_workloads() {
        let WorkloadKind::CountedLoop {
            source,
            fn_name,
            params,
        } = &w.kind
        else {
            continue;
        };
        seen += 1;
        let art = lower_counted_loop(source, fn_name, params, &t)
            .unwrap_or_else(|e| panic!("{}: {e}", w.name));
        let row = &art.row;
        assert_eq!(row.target, "sn50-rdu");
        assert_eq!(row.provenance, PROVENANCE_MODELED);
        assert_eq!((row.pcu_total, row.pmu_total), (1040, 1040), "{}", w.name);
        let carried = row.stages.iter().any(|s| s.recurrence_ii > 0);
        let expected = if carried { t.pcu_stages + 1 } else { 1 };
        assert_eq!(
            row.pipeline_ii, expected,
            "{}: II = pipeline depth + 1 with a carried register, 1 without (Tungsten-calibrated)",
            w.name
        );
        assert_eq!(
            w.name == "loop-no-carry",
            !carried,
            "{}: exactly the no-carry loop has no recurrence",
            w.name
        );
        let anchor = row.vm_anchor.as_ref().expect("a VM anchor");
        assert_eq!(
            (anchor.vm, anchor.hazards_at_ii),
            (anchor.fabric, 0),
            "{}",
            w.name
        );
        assert_eq!(anchor.vm, anchor.pipelined_at_ii, "{}", w.name);
        println!(
            "{}: II {} ({}), iterations {}, modeled cycles {}",
            w.name,
            row.pipeline_ii,
            if carried { "carried" } else { "no carry" },
            row.iterations,
            row.modeled_cycles
        );
    }
    assert_eq!(seen, 7, "the seven canonical counted loops");
}

// ───────────────────────────── D ─────────────────────────────

/// The gate: `Logos_rdu_dot` (the loop lane's Spatial regression app for `dot.lg`, its VM anchor as the
/// gold) runs through Spatial → PIR → plastiroute → Tungsten in the container and the harness prints
/// `PASS: true (Logos_rdu_dot)` — the simulated open RDU computed the VM's answer.
#[test]
fn the_loop_lane_spatial_source_passes_pir_in_the_container() {
    if !common::require_tool("docker") {
        return;
    }
    if !common::require_tool("plasticine-toolchain") {
        return;
    }
    let src = demo("dot.lg");
    let art = lower_loop_lane(&src, RduChip::Sn50).expect("dot.lg lowers");
    let app = art
        .plasticine
        .spatial_test
        .replace("Logos_dot", "Logos_rdu_dot");
    assert!(app.contains("class Logos_rdu_dot extends SpatialTest"));
    let regression = common::plasticine::regression_dir();
    std::fs::write(regression.join("Logos_rdu_dot.scala"), &app).expect("write the regression app");
    std::fs::write(regression.join("Logos_rdu_dot.lg"), &src)
        .expect("write the LOGOS source beside it");
    let run = common::plasticine::run_gate("Logos_rdu_dot");
    println!("gate log: {}", run.log_path.display());
    assert!(
        run.passed(),
        "the chain must print the harness's PASS line — log at {}:\n{}",
        run.log_path.display(),
        run.tail(60)
    );
    let cycles = run
        .tungsten_cycles()
        .expect("Tungsten reports `Simulation complete at cycle N`");
    println!("Logos_rdu_dot: PASS, Tungsten cycles {cycles}");
    println!(
        "ledger: python3 scripts/accel/plasticine-gate-ledger.py add --kind counted_loop --app Logos_rdu_dot --gen-dir target/plasticine-toolchain/spatial/gen/Tst_pirTest/Logos_rdu_dot --logos-source examples/rdu_demo/dot.lg --fn dot --params \"{}\" --toolchain \"…\"",
        anchor_params(1).iter().map(|p| p.to_string()).collect::<Vec<_>>().join(" ")
    );
}
