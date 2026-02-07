//! E2E Tests: Variables & Mutability
//!
//! Tests that Let/Set statements work correctly and mutability
//! is properly inferred.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::assert_exact_output;

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_let_and_show() {
    assert_exact_output("## Main\nLet x be 42.\nShow x.", "42");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_set_mutates() {
    assert_exact_output("## Main\nLet x be 5.\nSet x to 10.\nShow x.", "10");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_multiple_variables() {
    assert_exact_output("## Main\nLet a be 3.\nLet b be 4.\nShow a + b.", "7");
}
