//! E2E Tests: Basic Expressions
//!
//! Tests that basic arithmetic, boolean, and text expressions
//! compile to Rust and run correctly.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::assert_output;

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_arithmetic_add() {
    assert_output("## Main\nShow 2 + 3.", "5");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_arithmetic_subtract() {
    assert_output("## Main\nShow 10 - 4.", "6");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_arithmetic_multiply() {
    assert_output("## Main\nShow 3 * 4.", "12");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_arithmetic_divide() {
    assert_output("## Main\nShow 10 / 2.", "5");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_arithmetic_nested() {
    // Test parenthesized expressions
    assert_output("## Main\nShow (2 + 3) * 4.", "20");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_boolean_true() {
    assert_output("## Main\nShow true.", "true");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_boolean_false() {
    assert_output("## Main\nShow false.", "false");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_text_literal() {
    assert_output("## Main\nShow \"Hello\".", "Hello");
}

// === NEW TESTS ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_negative_via_subtraction() {
    // Test negative numbers via subtraction from zero
    assert_output("## Main\nLet x be 0 - 5.\nShow x.", "-5");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_zero_literal() {
    assert_output("## Main\nShow 0.", "0");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_large_number() {
    assert_output("## Main\nShow 999999.", "999999");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_operator_precedence() {
    // Multiplication before addition: 2 + 3 * 4 = 2 + 12 = 14
    assert_output("## Main\nShow 2 + 3 * 4.", "14");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_left_associative() {
    // Left-to-right: 10 - 5 - 2 = 5 - 2 = 3
    assert_output("## Main\nShow 10 - 5 - 2.", "3");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_division_truncation() {
    // Integer division should truncate
    assert_output("## Main\nShow 7 / 2.", "3");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_complex_arithmetic() {
    // (10 + 5) * 2 - 3 = 15 * 2 - 3 = 30 - 3 = 27
    assert_output("## Main\nShow (10 + 5) * 2 - 3.", "27");
}
