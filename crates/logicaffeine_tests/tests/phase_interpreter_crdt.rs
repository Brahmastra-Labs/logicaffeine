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

// =============================================================================
// Rich CRDTs on the VM tier — `run_interpreter` compiles to the bytecode VM and a
// debug shadow oracle asserts the tree-walker agrees, so a PASS here proves the VM
// executes OR-Set / RGA semantics identically to the reference engine.
// =============================================================================

#[test]
fn interpreter_shared_set_add_and_contains() {
    let source = r#"## Definition
A Party is Shared and has:
    a guests, which is a SharedSet of Text.

## Main
Let mutable p be a new Party.
Add "Alice" to p's guests.
If p's guests contains "Alice":
    Show "found".
Otherwise:
    Show "missing".
Show length of p's guests.
"#;
    let result = run_interpreter(source);
    assert!(result.success, "Should run without error: {}", result.error);
    assert!(result.output.contains("found"), "Should find Alice, got: {}", result.output);
    assert!(result.output.contains("1"), "Set has one element, got: {}", result.output);
}

#[test]
fn interpreter_shared_set_add_wins_over_concurrent_remove() {
    // The distinguishing OR-Set property, executed on the VM: b's concurrent add of "X"
    // survives a's observed-remove after the merge.
    let source = r#"## Definition
A Party is Shared and has:
    a guests, which is a SharedSet of Text.

## Main
Let mutable a be a new Party.
Let mutable b be a new Party.
Add "X" to a's guests.
Add "X" to b's guests.
Remove "X" from a's guests.
Merge b into a.
If a's guests contains "X":
    Show "present".
Otherwise:
    Show "absent".
"#;
    let result = run_interpreter(source);
    assert!(result.success, "Should run without error: {}", result.error);
    assert!(result.output.contains("present"), "Concurrent add must win, got: {}", result.output);
}

#[test]
fn interpreter_shared_sequence_append_and_length() {
    let source = r#"## Definition
A Document is Shared and has:
    a lines, which is a SharedSequence of Text.

## Main
Let mutable d be a new Document.
Append "Line 1" to d's lines.
Append "Line 2" to d's lines.
Show length of d's lines.
"#;
    let result = run_interpreter(source);
    assert!(result.success, "Should run without error: {}", result.error);
    assert!(result.output.contains("2"), "Sequence has two elements, got: {}", result.output);
}

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
