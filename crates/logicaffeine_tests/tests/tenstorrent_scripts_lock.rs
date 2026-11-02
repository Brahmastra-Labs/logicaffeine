//! ★ TENSTORRENT SCRIPTS LOCK (Phases 0 & 5) ★ — the build + measurement harness exists and carries
//! the steps that make the tier reproducible on a real card. Guards against silent deletion or gutting
//! of the "ready the moment a card lands" path.

use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn read(rel: &str) -> String {
    let p = repo_root().join(rel);
    std::fs::read_to_string(&p).unwrap_or_else(|e| panic!("read {}: {e}", p.display()))
}

#[test]
fn tt_build_script_is_present_and_complete() {
    let s = read("scripts/accel/tt-build.sh");
    for needle in [
        "emit tenstorrent",
        "TT_METAL_HOME",
        "git clone",
        "cmake",
        "logos_oracle.c",
    ] {
        assert!(s.contains(needle), "tt-build.sh missing `{needle}`");
    }
}

#[test]
fn measure_script_refuses_without_a_card_and_regenerates_the_frontier() {
    let s = read("scripts/accel/measure-tenstorrent-pareto.sh");
    // Refuses without a card (no fabricated measurement) and folds the MEASURED layer back into the frontier.
    for needle in [
        "tt-smi",
        "1e52",
        "REFUSING",
        "gen-pareto-frontier.py",
        "measured",
    ] {
        assert!(
            s.contains(needle),
            "measure-tenstorrent-pareto.sh missing `{needle}`"
        );
    }
}

#[test]
fn power_specs_are_published_not_measured() {
    let d: serde_json::Value =
        serde_json::from_str(&read("benchmarks/silicon/tenstorrent_power.json"))
            .expect("parse tenstorrent_power.json");
    let devices = d["devices"].as_array().expect("devices array");
    let ids: Vec<&str> = devices.iter().map(|x| x["id"].as_str().unwrap()).collect();
    for need in ["i9_14900k", "wormhole_n150", "wormhole_n300"] {
        assert!(ids.contains(&need), "power specs missing `{need}`");
    }
    // Every peak is published, none is measured, and each carries a source.
    for dev in devices {
        let id = dev["id"].as_str().unwrap();
        assert_eq!(
            dev["peak_measured"],
            serde_json::Value::Bool(false),
            "{id} peak must be measured:false"
        );
        assert!(
            dev["peak_source"]
                .as_str()
                .map(|s| !s.is_empty())
                .unwrap_or(false),
            "{id} needs a peak_source"
        );
    }
    // The projection model states its parameters (honesty of the projected layer).
    let m = &d["projection_model"];
    for k in [
        "baby_core_clock_ghz",
        "i9_pcore_clock_ghz",
        "ipc_ratio_babycore_vs_i9",
        "compute_baby_cores_per_card",
    ] {
        assert!(!m[k].is_null(), "projection_model missing `{k}`");
    }
}
