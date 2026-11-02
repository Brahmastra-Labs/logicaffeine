//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! spatial_decode_chip_lock — the decode-chip requirements are what the model computes, and each budget is a
//! boundary: met at it, missed just past it.
//!
//! `benchmarks/silicon/decode/decode_chip.json` states what a system of SRAM-resident LOGOS decode chips must
//! achieve to beat a GPU deployment's floor by 5x per user. A requirement nobody re-derives drifts into a
//! slogan, so the lock regenerates the whole report and holds the committed file to it; then it re-prices every
//! budget at its edge (a stage budget with chip-to-chip hops free, a hop budget at the design's own stage
//! latency) and requires the target met there and missed one percent beyond; then it holds each demonstrated
//! latency to the published rate it was read from; then it holds the node facts to their sources.
//!
//!   cargo nextest run -p logicaffeine-tests --test spatial_decode_chip_lock
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};

use logicaffeine_compile::codegen_sva::fabric_target::Precision;
use logicaffeine_compile::spatial::decode::{price, Chip, DecodeStep, GgufHeader, Machine};
use logicaffeine_compile::spatial::decode_chip::{
    report, DecodeChipDesign, DecodeChipReport, Node, OnChipNetwork, REPORT_MODELS,
};
use logicaffeine_compile::spatial::rdu_chip::Provenance;

fn decode_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/silicon/decode")
}

fn regenerated() -> DecodeChipReport {
    report(&decode_dir()).expect("the report regenerates")
}

fn design_of(node: &str, network: OnChipNetwork) -> DecodeChipDesign {
    let node = match node {
        "n5" => Node::n5(),
        "sky130" => Node::sky130(),
        other => panic!("an unknown node {other}"),
    };
    DecodeChipDesign { network, ..DecodeChipDesign::at(node, 800.0) }
}

#[test]
fn the_committed_report_is_what_the_model_computes() {
    let committed: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(decode_dir().join("decode_chip.json")).expect("the committed report"),
    )
    .expect("it parses");
    let fresh = serde_json::to_value(regenerated()).expect("it serializes");
    if let Some(path) = first_difference(&committed, &fresh, String::new()) {
        panic!(
            "benchmarks/silicon/decode/decode_chip.json is stale at `{path}`: regenerate it with `cargo run -p \
             logicaffeine-compile --example decode_chip_report`"
        );
    }
}

/// The first path where two JSON values part: structure and text exactly, numbers to 1e-12 relative — serde_json
/// parses a float to within an ulp of what it printed, not always to the same bits.
fn first_difference(a: &serde_json::Value, b: &serde_json::Value, at: String) -> Option<String> {
    use serde_json::Value::*;
    match (a, b) {
        (Number(x), Number(y)) => {
            let (x, y) = (x.as_f64()?, y.as_f64()?);
            ((x - y).abs() > 1e-12 * x.abs().max(y.abs())).then_some(at)
        }
        (Array(x), Array(y)) => {
            if x.len() != y.len() {
                return Some(format!("{at} (length {} vs {})", x.len(), y.len()));
            }
            x.iter().zip(y).enumerate().find_map(|(i, (x, y))| first_difference(x, y, format!("{at}/{i}")))
        }
        (Object(x), Object(y)) => {
            if x.len() != y.len() || x.keys().any(|k| !y.contains_key(k)) {
                return Some(format!("{at} (keys)"));
            }
            x.iter().find_map(|(k, v)| first_difference(v, &y[k], format!("{at}/{k}")))
        }
        _ => (a != b).then_some(at),
    }
}

#[test]
fn every_budget_is_met_at_its_edge_and_missed_past_it() {
    let r = regenerated();
    let mut edges = 0;
    for m in &r.models {
        let header = GgufHeader::read_ledger(&decode_dir().join(format!("headers/{}.json", m.model))).unwrap();
        let step = DecodeStep::of(&header, &r.deployment).unwrap();
        for row in &m.requirements {
            let q = &row.requirement;
            let design = design_of(q.node, q.network);
            let time = |stage_s: f64, hop_s: f64| {
                price(&step, &Machine::of(design.chip_with(stage_s, design.link(hop_s)), q.chips), Precision::Fp8)
                    .unwrap()
                    .time_s
                    .unwrap()
            };
            let target = 1.0 / q.target_tok_s;
            if let Some(s) = q.stage_budget_s {
                assert!(time(s, 0.0) <= target * (1.0 + 1e-9), "{} {}: the stage budget misses its own target", m.model, row.baseline);
                assert!(time(s * 1.01, 0.0) > target, "{} {}: one percent past the stage budget still meets the target", m.model, row.baseline);
                edges += 1;
            } else {
                assert!(time(0.0, 0.0) > target, "{} {}: an unreachable stage budget is reachable with free stages", m.model, row.baseline);
            }
            if let Some(h) = q.hop_budget_s {
                assert!(time(q.design_stage_s, h) <= target * (1.0 + 1e-9), "{} {}: the hop budget misses", m.model, row.baseline);
                assert!(time(q.design_stage_s, h * 1.01) > target, "{} {}: one percent past the hop budget still meets", m.model, row.baseline);
                edges += 1;
            } else {
                assert!(time(q.design_stage_s, 0.0) > target, "{} {}: an unreachable hop budget is reachable with free hops", m.model, row.baseline);
            }
            assert!(
                q.floor_tok_s >= q.target_tok_s || q.stage_budget_s.is_none(),
                "{} {}: a reachable target above the system's own floor",
                m.model,
                row.baseline
            );
        }
    }
    assert!(edges >= 40, "the edges checked: {edges}");
}

#[test]
fn the_baselines_are_the_gpu_floors() {
    let r = regenerated();
    for m in &r.models {
        let header = GgufHeader::read_ledger(&decode_dir().join(format!("headers/{}.json", m.model))).unwrap();
        let step = DecodeStep::of(&header, &r.deployment).unwrap();
        for f in &m.gpu_floors {
            let chip = Chip::by_name(f.machine).unwrap();
            let floor = price(&step, &Machine::of(chip, f.chips), Precision::Fp8).unwrap().floor_tokens_per_s();
            assert_eq!(f.floor_tok_s, floor, "{} {} x {}", m.model, f.chips, f.machine);
            for row in m.requirements.iter().filter(|row| row.baseline == format!("{} x {}", f.chips, f.machine)) {
                assert_eq!(row.requirement.target_tok_s, floor * r.speedup, "{} {}", m.model, row.baseline);
            }
        }
        assert_eq!(m.requirements.len(), m.gpu_floors.len() * 3, "{}: every floor against every design", m.model);
    }
    assert_eq!(r.models.iter().map(|m| m.model.as_str()).collect::<Vec<_>>(), REPORT_MODELS);
}

#[test]
fn a_demonstrated_latency_reproduces_its_published_rate() {
    let r = regenerated();
    let mut shown = 0;
    for m in &r.models {
        for d in &m.demonstrated {
            let published: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(decode_dir().join("published.json")).unwrap()).unwrap();
            let row = published["rows"]
                .as_array()
                .unwrap()
                .iter()
                .find(|row| row["label"].as_str() == Some(d.label.as_str()))
                .expect("the published row");
            assert_eq!(row["tok_s"].as_f64(), Some(d.published_tok_s), "{}", d.label);
            let header = GgufHeader::read_ledger(&decode_dir().join(format!("headers/{}.json", row["header"].as_str().unwrap()))).unwrap();
            let mut dep = logicaffeine_compile::spatial::decode::Deployment::as_stored(row["ctx"].as_u64().unwrap());
            dep.weights = logicaffeine_compile::spatial::decode::WeightBytes::PerWeight(row["bytes_per_weight"].as_f64().unwrap());
            let step = DecodeStep::of(&header, &dep).unwrap();
            let floor = price(&step, &Machine::of(Chip::by_name(d.machine).unwrap(), d.chips), Precision::Fp8).unwrap().floor_s;
            let rebuilt = floor + d.stage_latency_s * step.stages.len() as f64;
            assert!((rebuilt - 1.0 / d.published_tok_s).abs() <= 1e-12, "{}: floor + stages x latency is not the published time", d.label);
            shown += 1;
        }
    }
    assert!(shown >= 3, "the demonstrated latencies: {shown}");
}

#[test]
fn a_reduction_tree_never_takes_more_hops_than_a_mesh() {
    for node in [Node::n5(), Node::sky130()] {
        for die in [10.0, 100.0, 800.0] {
            let tree = DecodeChipDesign::at(node.clone(), die);
            let mesh = DecodeChipDesign { network: OnChipNetwork::Mesh, ..tree.clone() };
            assert!(tree.stage_hops() <= mesh.stage_hops(), "{} {die} mm²: tree {} vs mesh {}", node.name, tree.stage_hops(), mesh.stage_hops());
        }
    }
}

#[test]
fn every_node_fact_names_its_source() {
    for node in [Node::n5(), Node::sky130()] {
        for (path, provenance, cite) in node.facts() {
            match provenance {
                Provenance::Published | Provenance::Derived => assert!(
                    node.sources.iter().any(|s| cite.contains(s)),
                    "{} `{path}` cites `{cite}`, naming none of {:?}",
                    node.name,
                    node.sources
                ),
                Provenance::Measured => assert!(cite.contains("benchmarks/") && cite.contains(".json"), "{} `{path}`", node.name),
                Provenance::Modeled => assert!(
                    ["not public", "not published", "inherited", "not tabulated"].iter().any(|m| cite.contains(m)),
                    "{} `{path}` is modeled without saying what it fills",
                    node.name
                ),
            }
        }
    }
    // The sky130 macro facts are read off the PDK's own LEF, where the box has the PDK installed.
    if let Some(lef) = find_lef() {
        let text = std::fs::read_to_string(&lef).unwrap();
        assert!(text.contains("SIZE 683.1 BY 416.54"), "{}: the 2 KiB macro is no longer 683.1 x 416.54 um", lef.display());
    }
}

fn find_lef() -> Option<PathBuf> {
    let versions = PathBuf::from(std::env::var("HOME").ok()?).join(".ciel/ciel/sky130/versions");
    std::fs::read_dir(versions).ok()?.flatten().map(|v| {
        v.path().join("sky130A/libs.ref/sky130_sram_macros/lef/sky130_sram_2kbyte_1rw1r_32x512_8.lef")
    }).find(|p| p.exists())
}

#[test]
fn the_design_points_are_the_ones_the_report_names() {
    let r = regenerated();
    for m in &r.models {
        let designs: Vec<(&str, OnChipNetwork)> = m.requirements.iter().map(|row| (row.requirement.node, row.requirement.network)).collect();
        assert!(designs.contains(&("n5", OnChipNetwork::Tree)) && designs.contains(&("n5", OnChipNetwork::Mesh)) && designs.contains(&("sky130", OnChipNetwork::Tree)), "{}: {designs:?}", m.model);
        for row in &m.requirements {
            let q = &row.requirement;
            let d = design_of(q.node, q.network);
            assert_eq!((q.sram_per_chip_bytes, q.design_stage_s), (d.sram_bytes(), d.stage_latency_s()), "{} {}", m.model, row.baseline);
        }
    }
}
