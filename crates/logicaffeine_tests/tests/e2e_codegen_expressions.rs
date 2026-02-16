//! E2E Codegen Tests: Basic Expressions
//!
//! Mirrors e2e_expressions.rs but compiles through the Rust codegen pipeline.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::assert_exact_output;

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_arithmetic_add() {
    assert_exact_output("## Main\nShow 2 + 3.", "5");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_arithmetic_subtract() {
    assert_exact_output("## Main\nShow 10 - 4.", "6");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_arithmetic_multiply() {
    assert_exact_output("## Main\nShow 3 * 4.", "12");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_arithmetic_divide() {
    assert_exact_output("## Main\nShow 10 / 2.", "5");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_arithmetic_nested() {
    assert_exact_output("## Main\nShow (2 + 3) * 4.", "20");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_boolean_true() {
    assert_exact_output("## Main\nShow true.", "true");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_boolean_false() {
    assert_exact_output("## Main\nShow false.", "false");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_text_literal() {
    assert_exact_output("## Main\nShow \"Hello\".", "Hello");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_negative_via_subtraction() {
    assert_exact_output("## Main\nLet x be 0 - 5.\nShow x.", "-5");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_zero_literal() {
    assert_exact_output("## Main\nShow 0.", "0");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_large_number() {
    assert_exact_output("## Main\nShow 999999.", "999999");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_operator_precedence() {
    assert_exact_output("## Main\nShow 2 + 3 * 4.", "14");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_left_associative() {
    assert_exact_output("## Main\nShow 10 - 5 - 2.", "3");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_division_truncation() {
    assert_exact_output("## Main\nShow 7 / 2.", "3");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_complex_arithmetic() {
    assert_exact_output("## Main\nShow (10 + 5) * 2 - 3.", "27");
}
