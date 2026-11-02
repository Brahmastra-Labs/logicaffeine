//! Integer addition, subtraction and multiplication remain exact across the i64 boundary.
//!
//! Every pair of boundary operands runs in a direct function and inside a loop. The
//! independent i128 oracle covers the full result range of two i64 operands. Values
//! inside and outside i64 must both answer exactly, including operands loaded from lists.

#![cfg(feature = "wasm-jit")]

#[path = "compile_common/mod.rs"]
mod common;
use common::wasm_host::run_outcome;

/// Operands as LOGOS expressions over `max` and `min`, with their values.
const OPERANDS: &[(&str, i64)] = &[
    ("max", i64::MAX),
    ("min", i64::MIN),
    ("max - 1", i64::MAX - 1),
    ("min + 1", i64::MIN + 1),
    ("0", 0),
    ("1", 1),
    ("0 - 1", -1),
    ("2", 2),
    ("0 - 2", -2),
    ("5", 5),
    ("0 - 8", -8),
    ("3037000499", 3_037_000_499),
    ("0 - 3037000499", -3_037_000_499),
    ("3037000500", 3_037_000_500),
    ("4294967296", 4_294_967_296),
];

/// `f(a, b)` answers `a op b` straight-line, or from inside a one-iteration `While`; `Main` hands it
/// operands read back out of a list, so nothing folds them.
fn program(op: &str, a: &str, b: &str, in_loop: bool) -> String {
    let body = if in_loop {
        format!(
            "    Let mutable r be 0.\n    Let mutable j be 1.\n    While j is at most 1:\n        Set r to a {op} b.\n        Set j to j + 1.\n    Return r.\n"
        )
    } else {
        format!("    Return a {op} b.\n")
    };
    format!(
        "## To f(a: Int, b: Int) -> Int:\n{body}\n## Main\nLet max be 9223372036854775807.\nLet min be 0 - max - 1.\n\
         Let mutable xs be a new Seq of Int.\nPush {a} to xs.\nPush {b} to xs.\nShow f(item 1 of xs, item 2 of xs).\n"
    )
}

fn check(op: &str, exact: fn(i128, i128) -> i128) {
    let (mut in_word, mut beyond_word) = (0, 0);
    for in_loop in [false, true] {
        for &(a, x) in OPERANDS {
            for &(b, y) in OPERANDS {
                let src = program(op, a, b, in_loop);
                let place = if in_loop {
                    "inside a loop"
                } else {
                    "outside a loop"
                };
                let expected = exact(i128::from(x), i128::from(y));
                let run = run_outcome(&src)
                    .unwrap_or_else(|e| panic!("{x} {op} {y} {place}: {e}\n{src}"));
                assert_eq!(run.failure, None, "{x} {op} {y} {place} failed:\n{src}");
                assert_eq!(
                    run.output.trim(),
                    expected.to_string(),
                    "{x} {op} {y} {place}:\n{src}"
                );
                if i64::try_from(expected).is_ok() {
                    in_word += 1;
                } else {
                    beyond_word += 1;
                }
            }
        }
    }
    assert!(
        in_word > 0 && beyond_word > 0,
        "{op}: both sides of the i64 boundary must be exercised"
    );
}

#[test]
fn an_int_sum_is_exact_across_the_word_boundary() {
    check("+", |a, b| a + b);
}

#[test]
fn an_int_difference_is_exact_across_the_word_boundary() {
    check("-", |a, b| a - b);
}

#[test]
fn an_int_product_is_exact_across_the_word_boundary() {
    check("*", |a, b| a * b);
}
