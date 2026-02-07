//! E2E Tests: Extended Primitive Types
//!
//! Tests Char and Byte types at runtime.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::assert_exact_output;

// === CHAR E2E TESTS ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_char_literal() {
    assert_exact_output(
        r#"## Main
Let c be `a`.
Show c.
"#,
        "a",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_char_with_type() {
    assert_exact_output(
        r#"## Main
Let c: Char be `z`.
Show c.
"#,
        "z",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_char_comparison_equal() {
    assert_exact_output(
        r#"## Main
Let a be `x`.
Let b be `x`.
If a equals b:
    Show "same".
"#,
        "same",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_char_comparison_not_equal() {
    assert_exact_output(
        r#"## Main
Let a be `x`.
Let b be `y`.
If a equals b:
    Show "same".
Otherwise:
    Show "different".
"#,
        "different",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_char_escape_newline() {
    // Newline char should print as actual newline
    assert_exact_output(
        r#"## Main
Let nl be `\n`.
Show "before".
"#,
        "before",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_char_in_seq() {
    assert_exact_output(
        r#"## Main
Let chars be a new Seq of Char.
Push `a` to chars.
Push `b` to chars.
Push `c` to chars.
Show length of chars.
"#,
        "3",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_char_digit() {
    assert_exact_output(
        r#"## Main
Let d be `7`.
Show d.
"#,
        "7",
    );
}

// === BYTE E2E TESTS ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_byte_type() {
    assert_exact_output(
        r#"## Main
Let b: Byte be 255.
Show b.
"#,
        "255",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_byte_zero() {
    assert_exact_output(
        r#"## Main
Let b: Byte be 0.
Show b.
"#,
        "0",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_byte_arithmetic() {
    assert_exact_output(
        r#"## Main
Let a: Byte be 100.
Let b: Byte be 50.
Show a + b.
"#,
        "150",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_byte_in_seq() {
    assert_exact_output(
        r#"## Main
Let bytes be a new Seq of Byte.
Push 1 to bytes.
Push 2 to bytes.
Push 3 to bytes.
Show length of bytes.
"#,
        "3",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_byte_function_param() {
    assert_exact_output(
        r#"## To double (b: Byte) -> Byte:
    Return b * 2.

## Main
Show double(50).
"#,
        "100",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_byte_comparison() {
    assert_exact_output(
        r#"## Main
Let a: Byte be 100.
Let b: Byte be 50.
If a > b:
    Show "greater".
"#,
        "greater",
    );
}
