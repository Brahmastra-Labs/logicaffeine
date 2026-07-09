//! The benchmark corpus as a differential test suite — the ratchet.
//!
//! Every `benchmarks/programs/*/main.lg` runs (at a small size) on BOTH
//! engines: the bytecode VM with a private JIT tier installed, and the
//! tree-walker oracle. Output AND error must match exactly. This is the net
//! under every EXODIA performance milestone: a silent semantic drift in a new
//! opcode, stencil, or optimizer pass fails here before it can reach a
//! benchmark run.
//!
//! The same suite carries the TIER-COVERAGE RATCHET: each program records how
//! many JIT function/region compiles SUCCEEDED, asserted against a floor
//! table. Today most floors are zero (the adapter bails on Div/Mod, floats,
//! collections, and calls); each JIT milestone RAISES the floors for its
//! cluster first (RED), then implements until the native tier actually
//! engages. A regression that silently returns a hot loop to bytecode fails
//! the ratchet, not just the wall clock.

#![cfg(not(target_arch = "wasm32"))]

use logicaffeine_compile::compile::{tw_outcome_with_args, vm_outcome_with_args};
use logicaffeine_compile::vm::NativeTier;
use logicaffeine_jit::ForgeTier;

/// (program, argv[1], min JIT function successes, min JIT region successes).
///
/// Sizes are deliberately tiny: big enough to cross the 100-call /
/// 100-back-edge tier thresholds, small enough that the debug-build
/// tree-walker finishes instantly.
const CORPUS: &[(&str, &str, u32, u32)] = &[
    ("ackermann", "3", 1, 0),
    ("array_fill", "2000", 0, 2),
    ("array_reverse", "2000", 0, 2),
    ("binary_trees", "6", 1, 0),
    ("bubble_sort", "60", 0, 1),
    ("coins", "500", 0, 2),
    ("collatz", "300", 0, 2),
    ("collect", "300", 0, 0),
    ("counting_sort", "2000", 0, 6),
    ("fannkuch", "5", 0, 5),
    ("fib", "12", 1, 0),
    ("fib_iterative", "500", 0, 1),
    ("gcd", "60", 1, 1),
    ("graph_bfs", "200", 0, 6),
    ("heap_sort", "300", 0, 2),
    ("histogram", "2000", 0, 3),
    ("knapsack", "30", 0, 2),
    ("loop_sum", "2000", 0, 1),
    ("mandelbrot", "20", 0, 2),
    ("matrix_mult", "8", 0, 1),
    ("mergesort", "300", 1, 3),
    ("nbody", "100", 0, 1),
    ("nqueens", "5", 0, 0),
    ("pi_leibniz", "2000", 0, 1),
    ("prefix_sum", "2000", 0, 2),
    ("primes", "500", 0, 2),
    ("quicksort", "300", 0, 1),
    ("sieve", "2000", 0, 3),
    ("spectral_norm", "20", 0, 0),
    ("string_search", "200", 0, 0),
    ("strings", "200", 0, 0),
    ("two_sum", "300", 0, 1),
];

/// The differential suites' shared output normalization (see
/// `phase_vm_differential.rs::norm`): the VM's captured stream ends in a
/// newline, the tree-walker's joined lines do not.
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

/// Run one corpus program on both engines and return the private tier's
/// (function successes, region successes).
fn assert_engines_agree(name: &str, size: &str) -> (u32, u32) {
    let source = program_source(name);
    let argv = vec!["bench".to_string(), size.to_string()];

    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(&source, &argv, Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(&source, &argv);

    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "VM diverged from tree-walker on benchmark '{name}' at size {size}"
    );
    assert_eq!(vm.error, None, "benchmark '{name}' errored at size {size}");
    assert!(
        !vm.output.is_empty(),
        "benchmark '{name}' produced no output at size {size}"
    );

    let (_, fn_ok) = tier.function_counts();
    let (_, region_ok) = tier.region_counts();
    (fn_ok, region_ok)
}

/// Differential helper for the M1 mechanical-optimization nets below.
fn assert_both_engines(src: &str, expected: &str) {
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "VM diverged from tree-walker on:\n{src}"
    );
    assert_eq!(vm.error, None, "errored on:\n{src}");
    assert_eq!(norm(&vm.output), expected, "wrong output for:\n{src}");
}

/// Text values are immutable: appending through one name must never be
/// visible through an alias bound earlier. This pins the sole-owner
/// in-place-append optimization to observable-copy semantics.
#[test]
fn text_append_alias_keeps_old_value() {
    assert_both_engines(
        "## Main\n\
         Let mutable s be \"a\".\n\
         Let t be s.\n\
         Set s to s + \"b\".\n\
         Show s.\n\
         Show t.\n",
        "ab\na",
    );
}

/// Append in a loop — the shape the in-place fast path accelerates — with an
/// alias captured mid-loop that must keep its snapshot.
#[test]
fn text_append_loop_with_midway_alias() {
    assert_both_engines(
        "## Main\n\
         Let mutable s be \"\".\n\
         Let mutable snapshot be \"\".\n\
         Let mutable i be 0.\n\
         While i is less than 200:\n\
         \x20   Set s to s + \"x\".\n\
         \x20   If i equals 99:\n\
         \x20       Set snapshot to s.\n\
         \x20   Set i to i + 1.\n\
         Show length of s.\n\
         Show length of snapshot.\n",
        "200\n100",
    );
}

/// Non-ASCII text: 1-based `item N of` indexes CHARACTERS while `length of`
/// counts BYTES (locked kernel semantics, collections.rs). The ASCII byte
/// fast path must never capture these.
#[test]
fn non_ascii_text_indexing_is_char_based() {
    assert_both_engines(
        "## Main\n\
         Let s be \"héllo\".\n\
         Show length of s.\n\
         Show item 1 of s.\n\
         Show item 2 of s.\n\
         Show item 5 of s.\n",
        "6\nh\né\no",
    );
    assert_both_engines(
        "## Main\n\
         Let s be \"日本語abc\".\n\
         Show length of s.\n\
         Show item 3 of s.\n\
         Show item 4 of s.\n",
        "12\n語\na",
    );
}

/// Character scan over a long ASCII text (the string_search shape): the byte
/// fast path must agree with the char path on every position.
#[test]
fn ascii_text_scan_positions() {
    assert_both_engines(
        "## Main\n\
         Let mutable s be \"\".\n\
         Let mutable i be 0.\n\
         While i is less than 150:\n\
         \x20   Set s to s + \"ab\".\n\
         \x20   Set i to i + 1.\n\
         Let mutable count be 0.\n\
         Set i to 1.\n\
         While i is at most length of s:\n\
         \x20   If item i of s equals \"a\":\n\
         \x20       Set count to count + 1.\n\
         \x20   Set i to i + 1.\n\
         Show count.\n",
        "150",
    );
}

/// `Set x to x + …` chains: the in-place append peephole must preserve the
/// exact left-fold. When a later chain term references the target, the
/// rewrite is illegal (the term must see the PRE-assignment value).
#[test]
fn add_assign_chain_semantics() {
    // x + 1 + x: second term reads the ORIGINAL x (1), total 1+1+1 = 3.
    assert_both_engines(
        "## Main\n\
         Let mutable x be 1.\n\
         Set x to x + 1 + x.\n\
         Show x.\n",
        "3",
    );
    // The strings-benchmark shape: Text + Int + Text, left-folded.
    assert_both_engines(
        "## Main\n\
         Let mutable s be \"n\".\n\
         Set s to s + 4 + \" \".\n\
         Set s to s + 7 + \" \".\n\
         Show s.\n",
        "n4 7",
    );
    // Self-append: rhs is the target itself — sole-owner fast path must not
    // capture it (the scratch holds a second Rc reference).
    assert_both_engines(
        "## Main\n\
         Let mutable s be \"ab\".\n\
         Set s to s + s.\n\
         Show s.\n",
        "abab",
    );
    // Int chain through a mutating loop — the AddAssign shape every int
    // benchmark loop compiles to.
    assert_both_engines(
        "## Main\n\
         Let mutable sum be 0.\n\
         Let mutable i be 1.\n\
         While i is at most 300:\n\
         \x20   Set sum to sum + i.\n\
         \x20   Set i to i + 1.\n\
         Show sum.\n",
        "45150",
    );
}

/// Error parity mid-chain: `Int + Bool` is a kernel error; the chained form
/// must produce the identical error and identical partial output.
#[test]
fn add_assign_chain_error_parity() {
    let src = "## Main\n\
               Show 1.\n\
               Let mutable x be 1.\n\
               Let b be true.\n\
               Set x to x + 1 + b.\n\
               Show x.\n";
    let tier = ForgeTier::new();
    let vm = vm_outcome_with_args(src, &[], Some(&tier as &dyn NativeTier));
    let tw = tw_outcome_with_args(src, &[]);
    assert_eq!(
        (norm(&vm.output), &vm.error),
        (norm(&tw.output), &tw.error),
        "VM diverged from tree-walker on:\n{src}"
    );
    assert!(vm.error.is_some(), "Int + Bool must error");
}

/// Map semantics that must survive the SipHash→FxHash swap: insert, lookup,
/// overwrite, contains, remove, length.
#[test]
fn map_operations_differential() {
    assert_both_engines(
        "## Main\n\
         Let mutable m be a new Map of Int to Int.\n\
         Let mutable i be 0.\n\
         While i is less than 300:\n\
         \x20   Set item i of m to i * 7.\n\
         \x20   Set i to i + 1.\n\
         Show length of m.\n\
         Show item 250 of m.\n\
         Set item 250 of m to 1.\n\
         Show item 250 of m.\n\
         Remove 250 from m.\n\
         Show length of m.\n\
         Show m contains 251.\n\
         Show m contains 250.\n",
        "300\n1750\n1\n299\ntrue\nfalse",
    );
}

#[test]
fn corpus_vm_matches_treewalker_and_tier_floors_hold() {
    // The tree-walker evaluates recursively; debug-build frames on the
    // recursion-heavy programs (quicksort, mergesort, binary_trees) outgrow
    // the default 2 MiB test stack. Same remedy as compiled binaries' main:
    // a dedicated big-stack thread.
    std::thread::Builder::new()
        .stack_size(256 * 1024 * 1024)
        .spawn(|| {
            let mut observed = String::new();
            let mut failures = Vec::new();
            for &(name, size, min_fn, min_region) in CORPUS {
                let (fn_ok, region_ok) = assert_engines_agree(name, size);
                observed.push_str(&format!(
                    "{name:14} fn_jit={fn_ok} region_jit={region_ok}\n"
                ));
                if fn_ok < min_fn || region_ok < min_region {
                    failures.push(format!(
                        "'{name}': JIT coverage regressed — fn {fn_ok} (floor {min_fn}), \
                         region {region_ok} (floor {min_region})"
                    ));
                }
            }
            eprintln!("tier coverage observed:\n{observed}");
            assert!(failures.is_empty(), "{}", failures.join("\n"));
        })
        .expect("spawn corpus thread")
        .join()
        .expect("corpus thread panicked");
}
