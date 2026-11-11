//! samba_exam_fusion_report_lock — the row of the Samba book's exam E8 (section 10.8 of `benchmarks/PREPARE_FOR_SAMBA.md`,
//! the compiler's own quality) on the fusion report: every section of every program a bundle ships lists every fusion
//! class the shared optimizer fired and declined over the statements the section computes.
//!
//! The row emits the SN50 bundle `largo emit rdu` writes for a tiny checkpoint of each shared architecture, and the one it
//! writes for a program whose pointwise ops make a region, and reads each program's `report.json` beside its
//! `neutral.json`. Three things hold the report to the program rather than to itself:
//! - its decisions are the shared optimizer's own on the program's source — regenerated here from the checkpoint, its
//!   identity the one the manifest records, lifted and optimized for the SN50's spatial shim — decision for decision:
//!   rule, class, the statements covered, fired or declined and why;
//! - each section's statements are the ones whose results the section stores, and a section computing none is a layout
//!   bridge, which loads a tensor in one layout and stores it in the other;
//! - each decision is listed in exactly the sections holding a statement it covers, among the fired or the declined as
//!   the optimizer decided, and a fusion that fired is `fired, run apart`, with the sections it runs in, exactly where
//!   some section holds part of its statements and not the rest.
//!
//! The region program goes through `largo emit rdu FILE`, where every named tensor is a result, so each member of its
//! region stores what it makes: the region fires and runs apart, and the report says so.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --cargo-profile test-opt --test samba_exam_fusion_report_lock`

#![cfg(feature = "codegen")]

#[path = "cli_common/mod.rs"]
mod cli;
#[path = "common/tiny_models.rs"]
mod tiny_models;
#[path = "common/samba_handoff.rs"]
#[allow(dead_code)]
mod samba_handoff;

use logicaffeine_compile::model::gguf::Gguf;
use logicaffeine_compile::model::program::{written, Cut};
use logicaffeine_compile::model::quant::GgmlType;
use logicaffeine_compile::spatial::ir::{SpatialProgram, StageKind};
use logicaffeine_compile::spatial::lower::rdu::parse_neutral;
use logicaffeine_compile::spatial::rdu_chip::RduChip;
use logicaffeine_compile::spatial::shim::SpatialShim;
use logicaffeine_compile::spatial::streamed::Storage;
use logicaffeine_compile::tensor_graph::fuse::{optimize, Outcome, Report};
use logicaffeine_compile::tensor_graph::graph::{Storage as Stored, TensorGraph};
use samba_handoff::{Model, CTX};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use tiny_models::Tiny;

/// The architectures every row covers: attention alone, a hybrid with recurrent state, windowed attention, and an
/// expert mixture with sinks.
const SHARED: [Tiny; 4] = [Tiny::Qwen3, Tiny::Qwen35, Tiny::Gemma3, Tiny::GptOss];

/// A program whose pointwise ops make a region the optimizer fuses — `E` joins two of its results — feeding a weighted
/// norm through an addition.
const REGION: &str = "## Main
Let A be a 1 by 64 tensor of Real.
Let B be a 1 by 64 tensor of Real.
Let W be a 1 by 64 tensor of Real.
";

fn region_program() -> String {
    let mut s = String::from(REGION);
    for (name, k) in [("A", 3), ("B", 5), ("W", 7)] {
        for i in 0..64 {
            s += &format!("Set item {} of {name} to {:?}.\n", i + 1, ((i * k + name.len()) % 13) as f64 / 8.0 - 0.75);
        }
    }
    s += "Let C be A plus B.\nLet D be A hadamard B.\nLet E be C hadamard D.\nLet S be E plus A.\nLet N be rmsnorm of S with weight W.\nShow item 1 of N.\n";
    s
}

/// The name a tensor has in either layout: a twin (`X__dense`, `X__rows`) is the matrix it lays out.
fn untwinned(name: &str) -> &str {
    name.strip_suffix("__dense").or_else(|| name.strip_suffix("__rows")).unwrap_or(name)
}

/// The tensors section `k` of `p` loads and stores, by name.
fn moved(p: &SpatialProgram, k: usize) -> (BTreeSet<String>, BTreeSet<String>) {
    let range = match p.sections.get(k) {
        Some(s) => s.stages.0..s.stages.1,
        None => 0..p.stages.len(),
    };
    let (mut loads, mut stores) = (BTreeSet::new(), BTreeSet::new());
    for s in &p.stages[range] {
        match &s.kind {
            StageKind::Load { tensor, .. } => loads.insert(p.tensor(*tensor).name.clone()),
            StageKind::Store { tensor, .. } | StageKind::AllReduce { tensor, .. } => stores.insert(p.tensor(*tensor).name.clone()),
            _ => false,
        };
    }
    (loads, stores)
}

/// The ops `source` lifts — each op's statement and binder — and the shared optimizer's report on them under the SN50's
/// spatial shim, the weights `q8` read as a deployment reads a weight it stores Q8_0: rows of the checkpoint's tensor,
/// whose rows are the program's columns.
fn decided(what: &str, source: &str, q8: &BTreeSet<String>) -> (Vec<(u64, String)>, Report) {
    let stored = |t: &str| if q8.contains(t) { Stored::Rows { format: GgmlType::Q8_0, transposed: true } } else { Stored::Values };
    let lifted = TensorGraph::lift_source(source, &stored).unwrap_or_else(|e| panic!("{what}: the source lifts: {e:?}"));
    let target = RduChip::Sn50.facts().spatial_target();
    let report = optimize(&lifted.graph, &SpatialShim { target: &target }, &[]).report;
    (lifted.program.stages.iter().map(|s| (s.stmt_index as u64, s.out.clone())).collect(), report)
}

/// `report` — an object holding `fusion` and `sections` — held to `program` and to the shared optimizer's decisions on
/// `source`, its weights `q8` read as stored Q8_0: every section listed in order, each with the statements it computes
/// and every decision over them, fired or declined as the optimizer made it. Returns how many decisions fired and how
/// many were declined.
fn covers_every_section(what: &str, program: &SpatialProgram, report: &Value, source: &str, q8: &BTreeSet<String>) -> (usize, usize) {
    let (ops, optimized) = decided(what, source, q8);
    let op_of = |v: &Value| -> usize {
        let key = (v["stmt"].as_u64().expect("a statement index"), v["binder"].as_str().expect("a binder").to_string());
        ops.iter().position(|o| *o == key).unwrap_or_else(|| panic!("{what}: {v} is no statement the source lifts"))
    };

    // The decisions, the optimizer's own.
    let decisions = report["fusion"]["decisions"].as_array().unwrap_or_else(|| panic!("{what}: the report lists its fusion decisions: {}", report["fusion"]));
    let want = &optimized.decisions;
    assert_eq!(decisions.len(), want.len(), "{what}: the report lists every decision the optimizer made, and no other");
    assert_eq!(report["fusion"]["digest"], optimized.digest(), "{what}: the report's digest is the optimizer's report's");
    for (i, (d, w)) in decisions.iter().zip(want).enumerate() {
        assert_eq!(d["decision"], i, "{what}: the decisions in the optimizer's order");
        assert_eq!(d["rule"], w.rule, "{what}: decision {i}'s rule");
        assert_eq!(d["class"], w.class.id(), "{what}: decision {i}'s class");
        let (outcome, why) = match &w.outcome {
            Outcome::Fired => ("fired", None),
            Outcome::Refused(why) => ("refused", Some(why.as_str())),
            Outcome::Held => ("held", None),
        };
        let said = d["outcome"].as_str().unwrap_or_default();
        assert!(said == outcome || (outcome == "fired" && said == "fired, run apart"), "{what}: decision {i} ({} {}) {outcome}, not {said}", w.rule, w.class);
        assert_eq!(d["why"].as_str(), why, "{what}: decision {i} ({} {}) declined for the chip's reason", w.rule, w.class);
        let covers: Vec<usize> = d["covers"].as_array().expect("the statements a decision covers").iter().map(op_of).collect();
        assert_eq!(covers, w.covers, "{what}: decision {i} covers the statements the optimizer's does");
    }

    // A row for every section, in order.
    let names: Vec<String> = if program.sections.is_empty() { vec![program.name.clone()] } else { program.sections.iter().map(|s| s.name.clone()).collect() };
    let rows = report["sections"].as_array().unwrap_or_else(|| panic!("{what}: the report has its sections"));
    assert_eq!(rows.len(), names.len(), "{what}: a row for each of the program's {} sections", names.len());
    let mut holds: Vec<BTreeSet<usize>> = Vec::new();
    for (k, (row, name)) in rows.iter().zip(&names).enumerate() {
        assert_eq!(row["section"], k, "{what}: the rows in the program's order");
        assert_eq!(row["name"], name.as_str(), "{what}: row {k} is section `{name}`");
        let held: BTreeSet<usize> = row["statements"].as_array().unwrap_or_else(|| panic!("{what}: row {k} names its statements")).iter().map(op_of).collect();
        let (loads, stores) = moved(program, k);
        if held.is_empty() {
            let bridged = loads.iter().chain(&stores).all(|t| stores.iter().chain(&loads).any(|u| u != t && untwinned(u) == untwinned(t)));
            assert!(bridged && !stores.is_empty(), "{what}: section {k} `{name}` computes no statement, and is no layout bridge: loads {loads:?}, stores {stores:?}");
        }
        // A result the section stores is a statement's it computes.
        for t in &stores {
            let binding: Vec<usize> = (0..ops.len()).filter(|&o| ops[o].1 == untwinned(t)).collect();
            if !binding.is_empty() && !held.is_empty() {
                assert!(binding.iter().any(|o| held.contains(o)), "{what}: section {k} `{name}` stores `{t}`, and lists none of the statements binding it: {held:?}");
            }
        }
        holds.push(held);
    }
    // Every statement computed in a section that stores its result.
    for (o, (stmt, binder)) in ops.iter().enumerate() {
        let at: Vec<usize> = (0..holds.len()).filter(|&k| holds[k].contains(&o)).collect();
        assert!(!at.is_empty(), "{what}: statement {stmt} `{binder}` is in no section's row");
        assert!(
            at.iter().any(|&k| moved(program, k).1.iter().any(|t| untwinned(t) == binder)),
            "{what}: no section listing statement {stmt} `{binder}` stores it: sections {at:?}"
        );
    }

    // Each decision listed in exactly the sections holding a statement it covers, fired or declined as it was made — a
    // fusion that fired run apart wherever a section holds part of its statements and not the rest.
    let (mut fired, mut declined) = (0, 0);
    for (i, w) in want.iter().enumerate() {
        let reach: Vec<usize> = (0..holds.len()).filter(|&k| w.covers.iter().any(|o| holds[k].contains(o))).collect();
        assert!(!reach.is_empty(), "{what}: decision {i} ({} {}) covers statements no section holds", w.rule, w.class);
        assert_eq!(decisions[i]["sections"], serde_json::json!(reach), "{what}: decision {i} names the sections holding its statements");
        let apart = holds.iter().any(|h| {
            let held = w.covers.iter().filter(|o| h.contains(o)).count();
            held > 0 && held < w.covers.len()
        });
        let outcome = match &w.outcome {
            Outcome::Fired if apart => "fired, run apart",
            Outcome::Fired => "fired",
            Outcome::Refused(_) => "refused",
            Outcome::Held => "held",
        };
        assert_eq!(decisions[i]["outcome"], outcome, "{what}: decision {i} ({} {}) is {outcome}", w.rule, w.class);
        let is_fired = matches!(w.outcome, Outcome::Fired);
        if is_fired {
            fired += 1;
        } else {
            declined += 1;
        }
        for (k, row) in rows.iter().enumerate() {
            let list = |key: &str| -> Vec<&Value> {
                row[key].as_array().unwrap_or_else(|| panic!("{what}: row {k} lists what {key}")).iter().filter(|e| e["decision"] == i).collect()
            };
            let (as_fired, as_declined) = (list("fired"), list("declined"));
            let listed = reach.contains(&k);
            assert_eq!(as_fired.len(), usize::from(listed && is_fired), "{what}: section {k} lists decision {i} ({} {}) among the fired exactly when it holds its statements and it fired", w.rule, w.class);
            assert_eq!(as_declined.len(), usize::from(listed && !is_fired), "{what}: section {k} lists decision {i} ({} {}) among the declined exactly when it holds its statements and it was declined", w.rule, w.class);
            for e in as_fired.iter().chain(&as_declined) {
                assert_eq!((e["rule"].as_str(), e["class"].as_str()), (Some(w.rule), Some(w.class.id())), "{what}: section {k}'s entry for decision {i}");
                assert_eq!(e["outcome"], outcome, "{what}: section {k} lists decision {i} ({} {}) as {outcome}", w.rule, w.class);
                if apart && is_fired {
                    assert_eq!(e["sections"], serde_json::json!(reach), "{what}: section {k} names the sections decision {i} runs apart in");
                }
                if let Outcome::Refused(why) = &w.outcome {
                    assert_eq!((e["outcome"].as_str(), e["why"].as_str()), (Some("refused"), Some(why.as_str())), "{what}: section {k} says why decision {i} was declined");
                }
            }
        }
    }
    (fired, declined)
}

/// What the bundle lowered for its program in `dir`: the cut of the model's program it is — its identity the one the
/// manifest records — and the weights it reads as the checkpoint stores them, Q8_0. The embedding reads none so.
fn lowered_from(m: &Model, gguf: &Gguf, dir: &str, recorded: &Value) -> (String, BTreeSet<String>) {
    let cut = match dir {
        "embed" => Cut::DecodeEmbed,
        "head" => Cut::DecodeHead,
        _ => Cut::DecodeLayer { layer: recorded["representative_layer"].as_u64().expect("a kind's representative") as usize, ctx: CTX },
    };
    let ((source, _, q8w), identity) = written(&m.spec, cut);
    assert_eq!(recorded["program"], identity.line(), "{dir}: the source regenerated is the cut the bundle lowered");
    let stored = Storage::Checkpoint(gguf);
    let q8 = if dir == "embed" { BTreeSet::new() } else { q8w.keys().filter(|v| stored.q8(&m.spec, v)).cloned().collect() };
    (source, q8)
}

/// E8.7 — the fusion report lists every class fired and declined in every section: of every program of each shared
/// architecture's SN50 bundle, and of a program whose region the optimizer fuses.
#[test]
fn the_fusion_report_covers_every_section() {
    let dir = tempfile::tempdir().unwrap();
    for t in SHARED {
        let m = Model::write(t.spec(CTX), &format!("{t:?}"));
        let gguf = Gguf::open(&m.path).expect("the checkpoint");
        let out = dir.path().join(format!("{t:?}"));
        let o = cli::largo_in(
            dir.path(),
            &["emit", "rdu", "--chip", "sn50", "--model", m.path.to_str().unwrap(), "--ctx", &CTX.to_string(), "-o", out.to_str().unwrap()],
        );
        assert_eq!(o.status.code(), Some(0), "{t:?}: largo emit rdu failed:\n{}", cli::stderr(&o));
        let manifest: Value = serde_json::from_str(&std::fs::read_to_string(out.join("manifest.json")).unwrap()).unwrap();
        let mut programs: Vec<(String, Value)> = vec![("embed".into(), manifest["embed"].clone())];
        programs.extend(manifest["kinds"].as_array().expect("kinds").iter().map(|k| (k["dir"].as_str().unwrap().to_string(), k.clone())));
        programs.push(("head".into(), manifest["head"].clone()));
        let mut declined = BTreeMap::new();
        for (sub, recorded) in programs {
            let what = format!("{t:?} {sub}");
            let program = parse_neutral(&std::fs::read_to_string(out.join(&sub).join("neutral.json")).unwrap()).unwrap_or_else(|e| panic!("{what}: {e}"));
            let report: Value = serde_json::from_str(&std::fs::read_to_string(out.join(&sub).join("report.json")).unwrap_or_else(|e| panic!("{what}: report.json: {e}")))
                .unwrap_or_else(|e| panic!("{what}: report.json is JSON: {e}"));
            assert_eq!(report["schema"], "logos.rdu.section_report", "{what}: the report says what it is");
            assert_eq!(report["schedule_digest"], recorded["schedule_digest"], "{what}: the report is on the program the manifest records");
            let (source, q8) = lowered_from(&m, &gguf, &sub, &recorded);
            declined.insert(sub, covers_every_section(&what, &program, &report, &source, &q8).1);
        }
        assert!(declined.values().any(|n| *n > 0), "{t:?}: a model's programs decline some fusion on a spatial chip: {declined:?}");
    }

    let source = region_program();
    let file = dir.path().join("region.lg");
    std::fs::write(&file, &source).unwrap();
    let bundle = dir.path().join("region");
    let o = cli::largo_in(dir.path(), &["emit", "rdu", file.to_str().unwrap(), "-o", bundle.to_str().unwrap()]);
    assert_eq!(o.status.code(), Some(0), "the region program: largo emit rdu failed:\n{}", cli::stderr(&o));
    let program = parse_neutral(&std::fs::read_to_string(bundle.join("neutral.json")).unwrap()).expect("neutral.json parses");
    let report: Value = serde_json::from_str(&std::fs::read_to_string(bundle.join("report.json")).unwrap()).unwrap();
    let (fired, declined) = covers_every_section("the region program", &program, &report, &source, &BTreeSet::new());
    assert!(fired > 0 && declined > 0, "the region program: fusions fired ({fired}) and declined ({declined})");
    let region = report["fusion"]["decisions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|d| d["class"] == "elementwise_regions")
        .expect("the region program's region is decided");
    assert_eq!(region["outcome"], "fired, run apart", "the region fires and, every named tensor a result, runs apart: {region}");
    assert!(region["sections"].as_array().is_some_and(|s| s.len() > 1), "the region runs in sections of its own: {region}");
}
