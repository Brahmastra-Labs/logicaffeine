//! AOT-native corpus gate (HOTSWAP §11/§12.4) — EVERY benchmark program must run
//! correctly on the COMPILED native code. Each `benchmarks/programs/*/main.lg` is
//! compiled the full AOT way (Logos → generated Rust → `rustc` → native binary), run at
//! the corpus's tiny size, and its output asserted byte-identical to the tree-walker
//! reference (LOGOS's semantic ground truth). This is the soundness net for the
//! AOT-native tier across the whole suite: a codegen bug that diverges from the
//! interpreter fails here. `#[ignore]` — it invokes `rustc` once per program (very slow);
//! run on demand with `--run-ignored all`.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::run_logos_with_args;
use logicaffeine_compile::compile::tw_outcome_with_args;

/// (program, size) — the same corpus + tiny sizes as `bench_corpus.rs`, chosen so the
/// debug tree-walker reference finishes instantly.
const CORPUS: &[(&str, &str)] = &[
    ("ackermann", "3"),
    ("array_fill", "2000"),
    ("array_reverse", "2000"),
    ("binary_trees", "6"),
    ("bubble_sort", "60"),
    ("coins", "500"),
    ("collatz", "300"),
    ("collect", "300"),
    ("counting_sort", "2000"),
    ("fannkuch", "5"),
    ("fib", "12"),
    ("fib_iterative", "500"),
    ("gcd", "60"),
    ("graph_bfs", "200"),
    ("heap_sort", "300"),
    ("histogram", "2000"),
    ("knapsack", "30"),
    ("loop_sum", "2000"),
    ("mandelbrot", "20"),
    ("matrix_mult", "8"),
    ("mergesort", "300"),
    ("nbody", "100"),
    ("nqueens", "5"),
    ("pi_leibniz", "2000"),
    ("prefix_sum", "2000"),
    ("primes", "500"),
    ("quicksort", "300"),
    ("sieve", "2000"),
    ("spectral_norm", "20"),
    ("string_search", "200"),
    ("strings", "200"),
    ("two_sum", "300"),
];

fn program_source(name: &str) -> String {
    let path = format!(
        "{}/../../benchmarks/programs/{}/main.lg",
        env!("CARGO_MANIFEST_DIR"),
        name
    );
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("cannot read {path}: {e}"))
}

/// Shared output normalization (see `bench_corpus::norm`).
fn norm(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
#[ignore = "compiles EVERY benchmark to native via rustc (very slow) — AOT-native corpus gate"]
fn every_benchmark_runs_correctly_on_aot_native() {
    let mut failures = Vec::new();

    for (name, size) in CORPUS {
        let src = program_source(name);

        // Compiled-native: Logos → Rust → rustc → binary, run with the size argument.
        let compiled = run_logos_with_args(&src, &[size]);
        if !compiled.success {
            failures.push(format!(
                "{name} @ {size}: AOT-native build/run FAILED:\n{}",
                compiled.stderr.trim()
            ));
            continue;
        }

        // Reference: the tree-walker (argv = [program-name, size]).
        let reference = tw_outcome_with_args(&src, &["bench".to_string(), size.to_string()]);
        if let Some(err) = &reference.error {
            failures.push(format!("{name} @ {size}: interpreter reference errored: {err}"));
            continue;
        }

        if norm(&compiled.stdout) != norm(&reference.output) {
            failures.push(format!(
                "{name} @ {size}: AOT-native output != interpreter\n  native: {:?}\n  interp: {:?}",
                norm(&compiled.stdout),
                norm(&reference.output)
            ));
        }
    }

    assert!(
        failures.is_empty(),
        "AOT-native corpus: {}/{} benchmark(s) failed:\n\n{}",
        failures.len(),
        CORPUS.len(),
        failures.join("\n\n")
    );
}
