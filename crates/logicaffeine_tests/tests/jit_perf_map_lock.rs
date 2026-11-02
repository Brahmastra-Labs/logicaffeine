//! The JIT names its machine code, and the engine trace says what actually ran.
//!
//! `LOGOS_PERF_MAP=1 largo run --interpret` writes `/tmp/perf-<pid>.map`, the file Linux `perf` reads
//! to symbolize a JIT's anonymous executable memory: one `START SIZE name` line per compiled function
//! or loop, named after the LOGOS source. `LOGOS_ENGINE_TRACE=1` prints `logos-engine: vm+jit` only
//! when the native tier is attached, and a closing `logos-engine: native-tier exodia: …` line with what
//! it compiled — the proof a benchmark run measured the JIT and not the bytecode loop.
//!
//! `LOGOS_JIT_CODE=<dir>` writes the machine code itself: each extent's bytes to `<dir>/<n>.bin`, listed in
//! `<dir>/index.tsv` as `n START SIZE name` — the same extents and names as the map, for a disassembler.
//!
//! The program is the benchmark suite's own `quicksort`, whose recursive `qs` and three loops tier up
//! on the product path (the optimizer inlines a leaf like `gcd` away, so it would leave no function).
#![cfg(all(target_arch = "x86_64", not(target_os = "windows")))]

#[path = "cli_common/mod.rs"]
mod common;
use common::{largo, scaffold, stderr};

use std::path::PathBuf;
use std::process::Output;
use tempfile::TempDir;

fn quicksort_project() -> TempDir {
    let dir = TempDir::new().expect("temp dir");
    scaffold(dir.path(), "quicksort");
    let program = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../benchmarks/programs/quicksort/main.lg");
    std::fs::copy(&program, dir.path().join("src/main.lg"))
        .expect("copy benchmarks/programs/quicksort/main.lg");
    dir
}

/// Run the quicksort benchmark on the VM with `env`, returning the output and the child's pid.
fn run(dir: &TempDir, env: &[(&str, &str)]) -> (Output, u32) {
    let mut cmd = largo();
    cmd.current_dir(dir.path())
        .args(["-q", "run", "--interpret", "--", "2000"]);
    for key in [
        "LOGOS_PERF_MAP",
        "LOGOS_JIT_CODE",
        "LOGOS_ENGINE_TRACE",
        "LOGOS_NO_JIT",
        "LOGOS_NATIVE_TIER",
    ] {
        cmd.env_remove(key);
    }
    for (key, value) in env {
        cmd.env(key, value);
    }
    let child = cmd
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn largo");
    let pid = child.id();
    let out = child.wait_with_output().expect("wait for largo");
    assert!(
        out.status.success(),
        "largo run --interpret failed:\n{}",
        stderr(&out)
    );
    (out, pid)
}

fn map_of(pid: u32) -> PathBuf {
    PathBuf::from(format!("/tmp/perf-{pid}.map"))
}

/// `(start, size, name)` for every line, asserting the perf-map line shape as it parses.
fn parse_map(text: &str) -> Vec<(u64, u64, String)> {
    text.lines()
        .map(|line| {
            let mut parts = line.splitn(3, ' ');
            let start = u64::from_str_radix(parts.next().unwrap_or(""), 16)
                .unwrap_or_else(|_| panic!("start is not hex: {line:?}"));
            let size = u64::from_str_radix(parts.next().unwrap_or(""), 16)
                .unwrap_or_else(|_| panic!("size is not hex: {line:?}"));
            let name = parts
                .next()
                .unwrap_or_else(|| panic!("no symbol: {line:?}"))
                .to_string();
            (start, size, name)
        })
        .collect()
}

#[test]
fn the_perf_map_names_every_compiled_function_and_loop_after_the_source() {
    let dir = quicksort_project();
    let (_, pid) = run(&dir, &[("LOGOS_PERF_MAP", "1")]);
    let path = map_of(pid);
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("{} was not written: {e}", path.display()));
    let _ = std::fs::remove_file(&path);

    let entries = parse_map(&text);
    assert!(!entries.is_empty(), "the map is empty");
    for (start, size, name) in &entries {
        assert!(
            *start > 0 && *size > 0,
            "an extent must be real: {start:x} {size:x} {name}"
        );
        assert!(
            name.starts_with("logos::") && name.ends_with(" [jit]"),
            "symbol shape: {name:?}"
        );
    }
    assert!(
        entries.iter().any(|(_, _, n)| n == "logos::qs [jit]"),
        "the qs function must be named:\n{text}"
    );
    assert!(
        entries.iter().any(|(_, _, n)| n.contains("::loop@")),
        "a compiled loop must be named:\n{text}"
    );

    let mut spans: Vec<(u64, u64)> = entries.iter().map(|(s, z, _)| (*s, s + z)).collect();
    spans.sort();
    for pair in spans.windows(2) {
        assert!(
            pair[0].1 <= pair[1].0,
            "two code extents overlap: {pair:x?}\n{text}"
        );
    }
}

#[test]
fn without_the_variable_no_map_is_written() {
    let dir = quicksort_project();
    let (_, pid) = run(&dir, &[]);
    assert!(
        !map_of(pid).exists(),
        "a run without LOGOS_PERF_MAP must not write {}",
        map_of(pid).display()
    );
}

#[test]
fn the_engine_trace_reports_the_tier_that_actually_ran() {
    let dir = quicksort_project();

    let (out, _) = run(&dir, &[("LOGOS_ENGINE_TRACE", "1")]);
    let err = stderr(&out);
    assert!(
        err.lines().any(|l| l == "logos-engine: vm+jit"),
        "the JIT run must say vm+jit:\n{err}"
    );
    let summary = err
        .lines()
        .find(|l| l.starts_with("logos-engine: native-tier exodia: "))
        .unwrap_or_else(|| panic!("the JIT run must report what it compiled:\n{err}"));
    let counts: Vec<(u32, u32)> = summary
        .trim_start_matches("logos-engine: native-tier exodia: ")
        .split(", ")
        .map(|part| {
            let ratio = part.split(' ').next().unwrap();
            let (ok, tried) = ratio.split_once('/').unwrap();
            (ok.parse().unwrap(), tried.parse().unwrap())
        })
        .collect();
    assert_eq!(counts.len(), 2, "functions and loops: {summary}");
    assert!(
        counts[0].0 >= 1 && counts[1].0 >= 1,
        "quicksort tiers up a function and a loop: {summary}"
    );

    let (out, _) = run(&dir, &[("LOGOS_ENGINE_TRACE", "1"), ("LOGOS_NO_JIT", "1")]);
    let err = stderr(&out);
    assert!(
        err.lines().any(|l| l == "logos-engine: vm"),
        "without the JIT the trace must say vm:\n{err}"
    );
    assert!(
        !err.contains("vm+jit") && !err.contains("native-tier"),
        "no JIT, no JIT claim:\n{err}"
    );
}

#[test]
fn the_code_dump_writes_every_extent_s_bytes_under_its_name() {
    let dir = quicksort_project();
    let code = TempDir::new().expect("code dir");
    let code_dir = code.path().to_str().expect("utf-8 temp path").to_string();
    let (_, pid) = run(
        &dir,
        &[("LOGOS_JIT_CODE", &code_dir), ("LOGOS_PERF_MAP", "1")],
    );
    let map = std::fs::read_to_string(map_of(pid)).expect("the perf map");
    let _ = std::fs::remove_file(map_of(pid));
    let index = std::fs::read_to_string(code.path().join("index.tsv"))
        .unwrap_or_else(|e| panic!("LOGOS_JIT_CODE wrote no index.tsv: {e}"));
    let mut dumped: Vec<(u64, u64, String)> = Vec::new();
    for line in index.lines() {
        let f: Vec<&str> = line.splitn(4, '\t').collect();
        assert_eq!(f.len(), 4, "index line shape `n START SIZE name`: {line:?}");
        let start =
            u64::from_str_radix(f[1], 16).unwrap_or_else(|_| panic!("start is not hex: {line:?}"));
        let size =
            u64::from_str_radix(f[2], 16).unwrap_or_else(|_| panic!("size is not hex: {line:?}"));
        let bytes = std::fs::read(code.path().join(format!("{}.bin", f[0])))
            .unwrap_or_else(|e| panic!("{}.bin is missing: {e}", f[0]));
        assert_eq!(
            bytes.len() as u64,
            size,
            "{}.bin holds the extent's bytes: {line:?}",
            f[0]
        );
        assert!(
            bytes.iter().any(|&b| b != 0),
            "{}.bin is machine code, not zeroes",
            f[0]
        );
        dumped.push((start, size, f[3].to_string()));
    }
    let mut mapped = parse_map(&map);
    mapped.sort();
    dumped.sort();
    assert_eq!(
        dumped, mapped,
        "the dump covers exactly the perf map's extents, under the same names"
    );
    assert!(
        dumped.iter().any(|(_, _, n)| n == "logos::qs [jit]"),
        "qs is dumped:\n{index}"
    );
}

#[test]
fn without_the_jit_no_code_is_dumped() {
    let dir = quicksort_project();
    let code = TempDir::new().expect("code dir");
    let code_dir = code.path().to_str().expect("utf-8 temp path").to_string();
    run(
        &dir,
        &[("LOGOS_JIT_CODE", &code_dir), ("LOGOS_NO_JIT", "1")],
    );
    assert_eq!(
        std::fs::read_dir(code.path())
            .expect("read code dir")
            .count(),
        0,
        "no JIT, no machine code"
    );
}
