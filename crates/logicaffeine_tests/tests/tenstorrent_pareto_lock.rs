//! ★ TENSTORRENT PARETO-FRONTIER LOCK (Phase 4) ★ — the perf-vs-cost ($/throughput) frontier is
//! CORRECT (arithmetic recomputed from the published specs) and HONEST (no fabricated on-silicon
//! measurement; every un-measured number is flagged).
//!
//! Runs `scripts/accel/gen-pareto-frontier.py` into a scratch directory: the committed
//! `benchmarks/results/tenstorrent_pareto.json` and its chart must equal what it writes there, so a stale
//! frontier is RED and a test run never rewrites a tracked file (regenerating is the explicit
//! `python3 scripts/accel/gen-pareto-frontier.py`). Then, over the committed file:
//!   - `tops_per_dollar` / `tops_per_watt` equal `peak_tops/price` and `peak_tops/tdp` (no cooked ratios);
//!   - every peak-frontier entry is `published:true, measured:false` (peak specs, not measurements);
//!   - the Tenstorrent cards' TOPS/$ dominate the CPU's by a wide margin (the frontier claim);
//!   - every Tenstorrent WORKLOAD number is `projected:true` and the i9 workload number is `measured:true`
//!     — so no on-silicon result is ever passed off as real;
//!   - the top-level status announces the frontier is PROJECTED.
//!
//! ⚠️ YOU DO NOT GET TO WEAKEN THIS FILE. If it goes RED, fix the generator/specs, not the assertion.

use std::path::PathBuf;
use std::process::Command;

#[path = "common/mod.rs"]
mod common;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn f(v: &serde_json::Value) -> f64 {
    v.as_f64()
        .unwrap_or_else(|| panic!("expected a number, got {v:?}"))
}

fn close(a: f64, b: f64) -> bool {
    (a - b).abs() <= 1e-6 * a.abs().max(b.abs()).max(1.0)
}

#[test]
fn pareto_frontier_is_correct_and_honest() {
    if !common::require_tool("python3") {
        return;
    }
    let root = repo_root();
    let scratch = tempfile::tempdir().expect("a scratch directory for the generator");
    let gen = Command::new("python3")
        .arg(root.join("scripts/accel/gen-pareto-frontier.py"))
        .arg("--out-dir")
        .arg(scratch.path())
        .current_dir(&root)
        .output()
        .expect("run gen-pareto-frontier.py");
    assert!(
        gen.status.success(),
        "generator failed:\n{}",
        String::from_utf8_lossy(&gen.stderr)
    );
    for (committed, generated) in [
        (
            "benchmarks/results/tenstorrent_pareto.json",
            scratch.path().join("tenstorrent_pareto.json"),
        ),
        (
            "benchmarks/results/charts/tenstorrent_pareto.svg",
            scratch.path().join("charts/tenstorrent_pareto.svg"),
        ),
    ] {
        let want = std::fs::read(&generated)
            .unwrap_or_else(|e| panic!("the generator wrote {}: {e}", generated.display()));
        let have = std::fs::read(root.join(committed))
            .unwrap_or_else(|e| panic!("{committed} is committed: {e}"));
        assert!(
            have == want,
            "{committed} is stale: it is not what the generator writes from today's specs, benchmarks and ledger — \
             regenerate it with `python3 scripts/accel/gen-pareto-frontier.py` and commit the result"
        );
    }

    let json_path = root.join("benchmarks/results/tenstorrent_pareto.json");
    let s = std::fs::read_to_string(&json_path).expect("read tenstorrent_pareto.json");
    let d: serde_json::Value = serde_json::from_str(&s).expect("parse pareto json");

    // Status announces the frontier's provenance honestly: either PROJECTED (no sim run) or SIM
    // (cycle-accurate ttsim numbers present) — never "measured on silicon" (no card yet). Power is
    // always separated out as pending_hardware until a real tt-smi run.
    let status = d["status"].as_str().unwrap_or("");
    assert!(
        status.contains("PROJECTED") || status.contains("SIM"),
        "the frontier status must announce PROJECTED or SIM provenance, got: {status:?}"
    );
    assert_eq!(
        d["power"]["provenance"],
        serde_json::Value::String("pending_hardware".into()),
        "power must stay pending_hardware — ttsim reports cycles, not watts"
    );

    // If ttsim cycles are present, every one is `sim_functional_not_timed` (real simulated device cycles),
    // NOT measured-on-silicon — so nothing is passed off as a hardware result.
    let sim = &d["sim_workload"];
    if sim["available"].as_bool().unwrap_or(false) {
        assert_eq!(
            sim["provenance"],
            serde_json::Value::String("sim_functional_not_timed".into())
        );
        for k in sim["kernels"].as_array().expect("sim kernels") {
            assert_eq!(
                k["provenance"],
                serde_json::Value::String("sim_functional_not_timed".into()),
                "sim kernel {:?} must be sim_functional_not_timed, never measured_hw",
                k["name"]
            );
            assert!(
                k["cycles"].as_u64().unwrap_or(0) > 0,
                "every sim kernel needs a real cycle count"
            );
        }
    }

    // Peak-cost frontier: arithmetic recomputed, honesty flags enforced.
    let frontier = d["peak_frontier"].as_array().expect("peak_frontier array");
    let mut by_id = std::collections::HashMap::new();
    for r in frontier {
        let id = r["id"].as_str().unwrap().to_string();
        let price = f(&r["price_usd"]);
        let tops = f(&r["peak_tops"]);
        let tdp = f(&r["tdp_w"]);
        assert!(
            close(f(&r["tops_per_dollar"]), tops / price),
            "{id}: tops_per_dollar must equal peak_tops/price"
        );
        assert!(
            close(f(&r["tops_per_watt"]), tops / tdp),
            "{id}: tops_per_watt must equal peak_tops/tdp"
        );
        assert_eq!(
            r["measured"],
            serde_json::Value::Bool(false),
            "{id}: peak is a spec, must be measured:false"
        );
        assert_eq!(
            r["published"],
            serde_json::Value::Bool(true),
            "{id}: peak must be published:true"
        );
        assert!(
            r["source"].as_str().map(|x| !x.is_empty()).unwrap_or(false),
            "{id}: peak needs a source"
        );
        by_id.insert(id, f(&r["tops_per_dollar"]));
    }
    for need in ["i9_14900k", "wormhole_n150", "wormhole_n300"] {
        assert!(by_id.contains_key(need), "frontier missing device `{need}`");
    }

    // The frontier claim: both Tenstorrent cards dominate the CPU on TOPS/$ by a wide margin.
    let cpu = by_id["i9_14900k"];
    for tt in ["wormhole_n150", "wormhole_n300"] {
        assert!(
            by_id[tt] > cpu * 50.0,
            "{tt} TOPS/$ ({}) must dominate the CPU ({cpu}) by >50x",
            by_id[tt]
        );
    }
    assert!(
        d["headline"]["best_tops_per_dollar"]
            .as_str()
            .unwrap_or("")
            .starts_with("wormhole"),
        "the best TOPS/$ device must be a Tenstorrent card"
    );
    assert!(f(&d["headline"]["tt_vs_cpu_tops_per_dollar_ratio"]) > 50.0);

    // Workload layer: i9 MEASURED, every Tenstorrent number PROJECTED — nothing on-silicon faked.
    let wl = &d["workload_projection"];
    if wl["available"].as_bool().unwrap_or(false) {
        assert_eq!(
            wl["i9_measured"]["measured"],
            serde_json::Value::Bool(true),
            "i9 workload must be measured"
        );
        assert_eq!(
            wl["tt_babycore_projected"]["projected"],
            serde_json::Value::Bool(true),
            "the baby-core throughput must be projected"
        );
        // No Tenstorrent workload object may claim to be measured.
        for (k, v) in wl.as_object().unwrap() {
            if k.starts_with("tt_") {
                assert_ne!(
                    v["measured"],
                    serde_json::Value::Bool(true),
                    "Tenstorrent workload `{k}` must NOT be flagged measured (no card yet)"
                );
                assert_eq!(
                    v["projected"],
                    serde_json::Value::Bool(true),
                    "Tenstorrent workload `{k}` must be projected:true"
                );
            }
        }
    }

    // The one-pager SVG exists and is non-trivial.
    let svg =
        std::fs::read_to_string(root.join("benchmarks/results/charts/tenstorrent_pareto.svg"))
            .expect("read tenstorrent_pareto.svg");
    assert!(
        svg.len() > 400 && svg.contains("Tenstorrent"),
        "SVG one-pager missing/empty"
    );
}
