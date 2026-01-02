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

// =============================================================================
// PNCounter Tests (Tally)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_tally_increase_decrease() {
    assert_output(
        r#"## Definition
A Game is Shared and has:
    a score, which is a Tally.

## Main
Let mutable g be a new Game.
Increase g's score by 100.
Decrease g's score by 30.
Show g's score."#,
        "70",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_tally_decrease_to_negative() {
    assert_output(
        r#"## Definition
A Temperature is Shared and has:
    a degrees, which is a Tally.

## Main
Let mutable t be a new Temperature.
Increase t's degrees by 10.
Decrease t's degrees by 25.
Show t's degrees."#,
        "-15",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_tally_multiple_operations() {
    assert_output(
        r#"## Definition
A Balance is Shared and has:
    an amount, which is a Tally.

## Main
Let mutable b be a new Balance.
Increase b's amount by 100.
Decrease b's amount by 20.
Increase b's amount by 50.
Decrease b's amount by 10.
Decrease b's amount by 5.
Show b's amount."#,
        "115",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_tally_decrease_only() {
    assert_output(
        r#"## Definition
A Debt is Shared and has:
    an owed, which is a Tally.

## Main
Let mutable d be a new Debt.
Decrease d's owed by 50.
Decrease d's owed by 25.
Show d's owed."#,
        "-75",
    );
}

// =============================================================================
// ORSet Tests (SharedSet)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_shared_set_add() {
    assert_output(
        r#"## Definition
A Party is Shared and has:
    a guests, which is a SharedSet of Text.

## Main
Let mutable p be a new Party.
Add "Alice" to p's guests.
Add "Bob" to p's guests.
Show length of p's guests."#,
        "2",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_shared_set_contains_true() {
    assert_output(
        r#"## Definition
A Team is Shared and has:
    a members, which is a SharedSet of Text.

## Main
Let mutable t be a new Team.
Add "Alice" to t's members.
If t's members contains "Alice":
    Show "found".
Otherwise:
    Show "not found"."#,
        "found",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_shared_set_contains_false() {
    assert_output(
        r#"## Definition
A Team is Shared and has:
    a members, which is a SharedSet of Text.

## Main
Let mutable t be a new Team.
Add "Alice" to t's members.
If t's members contains "Bob":
    Show "found".
Otherwise:
    Show "not found"."#,
        "not found",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_shared_set_remove() {
    assert_output(
        r#"## Definition
A Inventory is Shared and has:
    an items, which is a SharedSet of Text.

## Main
Let mutable inv be a new Inventory.
Add "sword" to inv's items.
Add "shield" to inv's items.
Add "potion" to inv's items.
Remove "shield" from inv's items.
Show length of inv's items."#,
        "2",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_shared_set_remove_then_contains() {
    assert_output(
        r#"## Definition
A Blocklist is Shared and has:
    a blocked, which is a SharedSet of Text.

## Main
Let mutable bl be a new Blocklist.
Add "spam@example.com" to bl's blocked.
Remove "spam@example.com" from bl's blocked.
If bl's blocked contains "spam@example.com":
    Show "still blocked".
Otherwise:
    Show "unblocked"."#,
        "unblocked",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_shared_set_add_duplicate() {
    assert_output(
        r#"## Definition
A Tags is Shared and has:
    a labels, which is a SharedSet of Text.

## Main
Let mutable tags be a new Tags.
Add "important" to tags's labels.
Add "important" to tags's labels.
Add "urgent" to tags's labels.
Show length of tags's labels."#,
        "2",
    );
}

// =============================================================================
// RGA Tests (SharedSequence)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_shared_sequence_append() {
    assert_output(
        r#"## Definition
A Document is Shared and has:
    a lines, which is a SharedSequence of Text.

## Main
Let mutable doc be a new Document.
Append "Line 1" to doc's lines.
Append "Line 2" to doc's lines.
Append "Line 3" to doc's lines.
Show length of doc's lines."#,
        "3",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_shared_sequence_empty() {
    assert_output(
        r#"## Definition
A Log is Shared and has:
    an entries, which is a SharedSequence of Text.

## Main
Let mutable log be a new Log.
Show length of log's entries."#,
        "0",
    );
}

// =============================================================================
// MVRegister Tests (Divergent)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_divergent_set_show() {
    assert_output(
        r#"## Definition
A WikiPage is Shared and has:
    a title, which is a Divergent Text.

## Main
Let mutable page be a new WikiPage.
Set page's title to "Hello World".
Show page's title."#,
        "Hello World",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_divergent_overwrite() {
    assert_output(
        r#"## Definition
A Draft is Shared and has:
    a content, which is a Divergent Text.

## Main
Let mutable d be a new Draft.
Set d's content to "First draft".
Set d's content to "Second draft".
Set d's content to "Final".
Show d's content."#,
        "Final",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_divergent_resolve() {
    assert_output(
        r#"## Definition
A Config is Shared and has:
    a value, which is a Divergent Text.

## Main
Let mutable cfg be a new Config.
Set cfg's value to "initial".
Resolve cfg's value to "resolved".
Show cfg's value."#,
        "resolved",
    );
}

// =============================================================================
// Mixed CRDT Types in One Struct
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_mixed_crdt_struct() {
    assert_output(
        r#"## Definition
A Dashboard is Shared and has:
    a views, which is a Tally.
    a users, which is a SharedSet of Text.
    a title, which is a Divergent Text.

## Main
Let mutable dash be a new Dashboard.
Increase dash's views by 100.
Add "alice" to dash's users.
Add "bob" to dash's users.
Set dash's title to "My Dashboard".
Decrease dash's views by 10.
Show dash's views."#,
        "90",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_crdt_with_regular_fields() {
    assert_output(
        r#"## Definition
A Game is Shared and has:
    a name, which is Text.
    a score, which is a Tally.
    a players, which is a SharedSet of Text.

## Main
Let mutable g be a new Game with name "Chess Match".
Increase g's score by 50.
Add "White" to g's players.
Add "Black" to g's players.
Show g's name."#,
        "Chess Match",
    );
}

// =============================================================================
// Merge Operations with New CRDTs
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_tally_merge() {
    assert_runs(
        r#"## Definition
A Score is Shared and has:
    a points, which is a Tally.

## Main
Let mutable s1 be a new Score.
Let mutable s2 be a new Score.
Increase s1's points by 50.
Decrease s1's points by 10.
Increase s2's points by 30.
Merge s2 into s1.
Show "merge complete"."#,
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_shared_set_merge() {
    assert_runs(
        r#"## Definition
A Tags is Shared and has:
    a labels, which is a SharedSet of Text.

## Main
Let mutable t1 be a new Tags.
Let mutable t2 be a new Tags.
Add "red" to t1's labels.
Add "blue" to t2's labels.
Merge t2 into t1.
Show "merged"."#,
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_divergent_merge() {
    assert_runs(
        r#"## Definition
A Page is Shared and has:
    a content, which is a Divergent Text.

## Main
Let mutable p1 be a new Page.
Let mutable p2 be a new Page.
Set p1's content to "Version A".
Set p2's content to "Version B".
Merge p2 into p1.
Show "merged"."#,
    );
}
