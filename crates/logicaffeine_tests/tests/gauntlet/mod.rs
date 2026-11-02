//! ★ THE ULTIMATE GAUNTLET — shared corpus ★
//!
//! One aggregated reservoir of real LOGOS programs, so every invariant lock runs over the SAME broad set and
//! "nothing gets lost or broken":
//!   - `harvested()` — the cross-op corpus lifted verbatim from `op_backend_matrix_lock`'s `CORPUS`
//!     (the biggest hand-built reservoir of LOGOS source we own), pinned in `corpus.jsonl`.
//!   - `benchmarks()` — every `benchmarks/programs/*/interp.lg` (the largest, realest workloads).
//!   - `compute_corpus()` — a curated spread the optimizer's passes each rewrite.
//!
//! Each program carries the `oracle` that runs it (`plain` straight-line, `concurrent` scheduler-driven, `net`
//! offline networking) so the invariant locks route it to the right VM entry point. The existing generative
//! fuzzers (`soft_core_fuzz_differential`, `pe_jones_fuzz`, `phase_pe_differential`) already prove VM ≡ soft-core
//! and VM ≡ PE-residual over RANDOM programs; this gauntlet's distinct job is to drive the harvested + benchmark
//! corpus through the AOT tiers (C and Rust) and pin VM ≡ opt ≡ C ≡ Rust.
#![allow(dead_code)]

use std::path::PathBuf;

use serde::Deserialize;

#[derive(Deserialize, Clone, Debug)]
pub struct GauntletProg {
    pub name: String,
    pub op: String,
    pub src: String,
    pub expect_out: String,
    pub expect_err: bool,
    /// "plain" | "concurrent" | "net" — which VM oracle runs this program.
    pub oracle: String,
}

/// The cross-op programs harvested from `op_backend_matrix_lock`'s `CORPUS`.
pub fn harvested() -> Vec<GauntletProg> {
    let raw = include_str!("corpus.jsonl");
    let mut out = Vec::new();
    for line in raw.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let p: GauntletProg = serde_json::from_str(line)
            .unwrap_or_else(|e| panic!("corpus.jsonl parse error: {e}\n  line: {line}"));
        out.push(p);
    }
    out
}

fn benchmarks_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../benchmarks/programs")
}

/// Every `benchmarks/programs/*/interp.lg` — the biggest real workloads (all straight-line / `plain`).
pub fn benchmarks() -> Vec<GauntletProg> {
    let dir = benchmarks_dir();
    let mut out = Vec::new();
    if let Ok(entries) = std::fs::read_dir(&dir) {
        for e in entries.flatten() {
            let p = e.path();
            if p.is_dir() {
                let interp = p.join("interp.lg");
                if interp.is_file() {
                    let name = p.file_name().unwrap().to_string_lossy().to_string();
                    if let Ok(src) = std::fs::read_to_string(&interp) {
                        out.push(GauntletProg {
                            name: format!("bench_{name}"),
                            op: "Benchmark".into(),
                            src,
                            expect_out: String::new(),
                            expect_err: false,
                            oracle: "plain".into(),
                        });
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

/// A curated spread targeting the shapes the optimizer's passes each rewrite.
pub fn compute_corpus() -> Vec<GauntletProg> {
    const C: &[(&str, &str)] = &[
        ("const_fold", "## Main\n    Show 2 + 3 * 4 - 1.\n"),
        ("loop_sum_5050", "## Main\n    Let mutable acc be 0.\n    Let mutable i be 1.\n    While i is at most 100:\n        Set acc to acc + i.\n        Set i to i + 1.\n    Show acc.\n"),
        ("factorial_bignum", "## Main\n    Let mutable acc be 1.\n    Let mutable i be 1.\n    While i is at most 25:\n        Set acc to acc * i.\n        Set i to i + 1.\n    Show acc.\n"),
        ("nested_loops", "## Main\n    Let mutable s be 0.\n    Let mutable i be 1.\n    While i is at most 20:\n        Let mutable j be 1.\n        While j is at most 20:\n            Set s to s + i * j.\n            Set j to j + 1.\n        Set i to i + 1.\n    Show s.\n"),
        ("recursion", "## To fib (n: Int) -> Int:\n    If n is less than 2:\n        Return n.\n    Return fib(n - 1) + fib(n - 2).\n\n## Main\n    Show fib(20).\n"),
        ("collection_build", "## Main\n    Let mutable xs be a new Seq of Int.\n    Let mutable i be 1.\n    While i is at most 50:\n        Push i * i to xs.\n        Set i to i + 1.\n    Show length of xs.\n    Show item 50 of xs.\n"),
        ("text", "## Main\n    Show \"hello world\".\n"),
        ("div_by_zero_error", "## Main\n    Let a be 5.\n    Let b be 0.\n    Show a / b.\n"),
        ("floor_div", "## Main\n    Show 7 // 2.\n"),
        ("int_overflow_at_run_time", "## Main\n    Let mutable x be 0.\n    Let mutable i be 0.\n    While i is less than 30:\n        Set x to x + 1.\n        Set i to i + 1.\n    Let big be 4611686018427387904 + x - 30.\n    Show big * 4.\n"),
    ];
    C.iter()
        .map(|(n, s)| GauntletProg {
            name: format!("corpus_{n}"),
            op: "Compute".into(),
            src: s.to_string(),
            expect_out: String::new(),
            expect_err: n.contains("error"),
            oracle: "plain".into(),
        })
        .collect()
}

/// The whole gauntlet — harvested + benchmarks + compute corpus.
pub fn all() -> Vec<GauntletProg> {
    let mut v = harvested();
    v.extend(benchmarks());
    v.extend(compute_corpus());
    v
}

/// Only the straight-line (`plain`) programs — the ones the AOT tiers and the sequential VM run directly.
pub fn plain(progs: &[GauntletProg]) -> Vec<GauntletProg> {
    progs
        .iter()
        .filter(|p| p.oracle == "plain")
        .cloned()
        .collect()
}

/// Parse a shrink-only ledger file (one program name per line; `#` comments + blank lines ignored).
fn parse_ledger(text: &str) -> std::collections::BTreeSet<String> {
    text.lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.to_string())
        .collect()
}

/// The C AOT tier's shrink-only ledgers (Invariant-B): programs whose C execution diverges from VM(opt), and
/// programs `codegen_c` cannot yet compile. A program passes → delete it here (the ledger only shrinks).
pub fn c_known_divergences() -> std::collections::BTreeSet<String> {
    parse_ledger(include_str!("c_known_divergences.txt"))
}
pub fn c_known_compile_gaps() -> std::collections::BTreeSet<String> {
    parse_ledger(include_str!("c_known_compile_gaps.txt"))
}
/// The Rust AOT tier's shrink-only ledgers.
pub fn rust_known_divergences() -> std::collections::BTreeSet<String> {
    parse_ledger(include_str!("rust_known_divergences.txt"))
}
pub fn rust_known_compile_gaps() -> std::collections::BTreeSet<String> {
    parse_ledger(include_str!("rust_known_compile_gaps.txt"))
}
