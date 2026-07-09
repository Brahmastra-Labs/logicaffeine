//! WS-F wiring gate: the VM register-file `Value` produces output OBSERVABLY
//! IDENTICAL to the raw tree-walker over the whole benchmark corpus — under the
//! DEFAULT (`Value(RuntimeValue)`) build AND under `--features narrow-value`
//! (`Value(Narrow)`, the 8-byte NaN-boxed representation).
//!
//! The same test source compiles and runs under both cfgs, so a green run with
//! `--features narrow-value` is the proof that the narrow representation is
//! observationally equivalent to the fat one: the corpus exercises ints, floats
//! (nbody/mandelbrot/spectral_norm/pi_leibniz), bools, lists/maps/sets, strings
//! (strings/string_search), structs, recursion, and in-place mutation. This is
//! the VM-only mirror of `logicaffeine_tests::vm_opt_differential` (which adds
//! the JIT tier); no JIT is installed here, so it runs in this crate without a
//! dependency cycle and on the pure-bytecode path the narrow value most helps.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};

fn norm(s: &str) -> String {
    s.lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

/// Run one benchmark program on the VM (no JIT tier) and the raw tree-walker;
/// assert the outcomes match.
fn assert_vm_matches_treewalker(name: &str, arg: &str) {
    let path = format!(
        "{}/../../benchmarks/programs/{name}/main.lg",
        env!("CARGO_MANIFEST_DIR")
    );
    let src = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("reading benchmark program {path}: {e}"));
    let argv = vec![name.to_string(), arg.to_string()];

    let vm = vm_outcome_with_args(&src, &argv, None);
    let tw = tw_outcome_with_args(&src, &argv);

    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "VM diverged from the raw tree-walker on benchmark `{name}` (arg {arg})\n\
         --- VM error: {:?}\n--- TW error: {:?}",
        vm.error,
        tw.error,
    );
}

/// The full benchmark corpus (the same set the JIT-tiered differential gate
/// runs), at small calibration-independent sizes so the test is fast yet still
/// exercises every value kind and control-flow shape.
#[test]
fn narrow_corpus_vm_matches_raw_treewalker() {
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
        ("primes", "2000"),
        ("quicksort", "300"),
        ("sieve", "2000"),
        ("spectral_norm", "40"),
        ("strings", "200"),
        ("string_search", "200"),
        ("two_sum", "2000"),
    ];
    for (name, arg) in CORPUS {
        assert_vm_matches_treewalker(name, arg);
    }
}
