//! A counted loop leaves its counter where the loop leaves it, wherever that value can be read.
//!
//! The compiled tiers lower a counted loop to a `for` over a range and write the counter's exit value back after it.
//! They wrote it only when a later statement of the same block mentioned the counter, which holds for a counter the
//! block declares. A counter the loop reuses (`Set j to 1`) belongs to an enclosing scope: code after an enclosing
//! loop reads it, and it read the value the counter had before the inner loop. Every lowering that can take a reused
//! counter (the range loop, a tiled nest, a copy or a slice, a run continued from an earlier loop) now writes it back.
//! Each program here sets its counter inside an enclosing loop or block and shows it after it, on every tier.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::{assert_c_output, assert_compiled_equals_interpreted, run_all_tiers};

fn prints_alike(why: &str, source: &str, expected: &str) {
    let agreed = run_all_tiers(source);
    assert_eq!(
        (agreed.output.trim(), agreed.error.as_deref()),
        (expected, None),
        "{why}: the interpreter and the VM"
    );
    assert_c_output(source, expected);
    assert_compiled_equals_interpreted(source);
}

#[test]
fn a_range_loop_leaves_a_reused_counter_at_its_exit() {
    let source = "## Main\nLet n be 3.\nLet arr be [5, 4, 3].\nLet mutable j be 0.\nLet mutable i be 0.\nWhile i is less than n - 1:\n    Set j to 1.\n    While j is at most n - 1 - i:\n        Let a be item j of arr.\n        Let b be item (j + 1) of arr.\n        If a is greater than b:\n            Set item j of arr to b.\n            Set item (j + 1) of arr to a.\n        Set j to j + 1.\n    Set i to i + 1.\nShow j.\nShow arr.\n";
    prints_alike("bubble passes", source, "2\n[3, 4, 5]");
    let sum = "## Main\nLet mutable total be 0.\nLet mutable k be 0.\nLet mutable r be 0.\nWhile r is less than 3:\n    Set k to 1.\n    While k is less than 5 + r:\n        Set total to total + k.\n        Set k to k + 1.\n    Set r to r + 1.\nShow total.\nShow k.\n";
    prints_alike("an exclusive range", sum, "46\n7");
}

#[test]
fn a_copy_leaves_a_reused_counter_at_its_exit() {
    let copy = "## Main\nLet src be [7, 8, 9].\nLet mutable c be 0.\nLet mutable r be 0.\nWhile r is less than 2:\n    Let mutable dst be a new Seq of Int.\n    Set c to 1.\n    While c is at most length of src:\n        Push item c of src to dst.\n        Set c to c + 1.\n    Show length of dst.\n    Set r to r + 1.\nShow c.\n";
    prints_alike("a whole copy", copy, "3\n3\n4");
    let slice = "## Main\nLet src be [7, 8, 9, 10].\nLet mutable c be 0.\nLet mutable r be 0.\nWhile r is less than 2:\n    Let mutable dst be a new Seq of Int.\n    Set c to 2.\n    While c is at most 3:\n        Push item c of src to dst.\n        Set c to c + 1.\n    Show dst.\n    Set r to r + 1.\nShow c.\n";
    prints_alike("a slice", slice, "[8, 9]\n[8, 9]\n4");
    let runs = "## Main\nLet src be [7, 8, 9, 10, 11].\nLet mutable c be 0.\nLet mutable r be 0.\nWhile r is less than 2:\n    Let mutable dst be a new Seq of Int.\n    Set c to 1.\n    While c is at most 2:\n        Push item c of src to dst.\n        Set c to c + 1.\n    While c is at most 4:\n        Push item c of src to dst.\n        Set c to c + 1.\n    Show dst.\n    Set r to r + 1.\nShow c.\n";
    prints_alike("two runs", runs, "[7, 8, 9, 10]\n[7, 8, 9, 10]\n5");
    let continued = "## Main\nLet src be [7, 8, 9, 10, 11].\nLet mutable c be 0.\nLet mutable r be 0.\nWhile r is less than 2:\n    Set c to 2.\n    Let mutable dst be a new Seq of Int.\n    While c is at most 4:\n        Push item c of src to dst.\n        Set c to c + 1.\n    Show dst.\n    Set r to r + 1.\nShow c.\n";
    prints_alike(
        "a run from a counter set before the sequence",
        continued,
        "[8, 9, 10]\n[8, 9, 10]\n5",
    );
    let itself = "## Main\nLet mutable c be 5.\nLet mutable r be 0.\nWhile r is less than 2:\n    Let mutable dst be a new Seq of Int.\n    Set c to 1.\n    While c is at most length of dst:\n        Push item c of dst to dst.\n        Set c to c + 1.\n    Show length of dst.\n    Set r to r + 1.\nShow c.\n";
    prints_alike("a sequence copied into itself", itself, "0\n0\n1");
}

/// The matrix nest the tiler runs in tile order, inside a block that reuses its outer counter.
#[test]
fn a_tiled_nest_leaves_a_reused_outer_counter_at_its_exit() {
    let source = "## To native parseInt (s: Text) -> Int\n\n## Main\nLet n be parseInt(\"8\").\nLet mutable a be a new Seq of Int.\nLet mutable c be a new Seq of Int.\nLet mutable i be 0.\nWhile i is less than n * n:\n    Push i % 100 to a.\n    Push 0 to c.\n    Set i to i + 1.\nIf n is greater than 0:\n    Set i to 0.\n    While i is less than n:\n        Let mutable k be 0.\n        While k is less than n:\n            Let mutable j be 0.\n            While j is less than n:\n                Let idx be i * n + j + 1.\n                Set item idx of c to (item idx of c) + (item (i * n + k + 1) of a) * (item (k * n + j + 1) of a).\n                Set j to j + 1.\n            Set k to k + 1.\n        Set i to i + 1.\nShow item 1 of c.\nShow i.\n";
    let rust = common::compile_to_rust(source).unwrap();
    assert!(
        rust.contains("step_by"),
        "the nest runs tiled. Got:\n{rust}"
    );
    prints_alike("a tiled nest", source, "1120\n8");
    // A bound below zero: the guard holds, the tiled nest runs no pass, and the counter stays at 0.
    let below = source
        .replace("parseInt(\"8\")", "parseInt(\"-1\")")
        .replace("If n is greater than 0:", "If n is less than 100:");
    prints_alike("a tiled nest that runs no pass", &below, "0\n0");
}

/// A substring search whose counter an enclosing loop reuses runs as a plain nest and prints alike.
#[test]
fn a_search_leaves_a_reused_counter_at_its_exit() {
    let source = "## Main\nLet text be \"abcabcabd\".\nLet needle be \"abd\".\nLet mutable found be 0.\nLet mutable i be 0.\nLet mutable rep be 0.\nWhile rep is less than 2:\n    Set i to 1.\n    While i is at most length of text - length of needle + 1:\n        Let mutable j be 1.\n        Let mutable ok be true.\n        While j is at most length of needle:\n            If item (i + j - 1) of text is not item j of needle:\n                Set ok to false.\n            Set j to j + 1.\n        If ok:\n            Set found to found + 1.\n        Set i to i + 1.\n    Set rep to rep + 1.\nShow found.\nShow i.\n";
    prints_alike("a substring search", source, "2\n8");
}

/// A counter whose `Let` a lowering consumes keeps the Int type its `Let` decided. The flag fill below is lowered as
/// one fill and re-declares `i` itself; `i` then reached the next loop with no type at all, its arithmetic read as no
/// number, and `i * i` compiled raw: past 2^63 it overflowed (a wrong value in a release build). In the sieve the
/// same raw `i * i` started a store loop, and the wrapped index wrote outside the flags.
const CONSUMED_COUNTER: &str = "## To native parseInt (s: Text) -> Int\n\n\
## To walk (limit: Int) -> Int:\n    Let mutable flags be a new Seq of Bool.\n    Let mutable i be 0.\n    \
While i is at most 5:\n        Push false to flags.\n        Set i to i + 1.\n    Let mutable count be 0.\n    \
Set i to 3037000499.\n    While i is at most limit:\n        If count is less than 5:\n            \
Set count to count + 1.\n            Show i * i.\n        Set i to i + 1.\n    Return count.\n\n\
## Main\nShow walk(parseInt(\"3037000501\")).\n";

#[test]
fn a_counter_a_lowering_declares_keeps_its_int_type() {
    let agreed = run_all_tiers(CONSUMED_COUNTER);
    assert_eq!(
        (agreed.output.trim(), agreed.error.as_deref()),
        (
            "9223372030926249001\n9223372037000250000\n9223372043074251001\n3",
            None
        ),
        "the interpreter and the VM square past 2^63 exactly"
    );
    assert_compiled_equals_interpreted(CONSUMED_COUNTER);
    let rust = common::compile_to_rust(CONSUMED_COUNTER).unwrap();
    assert!(!rust.contains("(i * i)"), "no raw square of the counter:\n{rust}");
}

/// The sieve's own shape: the flags filled by a lowering, a counted loop over them, and under a test of the flag a
/// store loop from `i * i` stepping by `i`. Outside a version whose guard proves it fits, every step of the store
/// index is checked arithmetic, so no wrapped index can reach the unchecked store; and the sieve counts alike at small
/// limits on every tier and against C.
#[test]
fn the_sieve_store_loop_steps_on_checked_arithmetic() {
    let sieve = |limit: i64| {
        format!(
            "## To native parseInt (s: Text) -> Int\n\n\
             ## To walk (limit: Int) -> Int:\n    Let mutable flags be a new Seq of Bool.\n    Let mutable i be 0.\n    \
             While i is at most limit:\n        Push false to flags.\n        Set i to i + 1.\n    Let mutable count be 0.\n    \
             Set i to 2.\n    While i is at most limit:\n        If item (i + 1) of flags equals false:\n            \
             Set count to count + 1.\n            Let mutable j be i * i.\n            While j is at most limit:\n                \
             Set item (j + 1) of flags to true.\n                Set j to j + i.\n        Set i to i + 1.\n    Return count.\n\n\
             ## Main\nShow walk(parseInt(\"{limit}\")).\n"
        )
    };
    let rust = common::compile_to_rust(&sieve(100)).unwrap();
    let unguarded = match rust.find("if (limit <= ") {
        Some(at) => &rust[at + rust[at..].find("} else {").expect("the version's other copy")..],
        None => rust.as_str(),
    };
    assert!(
        unguarded.contains("logos_mul_i64(i, i)") && !unguarded.contains("(i * i)") && !unguarded.contains("(j + i)"),
        "outside a version proven to fit, the store index steps on checked arithmetic:\n{rust}"
    );
    for (limit, primes) in [(2, "1"), (10, "4"), (100, "25"), (1000, "168")] {
        prints_alike("the sieve", &sieve(limit), primes);
    }
}
