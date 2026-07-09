//! Part I correctness: numeric literals PARSE or ERROR — never silently
//! become 0.
//!
//! The audit rows: `1_000_000` and `0xff` parsed to **0** through
//! `unwrap_or(0)`; a negative float literal didn't exist (nbody writes
//! `0.0 - lit` everywhere); `infinity`/`nan` had no spelling. The spec:
//! digit separators, hex/binary/octal radix literals, unary-minus literals,
//! and `infinity`/`nan` word literals (the `true`/`false` precedent) — and a
//! literal that cannot fit is a LOUD error, never a silent 0.

mod common;
use common::{assert_compiled_equals_interpreted_eq, run_interpreter};

// =====================================================================
// Digit separators and radix literals
// =====================================================================

#[test]
fn underscore_separators_parse_exactly() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let n be 1_000_000.
Show n + 1.
"#,
        "1000001",
    );
}

#[test]
fn hex_literal_parses() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show 0xff.
"#,
        "255",
    );
}

#[test]
fn binary_literal_parses() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show 0b1010.
"#,
        "10",
    );
}

#[test]
fn octal_literal_parses() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show 0o755.
"#,
        "493",
    );
}

#[test]
fn radix_literals_with_separators() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show 0xff_ff.
"#,
        "65535",
    );
}

#[test]
fn overlong_literal_is_a_loud_error_never_zero() {
    let r = run_interpreter(
        r#"## Main
Show 99999999999999999999999999.
"#,
    );
    assert!(!r.success, "an unrepresentable literal must not silently run");
    assert!(
        !r.error.is_empty(),
        "the failure must carry a real error message"
    );
}

// =====================================================================
// Negative literals at operand position
// =====================================================================

#[test]
fn negative_float_literal_parses() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let x be -1.5.
Show x.
"#,
        "-1.5",
    );
}

#[test]
fn negative_int_literal_parses() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let x be -42.
Show x + 2.
"#,
        "-40",
    );
}

#[test]
fn negative_literal_in_arithmetic() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show -2.5 + 1.0.
"#,
        "-1.5",
    );
}

#[test]
fn binary_minus_is_untouched() {
    // `5 - 1` stays subtraction — unary minus only binds at operand position.
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show 5 - 1.
"#,
        "4",
    );
}

// =====================================================================
// infinity / nan word literals
// =====================================================================

#[test]
fn infinity_literal() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show infinity.
"#,
        "inf",
    );
}

#[test]
fn negative_infinity_literal() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show -infinity.
"#,
        "-inf",
    );
}

#[test]
fn nan_is_not_equal_to_itself() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show nan equals nan.
"#,
        "false",
    );
}
