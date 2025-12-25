//! Phase 27: Imperative Guards
//! Tests for Phase 2 gate requirements:
//! - Reject `x is [value]` pattern
//! - Index 0 Guard

use logos::compile::compile_to_rust;

#[test]
fn reject_is_for_value_equality() {
    // Phase 2 Gate: "x is 5" should error in imperative mode
    // Spec §4.1.3: Use `equals` for value equality, not `is`
    let source = "## Main\nIf x is 5:\n    Return.";
    let result = compile_to_rust(source);
    assert!(result.is_err(), "Should reject 'x is 5' in imperative mode: {:?}", result);
}

#[test]
fn reject_item_zero() {
    // Phase 2 Gate: "item 0 of list" should error
    // Spec §11.2.1: LOGOS uses 1-based indexing
    let source = "## Main\nLet x be item 0 of list.";
    let result = compile_to_rust(source);
    assert!(result.is_err(), "Should reject 'item 0 of list': {:?}", result);
}

#[test]
fn accept_equals_for_value_equality() {
    // `x equals 5` should work
    let source = "## Main\nIf x equals 5:\n    Return.";
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should accept 'x equals 5': {:?}", result);
}

#[test]
fn accept_item_one() {
    // `item 1 of list` should work (first element)
    let source = "## Main\nLet x be item 1 of list.";
    let result = compile_to_rust(source);
    assert!(result.is_ok(), "Should accept 'item 1 of list': {:?}", result);
}
