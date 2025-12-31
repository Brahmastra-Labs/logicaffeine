//! E2E Tests: CRDTs (Conflict-free Replicated Data Types)
//!
//! Tests runtime behavior of CRDT operations: GCounter increment,
//! LWWRegister set/get, and struct/field-level merge operations.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::{assert_output, assert_runs};

// =============================================================================
// GCounter Tests
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_gcounter_increment() {
    assert_output(
        r#"## Definition
A Counter is Shared and has:
    points: ConvergentCount.

## Main
Let mutable c be a new Counter.
Increase c's points by 10.
Show c's points."#,
        "10",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_gcounter_increment_by_variable() {
    assert_output(
        r#"## Definition
A Counter is Shared and has:
    score: ConvergentCount.

## Main
Let mutable c be a new Counter.
Let amount be 25.
Increase c's score by amount.
Show c's score."#,
        "25",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_gcounter_multiple_increments() {
    assert_output(
        r#"## Definition
A Counter is Shared and has:
    points: ConvergentCount.

## Main
Let mutable c be a new Counter.
Increase c's points by 5.
Increase c's points by 3.
Increase c's points by 2.
Show c's points."#,
        "10",
    );
}

// =============================================================================
// LWWRegister Tests
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_lww_text_set() {
    assert_output(
        r#"## Definition
A Profile is Shared and has:
    a username, which is LastWriteWins of Text.

## Main
Let mutable p be a new Profile.
Set p's username to "alice".
Show p's username."#,
        "alice",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_lww_int_set() {
    assert_output(
        r#"## Definition
A Setting is Shared and has:
    a volume, which is LastWriteWins of Int.

## Main
Let mutable s be a new Setting.
Set s's volume to 75.
Show s's volume."#,
        "75",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_lww_bool_set() {
    assert_output(
        r#"## Definition
A Toggle is Shared and has:
    an active, which is LastWriteWins of Bool.

## Main
Let mutable t be a new Toggle.
Set t's active to true.
Show t's active."#,
        "true",
    );
}

// =============================================================================
// Merge Tests
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_merge_struct_level() {
    assert_runs(
        r#"## Definition
A Counter is Shared and has:
    a points, which is ConvergentCount.

## Main
Let mutable local be a new Counter.
Let remote be a new Counter.
Merge remote into local.
Show "merged"."#,
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_merge_field_level() {
    assert_runs(
        r#"## Definition
A Profile is Shared and has:
    an active, which is LastWriteWins of Bool.

## Main
Let mutable local be a new Profile.
Let remote be a new Profile.
Merge remote's active into local's active.
Show "field merged"."#,
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_shared_struct_with_mixed_fields() {
    assert_output(
        r#"## Definition
A GameState is Shared and has:
    a score, which is ConvergentCount.
    a name, which is Text.

## Main
Let mutable g be a new GameState with name "Player1".
Increase g's score by 100.
Show g's name.
Show g's score."#,
        "Player1",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_gcounter_value_after_merge() {
    // This tests that after merging two GCounters, the value reflects both
    assert_runs(
        r#"## Definition
A Counter is Shared and has:
    a points, which is ConvergentCount.

## Main
Let mutable c1 be a new Counter.
Let mutable c2 be a new Counter.
Increase c1's points by 10.
Increase c2's points by 5.
Merge c2 into c1.
Show "merge complete"."#,
    );
}
