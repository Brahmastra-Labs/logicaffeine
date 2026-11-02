//! ═══════════════════════════════════════════════════════════════════════
//! PLASTICINE SIM LOCK — P5 of the Plasticine lane: the public toolchain's own numbers, and our model held
//! against them.
//!
//! A. `benchmarks/silicon/plasticine_toolchain.json` (written only by `scripts/accel/plasticine-gate-ledger.py`) is
//!    self-consistent: every row's cycle counts, PASS verdicts, PIR resource mapping, routing score and mesh
//!    re-parse from the COMMITTED log excerpts (whose hashes the row carries), and its provenance is
//!    `simulated_tungsten` — a simulation of the paper's architecture by its authors' chain, never "measured".
//! B. `benchmarks/silicon/plasticine_calibration.json` regenerates BYTE-EQUAL from the toolchain ledger and the
//!    model (`plasticine_report::calibration`), so the modeled counterpart of every simulated row is a
//!    function of the two artifacts and nothing else.
//! C. For every simulated app, our MODELED row on the same mesh is within `CALIBRATION_TOLERANCE_PCT` of
//!    Tungsten's cycle count — Spatial's GEMMs with the same shape, tile and unroll (same lane width, II 1),
//!    dot products as the `1 x 1 x N` GEMM, and OUR OWN emitted loops (a carried register costs the PCU's
//!    pipeline depth + 1 per iteration, a loop without one costs 1: the calibrated `UnitModel::plasticine_pcu`)
//!    — the model is anchored to the vendor simulator, and the difference is printed, not hidden. Every
//!    ledger row is joined exactly once or listed uncalibrated, only for a kind the model does not describe
//!    (`plasticine_report::UNMODELED_KINDS`, with that kind's reason) or a routed run that did not finish clean (with
//!    what it came to).
//! D. Spatial's own GEMM_0 is in the ledger (the credibility gate's subject): PASS on both networks, PIR's
//!    mapping on the 20x20 mesh, plastiroute's score — the run `scripts/plasticine/README.md` reproduces.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(plasticine_sim_lock)'`
//! ═══════════════════════════════════════════════════════════════════════

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;

use logicaffeine_compile::codegen_sva::plasticine_report::{
    calibration, render_report, target_for_mesh, CALIBRATION_PATH, CALIBRATION_TOLERANCE_PCT,
    PROVENANCE_SIMULATED, TOOLCHAIN_LEDGER_PATH, UNMODELED_KINDS,
};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn read(rel: &str) -> String {
    std::fs::read_to_string(repo_root().join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"))
}

fn ledger() -> serde_json::Value {
    serde_json::from_str(&read(TOOLCHAIN_LEDGER_PATH)).expect("the toolchain ledger parses")
}

/// The first 16 hex digits of SHA-256 (what the ledger tool records), via our own FIPS-vectored implementation.
fn sha16(text: &str) -> String {
    logicaffeine_compile::crypto_accel::sha256(text.as_bytes())[..8]
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn last_cycle(excerpt: &str) -> Option<u64> {
    excerpt
        .lines()
        .filter_map(|l| l.strip_prefix("Simulation complete at cycle "))
        .last()
        .and_then(|n| n.split_whitespace().next()?.parse().ok())
}

/// What a simulation came to when it did not finish clean, read from its excerpt on its own (the recorder's reading
/// is checked against this one): deadlocked on a cycle, threw on one, printed no completion, or failed its gold check.
fn outcome(excerpt: &str) -> Option<String> {
    let plain: String = {
        let mut out = String::new();
        let mut rest = excerpt;
        while let Some(i) = rest.find("\u{1b}[") {
            out.push_str(&rest[..i]);
            rest = &rest[i..];
            let end = rest.find('m').map_or(rest.len(), |e| e + 1);
            rest = &rest[end..];
        }
        out.push_str(rest);
        out
    };
    let deadlock = plain.lines().filter_map(|l| l.strip_prefix("Simulation complete at cycle ")).filter_map(|r| {
        let (n, tail) = r.split_once(' ')?;
        (tail.trim() == "(DEADLOCK)").then(|| n.to_string())
    }).last();
    if let Some(n) = deadlock {
        return Some(format!("deadlocked at cycle {n}"));
    }
    if let Some(t) = plain.lines().filter_map(|l| l.split_once("Caught exception on cycle ").map(|(_, t)| t)).last() {
        let (n, msg) = t.split_once(": ").unwrap_or((t, ""));
        return Some(format!("threw on cycle {n}: {}", msg.trim()));
    }
    if last_cycle(&plain).is_none() {
        return Some("printed no completion".into());
    }
    match plain.lines().find_map(|l| l.strip_prefix("PASS: ")) {
        Some(v) if v.starts_with("true") => None,
        _ => Some("failed its gold check".into()),
    }
}

// ───────────────────────────── A ─────────────────────────────

#[test]
fn every_toolchain_row_reparses_from_its_committed_excerpts() {
    let doc = ledger();
    assert_eq!(doc["provenance"], PROVENANCE_SIMULATED);
    assert!(doc["_generated_by"]
        .as_str()
        .unwrap()
        .contains("plasticine-gate-ledger.py"));
    let text = read(TOOLCHAIN_LEDGER_PATH).to_lowercase();
    assert!(
        !text.contains("measured"),
        "a simulation ledger never says 'measured'"
    );
    let rows = doc["rows"].as_array().expect("rows");
    assert!(!rows.is_empty(), "the ledger has at least the gate's row");
    for r in rows {
        let key = r["key"].as_str().unwrap();
        assert_eq!(r["provenance"], PROVENANCE_SIMULATED, "{key}");
        let ex = r["excerpts"].as_object().expect("excerpts");
        for name in [
            "gentst.sh",
            "resource.csv",
            "runp2p.log",
            "runproute.log",
            "runhybrid.log",
        ] {
            let rel = ex[name]
                .as_str()
                .unwrap_or_else(|| panic!("{key}: excerpt {name}"));
            let body = read(rel);
            assert_eq!(
                r["excerpt_sha256"][name].as_str().unwrap(),
                sha16(&body),
                "{key}: {name} was edited after recording"
            );
            match name {
                "runp2p.log" => {
                    assert_eq!(
                        last_cycle(&body),
                        r["ideal_network"]["cycles"].as_u64(),
                        "{key}: ideal-network cycles re-parse"
                    );
                    assert_eq!(outcome(&body), None, "{key}: a row's ideal-network run finished clean");
                }
                "runhybrid.log" => {
                    assert_eq!(
                        last_cycle(&body),
                        r["hybrid_network"]["cycles"].as_u64(),
                        "{key}: hybrid-network cycles re-parse"
                    );
                    assert_eq!(
                        outcome(&body).as_deref(),
                        r["hybrid_network"]["why"].as_str(),
                        "{key}: what the routed run came to, recorded as it happened"
                    );
                }
                "runproute.log" => {
                    let score: f64 = body
                        .lines()
                        .find_map(|l| l.strip_prefix("Best score is "))
                        .unwrap()
                        .trim()
                        .parse()
                        .unwrap();
                    assert_eq!(
                        Some(score),
                        r["plastiroute"]["best_score"].as_f64(),
                        "{key}: routing score re-parse"
                    );
                }
                "resource.csv" => {
                    for line in body.lines().skip(1) {
                        let f: Vec<&str> = line.split(',').collect();
                        let kind = f[0];
                        assert_eq!(
                            r["pir"][kind]["used"].as_u64().unwrap(),
                            f[1].parse::<u64>().unwrap(),
                            "{key}: PIR {kind} used"
                        );
                        assert_eq!(
                            r["pir"][kind]["total"].as_u64().unwrap(),
                            f[3].parse::<u64>().unwrap(),
                            "{key}: PIR {kind} total"
                        );
                    }
                }
                "gentst.sh" => {
                    let rows_ = r["mesh"]["rows"].as_u64().unwrap();
                    let cols_ = r["mesh"]["cols"].as_u64().unwrap();
                    assert!(
                        body.contains(&format!("--row={rows_} --col={cols_}")),
                        "{key}: the mesh PIR mapped onto"
                    );
                }
                _ => unreachable!(),
            }
        }
        assert_eq!(r["ideal_network"]["pass"], true, "{key}");
        assert!(
            r["hybrid_network"]["why"].is_string() || r["hybrid_network"]["pass"] == true,
            "{key}: a routed run is clean or says why not"
        );
        assert!(
            r["toolchain"].as_str().unwrap().contains("tungsten@"),
            "{key}: the toolchain snapshot is named"
        );
    }
}

// ───────────────────────────── B ─────────────────────────────

#[test]
fn the_calibration_regenerates_byte_equal_from_the_ledger_and_the_model() {
    let regenerated = render_report(&calibration(&ledger()).expect("calibration"));
    let on_disk = read(CALIBRATION_PATH);
    assert_eq!(on_disk, regenerated, "{CALIBRATION_PATH} is stale — regenerate it with the private place-and-route engine's report emitter, emit_plasticine_report");
    let low = on_disk.to_lowercase();
    assert!(
        !low.contains("measured"),
        "modeled vs simulated, never measured"
    );
    assert!(low.contains("not silicon"));
}

// ───────────────────────────── C ─────────────────────────────

#[test]
fn every_simulated_app_is_within_tolerance_of_the_model_on_the_same_mesh() {
    let cal: serde_json::Value = serde_json::from_str(&read(CALIBRATION_PATH)).unwrap();
    assert_eq!(
        cal["tolerance_pct"].as_f64().unwrap(),
        CALIBRATION_TOLERANCE_PCT
    );
    let rows = cal["rows"].as_array().unwrap();
    assert!(!rows.is_empty());
    for r in rows {
        let key = r["key"].as_str().unwrap();
        let modeled = r["modeled"]["cycles"].as_u64().unwrap() as f64;
        let simulated = r["simulated"]["cycles_hybrid_network"].as_u64().unwrap() as f64;
        let diff = (modeled / simulated - 1.0).abs() * 100.0;
        assert!(diff <= CALIBRATION_TOLERANCE_PCT, "{key}: modeled {modeled} vs simulated {simulated} cycles differ by {diff:.2}% (> {CALIBRATION_TOLERANCE_PCT}%)");
        assert_eq!(
            r["difference_pct"].as_f64().unwrap(),
            (diff * 1000.0).round() / 1000.0,
            "{key}: the printed difference is the recomputed one"
        );
        assert_eq!(
            r["modeled_over_simulated"].as_f64().unwrap(),
            ((modeled / simulated) * 1000.0).round() / 1000.0
        );
        if r["kind"] == "counted_loop" {
            // OUR loop through THEIR chain: the modeled II is the calibrated PCU's (pipeline depth + 1 per
            // carried register, 1 without one), and the simulator's cycles per iteration are printed.
            let iters = r["loop"]["iterations"].as_u64().unwrap() as f64;
            assert!(
                iters >= 12.0,
                "{key}: enough iterations to see the steady state"
            );
            assert_eq!(
                r["simulated_cycles_per_iteration"].as_f64().unwrap(),
                (simulated / iters * 1000.0).round() / 1000.0,
                "{key}"
            );
            assert!(r["modeled"]["pipeline_ii"].as_u64().unwrap() >= 1);
        } else {
            assert_eq!(
                r["lanes_agree"], true,
                "{key}: Spatial's `ip` is the PCU's SIMD width in our model"
            );
            assert_eq!(
                r["modeled"]["pipeline_ii"], 1,
                "{key}: a conflict-free MAC-reduce issues every cycle"
            );
        }
        assert_eq!(r["modeled"]["provenance"], "modeled");
        assert_eq!(r["simulated"]["provenance"], PROVENANCE_SIMULATED);
        assert_eq!(r["simulated"]["pass"], true);
        let mesh_target = r["target"].as_str().unwrap();
        assert!(
            target_for_mesh(20, 20).map_or(false, |t| t.name == mesh_target)
                || target_for_mesh(16, 8).map_or(false, |t| t.name == mesh_target)
        );
        // The 20x20 rows calibrate against the system the chain simulates: PIR's default mesh in front of
        // Tungsten's DDR4 memory, not the HBM2 SARA preset.
        if mesh_target.contains("20x20") {
            assert!(
                mesh_target.contains("ddr4"),
                "{key}: the toolchain's memory is DDR4, calibrated as such"
            );
        }
    }
}

#[test]
fn every_toolchain_row_is_joined_once_or_listed_uncalibrated_with_why() {
    let cal: serde_json::Value = serde_json::from_str(&read(CALIBRATION_PATH)).unwrap();
    let doc = ledger();
    let keys = |v: &serde_json::Value| -> Vec<String> {
        v.as_array().unwrap().iter().map(|r| r["key"].as_str().unwrap().to_string()).collect()
    };
    let (joined, listed) = (keys(&cal["rows"]), keys(&cal["uncalibrated"]));
    let mut seen: Vec<String> = joined.iter().chain(&listed).cloned().collect();
    seen.sort();
    let mut all = keys(&doc["rows"]);
    all.sort();
    assert_eq!(seen, all, "every toolchain row appears exactly once: joined with the model or listed uncalibrated");
    for u in cal["uncalibrated"].as_array().unwrap() {
        let kind = u["kind"].as_str().unwrap();
        let row = doc["rows"].as_array().unwrap().iter().find(|r| r["key"] == u["key"]).expect("its row");
        let why = match UNMODELED_KINDS.iter().find(|(k, _)| *k == kind) {
            Some((_, why)) => why.to_string(),
            None => format!(
                "its routed-network simulation {}",
                row["hybrid_network"]["why"].as_str().unwrap_or_else(|| panic!(
                    "{}: listed uncalibrated, but the model describes `{kind}` and its routed run finished clean",
                    u["key"]
                ))
            ),
        };
        assert_eq!(u["why"], why, "{}", u["key"]);
    }
}

// ───────────────────────────── D ─────────────────────────────

#[test]
fn spatials_own_gemm_is_the_gates_subject_and_passed_both_networks() {
    let doc = ledger();
    let row = doc["rows"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["key"] == "GEMM_0/20x20")
        .expect("GEMM_0 on the 20x20 mesh");
    assert!(
        row["app_source"]
            .as_str()
            .unwrap()
            .contains("regression/GEMM.scala"),
        "Spatial's own regression app, unmodified"
    );
    assert_eq!(row["shape"]["m"], 128);
    assert_eq!(row["params"]["ts"], 64);
    assert_eq!(row["params"]["ip"], 16);
    assert_eq!(row["mesh"]["pcus"], 200);
    assert!(row["pir"]["PCU"]["used"].as_u64().unwrap() >= 1);
    assert!(row["pir"]["PMU"]["used"].as_u64().unwrap() >= 1);
    assert!(
        row["hybrid_network"]["cycles"].as_u64().unwrap() > 100_000,
        "128^3 at one 16-lane MAC per cycle is above 100k cycles"
    );
    assert!(row["plastiroute"]["best_score"].as_f64().unwrap() >= 1.0);
    let runbook = read("scripts/plasticine/README.md");
    assert!(
        runbook.contains("gate.sh GEMM_0"),
        "the runbook says how to reproduce the gate"
    );
}

/// The recorder takes a run whose ideal-network simulation finished clean — a cycle count, no deadlock, no exception,
/// its gold check passed — and records what the routed (hybrid) network did with it, failure included: deadlocked on a
/// cycle (Tungsten prints the verdict in colour), threw (BlackScholes_0's routed run: `no route for packet` on cycle
/// 166), printed no completion, or failed its gold check. A run that did not finish clean on the ideal network never
/// ran clean anywhere and is refused. `drop` removes a recorded row and its excerpts, the recorder being the ledger's
/// only writer. Driven on fixture runs in a scratch ledger.
#[test]
fn the_recorder_takes_a_routed_failure_as_data_and_refuses_an_ideal_one() {
    let work = tempfile::tempdir().unwrap();
    let out = std::process::Command::new("python3")
        .arg("-c")
        .arg(
            r#"
import argparse, contextlib, importlib.util, io, json, os, sys
spec = importlib.util.spec_from_file_location("ledger", sys.argv[1])
g = importlib.util.module_from_spec(spec)
spec.loader.exec_module(g)
work = sys.argv[2]
g.ROOT, g.LEDGER, g.LOGS = work, os.path.join(work, "ledger.json"), os.path.join(work, "logs")
OK = "Simulation complete at cycle 6030\nPASS: true (x)\n"
def gen(name, ideal, hybrid):
    d = os.path.join(work, name)
    os.makedirs(os.path.join(d, "log"))
    os.makedirs(os.path.join(d, "pir", "out"))
    files = {"log/gentst.sh": "pir --row=20 --col=20\n", "log/runp2p.log": ideal,
             "log/runproute.log": "Best score is 0.51\nUsed 1 VCs.\n", "log/runhybrid.log": hybrid,
             "pir/out/resource.csv": "Type,Used,Total,Usage\nPCU,1,200,0.5\nPMU,2,200,1.0\n"}
    for rel, text in files.items():
        open(os.path.join(d, rel), "w").write(text)
    return d
def add(name, ideal, hybrid):
    a = argparse.Namespace(app=name, kind="dot", app_source=None, gen_dir=gen(name, ideal, hybrid), shape=None,
                           params="N=65536 ts=16384 ip=16 op=1", logos_source=None, fn_name=None, toolchain="tc",
                           reproduce="r")
    err = io.StringIO()
    try:
        with contextlib.redirect_stderr(err), contextlib.redirect_stdout(io.StringIO()):
            g.cmd_add(a)
    except SystemExit:
        return err.getvalue().strip()
    h = next(r for r in g.load()["rows"] if r["app"] == name)["hybrid_network"]
    return "recorded %s %s %s" % (h["cycles"], h["pass"], h.get("why"))
print(add("DotProduct_ok", OK, "Simulation complete at cycle 6056\nPASS: true (x)\n"))
print(add("DotProduct_stuck", OK, "Simulation complete at cycle 8853 \x1b[0;31m(DEADLOCK)\x1b[0m\nPASS: false (x)\n"))
print(add("DotProduct_threw", OK, "\x1b[0;31mCaught exception on cycle 166: /DUT/Top/net/router40: no route for packet 343\x1b[0m\nPASS: true (x)\nComplete Simulation\n"))
print(add("DotProduct_wrong", OK, "Simulation complete at cycle 9000\nPASS: false (x)\n"))
print(add("DotProduct_mute", OK, "PASS: true (x)\n"))
print(add("DotProduct_ideal_stuck", "Simulation complete at cycle 5642 (DEADLOCK)\nPASS: true (x)\n", OK))
print(add("DotProduct_ideal_wrong", "Simulation complete at cycle 7000\nPASS: false (x)\n", OK))
with contextlib.redirect_stdout(io.StringIO()):
    g.cmd_verify(argparse.Namespace())
print("verified")
with contextlib.redirect_stdout(io.StringIO()):
    g.cmd_drop(argparse.Namespace(keys=["DotProduct_ok/20x20"]))
print(json.dumps(sorted(r["key"] for r in g.load()["rows"])), json.dumps(os.path.exists(os.path.join(work, "logs", "DotProduct_ok"))))
try:
    with contextlib.redirect_stderr(io.StringIO()):
        g.cmd_drop(argparse.Namespace(keys=["Nothing/20x20"]))
    print("dropped nothing")
except SystemExit:
    print("refused an unknown key")
"#,
        )
        .arg(repo_root().join("scripts/accel/plasticine-gate-ledger.py"))
        .arg(work.path())
        .output()
        .expect("python3 runs");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8_lossy(&out.stdout);
    assert_eq!(
        text.lines().collect::<Vec<_>>(),
        [
            "recorded 6056 True None",
            "recorded 8853 False deadlocked at cycle 8853",
            "recorded None True threw on cycle 166: /DUT/Top/net/router40: no route for packet 343",
            "recorded 9000 False failed its gold check",
            "recorded None True printed no completion",
            "REFUSING: the ideal-network simulation deadlocked at cycle 5642: the app never ran clean",
            "REFUSING: the ideal-network simulation failed its gold check: the app never ran clean",
            "verified",
            r#"["DotProduct_mute/20x20", "DotProduct_stuck/20x20", "DotProduct_threw/20x20", "DotProduct_wrong/20x20"] false"#,
            "refused an unknown key",
        ]
    );
}
