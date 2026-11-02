//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! THE QWEN3.8 DECODE STEP ON A PLASTICINE FABRIC — every kind of section it has, mapped by PIR and run cycle by
//! cycle in Tungsten, against the oracle.
//!
//! `qwen38_spatial_source_lock` runs the whole step in Spatial's simulator. The FABRIC half of the chain — PIR maps
//! an app onto Plasticine PCUs and PMUs, plastiroute routes it, Tungsten simulates it — cannot hold the whole step:
//! PIR writes an accelerator as one JVM class, and 121 sections are past what the JVM allows one. So the step is cut
//! into SEGMENTS of consecutive sections (`spatial_lang::emit_spatial_segment`), each its own app: entered with the
//! oracle's value of every tensor an earlier section stored, and checked against the oracle's value of every tensor
//! it stores. Four sections map in minutes; all 121 take hours. So the segments run are the fewest 4-section
//! segments that between them hold EVERY KIND of section the step has — rmsnorm, matmul, rope, the conv ring and
//! taps, the three gated-delta sections, the attention scores / softmax / weighted sum, the row gather, the cache
//! append, the arg-max, the pointwise maps.
//!
//! Tungsten runs every mapped app twice: on the IDEAL network (point to point) and on the ROUTED network plastiroute
//! built. The chain runs are RECORDED, like the Tenstorrent GEMM ledger: `benchmarks/silicon/qwen38_fabric_segments.json`
//! holds, per segment, the SHA-256 of the app that ran, each network's verdict, cycles and check lines, and PIR's
//! resources. What every suite run pins, fast: the segments cover every kind; every ledger row is the app the
//! compiler emits TODAY; every segment passes on the ideal network; and on the routed network every segment either
//! passes or stops on a fault the NETWORK names — a packet the router cannot route, or a deadlock of the network
//! itself, on an app whose ideal run passed — never a routed run that completes with a wrong answer. A compiler
//! change to any of these apps reds this lock until the chain is run again:
//!
//!   LOGOS_RECORD_FABRIC=1 cargo test -p logicaffeine-tests --test qwen38_fabric_segments_lock -- --nocapture record
//!
//! (about 70 minutes for every segment, one container; `cargo test`, because a recording is longer than nextest's
//! chain backstop). A segment whose app is unchanged since its recorded pass keeps its row and is not run again.
//!
//! ★ MUTATION PROBE: change one word of a comment `spatial_lang` prints into every app (`site(s)` → `sites` in the
//! section marker) — `every_segment_on_the_ledger_is_todays_app_and_passed_on_the_fabric` fails on the first
//! segment's hash and names the re-record command: a ledger row is evidence for exactly one app.
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

mod common;
#[path = "common/qwen38_fixture.rs"]
mod qwen38_fixture;

use logicaffeine_compile::spatial::eval::{eval_choreography_every_tensor, Inputs, Value};
use logicaffeine_compile::spatial::ir::{SpatialProgram, TensorRole};
use logicaffeine_compile::spatial::lower::spatial_lang::{emit_spatial_segment, SpatialFlavor};
use qwen38_fixture::{decode_step, target};
use std::collections::BTreeSet;

const LEDGER: &str = "benchmarks/silicon/qwen38_fabric_segments.json";
const SEGMENT: usize = 4;
/// The oracle computes the step on the SN50's bf16 datapath; the fabric computes in fp32. Two percent of a tensor's
/// largest value holds bf16's rounding across a segment and nothing coarser.
const REL_TOL: f64 = 2e-2;

/// The repository root, canonical — so a run log under it is recorded by its repo-relative path.
fn repo_root() -> std::path::PathBuf {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    root.canonicalize().unwrap_or(root)
}

/// A section's KIND: its name with the sizes and layer indices dropped (`matmul_1x4x8` → `matmul`).
fn kind(name: &str) -> String {
    name.split('_')
        .map(|w| w.trim_end_matches(|c: char| c.is_ascii_digit()))
        .filter(|w| !w.is_empty() && !w.chars().all(|c| c.is_ascii_digit() || c == 'x'))
        .collect::<Vec<_>>()
        .join("_")
}

/// The fewest aligned `SEGMENT`-section segments holding every kind: greedy, by kinds not yet held, earliest first
/// on a tie — deterministic.
fn covering_segments(p: &SpatialProgram) -> Vec<std::ops::Range<usize>> {
    let kinds: Vec<String> = p.sections.iter().map(|s| kind(&s.name)).collect();
    let mut left: BTreeSet<String> = kinds.iter().cloned().collect();
    let segments: Vec<std::ops::Range<usize>> = (0..kinds.len())
        .step_by(SEGMENT)
        .map(|a| a..(a + SEGMENT).min(kinds.len()))
        .collect();
    let mut chosen = Vec::new();
    while !left.is_empty() {
        let new = |r: &std::ops::Range<usize>| {
            kinds[r.clone()]
                .iter()
                .filter(|k| left.contains(*k))
                .count()
        };
        let best = segments
            .iter()
            .fold(None::<&std::ops::Range<usize>>, |b, r| match b {
                Some(b) if new(b) >= new(r) => Some(b),
                _ => Some(r),
            });
        let best = best.expect("a program with sections").clone();
        for k in &kinds[best.clone()] {
            left.remove(k);
        }
        chosen.push(best);
    }
    chosen.sort_by_key(|r| r.start);
    chosen
}

/// Every tensor the oracle holds at the end of the step, from the step's own fills.
fn oracle_values(
    p: &SpatialProgram,
) -> std::collections::BTreeMap<logicaffeine_compile::spatial::ir::TensorId, Value> {
    let mut inputs = Inputs::default();
    for t in p.tensors.iter().filter(|t| t.role != TensorRole::Output) {
        let n = t.elems.rows * t.elems.cols;
        let v = if t.init.is_float() {
            Value::Float(
                (0..n)
                    .map(|i| t.init.element_f64(i).expect("a fill as long as its tensor"))
                    .collect(),
            )
        } else {
            Value::Int(
                (0..n)
                    .map(|i| t.init.element(i).expect("a fill as long as its tensor"))
                    .collect(),
            )
        };
        inputs.tensors.insert(t.id, v);
    }
    eval_choreography_every_tensor(p, &inputs)
        .expect("the oracle runs the step")
        .tensors
}

struct Segment {
    sections: std::ops::Range<usize>,
    kinds: Vec<String>,
    class: String,
    app: String,
}

fn segments() -> Vec<Segment> {
    let (_, p) = decode_step();
    let values = oracle_values(&p);
    covering_segments(&p)
        .into_iter()
        .map(|r| {
            let app = emit_spatial_segment(
                &p,
                &target(),
                r.clone(),
                &values,
                REL_TOL,
                SpatialFlavor::Fabric,
            )
            .unwrap_or_else(|e| panic!("segment {r:?} prints: {e:?}"));
            let class = app
                .split("@spatial class ")
                .nth(1)
                .and_then(|s| s.split_whitespace().next())
                .expect("the app's class")
                .to_string();
            let kinds = p.sections[r.clone()]
                .iter()
                .map(|s| kind(&s.name))
                .collect();
            Segment {
                sections: r,
                kinds,
                class,
                app,
            }
        })
        .collect()
}

/// The chain's log without its terminal colors — the summary it prints is colored.
fn plain(log: &str) -> String {
    let mut out = String::with_capacity(log.len());
    let mut chars = log.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' && chars.peek() == Some(&'[') {
            chars.next();
            while let Some(d) = chars.next() {
                if d.is_ascii_alphabetic() {
                    break;
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// One Tungsten run's verdict from its own log: the harness's PASS, the cycle it finished on, its check lines, and
/// the fault that stopped it early, if one did.
fn network_run(log: &std::path::Path, class: &str) -> serde_json::Value {
    let text = plain(&std::fs::read_to_string(log).unwrap_or_default());
    let checks: Vec<&str> = text.lines().filter(|l| l.starts_with("check ")).collect();
    let cycles = text
        .lines()
        .filter_map(|l| l.split("Simulation complete at cycle ").nth(1))
        .filter_map(|r| {
            r.split(|c: char| !c.is_ascii_digit())
                .next()?
                .parse::<u64>()
                .ok()
        })
        .last();
    let fault = text
        .lines()
        .find(|l| l.contains("Caught exception") || l.contains("DEADLOCK"))
        .map(str::trim);
    serde_json::json!({
        "pass": text.contains(&format!("PASS: true ({class})")),
        "cycles": cycles,
        "checks": checks,
        "fault": fault,
    })
}

fn sha(app: &str) -> String {
    let mut h = common::ttsim::Sha256::new();
    h.update(app.as_bytes());
    h.hex()
}

fn ledger() -> serde_json::Value {
    let text = std::fs::read_to_string(repo_root().join(LEDGER)).unwrap_or_else(|e| panic!("{LEDGER} is recorded ({e}) — record it:\n  LOGOS_RECORD_FABRIC=1 cargo test -p logicaffeine-tests --test qwen38_fabric_segments_lock -- --nocapture record"));
    serde_json::from_str(&text).expect("the ledger is JSON")
}

/// ── THE SEGMENTS HOLD EVERY KIND OF SECTION THE STEP HAS.
#[test]
fn the_segments_hold_every_kind_of_section() {
    let (_, p) = decode_step();
    let all: BTreeSet<String> = p.sections.iter().map(|s| kind(&s.name)).collect();
    let held: BTreeSet<String> = covering_segments(&p)
        .into_iter()
        .flat_map(|r| {
            p.sections[r]
                .iter()
                .map(|s| kind(&s.name))
                .collect::<Vec<_>>()
        })
        .collect();
    assert_eq!(held, all, "every kind of section runs on the fabric");
    for k in [
        "argmax",
        "row_gather",
        "set_row",
        "attn_scores",
        "gd_update",
        "conv_ring",
        "rope_half",
    ] {
        assert!(
            all.contains(k),
            "the step has a `{k}` section — the kinds are read off the program, not typed"
        );
    }
}

/// ── EVERY LEDGER ROW IS THE APP EMITTED TODAY: IT PASSED ON THE IDEAL NETWORK, AND ON THE ROUTED ONE IT PASSED OR
/// STOPPED ON A FAULT OF THE NETWORK'S OWN.
#[test]
fn every_segment_on_the_ledger_is_todays_app_and_passed_on_the_fabric() {
    let d = ledger();
    let rows = d["segments"].as_array().expect("a segments array");
    let today = segments();
    assert_eq!(rows.len(), today.len(), "one row per covering segment");
    for (row, seg) in rows.iter().zip(&today) {
        assert_eq!(row["class"], seg.class.as_str());
        assert_eq!(
            row["sections"],
            serde_json::json!([seg.sections.start, seg.sections.end])
        );
        assert_eq!(
            row["app_sha256"],
            sha(&seg.app).as_str(),
            "{}: the recorded run is of a different app than the compiler emits today — re-record:\n  LOGOS_RECORD_FABRIC=1 cargo test -p logicaffeine-tests --test qwen38_fabric_segments_lock -- --nocapture record",
            seg.class
        );
        assert!(
            row["pir_resources"]
                .as_str()
                .is_some_and(|r| r.contains("PCU:") && r.contains("PMU:")),
            "{}: PIR placed it on PCUs and PMUs",
            seg.class
        );
        let ideal = &row["ideal"];
        assert_eq!(
            ideal["pass"], true,
            "{}: the ideal-network run passed: {ideal}",
            seg.class
        );
        let checks = ideal["checks"].as_array().expect("check lines");
        assert!(
            !checks.is_empty()
                && checks
                    .iter()
                    .all(|c| c.as_str().is_some_and(|l| l.ends_with("ok true"))),
            "{}: every check passed on the ideal network: {checks:?}",
            seg.class
        );
        assert!(
            ideal["cycles"].as_u64().is_some_and(|c| c > 0),
            "{}: Tungsten ran it",
            seg.class
        );
        let routed = &row["routed"];
        let network_fault = routed["fault"]
            .as_str()
            .is_some_and(|f| f.contains("no route for packet") || f.contains("DEADLOCK"));
        assert!(
            routed["pass"] == true || network_fault,
            "{}: on the routed network it passed, or stopped on the network's own fault: {routed}",
            seg.class
        );
    }
}

/// ── RECORD: run every covering segment through Spatial → PIR → plastiroute → Tungsten and write the ledger. Runs
/// only under `LOGOS_RECORD_FABRIC=1` (and with the container), since it takes about 70 minutes.
#[test]
fn record_every_segment_through_the_fabric() {
    if std::env::var("LOGOS_RECORD_FABRIC").as_deref() != Ok("1") {
        eprintln!("LOGOS_TOOL_SKIP: fabric recording runs under LOGOS_RECORD_FABRIC=1");
        return;
    }
    if !common::require_tool("docker") || !common::require_tool("plasticine-toolchain") {
        return;
    }
    let regression = common::plasticine::regression_dir();
    // A row whose app is unchanged and passed is still the evidence for that app; only a changed or failed segment
    // goes through the chain again.
    let recorded: Vec<serde_json::Value> = std::fs::read_to_string(repo_root().join(LEDGER))
        .ok()
        .and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok())
        .and_then(|d| d["segments"].as_array().cloned())
        .unwrap_or_default();
    let mut rows = Vec::new();
    for seg in segments() {
        if let Some(kept) = recorded.iter().find(|r| {
            r["class"] == seg.class.as_str()
                && r["app_sha256"] == sha(&seg.app).as_str()
                && r["ideal"]["pass"] == true
        }) {
            println!("{}: unchanged since its recorded pass — kept", seg.class);
            rows.push(kept.clone());
            continue;
        }
        std::fs::write(regression.join(format!("{}.scala", seg.class)), &seg.app)
            .expect("write the segment app");
        let run = common::plasticine::run_gate(&seg.class);
        let resources = plain(&run.log)
            .lines()
            .map(str::trim)
            .find(|l| l.starts_with(&format!("Tst {} PCU:", seg.class)))
            .map(str::to_string)
            .unwrap_or_default();
        let logs = common::plasticine::toolchain_dir()
            .join(format!("spatial/gen/Tst_pirTest/{}/log", seg.class));
        let ideal = network_run(&logs.join("runp2p.log"), &seg.class);
        let routed = network_run(&logs.join("runhybrid.log"), &seg.class);
        println!(
            "{}: ideal {} / routed {} — {resources}",
            seg.class, ideal["pass"], routed["pass"]
        );
        rows.push(serde_json::json!({
            "class": seg.class,
            "sections": [seg.sections.start, seg.sections.end],
            "kinds": seg.kinds,
            "app_sha256": sha(&seg.app),
            "pir_resources": resources,
            "ideal": ideal,
            "routed": routed,
            "log": run.log_path.strip_prefix(repo_root()).unwrap_or(&run.log_path).display().to_string(),
        }));
    }
    let doc = serde_json::json!({
        "schema": "logos.qwen38_fabric_segments",
        "version": 1,
        "program": "Qwen3.8 tiny decode step (token 5 at position 3, KV cache 6), composed for sn50-rdu, printed per segment by spatial_lang (Fabric flavor)",
        "chain": "Spatial -> PIR -> plastiroute -> Tungsten (scripts/plasticine/gate.sh)",
        "segment_sections": SEGMENT,
        "rel_tol": REL_TOL,
        "provenance": "simulated_tungsten",
        "segments": rows,
    });
    std::fs::write(
        repo_root().join(LEDGER),
        serde_json::to_string_pretty(&doc).expect("serializes") + "\n",
    )
    .expect("write the ledger");
    let failed: Vec<&serde_json::Value> = doc["segments"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|r| r["ideal"]["pass"] != true)
        .collect();
    assert!(
        failed.is_empty(),
        "every segment passes on the ideal network: {failed:?}"
    );
}
