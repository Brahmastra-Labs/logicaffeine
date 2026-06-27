//! P4a — tier-invariance: the run-path optimizer's hotness tiers change SPEED, not
//! OUTPUT. For every corpus program, the bytecode VM run through
//! `optimize_for_run_tiered` at EVERY tier T0..T3 must produce byte-identical output
//! to the tree-walker oracle (and to T0). This is the load-bearing soundness gate for
//! the whole HOTSWAP tiered optimizer (HOTSWAP §11); every later phase re-runs it.
//!
//! Output equality is the contract, NOT bytecode equality: each pass is individually
//! output-preserving, so any in-order subset (a lower tier) is too. Float passes are
//! guarded bit-exact (FloatStrength's runtime trip-count guard; ClosedForm is
//! integer-only), so byte-identical OUTPUT holds with no ULP tolerance (HOTSWAP §11).

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_tiered};
use logicaffeine_compile::optimization::Tier;

/// (program, argv[1]) — the same tiny sizes the bench differential uses: big enough
/// to exercise the loops/recursion the optimizer rewrites, small enough that the
/// debug-build tree-walker finishes instantly.
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

/// Every AST tier. T0 is the baseline (no optimizer); T3 reproduces today's pipeline.
const TIERS: &[Tier] = &[Tier::T0, Tier::T1, Tier::T2, Tier::T3];

/// The differential suites' shared normalization (matches `bench_corpus.rs::norm`).
fn norm(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn program_source(name: &str) -> String {
    let path = format!(
        "{}/../../benchmarks/programs/{}/main.lg",
        env!("CARGO_MANIFEST_DIR"),
        name
    );
    std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read benchmark program {path}: {e}"))
}

#[test]
fn tier_invariance_holds_on_corpus() {
    // The tree-walker recurses; the recursion-heavy programs (ackermann,
    // binary_trees, fib, quicksort, mergesort) outgrow the default 2 MiB test stack,
    // so the whole sweep runs on a dedicated big-stack thread (same remedy as
    // `bench_corpus.rs`).
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(|| {
            let mut failures = Vec::new();
            for &(name, size) in CORPUS {
                let source = program_source(name);
                let argv = vec!["bench".to_string(), size.to_string()];

                // The oracle: the unoptimized tree-walker defines the correct output.
                let tw = tw_outcome_with_args(&source, &argv);
                let oracle = (norm(&tw.output), tw.error.clone());
                if oracle.1.is_some() || oracle.0.is_empty() {
                    failures.push(format!(
                        "'{name}': tree-walker oracle errored or produced no output: {oracle:?}"
                    ));
                    continue;
                }

                // Pure VM (no native tier) at every AST tier must match the oracle
                // exactly — and each other (T0..T3 identical).
                let mut t0_out: Option<(String, Option<String>)> = None;
                for &tier in TIERS {
                    let r = vm_outcome_tiered(&source, &argv, tier, None);
                    let got = (norm(&r.output), r.error.clone());
                    if got != oracle {
                        failures.push(format!(
                            "'{name}' @ {tier:?}: diverged from tree-walker oracle\n  \
                             oracle: {oracle:?}\n  got:    {got:?}"
                        ));
                    }
                    match &t0_out {
                        None => t0_out = Some(got),
                        Some(t0) if &got != t0 => failures.push(format!(
                            "'{name}' @ {tier:?}: output differs from T0 (tier-variance!)\n  \
                             T0:     {t0:?}\n  {tier:?}: {got:?}"
                        )),
                        Some(_) => {}
                    }
                }
            }
            assert!(
                failures.is_empty(),
                "tier-invariance violations ({} found):\n\n{}",
                failures.len(),
                failures.join("\n\n")
            );
        })
        .expect("spawn tier_invariance thread")
        .join()
        .expect("tier_invariance thread panicked");
}
