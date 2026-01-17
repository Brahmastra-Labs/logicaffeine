//! Phase: Interpreter CRDT Support
//!
//! Tests for CRDT operations (Increase, Decrease) in the interpreter.
//! These currently fail because the interpreter returns "not supported".

mod common;
use common::run_interpreter;

// =============================================================================
// CRDT Increase Tests
// =============================================================================

#[test]
fn interpreter_crdt_increase_simple() {
    let source = r#"## Definition
A Counter is Shared and has:
    points: ConvergentCount.

## Main
Let mutable c be a new Counter.
Increase c's points by 10.
Show c's points.
"#;
    let result = run_interpreter(source);
    assert!(result.success, "Should run without error: {}", result.error);
    assert!(result.output.contains("10"), "Should output 10, got: {}", result.output);
}

#[test]
fn interpreter_crdt_increase_multiple() {
    let source = r#"## Definition
A Counter is Shared and has:
    points: ConvergentCount.

## Main
Let mutable c be a new Counter.
Increase c's points by 10.
Increase c's points by 5.
Increase c's points by 3.
Show c's points.
"#;
    let result = run_interpreter(source);
    assert!(result.success, "Should run without error: {}", result.error);
    assert!(result.output.contains("18"), "Should output 18, got: {}", result.output);
}

#[test]
fn interpreter_crdt_increase_variable_amount() {
    let source = r#"## Definition
A Counter is Shared and has:
    score: ConvergentCount.

## Main
Let mutable c be a new Counter.
Let amount be 25.
Increase c's score by amount.
Show c's score.
"#;
    let result = run_interpreter(source);
    assert!(result.success, "Should run without error: {}", result.error);
    assert!(result.output.contains("25"), "Should output 25, got: {}", result.output);
}

// =============================================================================
// CRDT Decrease Tests (TallyCount / PNCounter)
// =============================================================================

#[test]
fn interpreter_crdt_decrease_simple() {
    let source = r#"## Definition
A Tally is Shared and has:
    count: TallyCount.

## Main
Let mutable t be a new Tally.
Increase t's count by 10.
Decrease t's count by 3.
Show t's count.
"#;
    let result = run_interpreter(source);
    assert!(result.success, "Should run without error: {}", result.error);
    assert!(result.output.contains("7"), "Should output 7, got: {}", result.output);
}

#[test]
fn interpreter_crdt_decrease_negative_result() {
    let source = r#"## Definition
A Tally is Shared and has:
    count: TallyCount.

## Main
Let mutable t be a new Tally.
Decrease t's count by 5.
Show t's count.
"#;
    let result = run_interpreter(source);
    assert!(result.success, "Should run without error: {}", result.error);
    assert!(result.output.contains("-5"), "Should output -5, got: {}", result.output);
}

// =============================================================================
// CRDT with Multiple Fields
// =============================================================================

#[test]
fn interpreter_crdt_multiple_fields() {
    let source = r#"## Definition
A Scoreboard is Shared and has:
    points: ConvergentCount.
    penalties: TallyCount.

## Main
Let mutable s be a new Scoreboard.
Increase s's points by 100.
Increase s's penalties by 10.
Show s's points.
Show s's penalties.
"#;
    let result = run_interpreter(source);
    assert!(result.success, "Should run without error: {}", result.error);
    assert!(result.output.contains("100"), "Should output points 100, got: {}", result.output);
    assert!(result.output.contains("10"), "Should output penalties 10, got: {}", result.output);
}
