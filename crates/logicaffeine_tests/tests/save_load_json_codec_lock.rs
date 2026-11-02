//! Save/Load and store persistence of ANY value — lists, records, nested — through the derived
//! JSON codec (`parser/lgx_lower/{json,stores,elements}.rs`). Closes gauntlet's G2 (`Save` of a
//! list silently miscompiled), G3 (`Load` into a list silently dropped the action), and G18 (a
//! `## Store` could not hold a `Seq`/record field at all).
//!
//! Every test compares the page's lowered program on the TREE-WALKER (the reference) against the
//! same program on the LOWERED BYTECODE VM via `page_host::compare_tests`: two engines running the
//! one `parser/lgx_lower` output this codec writes, every axis compared — the browser storage the
//! codec's text lands in included — so a divergence fails loudly rather than merely "it compiled".

use logicaffeine_compile::compile::compile_to_wasm;
use logicaffeine_compile::page_host::{compare_tests, PageTierKind, TierRun};

const BUDGET: u64 = 50_000_000;

/// Run every `## Test` in `source` on the tree-walker and the lowered VM, asserting neither tier
/// refuses the scenario and the two tiers trace IDENTICALLY (no axis differs at any statement).
fn assert_no_divergence(source: &str) {
    let all = compare_tests(source, &[PageTierKind::LoweredVm], 0, BUDGET, None)
        .unwrap_or_else(|e| panic!("failed to parse/run the test scenarios: {e}\n{source}"));
    assert!(
        !all.is_empty(),
        "no page scenario was discovered in this source:\n{source}"
    );
    for c in &all {
        match &c.candidates[0].1 {
            TierRun::Refused { statement, reason } => panic!(
                "test `{}` was refused by the lowered VM at statement {statement}: {reason}\n{source}",
                c.test
            ),
            TierRun::Traced(t) => {
                let diff = c.reference.compare(t);
                assert!(
                    diff.axes.is_empty(),
                    "test `{}` diverged between the tree-walker and the lowered VM: {:?}\n{source}",
                    c.test,
                    diff.axes
                );
            }
        }
    }
}

/// G2/G3 — `Save`/`Load` of a `Seq of Text` field round-trips: added items survive a `Save`, and a
/// cleared field is restored by a `Load` from the same key, on both the tree-walker and the
/// lowered VM identically.
#[test]
fn seq_of_text_save_and_load_round_trips_on_every_tier() {
    let source = "\
## Page /

State:
    Let names be a new Seq of Text.

Actions:
    To addName:
        Push \"ada\" to names.
        Save names as \"names\".
    To forgetAndReload:
        Set names to a new Seq of Text.
        Load \"names\" into names.

View:
    <button id=\"add\" on-click={addName}>add</button>
    <button id=\"reload\" on-click={forgetAndReload}>reload</button>
    <p id=\"count\">{length of names}</p>

## Test: a saved list survives being cleared and reloaded

Click \"#add\".
Expect that the text of \"#count\" is equal to \"1\".
Click \"#reload\".
Expect that the text of \"#count\" is equal to \"1\".
";
    assert_no_divergence(source);
    assert!(
        compile_to_wasm(source).is_ok(),
        "a Seq of Text field's Save/Load must compile for the browser"
    );
}

/// G18 — a plain, unpersisted `## Store` can hold a `Seq of Text` field, mutated by one component
/// and observed by a sibling through the shared store, on the tree-walker and the lowered VM.
#[test]
fn a_local_store_holding_a_seq_field_shares_state_across_components() {
    let source = "\
## Store Tags

State:
    Let names be a new Seq of Text.

Actions:
    To addName (n: Text):
        Push n to names.

## Component Badge

Use Tags.

View:
    <span id=\"count\">{length of Tags's names}</span>

## Page /

Use Tags.

Actions:
    To add:
        Call Tags's addName with \"x\".

View:
    <Badge/>
    <button id=\"add\" on-click={add}>add</button>

## Test: adding through the page's own action is seen by the sibling badge

Expect that the text of \"#count\" is equal to \"0\".
Click \"#add\".
Expect that the text of \"#count\" is equal to \"1\".
Click \"#add\".
Expect that the text of \"#count\" is equal to \"2\".
";
    assert_no_divergence(source);
    assert!(
        compile_to_wasm(source).is_ok(),
        "a local store's Seq field must compile for the browser"
    );
}

/// G2/G3, nested — a `Seq of <record>` field round-trips through `Save`/`Load` too: the record's
/// own fields survive, not merely the list's length.
#[test]
fn seq_of_record_save_and_load_round_trips_on_every_tier() {
    let source = "\
## A Todo has:
    an id, which is Int.
    a text, which is Text.

## Page /

State:
    Let todos be a new Seq of Todo.

Actions:
    To addOne:
        Push (a new Todo with id 1 and text \"milk\") to todos.
        Save todos as \"todos\".
    To forgetAndReload:
        Set todos to a new Seq of Todo.
        Load \"todos\" into todos.

View:
    <button id=\"add\" on-click={addOne}>add</button>
    <button id=\"reload\" on-click={forgetAndReload}>reload</button>
    <ul>
        {for each t in todos, keyed by t's id:}
            <li id=\"row-{t's id}\">{t's text}</li>
        {end}
    </ul>

## Test: a saved list of records keeps its own field values after a reload

Click \"#add\".
Expect that the text of \"#row-1\" is equal to \"milk\".
Click \"#reload\".
Expect that the text of \"#row-1\" is equal to \"milk\".
";
    assert_no_divergence(source);
    assert!(
        compile_to_wasm(source).is_ok(),
        "a Seq of <record> field's Save/Load must compile for the browser"
    );
}

/// A PERSISTED `## Store`'s `Seq of Text` field round-trips through the derived JSON codec too —
/// `store_read_back`/`store_write_through`, not merely a bare `Save`/`Load` statement.
#[test]
fn a_persisted_stores_seq_field_round_trips_through_storage() {
    let source = "\
## Store Tags persisted as \"tags\"

State:
    Let names be a new Seq of Text.

Actions:
    To addName (n: Text):
        Push n to names.

## Page /

Use Tags.

Actions:
    To add:
        Call Tags's addName with \"x\".

View:
    <span id=\"count\">{length of Tags's names}</span>
    <button id=\"add\" on-click={add}>add</button>

## Test: a persisted store's own list field is written through

Expect that the text of \"#count\" is equal to \"0\".
Click \"#add\".
Expect that the text of \"#count\" is equal to \"1\".
";
    assert_no_divergence(source);
    assert!(
        compile_to_wasm(source).is_ok(),
        "a persisted store's Seq field must compile for the browser"
    );
}
