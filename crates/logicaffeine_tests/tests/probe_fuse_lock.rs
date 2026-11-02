//! `Opt::ProbeFuse`: a map's membership test and the reads of the same key under it are one lookup.
//!
//! `If m contains k:` over a branch that reads `item k of m` asked the map for `k` twice, once for the test and
//! once for the value. AOT-Rust emits the test as `if let Some(v) = m.get(&k)` and each `item k of m` beneath it as
//! `v`. Every program prints what it printed on every tier, fused and unfused; a branch that calls anything or can
//! change `m` or `k`, a key that is any other expression, and a map of anything but `Int`, `Real` or `Bool` keep both
//! lookups. The collect benchmark's arithmetic stays on machine words: the reads keep their range.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::{assert_c_output, assert_compiled_equals_interpreted, compile_to_rust, run_all_tiers};
use logicaffeine_compile::optimization::decorate_source;

fn benchmark(name: &str) -> String {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join(format!("../../benchmarks/programs/{name}/main.lg"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// collect with `n` in the source instead of on the command line.
fn collect_at(n: i64) -> String {
    benchmark("collect").replace(
        "Let n be parseInt(item 2 of arguments).",
        &format!("Let n be {n}."),
    )
}

/// The program's generated Rust, and how many tests of a map are one lookup in it.
fn fused(source: &str) -> (usize, String) {
    let rust = compile_to_rust(source).unwrap_or_else(|e| panic!("{e:?}\n{source}"));
    (rust.matches("if let Some::<").count(), rust)
}

/// Every tier prints the same, compiled Rust prints the interpreter's output, C prints it too, and so does the
/// program with the fusion switched off.
fn answers_alike(source: &str) {
    let agreed = run_all_tiers(source);
    assert!(agreed.error.is_none(), "{:?}\n{source}", agreed.error);
    assert_c_output(source, agreed.output.trim());
    assert_compiled_equals_interpreted(source);
    let off = decorate_source(source, &["probefuse"]);
    assert_eq!(
        run_all_tiers(&off).output,
        agreed.output,
        "fused and unfused print alike:\n{source}"
    );
    assert_compiled_equals_interpreted(&off);
}

#[test]
fn the_collect_benchmark_looks_each_key_up_once() {
    let (n, rust) = fused(&benchmark("collect"));
    assert_eq!(
        n, 2,
        "the hit test and the miss test are each one lookup. Got:\n{rust}"
    );
    assert!(
        !rust.contains("logos_contains"),
        "no test asks the map twice. Got:\n{rust}"
    );
    assert!(
        !rust.contains("_exact("),
        "the hash stays on machine words. Got:\n{rust}"
    );
}

#[test]
fn turning_the_optimization_off_tests_and_reads_separately() {
    let (n, rust) = fused(&decorate_source(&benchmark("collect"), &["probefuse"]));
    assert_eq!(n, 0, "Got:\n{rust}");
    assert!(rust.contains("logos_contains"), "Got:\n{rust}");
}

#[test]
fn the_collect_benchmark_hashes_alike_at_every_size() {
    for n in [0, 1, 2, 3, 10, 100, 1000, 4096] {
        answers_alike(&collect_at(n));
    }
}

/// A `Main` with a map `m` of `n` keys `0, 3, 6, …` to `key * 7 + 1`, then `body` once per `k` in `0..=3n`.
fn over_keys(value_ty: &str, fill: &str, body: &str) -> String {
    format!(
        "## Main\nLet m be a new Map of Int to {value_ty}.\nLet mutable i be 0.\nWhile i is less than 40:\n    Set m at i * 3 to {fill}.\n    Set i to i + 1.\nLet mutable total be 0.\nLet mutable k be 0.\nWhile k is at most 120:\n{body}    Set k to k + 1.\nShow total.\n"
    )
}

/// Branches that read the map once.
fn fusing() -> Vec<(&'static str, String)> {
    vec![
        (
            "an inline read",
            over_keys("Int", "i * 7 + 1", "    If m contains k:\n        Set total to total + item k of m.\n"),
        ),
        (
            "the read written as the branch's first Let",
            over_keys(
                "Int",
                "i * 7 + 1",
                "    If m contains k:\n        Let v be item k of m.\n        Set total to total * 3 % 1000003 + v.\n",
            ),
        ),
        (
            "two reads and an otherwise",
            over_keys(
                "Int",
                "i * 7 + 1",
                "    If m contains k:\n        Set total to total + item k of m * item k of m % 97.\n    Otherwise:\n        Set total to total + 1.\n",
            ),
        ),
        (
            "a read inside a loop of the branch",
            over_keys(
                "Int",
                "i * 7 + 1",
                "    If m contains k:\n        Let mutable j be 0.\n        While j is less than 3:\n            Set total to total + item k of m + j.\n            Set j to j + 1.\n",
            ),
        ),
        (
            "a read under a nested test",
            over_keys(
                "Int",
                "i * 7 + 1",
                "    If m contains k:\n        If k % 2 equals 0:\n            Set total to total + item k of m.\n",
            ),
        ),
        (
            "a map of Bool",
            over_keys(
                "Bool",
                "(i % 2 equals 0)",
                "    If m contains k:\n        If item k of m:\n            Set total to total + k.\n",
            ),
        ),
        (
            "a literal key",
            "## Main\nLet m be a new Map of Int to Int.\nSet m at 5 to 42.\nIf m contains 5:\n    Show item 5 of m.\nIf m contains 6:\n    Show item 6 of m.\nShow 0.\n".to_string(),
        ),
        (
            "a Text key",
            "## Main\nLet m be a new Map of Text to Int.\nSet m at \"a\" to 1.\nSet m at \"b\" to 2.\nLet mutable total be 0.\nRepeat for w in [\"a\", \"b\", \"c\", \"a\"]:\n    If m contains w:\n        Set total to total * 10 + item w of m.\nShow total.\n".to_string(),
        ),
        (
            "a read of another key beside it",
            over_keys(
                "Int",
                "i * 7 + 1",
                "    If m contains k:\n        Set total to total + item k of m * 2 + item 3 of m.\n",
            ),
        ),
        (
            "a read of the key after the branch",
            over_keys(
                "Int",
                "i * 7 + 1",
                "    If m contains k:\n        Set total to total + item k of m.\n    If k % 3 equals 0 and k is less than 120:\n        Set total to total + item k of m % 5.\n",
            ),
        ),
        (
            "inside a function",
            "## To lookup (m: Map of Int to Int) and (k: Int) -> Int:\n    If m contains k:\n        Return item k of m * 2.\n    Return 0 - 1.\n\n## Main\nLet m be a new Map of Int to Int.\nSet m at 3 to 4.\nShow lookup(m, 3).\nShow lookup(m, 9).\n".to_string(),
        ),
    ]
}

#[test]
fn every_fusing_branch_is_one_lookup_and_prints_alike() {
    for (why, program) in fusing() {
        let (n, rust) = fused(&program);
        assert!(n >= 1, "{why}: one lookup. Got:\n{rust}");
        answers_alike(&program);
    }
}

/// Branches that keep both lookups, each printing alike.
fn other_branches() -> Vec<(&'static str, String)> {
    vec![
        (
            "the branch writes the map",
            over_keys(
                "Int",
                "i * 7 + 1",
                "    If m contains k:\n        Set m at k to item k of m + 1.\n        Set total to total + item k of m.\n",
            ),
        ),
        (
            "the branch moves the key",
            over_keys(
                "Int",
                "i * 7 + 1",
                "    If m contains k:\n        Set total to total + item k of m.\n        Set k to k + 1.\n",
            ),
        ),
        (
            "a key that is an expression",
            over_keys(
                "Int",
                "i * 7 + 1",
                "    If m contains (k + 3):\n        Set total to total + item (k + 3) of m.\n",
            ),
        ),
        (
            "a read of another key",
            over_keys(
                "Int",
                "i * 7 + 1",
                "    If m contains k:\n        If m contains (k + 3):\n            Set total to total + item (k + 3) of m.\n",
            ),
        ),
        (
            "a branch that reads no value",
            over_keys("Int", "i * 7 + 1", "    If m contains k:\n        Set total to total + 1.\n"),
        ),
        (
            "a map of Text",
            "## Main\nLet m be a new Map of Int to Text.\nSet m at 1 to \"one\".\nLet mutable out be \"\".\nLet mutable k be 0.\nWhile k is less than 3:\n    If m contains k:\n        Set out to out + item k of m.\n    Set k to k + 1.\nShow out.\n".to_string(),
        ),
    ]
}

#[test]
fn a_branch_outside_the_shape_keeps_both_lookups_and_prints_alike() {
    for (why, program) in other_branches() {
        let (n, rust) = fused(&program);
        assert_eq!(n, 0, "{why}: keeps both lookups. Got:\n{rust}");
        let agreed = run_all_tiers(&program);
        assert!(agreed.error.is_none(), "{why}: {:?}", agreed.error);
        assert_compiled_equals_interpreted(&program);
    }
}
