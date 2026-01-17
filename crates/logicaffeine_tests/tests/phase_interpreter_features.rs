//! Phase: Interpreter Feature Tests
//!
//! Tests for features that should work in the interpreter.

mod common;
use common::run_interpreter;

// =============================================================================
// From Parameter Syntax
// =============================================================================

#[test]
fn interpreter_from_parameter_syntax() {
    let source = r#"## To withdraw (amount: Int) from (balance: Int) -> Int:
    Return balance - amount.

## Main
Let result be withdraw(50, 100).
Show result.
"#;
    let result = run_interpreter(source);
    assert!(result.success, "Should run without error: {}", result.error);
    assert!(result.output.contains("50"), "Should output 50, got: {}", result.output);
}

// =============================================================================
// String Concatenation with Bool
// =============================================================================

#[test]
fn interpreter_string_concat_bool() {
    let source = r#"## Main
Let x be true.
Let y be false.
Show x and y.
Show x or y.
Show not x.
"#;
    let result = run_interpreter(source);
    assert!(result.success, "Should run without error: {}", result.error);
    assert!(result.output.contains("false"), "Should output false for x and y, got: {}", result.output);
    assert!(result.output.contains("true"), "Should output true for x or y, got: {}", result.output);
}

// =============================================================================
// Set Iteration
// =============================================================================

#[test]
fn interpreter_set_iteration() {
    let source = r#"## Main
Let numbers be a new Set of Int.
Add 10 to numbers.
Add 20 to numbers.
Add 30 to numbers.

Let sum be 0.
Repeat for n in numbers:
    Set sum to sum + n.
Show sum.
"#;
    let result = run_interpreter(source);
    assert!(result.success, "Should run without error: {}", result.error);
    assert!(result.output.contains("60"), "Should output 60, got: {}", result.output);
}

// =============================================================================
// CRDT Merge
// =============================================================================

#[test]
fn interpreter_crdt_merge() {
    let source = r#"## Definition
A Stats is Shared and has:
    views: ConvergentCount.

## Main
Let local be a new Stats.
Increase local's views by 100.

Let remote be a new Stats.
Increase remote's views by 50.

Merge remote into local.
Show local's views.
"#;
    let result = run_interpreter(source);
    assert!(result.success, "Should run without error: {}", result.error);
    assert!(result.output.contains("150"), "Should output 150, got: {}", result.output);
}
