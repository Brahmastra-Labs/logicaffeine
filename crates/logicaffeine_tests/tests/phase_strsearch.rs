//! Naive substring-search → SIMD overlapping-count kernel.
//!
//! A doubly-nested "for each start position, does the fixed window equal the
//! needle? if so, count it" loop is an overlapping occurrence count. The AOT
//! recognizer (`peephole::try_emit_naive_search`) proves the shape and lowers
//! the whole nest to a single call into the emitted SIMD kernel
//! `__logos_count_window_matches` (scans for the first needle byte with a vector
//! compare, verifies each candidate window). The kernel's own algorithm is
//! differential-fuzzed in `logicaffeine_compile::codegen::strsearch`; here we
//! test the codegen recognition and end-to-end correctness, including the
//! overlapping semantics the naive loop has.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::{compile_to_rust, run_logos};

/// A full naive-occurrence-COUNT over a literal haystack (no args, so it runs
/// under `run_logos`). "abcabcabcXXXXXabcdeXXXXX" contains "XXXXX" twice.
const COUNT_SEARCH: &str = r#"## Main
Let text be "abcabcabcXXXXXabcdeXXXXX".
Let needle be "XXXXX".
Let needleLen be 5.
Let textLen be length of text.
Let mutable count be 0.
Let mutable i be 1.
While i is at most textLen - needleLen + 1:
    Let mutable match be 1.
    Let mutable j be 0.
    While j is less than needleLen:
        If item (i + j) of text is not item (j + 1) of needle:
            Set match to 0.
            Set j to needleLen.
        Set j to j + 1.
    If match equals 1:
        Set count to count + 1.
    Set i to i + 1.
Show count.
"#;

/// Overlapping matches: "aa" occurs at positions 1,2,3 of "aaaa" → 3.
const COUNT_OVERLAPPING: &str = r#"## Main
Let text be "aaaa".
Let needle be "aa".
Let needleLen be 2.
Let textLen be length of text.
Let mutable count be 0.
Let mutable i be 1.
While i is at most textLen - needleLen + 1:
    Let mutable match be 1.
    Let mutable j be 0.
    While j is less than needleLen:
        If item (i + j) of text is not item (j + 1) of needle:
            Set match to 0.
            Set j to needleLen.
        Set j to j + 1.
    If match equals 1:
        Set count to count + 1.
    Set i to i + 1.
Show count.
"#;

/// Same window compare, but the body records the LAST match position instead of
/// counting — not an occurrence count, so the kernel must decline and leave the
/// (bcmp) element-wise lowering in place.
const FIND_LAST: &str = r#"## Main
Let text be "abcabcabcXXXXXabcdeXXXXX".
Let needle be "XXXXX".
Let needleLen be 5.
Let textLen be length of text.
Let mutable lastMatch be 0.
Let mutable i be 1.
While i is at most textLen - needleLen + 1:
    Let mutable match be 1.
    Let mutable j be 0.
    While j is less than needleLen:
        If item (i + j) of text is not item (j + 1) of needle:
            Set match to 0.
            Set j to needleLen.
        Set j to j + 1.
    If match equals 1:
        Set lastMatch to i.
    Set i to i + 1.
Show lastMatch.
"#;

#[test]
fn count_nest_lowers_to_simd_kernel() {
    let rust = compile_to_rust(COUNT_SEARCH).unwrap();
    assert!(
        rust.contains("__logos_count_window_matches("),
        "the naive-count nest must lower to a kernel call. Got:\n{}",
        rust
    );
    assert!(
        !rust.contains("while (j"),
        "the scalar inner byte-compare loop over `j` must be gone. Got:\n{}",
        rust
    );
    // The kernel definition must be emitted into the program (it cannot be linked).
    assert!(
        rust.contains("fn __logos_count_window_matches"),
        "the kernel definition must be emitted into the generated program. Got:\n{}",
        rust
    );
}

#[test]
fn count_is_correct_non_overlapping() {
    let r = run_logos(COUNT_SEARCH);
    assert!(r.success, "run failed: {}\n{}", r.stderr, r.rust_code);
    assert_eq!(r.stdout.trim(), "2", "two XXXXX occurrences expected");
}

#[test]
fn count_is_correct_overlapping() {
    // The kernel must reproduce the naive loop's OVERLAPPING semantics exactly.
    let r = run_logos(COUNT_OVERLAPPING);
    assert!(r.success, "run failed: {}\n{}", r.stderr, r.rust_code);
    assert_eq!(r.stdout.trim(), "3", "aa in aaaa overlaps 3 times");
}

#[test]
fn non_count_window_does_not_lower_to_kernel() {
    let rust = compile_to_rust(FIND_LAST).unwrap();
    assert!(
        !rust.contains("__logos_count_window_matches"),
        "a non-count window compare must NOT lower to the count kernel. Got:\n{}",
        rust
    );
}

#[test]
fn unrelated_program_has_no_kernel() {
    // Zero footprint: programs without a naive search never emit the kernel.
    let rust = compile_to_rust("## Main\nShow 1.\n").unwrap();
    assert!(
        !rust.contains("__logos_count_window_matches"),
        "kernel must only be emitted when the idiom is present. Got:\n{}",
        rust
    );
}
