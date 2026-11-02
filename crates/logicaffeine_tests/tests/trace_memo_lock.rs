//! `Opt::TraceMemo`: a pure iteration run from every start of a counted range stops where it reaches a start already
//! finished.
//!
//! collatz walks, for every start up to `n`, the iteration from that start to 1 and counts its steps. What a walk adds
//! from a value is a function of the value alone, so the loop keeps each finished start's count and a walk that lands
//! on one adds it and stops. Every program prints what it printed, on every tier, memoized and not; an iteration that
//! reads anything that changes across starts, reads its accumulator, shows anything, or never shrinks keeps its walk.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::{assert_c_output, assert_compiled_equals_interpreted, compile_to_rust, run_all_tiers};
use logicaffeine_compile::optimization::decorate_source;

fn benchmark(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../benchmarks/programs/{name}/main.lg"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// collatz with `n` in the source instead of on the command line.
fn collatz_at(n: i64) -> String {
    benchmark("collatz").replace(
        "Let n be parseInt(item 2 of arguments).",
        &format!("Let n be {n}."),
    )
}

/// The program's generated Rust, and whether an iteration keeps a table of finished starts.
fn memoized(source: &str) -> (bool, String) {
    let rust = compile_to_rust(source).unwrap_or_else(|e| panic!("{e:?}\n{source}"));
    (rust.contains("__tracememo"), rust)
}

fn answers_alike(source: &str) {
    let agreed = run_all_tiers(source);
    assert!(agreed.error.is_none(), "{:?}\n{source}", agreed.error);
    assert_c_output(source, agreed.output.trim());
    assert_compiled_equals_interpreted(source);
    let off = decorate_source(source, &["tracememo"]);
    assert_eq!(
        run_all_tiers(&off).output,
        agreed.output,
        "memoized and walked print alike:\n{source}"
    );
}

#[test]
fn the_collatz_benchmark_keeps_a_table_of_finished_starts() {
    let (yes, rust) = memoized(&benchmark("collatz"));
    assert!(yes, "Got:\n{rust}");
}

#[test]
fn turning_the_optimization_off_walks_every_start() {
    let (yes, rust) = memoized(&decorate_source(&benchmark("collatz"), &["tracememo"]));
    assert!(!yes, "Got:\n{rust}");
}

#[test]
fn the_collatz_benchmark_counts_alike_at_every_size() {
    for n in [0, 1, 2, 3, 7, 27, 100, 1000, 5000] {
        answers_alike(&collatz_at(n));
    }
}

/// A `Main` walking `step` from every start in `from..=to` while `cond`, adding `add` per step to `total`.
fn walks(from: i64, to: i64, cond: &str, step: &str, add: &str) -> String {
    format!(
        "## Main\nLet mutable total be 0.\nLet mutable i be {from}.\nWhile i is at most {to}:\n    Let mutable k be i.\n    While {cond}:\n{step}        {add}\n    Set i to i + 1.\nShow total.\n"
    )
}

const COLLATZ_STEP: &str = "        If k % 2 equals 0:\n            Set k to k / 2.\n        Otherwise:\n            Set k to 3 * k + 1.\n";

/// Iterations that keep a table.
fn memoizing() -> Vec<(&'static str, String)> {
    vec![
        ("collatz from 5", walks(5, 3000, "k is not 1", COLLATZ_STEP, "Set total to total + 1.")),
        ("two per step", walks(1, 3000, "k is not 1", COLLATZ_STEP, "Set total to total + 2.")),
        ("a greater-than test", walks(1, 3000, "k is greater than 1", COLLATZ_STEP, "Set total to total + 1.")),
        (
            "digit sums down to one digit",
            walks(
                1,
                5000,
                "k is at least 10",
                "        Let mutable s be 0.\n        Let mutable r be k.\n        If r is at least 1000:\n            Set s to s + r / 1000.\n            Set r to r % 1000.\n        If r is at least 100:\n            Set s to s + r / 100.\n            Set r to r % 100.\n        Set s to s + r / 10 + r % 10.\n        Set k to s.\n",
                "Set total to total + 1.",
            ),
        ),
        (
            "an accumulator only some steps add to",
            "## Main\nLet mutable odd be 0.\nLet mutable i be 1.\nWhile i is at most 3000:\n    Let mutable k be i.\n    While k is not 1:\n        If k % 2 equals 0:\n            Set k to k / 2.\n        Otherwise:\n            Set k to 3 * k + 1.\n            Set odd to odd + 1.\n    Set i to i + 1.\nShow odd.\n".to_string(),
        ),
        (
            "a Repeat loop",
            "## Main\nLet mutable total be 0.\nRepeat for i from 1 to 2000:\n    Let mutable k be i.\n    While k is greater than 1:\n        If k % 2 equals 0:\n            Set k to k / 2.\n        Otherwise:\n            Set k to 3 * k + 1.\n        Set total to total + 1.\nShow total.\n".to_string(),
        ),
        (
            "a sum past what an entry holds",
            walks(1, 3000, "k is not 1", COLLATZ_STEP, "Set total to total + 1000."),
        ),
    ]
}

#[test]
fn every_memoizable_iteration_keeps_a_table_and_prints_alike() {
    for (why, program) in memoizing() {
        let (yes, rust) = memoized(&program);
        assert!(yes, "{why}: a table of finished starts. Got:\n{rust}");
        answers_alike(&program);
    }
}

/// Iterations outside the shape: each keeps its walk and prints alike.
fn other_walks() -> Vec<(&'static str, String)> {
    vec![
        (
            "a step that reads the start",
            walks(1, 300, "k is greater than 1", "        Set k to k / 2 - i % 2.\n", "Set total to total + 1."),
        ),
        (
            "a test on the accumulator",
            walks(1, 300, "k is not 1 and total is less than 1000000", COLLATZ_STEP, "Set total to total + 1."),
        ),
        (
            "two accumulators",
            "## Main\nLet mutable a be 0.\nLet mutable b be 0.\nLet mutable i be 1.\nWhile i is at most 300:\n    Let mutable k be i.\n    While k is not 1:\n        If k % 2 equals 0:\n            Set k to k / 2.\n            Set a to a + 1.\n        Otherwise:\n            Set k to 3 * k + 1.\n            Set b to b + 1.\n    Set i to i + 1.\nShow a.\nShow b.\n".to_string(),
        ),
        (
            "the walk's end read afterwards",
            "## Main\nLet mutable total be 0.\nLet mutable i be 2.\nWhile i is at most 300:\n    Let mutable k be i.\n    While k is greater than 9:\n        Set k to k / 3.\n        Set total to total + 1.\n    Set total to total + k.\n    Set i to i + 1.\nShow total.\n".to_string(),
        ),
        (
            "a walk that never shrinks",
            walks(1, 300, "k is less than 100000", "        Set k to 2 * k + 1.\n", "Set total to total + 1."),
        ),
        (
            "a negative step",
            walks(1, 300, "k is not 1", COLLATZ_STEP, "Set total to total + -1."),
        ),
        (
            "a show inside the walk",
            walks(1, 20, "k is greater than 1", "        Set k to k / 2.\n        Show k.\n", "Set total to total + 1."),
        ),
    ]
}

#[test]
fn an_iteration_outside_the_shape_keeps_its_walk_and_prints_alike() {
    for (why, program) in other_walks() {
        let (yes, rust) = memoized(&program);
        assert!(!yes, "{why}: must keep its walk. Got:\n{rust}");
        let agreed = run_all_tiers(&program);
        assert!(agreed.error.is_none(), "{why}: {:?}", agreed.error);
        assert_compiled_equals_interpreted(&program);
    }
}

/// The table holds at most 2^24 starts: the guard says so.
#[test]
fn the_table_is_bounded() {
    let (yes, rust) = memoized(&benchmark("collatz"));
    assert!(
        yes && rust.contains("16777216"),
        "the guard bounds the table at 2^24 starts. Got:\n{rust}"
    );
}
