//! ★ LOCK — `benchmarks/compare-runs.py`, the verdict on a benchmark run against the runs before it. ★
//!
//! A run is only useful if a reader can tell, without squinting at thirty-five tables, whether LOGOS got slower. This
//! lock drives the comparator over synthetic result directories (the three files `run.sh` writes: `latest.json`,
//! `latest-interp.json`, `latest-codec.json`) and pins its judgement:
//!
//! - **Same chip** (equal `metadata.cpu`): a LOGOS row regresses only when it is slower beyond the noise band *and* its
//!   speed relative to the suite's yardstick (C, Node, postcard) fell beyond the band, so a machine that got slower
//!   across the board reports drift instead of a regression. The band is the tolerance or three times the combined
//!   coefficient of variation, whichever is wider. A row that had a timing and lost it regresses.
//! - **Another chip**: only yardstick-normalized ratios are compared; a row outside the cross-chip tolerance is a
//!   finding that never fails the run, and the chip's own speed against the baseline is reported.
//! - **Codec sizes** are deterministic: a changed encoded size at the same commit fails the run.
//!
//! Exit 0 is a pass, 1 a regression, 2 unusable input. The real published results compared with themselves pass with
//! no findings, which keeps the comparator honest against the schema `run.sh` actually writes.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test benchmark_compare_runs_lock`

#![cfg(all(unix, not(target_arch = "wasm32")))]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{json, Value};
use tempfile::TempDir;

const I9: &str = "Intel(R) Core(TM) i9-14900K";
const HASWELL: &str = "Intel(R) Core(TM) i7-4790K CPU @ 4.00GHz";

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn timing(mean_ms: f64, cv: f64) -> Value {
    json!({
        "mean_ms": mean_ms, "median_ms": mean_ms, "stddev_ms": mean_ms * cv, "cv": cv,
        "min_ms": mean_ms * (1.0 - 2.0 * cv), "max_ms": mean_ms * (1.0 + 2.0 * cv), "runs": 10,
    })
}

/// A results directory as `run.sh` leaves it; each suite is edited in place before it is written.
struct Run {
    main: Value,
    interp: Option<Value>,
    codec: Option<Value>,
}

const MAIN_ROWS: &[(&str, bool, &str, &[(&str, f64)])] = &[
    ("fib", true, "40", &[("c", 100.0), ("logos_release", 50.0), ("logos_c", 80.0), ("logos_jit", 150.0), ("js", 300.0)]),
    ("sieve", false, "1000", &[("c", 200.0), ("logos_release", 180.0), ("logos_c", 210.0), ("logos_jit", 400.0), ("js", 500.0)]),
    ("nbody", false, "500", &[("c", 300.0), ("logos_release", 290.0), ("logos_c", 320.0), ("logos_jit", 900.0), ("js", 700.0)]),
];

const INTERP_ROWS: &[(&str, &str, &[(&str, f64)])] = &[
    ("fib", "38", &[("logos_interp", 14.0), ("logos_tiered", 340.0), ("js", 200.0)]),
    ("sieve", "100000", &[("logos_interp", 80.0), ("logos_tiered", 60.0), ("js", 50.0)]),
];

const CODEC_ROWS: &[(&str, &[(&str, u64, f64, f64)])] = &[
    ("ints", &[("postcard", 3000, 1000.0, 1200.0), ("json", 7000, 9000.0, 11000.0), ("logos (varint)", 3231, 1992.0, 1749.0)]),
    ("strings", &[("postcard", 9000, 4000.0, 5000.0), ("messagepack", 9500, 6000.0, 7000.0), ("logos (varint)", 8800, 3500.0, 4200.0)]),
];

impl Run {
    fn new(cpu: &str, commit: &str) -> Run {
        let metadata = json!({
            "date": "2026-11-01T23:51:18Z", "commit": commit, "logos_version": "0.11.0", "cpu": cpu,
            "os": "Ubuntu 24.04 x86_64", "kernel": "6.8.0", "warmup": 2, "runs": 10,
            "versions": {"c": "gcc 13.3.0", "rust": "rustc 1.96.0", "js": "v22.22.3"},
            "environment": {"governor": "performance", "pinning": {"cpus": "0-7", "source": "all-online"},
                            "quiet_box": {"gate": "passed", "waited_seconds": 0}},
        });
        let benchmarks: Vec<Value> = MAIN_ROWS
            .iter()
            .map(|(id, collapse, n, rows)| {
                let row: serde_json::Map<String, Value> =
                    rows.iter().map(|(lang, ms)| (lang.to_string(), timing(*ms, 0.01))).collect();
                json!({
                    "id": id, "name": id, "reference_size": n, "sizes": [n],
                    "collapse": if *collapse { json!("Collapsed: memoized.") } else { Value::Null },
                    "scaling": {*n: row}, "timed_out": {},
                })
            })
            .collect();
        let interp: Vec<Value> = INTERP_ROWS
            .iter()
            .map(|(id, n, rows)| {
                let row: serde_json::Map<String, Value> =
                    rows.iter().map(|(lang, ms)| (lang.to_string(), timing(*ms, 0.01))).collect();
                json!({"id": id, "name": id, "reference_size": n, "sizes": [n], "scaling": {*n: row}})
            })
            .collect();
        let scenarios: Vec<Value> = CODEC_ROWS
            .iter()
            .map(|(id, rows)| {
                let rows: Vec<Value> = rows
                    .iter()
                    .map(|(codec, size, enc, dec)| {
                        json!({"codec": codec, "size": size, "enc_ns": enc, "dec_ns": dec, "read_one_ns": null})
                    })
                    .collect();
                json!({"id": id, "title": id, "kind": "fair", "rows": rows})
            })
            .collect();
        Run {
            main: json!({"schema_version": 1, "metadata": metadata.clone(), "benchmarks": benchmarks}),
            interp: Some(json!({"schema_version": 1, "metadata": metadata.clone(), "benchmarks": interp})),
            codec: Some(json!({"schema_version": 2, "metadata": metadata, "iters": 20000, "scenarios": scenarios})),
        }
    }

    fn main_row(&mut self, bench: &str) -> &mut serde_json::Map<String, Value> {
        let b = self.main["benchmarks"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|b| b["id"] == bench)
            .unwrap();
        let n = b["reference_size"].as_str().unwrap().to_string();
        b["scaling"][&n].as_object_mut().unwrap()
    }

    /// Multiply one main-suite timing.
    fn scale(mut self, bench: &str, lang: &str, by: f64) -> Run {
        let t = self.main_row(bench).get_mut(lang).unwrap();
        let mean = t["mean_ms"].as_f64().unwrap() * by;
        let cv = t["cv"].as_f64().unwrap();
        *t = timing(mean, cv);
        self
    }

    /// Multiply every timing of every suite: the same build on a faster or slower machine.
    fn scale_all(mut self, by: f64) -> Run {
        for b in self.main["benchmarks"].as_array_mut().unwrap().iter_mut().chain(
            self.interp.as_mut().map(|i| i["benchmarks"].as_array_mut().unwrap().iter_mut()).into_iter().flatten(),
        ) {
            for row in b["scaling"].as_object_mut().unwrap().values_mut() {
                for t in row.as_object_mut().unwrap().values_mut() {
                    let (mean, cv) = (t["mean_ms"].as_f64().unwrap() * by, t["cv"].as_f64().unwrap());
                    *t = timing(mean, cv);
                }
            }
        }
        if let Some(codec) = self.codec.as_mut() {
            for s in codec["scenarios"].as_array_mut().unwrap() {
                for r in s["rows"].as_array_mut().unwrap() {
                    for k in ["enc_ns", "dec_ns"] {
                        r[k] = json!(r[k].as_f64().unwrap() * by);
                    }
                }
            }
        }
        self
    }

    fn noisy(mut self, bench: &str, cv: f64) -> Run {
        for t in self.main_row(bench).values_mut() {
            let mean = t["mean_ms"].as_f64().unwrap();
            *t = timing(mean, cv);
        }
        self
    }

    fn drop_lang(mut self, bench: &str, lang: &str) -> Run {
        self.main_row(bench).remove(lang);
        self
    }

    fn codec_row(mut self, scenario: &str, codec: &str, edit: impl Fn(&mut Value)) -> Run {
        let s = self.codec.as_mut().unwrap()["scenarios"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|s| s["id"] == scenario)
            .unwrap();
        let r = s["rows"].as_array_mut().unwrap().iter_mut().find(|r| r["codec"] == codec).unwrap();
        edit(r);
        self
    }

    fn interp_scale(mut self, bench: &str, lang: &str, by: f64) -> Run {
        let b = self.interp.as_mut().unwrap()["benchmarks"]
            .as_array_mut()
            .unwrap()
            .iter_mut()
            .find(|b| b["id"] == bench)
            .unwrap();
        let n = b["reference_size"].as_str().unwrap().to_string();
        let t = &mut b["scaling"][&n][lang];
        let (mean, cv) = (t["mean_ms"].as_f64().unwrap() * by, t["cv"].as_f64().unwrap());
        *t = timing(mean, cv);
        self
    }

    fn write(&self, dir: &Path) -> PathBuf {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join("latest.json"), serde_json::to_string_pretty(&self.main).unwrap()).unwrap();
        if let Some(i) = &self.interp {
            std::fs::write(dir.join("latest-interp.json"), i.to_string()).unwrap();
        }
        if let Some(c) = &self.codec {
            std::fs::write(dir.join("latest-codec.json"), c.to_string()).unwrap();
        }
        dir.to_path_buf()
    }
}

struct Verdict {
    out: Output,
    json: Value,
    report: String,
}

impl Verdict {
    fn code(&self) -> i32 {
        self.out.status.code().expect("exit code")
    }

    fn comparison(&self, i: usize) -> &Value {
        &self.json["comparisons"][i]
    }

    fn findings(&self, i: usize, suite: &str) -> Vec<Value> {
        self.comparison(i)["suites"][suite]["findings"].as_array().cloned().unwrap_or_default()
    }

    fn finding(&self, i: usize, suite: &str, case: &str, lang: &str) -> Option<Value> {
        self.findings(i, suite).into_iter().find(|f| f["case"] == case && f["lang"] == lang)
    }

    fn explain(&self) -> String {
        format!(
            "exit {:?}\nstdout:\n{}\nstderr:\n{}",
            self.out.status.code(),
            String::from_utf8_lossy(&self.out.stdout),
            String::from_utf8_lossy(&self.out.stderr)
        )
    }
}

fn compare(candidate: &Path, against: &[&Path], extra: &[&str]) -> Verdict {
    let tmp = TempDir::new().unwrap();
    let json_out = tmp.path().join("verdict.json");
    let report_out = tmp.path().join("report.md");
    let mut cmd = Command::new("python3");
    cmd.arg(repo_root().join("benchmarks/compare-runs.py")).arg(candidate);
    for a in against {
        cmd.arg("--against").arg(a);
    }
    cmd.arg("--json").arg(&json_out).arg("--report").arg(&report_out).args(extra);
    let out = cmd.output().expect("run compare-runs.py");
    let json = std::fs::read_to_string(&json_out)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(Value::Null);
    let report = std::fs::read_to_string(&report_out).unwrap_or_default();
    Verdict { out, json, report }
}

fn pair(baseline: Run, candidate: Run) -> (TempDir, PathBuf, PathBuf) {
    let tmp = TempDir::new().unwrap();
    let b = baseline.write(&tmp.path().join("baseline"));
    let c = candidate.write(&tmp.path().join("candidate"));
    (tmp, b, c)
}

#[test]
fn identical_runs_on_one_chip_pass_with_nothing_to_report() {
    let (_t, b, c) = pair(Run::new(I9, "aaa"), Run::new(I9, "bbb"));
    let v = compare(&c, &[&b], &[]);
    assert_eq!(v.code(), 0, "{}", v.explain());
    assert_eq!(v.json["verdict"], "pass");
    assert_eq!(v.comparison(0)["kind"], "same_chip");
    for suite in ["main", "interp", "codec"] {
        assert_eq!(v.comparison(0)["suites"][suite]["status"], "compared", "{suite}");
        assert!(v.findings(0, suite).is_empty(), "{suite}: {:?}", v.findings(0, suite));
    }
    assert!(v.report.contains("PASS"), "{}", v.report);
}

#[test]
fn a_logos_row_slower_beyond_noise_on_the_same_chip_is_a_regression_named_in_the_report() {
    let (_t, b, c) = pair(Run::new(I9, "aaa"), Run::new(I9, "bbb").scale("sieve", "logos_release", 1.20));
    let v = compare(&c, &[&b], &[]);
    assert_eq!(v.code(), 1, "{}", v.explain());
    assert_eq!(v.json["verdict"], "regression");
    let f = v.finding(0, "main", "sieve", "logos_release").expect("sieve/logos_release finding");
    assert_eq!(f["class"], "regressed");
    assert_eq!(f["blocking"], true);
    assert_eq!(f["size"], "1000");
    let pct = f["change_pct"].as_f64().unwrap();
    assert!((pct - 20.0).abs() < 0.01, "change_pct {pct}");
    assert_eq!(v.findings(0, "main").len(), 1, "only the slowed row: {:?}", v.findings(0, "main"));
    assert!(v.report.contains("REGRESSION") && v.report.contains("sieve") && v.report.contains("logos_release"));
}

#[test]
fn a_slowdown_inside_the_tolerance_is_noise() {
    let (_t, b, c) = pair(Run::new(I9, "aaa"), Run::new(I9, "bbb").scale("sieve", "logos_release", 1.03));
    let v = compare(&c, &[&b], &[]);
    assert_eq!(v.code(), 0, "{}", v.explain());
    assert!(v.findings(0, "main").is_empty(), "{:?}", v.findings(0, "main"));
}

#[test]
fn the_noise_band_widens_with_the_runs_own_variation() {
    let base = Run::new(I9, "aaa").noisy("nbody", 0.06);
    let cand = Run::new(I9, "bbb").noisy("nbody", 0.06).scale("nbody", "logos_release", 1.20);
    let (_t, b, c) = pair(base, cand);
    let v = compare(&c, &[&b], &[]);
    assert_eq!(v.code(), 0, "a 20% move inside a 25% band is noise: {}", v.explain());
    assert!(v.finding(0, "main", "nbody", "logos_release").is_none());
}

#[test]
fn the_tolerance_is_a_knob() {
    let base = Run::new(I9, "aaa").noisy("sieve", 0.002);
    let cand = Run::new(I9, "bbb").noisy("sieve", 0.002).scale("sieve", "logos_release", 1.03);
    let (_t, b, c) = pair(base, cand);
    let v = compare(&c, &[&b], &["--tolerance", "2"]);
    assert_eq!(v.code(), 1, "{}", v.explain());
    assert_eq!(v.finding(0, "main", "sieve", "logos_release").unwrap()["class"], "regressed");
}

#[test]
fn a_machine_that_slowed_down_across_the_board_reports_drift_not_a_regression() {
    let (_t, b, c) = pair(Run::new(I9, "aaa"), Run::new(I9, "bbb").scale_all(1.25));
    let v = compare(&c, &[&b], &[]);
    assert_eq!(v.code(), 0, "{}", v.explain());
    let f = v.finding(0, "main", "sieve", "logos_release").expect("drift finding");
    assert_eq!(f["class"], "drift");
    assert_eq!(f["blocking"], false);
    let speed = v.comparison(0)["suites"]["main"]["chip_factor"].as_f64().unwrap();
    assert!((speed - 0.8).abs() < 0.001, "yardstick ran at 1/1.25 of the baseline: {speed}");
}

#[test]
fn a_logos_row_that_got_faster_relative_to_c_is_an_improvement() {
    let (_t, b, c) = pair(Run::new(I9, "aaa"), Run::new(I9, "bbb").scale("fib", "logos_jit", 0.5));
    let v = compare(&c, &[&b], &[]);
    assert_eq!(v.code(), 0, "{}", v.explain());
    let f = v.finding(0, "main", "fib", "logos_jit").unwrap();
    assert_eq!(f["class"], "improved");
    assert_eq!(f["blocking"], false);
}

#[test]
fn only_logos_rows_are_judged() {
    let (_t, b, c) = pair(Run::new(I9, "aaa"), Run::new(I9, "bbb").scale("sieve", "js", 1.5));
    let v = compare(&c, &[&b], &[]);
    assert_eq!(v.code(), 0, "{}", v.explain());
    assert!(v.finding(0, "main", "sieve", "js").is_none());
}

#[test]
fn a_logos_row_that_lost_its_timing_on_the_same_chip_is_a_regression() {
    let (_t, b, c) = pair(Run::new(I9, "aaa"), Run::new(I9, "bbb").drop_lang("nbody", "logos_jit"));
    let v = compare(&c, &[&b], &[]);
    assert_eq!(v.code(), 1, "{}", v.explain());
    let f = v.finding(0, "main", "nbody", "logos_jit").unwrap();
    assert_eq!(f["class"], "lost");
    assert_eq!(f["blocking"], true);
}

#[test]
fn a_row_only_the_candidate_has_is_reported_as_new() {
    let (_t, b, c) = pair(Run::new(I9, "aaa").drop_lang("nbody", "logos_jit"), Run::new(I9, "bbb"));
    let v = compare(&c, &[&b], &[]);
    assert_eq!(v.code(), 0, "{}", v.explain());
    assert_eq!(v.finding(0, "main", "nbody", "logos_jit").unwrap()["class"], "new");
}

#[test]
fn another_chip_that_is_uniformly_slower_is_in_line_and_its_speed_is_measured() {
    let (_t, b, c) = pair(Run::new(I9, "aaa"), Run::new(HASWELL, "aaa").scale_all(1.6));
    let v = compare(&c, &[&b], &[]);
    assert_eq!(v.code(), 0, "{}", v.explain());
    assert_eq!(v.comparison(0)["kind"], "cross_chip");
    for suite in ["main", "interp", "codec"] {
        assert!(v.findings(0, suite).is_empty(), "{suite}: {:?}", v.findings(0, suite));
        let speed = v.comparison(0)["suites"][suite]["chip_factor"].as_f64().unwrap();
        assert!((speed - 0.625).abs() < 0.001, "{suite}: {speed}");
    }
    assert!(v.report.contains(HASWELL) && v.report.contains(I9), "{}", v.report);
}

#[test]
fn on_another_chip_a_tier_out_of_line_with_the_reference_is_a_finding_that_does_not_fail_the_run() {
    let (_t, b, c) = pair(Run::new(I9, "aaa"), Run::new(HASWELL, "aaa").scale_all(1.6).scale("nbody", "logos_c", 1.4));
    let v = compare(&c, &[&b], &[]);
    assert_eq!(v.code(), 0, "{}", v.explain());
    let f = v.finding(0, "main", "nbody", "logos_c").unwrap();
    assert_eq!(f["class"], "out_of_line_slower");
    assert_eq!(f["blocking"], false);
    assert_eq!(v.findings(0, "main").len(), 1, "{:?}", v.findings(0, "main"));
}

#[test]
fn on_another_chip_a_lost_row_is_reported_without_failing() {
    let (_t, b, c) = pair(Run::new(I9, "aaa"), Run::new(HASWELL, "aaa").scale_all(1.6).drop_lang("fib", "logos_jit"));
    let v = compare(&c, &[&b], &[]);
    assert_eq!(v.code(), 0, "{}", v.explain());
    let f = v.finding(0, "main", "fib", "logos_jit").unwrap();
    assert_eq!(f["class"], "lost");
    assert_eq!(f["blocking"], false);
}

#[test]
fn the_tier_summary_carries_both_geometric_means_for_every_logos_tier() {
    let (_t, b, c) = pair(Run::new(I9, "aaa"), Run::new(I9, "bbb"));
    let v = compare(&c, &[&b], &[]);
    let tiers = &v.comparison(0)["suites"]["main"]["tiers"];
    for lang in ["logos_release", "logos_c", "logos_jit"] {
        let all = tiers["all"][lang]["candidate"].as_f64().unwrap();
        let same = tiers["same_algorithm"][lang]["candidate"].as_f64().unwrap();
        assert_eq!(tiers["all"][lang]["baseline"].as_f64().unwrap(), all);
        assert!(all > 0.0 && same > 0.0, "{lang}");
    }
    let expected_release = ((100.0_f64 / 50.0) * (200.0 / 180.0) * (300.0 / 290.0)).powf(1.0 / 3.0);
    let got = tiers["all"]["logos_release"]["candidate"].as_f64().unwrap();
    assert!((got - expected_release).abs() < 1e-3, "{got} vs {expected_release}");
    let expected_same = ((200.0_f64 / 180.0) * (300.0 / 290.0)).sqrt();
    let got = tiers["same_algorithm"]["logos_release"]["candidate"].as_f64().unwrap();
    assert!((got - expected_same).abs() < 1e-3, "the collapsed fib is left out: {got} vs {expected_same}");
    assert!(tiers["all"].get("c").is_none() && tiers["all"].get("js").is_none(), "only LOGOS tiers: {tiers}");
}

#[test]
fn the_interpreter_suite_is_judged_against_node() {
    let (_t, b, c) = pair(Run::new(I9, "aaa"), Run::new(I9, "bbb").interp_scale("sieve", "logos_tiered", 1.3));
    let v = compare(&c, &[&b], &[]);
    assert_eq!(v.code(), 1, "{}", v.explain());
    let f = v.finding(0, "interp", "sieve", "logos_tiered").unwrap();
    assert_eq!(f["class"], "regressed");
    assert_eq!(v.comparison(0)["suites"]["interp"]["yardstick"], "js");
}

#[test]
fn the_codec_suite_is_judged_per_operation_against_postcard() {
    let (_t, b, c) = pair(
        Run::new(I9, "aaa"),
        Run::new(I9, "bbb").codec_row("strings", "logos (varint)", |r| r["dec_ns"] = json!(4200.0 * 1.5)),
    );
    let v = compare(&c, &[&b], &[]);
    assert_eq!(v.code(), 1, "{}", v.explain());
    assert_eq!(v.comparison(0)["suites"]["codec"]["yardstick"], "postcard");
    let f = v.finding(0, "codec", "strings/dec", "logos (varint)").unwrap();
    assert_eq!(f["class"], "regressed");
    assert!(v.finding(0, "codec", "strings/enc", "logos (varint)").is_none());
}

#[test]
fn a_changed_encoded_size_at_the_same_commit_fails_on_any_chip() {
    let (_t, b, c) = pair(
        Run::new(I9, "aaa"),
        Run::new(HASWELL, "aaa").scale_all(1.6).codec_row("ints", "logos (varint)", |r| r["size"] = json!(3240)),
    );
    let v = compare(&c, &[&b], &[]);
    assert_eq!(v.code(), 1, "{}", v.explain());
    let f = v.finding(0, "codec", "ints/size", "logos (varint)").unwrap();
    assert_eq!(f["class"], "size_changed");
    assert_eq!(f["blocking"], true);
    assert_eq!(f["baseline"], 3231);
    assert_eq!(f["candidate"], 3240);
}

#[test]
fn a_changed_encoded_size_across_commits_is_reported_without_failing() {
    let (_t, b, c) = pair(
        Run::new(I9, "aaa"),
        Run::new(I9, "bbb").codec_row("ints", "logos (varint)", |r| r["size"] = json!(3100)),
    );
    let v = compare(&c, &[&b], &[]);
    assert_eq!(v.code(), 0, "{}", v.explain());
    let f = v.finding(0, "codec", "ints/size", "logos (varint)").unwrap();
    assert_eq!(f["class"], "size_changed");
    assert_eq!(f["blocking"], false);
}

#[test]
fn a_suite_the_candidate_did_not_run_is_named_not_failed() {
    let mut cand = Run::new(I9, "bbb");
    cand.interp = None;
    let (_t, b, c) = pair(Run::new(I9, "aaa"), cand);
    let v = compare(&c, &[&b], &[]);
    assert_eq!(v.code(), 0, "{}", v.explain());
    assert_eq!(v.comparison(0)["suites"]["interp"]["status"], "missing_in_candidate");
    assert_eq!(v.comparison(0)["suites"]["main"]["status"], "compared");
}

#[test]
fn every_baseline_is_its_own_comparison_and_any_blocking_finding_fails_the_run() {
    let tmp = TempDir::new().unwrap();
    let prior_same_chip = Run::new(HASWELL, "aaa").scale_all(1.6).write(&tmp.path().join("coda-prior"));
    let reference = Run::new(I9, "aaa").write(&tmp.path().join("reference"));
    let cand = Run::new(HASWELL, "bbb").scale_all(1.6).scale("sieve", "logos_release", 1.3).write(&tmp.path().join("cand"));
    let v = compare(&cand, &[&reference, &prior_same_chip], &[]);
    assert_eq!(v.code(), 1, "{}", v.explain());
    assert_eq!(v.comparison(0)["kind"], "cross_chip");
    assert_eq!(v.comparison(1)["kind"], "same_chip");
    assert_eq!(v.finding(0, "main", "sieve", "logos_release").unwrap()["blocking"], false);
    assert_eq!(v.finding(1, "main", "sieve", "logos_release").unwrap()["class"], "regressed");
}

#[test]
fn toolchain_and_environment_differences_are_in_the_verdict() {
    let mut cand = Run::new(I9, "bbb");
    cand.main["metadata"]["versions"]["c"] = json!("gcc 14.2.0");
    cand.main["metadata"]["environment"]["quiet_box"]["gate"] = json!("overridden");
    let (_t, b, c) = pair(Run::new(I9, "aaa"), cand);
    let v = compare(&c, &[&b], &[]);
    let diffs = &v.comparison(0)["toolchain_differences"];
    assert_eq!(diffs["c"]["baseline"], "gcc 13.3.0");
    assert_eq!(diffs["c"]["candidate"], "gcc 14.2.0");
    assert!(diffs.get("rust").is_none(), "{diffs}");
    let warnings = v.json["warnings"].as_array().unwrap();
    assert!(
        warnings.iter().any(|w| w.as_str().unwrap().contains("quiet")),
        "a run measured on a busy machine is called out: {warnings:?}"
    );
    assert!(v.report.contains("gcc 14.2.0"));
}

#[test]
fn a_file_path_names_its_directory_s_suites_too() {
    let (_t, b, c) = pair(Run::new(I9, "aaa"), Run::new(I9, "bbb"));
    let v = compare(&c.join("latest.json"), &[&b.join("latest.json")], &[]);
    assert_eq!(v.code(), 0, "{}", v.explain());
    assert_eq!(v.comparison(0)["suites"]["codec"]["status"], "compared");
}

#[test]
fn unusable_input_exits_2_by_name() {
    let tmp = TempDir::new().unwrap();
    let empty = tmp.path().join("empty");
    std::fs::create_dir_all(&empty).unwrap();
    let base = Run::new(I9, "aaa").write(&tmp.path().join("b"));
    let v = compare(&empty, &[&base], &[]);
    assert_eq!(v.code(), 2, "{}", v.explain());
    assert!(String::from_utf8_lossy(&v.out.stderr).contains("latest.json"), "{}", v.explain());

    std::fs::write(empty.join("latest.json"), "{not json").unwrap();
    let v = compare(&empty, &[&base], &[]);
    assert_eq!(v.code(), 2, "{}", v.explain());
}

#[test]
fn the_published_results_compared_with_themselves_pass_clean() {
    let published = repo_root().join("benchmarks/results");
    let v = compare(&published, &[&published], &[]);
    assert_eq!(v.code(), 0, "{}", v.explain());
    for suite in ["main", "interp", "codec"] {
        assert_eq!(v.comparison(0)["suites"][suite]["status"], "compared", "{suite}");
        assert!(v.findings(0, suite).is_empty(), "{suite}: {:?}", v.findings(0, suite));
    }
}

#[test]
fn the_default_baseline_is_the_published_reference_run() {
    let published = repo_root().join("benchmarks/results");
    let tmp = TempDir::new().unwrap();
    let json_out = tmp.path().join("v.json");
    let out = Command::new("python3")
        .arg(repo_root().join("benchmarks/compare-runs.py"))
        .arg(&published)
        .arg("--json")
        .arg(&json_out)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let v: Value = serde_json::from_str(&std::fs::read_to_string(&json_out).unwrap()).unwrap();
    let base = PathBuf::from(v["comparisons"][0]["baseline"]["path"].as_str().unwrap());
    assert_eq!(base.canonicalize().unwrap(), published.canonicalize().unwrap());
    assert!(String::from_utf8_lossy(&out.stdout).contains("PASS"), "the report goes to stdout too");
}
