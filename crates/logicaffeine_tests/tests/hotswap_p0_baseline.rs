//! P0 — the studio's run path now uses the BASELINE tier
//! (`interpret_for_ui_baseline`): parse (unoptimized) → bytecode VM (no oracle), so
//! a keystroke-triggered run pays zero optimizer cost. This locks the contract the
//! studio now depends on: the baseline UI entry point produces exactly the
//! tree-walker oracle's output (HOTSWAP §9 W0). Output identity is what makes the
//! swap safe — only cold-start latency changes, never results.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::tw_outcome_with_args;
use logicaffeine_compile::interpret_for_ui_baseline_sync_with_args;

/// A representative slice of the corpus spanning recursion, integer loops, the float
/// cluster, text, maps, and `while` — the constructs the studio exercises. (The full
/// corpus is swept by `tier_invariance.rs` and `bench_corpus.rs`.)
const PROGRAMS: &[(&str, &str)] = &[
    ("fib", "12"),
    ("gcd", "60"),
    ("collatz", "300"),
    ("bubble_sort", "60"),
    ("nbody", "100"),
    ("mandelbrot", "20"),
    ("strings", "200"),
    ("two_sum", "300"),
];

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
fn baseline_ui_path_matches_treewalker() {
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(|| {
            for &(name, size) in PROGRAMS {
                let source = program_source(name);
                let argv = vec!["bench".to_string(), size.to_string()];
                let base = interpret_for_ui_baseline_sync_with_args(&source, &argv);
                let tw = tw_outcome_with_args(&source, &argv);
                assert_eq!(
                    (norm(&base.lines.join("\n")), &base.error),
                    (norm(&tw.output), &tw.error),
                    "baseline UI path diverged from the tree-walker oracle on '{name}'"
                );
                assert!(base.error.is_none(), "'{name}' errored on the baseline UI path");
            }
        })
        .expect("spawn baseline thread")
        .join()
        .expect("baseline thread panicked");
}
