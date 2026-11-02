//! sky130_corner_coverage_lock — "worst-case across every corner" must be TRUE of the recorder.
//!
//! WHY THIS EXISTS. `record-sky130-signoff.py` documented that timing is read "across ALL STA
//! corners (worst-case), so an optimistic nominal-corner report can never mask a failing slow
//! corner". It read only the TOP-LEVEL `reports/signoff/*rcx_sta.*.rpt` — the typical corner. The
//! real multi-corner STA lives one directory down, in `reports/signoff/NN-mca/rcx_{min,nom,max}_sta.*`
//! (library corners Slowest = ss_100C_1v60, Typical, Fastest = ff_n40C_1v95), and `multi_corner`
//! only checked that an `mca` directory EXISTED.
//!
//! MEASURED 2026-09-23, every block matched to its run by exact GDS byte size: of 18 blocks recorded
//! `signoff_clean: true`, 16 fail setup at the slow corner. `fabric_soc` recorded setup +52.46 ns;
//! its slowest corner reads -121.40 ns on a 300 ns clock. Only `soft_core_scalar` (+0.14) and
//! `decision_hft_bist` (+9.69) closed.
//!
//! This lock builds a run directory where the typical corner passes and the slow corner fails, and
//! requires the recorder's own extractors to report the FAILURE.

#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};
use std::process::Command;

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn summary(setup: f64, hold: f64) -> String {
    format!(
        "\n===\nreport_tns\n===\ntns 0.00\n\n===\nreport_wns\n===\nwns 0.00\n\n===\n\
         report_worst_slack -max (Setup)\n===\nworst slack {setup:.2}\n\n===\n\
         report_worst_slack -min (Hold)\n===\nworst slack {hold:.2}\n"
    )
}

fn checks(slew: u32, fanout: u32, cap: u32) -> String {
    format!(
        "max slew violation count {slew}\nmax fanout violation count {fanout}\nmax cap violation count {cap}\n"
    )
}

/// A signoff dir: typical corner at the top level, the three RC-corner multi-corner runs in
/// `NN-mca/` subdirectories, exactly as OpenLane v1 lays them out.
fn fixture(name: &str, corners: &[(&str, f64, f64, u32)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("corner_lock_{}_{name}", std::process::id()));
    let sig = dir.join("reports/signoff");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&sig).unwrap();
    std::fs::write(sig.join("36-rcx_sta.summary.rpt"), summary(5.00, 0.30)).unwrap();
    std::fs::write(sig.join("36-rcx_sta.checks.rpt"), checks(0, 0, 0)).unwrap();
    std::fs::write(sig.join("36-rcx_sta.max.rpt"), "slack (MET)\n").unwrap();
    std::fs::write(sig.join("36-rcx_sta.min.rpt"), "slack (MET)\n").unwrap();
    for (i, (rc, setup, hold, slew)) in corners.iter().enumerate() {
        let mca = sig.join(format!("{}-mca", 30 + 2 * i));
        std::fs::create_dir_all(&mca).unwrap();
        std::fs::write(
            mca.join(format!("rcx_{rc}_sta.summary.rpt")),
            summary(*setup, *hold),
        )
        .unwrap();
        std::fs::write(
            mca.join(format!("rcx_{rc}_sta.checks.rpt")),
            checks(*slew, 3, 0),
        )
        .unwrap();
        let viol = if *setup < 0.0 {
            "slack (VIOLATED)\nslack (VIOLATED)\n"
        } else {
            "slack (MET)\n"
        };
        std::fs::write(mca.join(format!("rcx_{rc}_sta.max.rpt")), viol).unwrap();
        std::fs::write(mca.join(format!("rcx_{rc}_sta.min.rpt")), "slack (MET)\n").unwrap();
    }
    sig
}

/// Run the recorder's extractors on a signoff dir and return their JSON.
fn extract(sig: &Path) -> serde_json::Value {
    let script = root().join("scripts/record-sky130-signoff.py");
    let prog = format!(
        "import importlib.util,json,sys\n\
         s=importlib.util.spec_from_file_location('r',{script:?}); r=importlib.util.module_from_spec(s); s.loader.exec_module(r)\n\
         sig=sys.argv[1]; setup,hold=r.violated_paths(sig)\n\
         print(json.dumps({{**r.sta_slacks(sig), **r.check_types(sig), 'multi_corner': r.multi_corner(sig), \
         'setup_violated_paths': setup, 'hold_violated_paths': hold}}))",
        script = script.display().to_string()
    );
    let out = Command::new("python3")
        .arg("-c")
        .arg(&prog)
        .arg(sig)
        .output()
        .expect("python3");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    serde_json::from_slice(&out.stdout).expect("extractor json")
}

#[test]
fn a_failing_slow_corner_is_never_masked_by_a_passing_typical_one() {
    let sig = fixture(
        "slow_fails",
        &[
            ("min", -1.00, 0.10, 4),
            ("nom", -2.00, 0.12, 9),
            ("max", -3.25, -0.05, 17),
        ],
    );
    let v = extract(&sig);
    assert_eq!(
        v["setup_worst_slack_ns"],
        serde_json::json!(-3.25),
        "setup must be the WORST corner (the ss/max-RC -3.25), not the typical +5.00: {v}"
    );
    assert_eq!(
        v["hold_worst_slack_ns"],
        serde_json::json!(-0.05),
        "hold must be the worst corner: {v}"
    );
    assert_eq!(
        v["max_slew_violations"],
        serde_json::json!(17),
        "slew must be the worst corner's count: {v}"
    );
    assert_eq!(
        v["max_fanout_violations"],
        serde_json::json!(3),
        "fanout must be the worst corner's count: {v}"
    );
    assert_eq!(
        v["setup_violated_paths"],
        serde_json::json!(2),
        "violated setup paths must be counted in the corner reports too: {v}"
    );
    assert_eq!(
        v["multi_corner"],
        serde_json::json!(true),
        "all three RC corners are present: {v}"
    );
}

#[test]
fn multi_corner_means_every_rc_corner_reported_not_a_directory_existing() {
    let sig = fixture("missing_max", &[("min", 1.0, 0.1, 0), ("nom", 1.0, 0.1, 0)]);
    assert_eq!(
        extract(&sig)["multi_corner"],
        serde_json::json!(false),
        "with the max-RC corner's summary missing, multi-corner STA did not complete"
    );
    let sig = fixture("empty_mca", &[]);
    std::fs::create_dir_all(sig.join("33-mca")).unwrap();
    assert_eq!(
        extract(&sig)["multi_corner"],
        serde_json::json!(false),
        "an empty mca directory is not multi-corner evidence"
    );
}

#[test]
fn a_block_passing_every_corner_still_reads_as_passing() {
    let sig = fixture(
        "all_pass",
        &[
            ("min", 2.0, 0.2, 0),
            ("nom", 1.5, 0.15, 0),
            ("max", 0.75, 0.11, 0),
        ],
    );
    let v = extract(&sig);
    assert_eq!(v["setup_worst_slack_ns"], serde_json::json!(0.75), "{v}");
    assert_eq!(v["hold_worst_slack_ns"], serde_json::json!(0.11), "{v}");
    assert_eq!(v["max_slew_violations"], serde_json::json!(0), "{v}");
    assert_eq!(v["setup_violated_paths"], serde_json::json!(0), "{v}");
}

/// The recorder strips binary-float NOISE from metrics without discarding real PRECISION.
///
/// OpenLane writes `4.140000000000001` for rev_alu_core's critical path; every document reciting a
/// ledger number would have had to recite the noise. The first fix rounded to 6 decimal places and
/// moved a real die area of 0.037064104425 mm^2 by 4e-7 — and the provenance verifier (1e-9) then
/// rejected 16 true records at once. Twelve SIGNIFICANT digits removes the one and keeps the other.
#[test]
fn recorded_metrics_lose_float_noise_but_keep_real_precision() {
    let dir = std::env::temp_dir().join(format!("rec_round_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("reports")).unwrap();
    std::fs::create_dir_all(dir.join("results/final/gds")).unwrap();
    std::fs::write(dir.join("reports/metrics.csv"),
        "flow_status,critical_path_ns,DIEAREA_mm^2\nflow completed,4.140000000000001,0.03706410442500001\n").unwrap();
    std::fs::write(dir.join("results/final/gds/blk.gds"), b"x").unwrap();
    let script = root().join("scripts/record-sky130-signoff.py");
    let prog = format!(
        "import importlib.util,json,sys\n\
         s=importlib.util.spec_from_file_location('r',{script:?}); r=importlib.util.module_from_spec(s); s.loader.exec_module(r)\n\
         t=r.top_line(sys.argv[1],'blk'); print(json.dumps([t['critical_path_ns'], t['die_area_mm2']]))",
        script = script.display().to_string());
    let out = Command::new("python3")
        .arg("-c")
        .arg(&prog)
        .arg(&dir)
        .output()
        .expect("python3");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).unwrap();
    assert_eq!(
        v[0],
        serde_json::json!(4.14),
        "the float noise must be gone: {v}"
    );
    assert!(
        (v[1].as_f64().unwrap() - 0.037064104425).abs() < 1e-12,
        "real precision must be kept: {v}"
    );
}
