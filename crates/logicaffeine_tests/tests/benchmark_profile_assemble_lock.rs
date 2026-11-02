//! `benchmarks/lib/profile_assemble.py` turns `scripts/bench/profile-benchmarks.sh`'s per-tier files — the trimmed
//! flamegraph trees, `perf stat -x,` CSVs and the census rows — into `latest-profile.json`, the file the /benchmarks page
//! reads for each benchmark's "under the hood" section.
//!
//! Driven here on a synthetic profile directory: counters parse from perf's CSV with and without `-r` variance and with
//! hybrid-PMU event names, an uncounted event is null rather than zero, tiers come out in registry order, the JIT carries
//! no static census, a tier without a counters file keeps its flamegraph, and every inconsistency — a census row whose
//! tree is missing, two sizes for one benchmark, an unknown tier — fails the assembly by name instead of shipping a page
//! that quietly lies.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test benchmark_profile_assemble_lock`

#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::{json, Value};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn tree(total: u64, workload: &str) -> Value {
    json!({
        "total": total,
        "frames": ["all", "main", workload],
        "kinds": ["synthetic", "compute", "compute"],
        "tree": [0, total, [[1, total, [[2, total * 9 / 10, []]]]]],
        "buckets": {"alloc_pct": 1.0, "stdout_pct": 0.0, "fmt_pct": 0.0, "output_pct": 0.0, "compute_pct": 99.0},
        "top_self": [{"frame": workload, "self_pct": 90.0}],
    })
}

const STAT_REPEATED: &str = "# started on Wed Sep 30 19:00:00 2026\n\n\
2.04,msec,task-clock,0.31%,2040000,100.00,0.999,CPUs utilized\n\
8123456789,,cpu_core/instructions/u,0.02%,2031000,100.00,3.21,insn per cycle\n\
2530000000,,cpu_core/cycles/u,0.10%,2031000,100.00,,\n\
1200000000,,cpu_core/branches/u,0.01%,2031000,100.00,,\n\
3000000,,cpu_core/branch-misses/u,1.50%,2031000,100.00,0.25,of all branches\n\
45000000,,cpu_core/cache-references/u,0.80%,2031000,100.00,,\n\
<not counted>,,cpu_core/cache-misses/u,,0,0.00,,\n";

const STAT_ONCE: &str = "# started on Wed Sep 30 19:00:00 2026\n\n\
9.50,msec,task-clock:u,19500000,100.00,0.998,CPUs utilized\n\
30000000000,,instructions:u,19500000,100.00,2.50,insn per cycle\n\
12000000000,,cycles:u,19500000,100.00,,\n";

fn row(bench: &str, lang: &str, n: &str) -> Value {
    let jit = lang == "logos_jit";
    json!({
        "bench": bench, "lang": lang, "n": n, "seconds": 2.04, "sample_hz": 2000, "stat_repeats": 5,
        "A_perf": {"hot_symbol": "work", "hot_pct": 90.0},
        "B_static": {"alloc_call_sites": if jit { Value::Null } else { json!(2) },
                     "output_call_sites": if jit { Value::Null } else { json!(1) }},
        "C_strace": {"write_calls": if jit { Value::Null } else { json!(1) },
                     "mem_write_syscall_seconds": if jit { Value::Null } else { json!(0.0001) },
                     "scaling_ratio_n_to_2n": 2.01},
    })
}

struct Fixture {
    _dir: tempfile::TempDir,
    prof: PathBuf,
    rows: PathBuf,
}

fn fixture(rows: &[Value]) -> Fixture {
    let dir = tempfile::TempDir::new().unwrap();
    let prof = dir.path().join("profile");
    for sub in ["tree", "counters"] {
        std::fs::create_dir_all(prof.join(sub)).unwrap();
    }
    for r in rows {
        let (b, l) = (r["bench"].as_str().unwrap(), r["lang"].as_str().unwrap());
        std::fs::write(
            prof.join(format!("tree/{b}_{l}.json")),
            tree(1_000_000, &format!("{b}::work")).to_string(),
        )
        .unwrap();
    }
    let rows_path = dir.path().join("profile.json");
    std::fs::write(
        &rows_path,
        json!({"generated": "profile-benchmarks.sh", "rows": rows}).to_string(),
    )
    .unwrap();
    Fixture {
        _dir: dir,
        prof,
        rows: rows_path,
    }
}

fn assemble(f: &Fixture) -> Result<Value, String> {
    assemble_text(f).map(|text| serde_json::from_str(&text).expect("the assembly is JSON"))
}

/// The assembly as printed, for what a parsed map cannot show: the order of its keys.
fn assemble_text(f: &Fixture) -> Result<String, String> {
    let out = Command::new("python3")
        .arg(repo_root().join("benchmarks/lib/profile_assemble.py"))
        .args(["--prof", path_str(&f.prof), "--rows", path_str(&f.rows)])
        .args([
            "--events",
            "task-clock,cpu_core/instructions/u,cpu_core/cycles/u",
            "--cpus",
            "0-15",
        ])
        .args(["--gate", "passed", "--paranoid", "1"])
        .output()
        .expect("python3 runs profile_assemble.py");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    if out.status.success() {
        Ok(String::from_utf8(out.stdout).expect("the assembly is UTF-8"))
    } else {
        assert!(
            stderr.starts_with("profile_assemble: ") && !stderr.contains("Traceback"),
            "a refusal is the assembler's own message, never a crash: {stderr}"
        );
        Err(stderr)
    }
}

fn path_str(p: &Path) -> &str {
    p.to_str().unwrap()
}

#[test]
fn every_tier_carries_its_tree_counters_and_census_in_registry_order() {
    let rows = [
        row("fib", "logos_jit", "40"),
        row("fib", "c", "40"),
        row("fib", "logos_release", "40"),
        row("fib", "logos_c", "40"),
    ];
    let f = fixture(&rows);
    std::fs::write(f.prof.join("counters/fib_c.csv"), STAT_REPEATED).unwrap();
    std::fs::write(f.prof.join("counters/fib_logos_jit.csv"), STAT_ONCE).unwrap();
    let text = assemble_text(&f).expect("a consistent profile directory assembles");
    let doc: Value = serde_json::from_str(&text).unwrap();

    assert_eq!(doc["schema"], 1);
    assert_eq!(doc["environment"]["cpus"], "0-15");
    assert_eq!(doc["environment"]["quiet_box"], "passed");
    let fib = &doc["benchmarks"]["fib"];
    assert_eq!(fib["n"], 40, "the one size every tier ran at, as a number");
    let mut order: Vec<(usize, &str)> = ["c", "logos_c", "logos_jit", "logos_release"]
        .into_iter()
        .map(|id| {
            (
                text.find(&format!("\"{id}\":{{\"seconds\""))
                    .unwrap_or_else(|| panic!("{id} is in the assembly")),
                id,
            )
        })
        .collect();
    order.sort();
    let order: Vec<&str> = order.into_iter().map(|(_, id)| id).collect();
    assert_eq!(
        order,
        ["c", "logos_release", "logos_c", "logos_jit"],
        "tiers in lib/languages.tsv order"
    );

    let c = &fib["tiers"]["c"];
    assert_eq!(
        c["frames"],
        json!(["all", "main", "fib::work"]),
        "the tree is carried verbatim"
    );
    assert_eq!(
        c["kinds"],
        json!(["synthetic", "compute", "compute"]),
        "and each frame's kind with it"
    );
    assert_eq!(c["tree"][1], 1_000_000);
    assert_eq!(c["buckets"]["compute_pct"], 99.0);
    assert_eq!(c["top_self"][0]["frame"], "fib::work");
    let counters = &c["counters"];
    assert_eq!(counters["instructions"]["value"], 8_123_456_789_u64);
    assert_eq!(counters["instructions"]["variance_pct"], 0.02);
    assert_eq!(counters["branch-misses"]["value"], 3_000_000);
    assert_eq!(counters["task-clock"]["value"], 2.04);
    assert_eq!(counters["task-clock"]["unit"], "msec");
    assert!(
        counters["cache-misses"]["value"].is_null(),
        "<not counted> is null, never zero: {counters}"
    );
    assert_eq!(c["census"]["alloc_call_sites"], 2);
    assert_eq!(c["census"]["write_calls"], 1);
    assert_eq!(c["scaling_n_to_2n"], 2.01);
    assert_eq!(c["seconds"], 2.04);
    assert_eq!(c["sample_hz"], 2000);

    let jit = &fib["tiers"]["logos_jit"];
    assert_eq!(
        jit["counters"]["instructions"]["value"], 30_000_000_000_u64,
        "a :u event name from a non-hybrid PMU"
    );
    assert!(
        jit["counters"]["instructions"]["variance_pct"].is_null(),
        "one run has no variance"
    );
    assert!(
        jit["census"].is_null(),
        "the JIT has no program binary to census: {jit}"
    );

    let rust = &fib["tiers"]["logos_release"];
    assert!(
        rust["counters"].is_null(),
        "no counters file: counters null, the flamegraph kept"
    );
    assert_eq!(rust["frames"][2], "fib::work");
}

#[test]
fn a_row_without_its_tree_fails_by_name() {
    let f = fixture(&[row("nbody", "c", "5000000")]);
    std::fs::remove_file(f.prof.join("tree/nbody_c.json")).unwrap();
    let err = assemble(&f).expect_err("a census row without a flamegraph must not assemble");
    assert!(
        err.contains("nbody_c.json"),
        "the error names the missing tree: {err}"
    );
}

#[test]
fn two_sizes_for_one_benchmark_fail() {
    let f = fixture(&[
        row("nbody", "c", "5000000"),
        row("nbody", "logos_c", "2500000"),
    ]);
    let err = assemble(&f).expect_err("tiers profiled at different sizes do not compare");
    assert!(
        err.contains("nbody") && err.contains("2500000"),
        "the error names the benchmark and the size: {err}"
    );
}

#[test]
fn an_unknown_tier_fails() {
    let f = fixture(&[row("nbody", "cobol", "5000000")]);
    let err =
        assemble(&f).expect_err("a tier the registry does not profile has no place on the page");
    assert!(err.contains("cobol"), "the error names the tier: {err}");
}

/// `profile-benchmarks.sh` calls the assembler with the flags it declares: every flag the call passes is one
/// `profile_assemble.py` accepts, and every flag it requires is passed. A change to either side fails here, in
/// milliseconds, rather than at the end of a profiling run hours long.
#[test]
fn the_profile_script_calls_the_assembler_with_the_flags_it_declares() {
    let script = std::fs::read_to_string(repo_root().join("scripts/bench/profile-benchmarks.sh"))
        .expect("read scripts/bench/profile-benchmarks.sh");
    let at = script
        .find("lib/profile_assemble.py\"")
        .expect("the profile script runs the assembler");
    let mut call = String::new();
    for line in script[at..].lines() {
        call.push_str(line);
        call.push(' ');
        if !line.trim_end().ends_with('\\') {
            break;
        }
    }
    let passed: std::collections::BTreeSet<String> = call
        .split_whitespace()
        .filter(|w| w.starts_with("--"))
        .map(str::to_string)
        .collect();
    let assembler = std::fs::read_to_string(repo_root().join("benchmarks/lib/profile_assemble.py"))
        .expect("read benchmarks/lib/profile_assemble.py");
    let mut declared = std::collections::BTreeSet::new();
    let mut required = std::collections::BTreeSet::new();
    for line in assembler.lines() {
        let Some(rest) = line.trim_start().strip_prefix("ap.add_argument('") else {
            continue;
        };
        let flag = rest.split('\'').next().unwrap_or("").to_string();
        if line.contains("required=True") {
            required.insert(flag.clone());
        }
        declared.insert(flag);
    }
    assert!(
        passed.contains("--prof") && required.contains("--prof"),
        "the census reads the call ({passed:?}) and the assembler's flags ({declared:?})"
    );
    let unknown: Vec<&String> = passed.difference(&declared).collect();
    let missing: Vec<&String> = required.difference(&passed).collect();
    assert!(
        unknown.is_empty() && missing.is_empty(),
        "profile-benchmarks.sh passes flags profile_assemble.py does not accept: {unknown:?}; and omits flags it \
         requires: {missing:?}"
    );
}
