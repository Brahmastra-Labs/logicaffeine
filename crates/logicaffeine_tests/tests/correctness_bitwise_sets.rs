//! Part I correctness: REAL bitwise operators — and the set operators the
//! symbols mean on Sets.
//!
//! The audit rows: `~` wasn't even a lexer character (silently dropped, so
//! `Show ~5.` printed 5); `a | b` on two Sets returned a BOOL (the truthiness
//! OR) instead of the union. The spec: `& | ^ ~` are the bitwise spellings on
//! Int (the logical words stay logical — Wave 5a), and on Sets the same
//! symbols dispatch to intersection/union/symmetric-difference, with `-` as
//! difference and `a without b` as its English spelling. The natural-language
//! `&` (firm names, coordination) is untouched — prose never reaches the
//! imperative operator grammar.

mod common;
use common::assert_compiled_equals_interpreted_eq;

// =====================================================================
// Bitwise on Int
// =====================================================================

#[test]
fn ampersand_is_bitwise_and() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show 6 & 3.
"#,
        "2",
    );
}

#[test]
fn pipe_is_bitwise_or() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show 6 | 3.
"#,
        "7",
    );
}

#[test]
fn caret_is_bitwise_xor() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show 6 ^ 3.
"#,
        "5",
    );
}

#[test]
fn tilde_is_bitwise_complement() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show ~5.
"#,
        "-6",
    );
}

#[test]
fn bitwise_chain_with_masks() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let flags be 0b1010.
Show flags & 0b0110 | 0b0001.
"#,
        "3",
    );
}

// =====================================================================
// The same symbols on Sets are the set operations
// =====================================================================

#[test]
fn pipe_on_sets_is_union() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show ({1, 2} | {2, 3}) equals {1, 2, 3}.
"#,
        "true",
    );
}

#[test]
fn ampersand_on_sets_is_intersection() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show ({1, 2, 3} & {2, 3, 4}) equals {2, 3}.
"#,
        "true",
    );
}

#[test]
fn minus_on_sets_is_difference() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show ({1, 2, 3} - {2}) equals {1, 3}.
"#,
        "true",
    );
}

#[test]
fn caret_on_sets_is_symmetric_difference() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show ({1, 2} ^ {2, 3}) equals {1, 3}.
"#,
        "true",
    );
}

#[test]
fn without_is_the_english_difference() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show ({1, 2, 3} without {2}) equals {1, 3}.
"#,
        "true",
    );
}
