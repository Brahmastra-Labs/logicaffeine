//! Phase 27: Imperative Guards
//! Tests for Phase 2 gate requirements:
//! - Index 0 Guard
//!
//! Note: the condition-position `is`-for-equality rejection this file once held is
//! obsolete — `If x is 5:` is legal value equality (see `correctness_is_equality`),
//! and the still-valid statement-position rejection (`x is 5.`) is guarded by
//! `phase22_is_rejection`. Spec §4.1.3 documents the context-sensitive rule.

use logicaffeine_compile::compile::compile_to_rust;

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
