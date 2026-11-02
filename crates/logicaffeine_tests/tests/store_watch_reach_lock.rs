//! LOCK — a store watcher hears every move of the field it watches, wherever the call that moved
//! it is written, once per turn, and a call that does not run moves nothing.
//!
//! `When <Store>'s <field> changes:` runs after the turn that moved the field commits. The lowered
//! page learns of the move from the STORE: its action flags each watched field its commit changed,
//! and the round every turn ends with (`__lgx_wake_stores`) takes the flags and runs the watchers.
//! So a call in a branch, a loop, a sibling's body, another store's action or `When the page
//! loads:` is heard like a call at the top of a turn — and a branch that is not taken reads no
//! "before" value that was never bound.
//!
//! Each page runs on the reference and on both lowered tiers. Where the reference answers
//! differently, the test says why and holds the lowered tiers alone.

use logicaffeine_compile::page_host::{compare_tests, PageTierKind, PageTrace, TierRun};

/// The one `## Test` in `source`: the reference's trace, and each lowered tier's.
fn traced(source: &str) -> (PageTrace, Vec<(&'static str, PageTrace)>) {
    let mut all = compare_tests(
        source,
        &[PageTierKind::LoweredHere, PageTierKind::LoweredVm],
        0,
        50_000_000,
        None,
    )
    .unwrap_or_else(|e| panic!("the page must parse: {e}\n{source}"));
    assert_eq!(all.len(), 1, "exactly one page scenario:\n{source}");
    let c = all.remove(0);
    let lowered = c
        .candidates
        .into_iter()
        .map(|(kind, run)| match run {
            TierRun::Traced(t) => (kind.label(), t),
            other => panic!(
                "the {} tier must trace the scenario, not {other:?}",
                kind.label()
            ),
        })
        .collect();
    (c.reference, lowered)
}

/// After the last statement: no failure, nothing stopped, and `shown` in the document.
fn settles_showing(tier: &str, trace: &PageTrace, shown: &str) {
    assert!(
        trace.stopped.is_none(),
        "{tier}: the run goes on: {:?}",
        trace.stopped
    );
    for step in &trace.steps {
        assert!(
            step.snapshot.failure.is_none(),
            "{tier}: no turn fails: {:?}",
            step.snapshot.failure
        );
    }
    let last = &trace.steps.last().expect("a traced step").snapshot.document;
    assert!(
        last.contains(shown),
        "{tier}: the page shows `{shown}`:\n{last}"
    );
}

fn builds_for_the_browser(source: &str) {
    let built = logicaffeine_compile::compile::compile_to_wasm(source);
    assert!(
        built.is_ok(),
        "the page builds for the browser: {:?}",
        built.err()
    );
}

/// `{body}` is `go`'s body; W counts how often it heard S's v move.
fn page(body: &str) -> String {
    format!(
        r##"## Store S
State:
    Let v be 0.
Actions:
    To bump:
        Set v to v + 1.

## Component W
Use S.
State:
    Let heard be 0.
Actions:
    When S's v changes:
        Set heard to heard + 1.
View:
    <span id="w">{{heard}}|{{S's v}}</span>

## Page /
Use S.
State:
    Let n be 0.
Actions:
    To go:
{body}
View:
    <main><W/><button id="x" on-click={{go}}>x</button></main>

## Test: go
    Click "#x".
"##
    )
}

#[test]
fn a_store_call_inside_a_branch_wakes_its_watcher() {
    let source = page("        If n is 0:\n            Call S's bump.");
    let (reference, lowered) = traced(&source);
    settles_showing("reference", &reference, "<span id=\"w\">1|1</span>");
    for (tier, trace) in lowered {
        settles_showing(tier, &trace, "<span id=\"w\">1|1</span>");
    }
    builds_for_the_browser(&source);
}

#[test]
fn a_store_call_the_branch_skips_wakes_nothing_and_fails_nothing() {
    let source = page("        If n is 1:\n            Call S's bump.");
    let (reference, lowered) = traced(&source);
    settles_showing("reference", &reference, "<span id=\"w\">0|0</span>");
    for (tier, trace) in lowered {
        settles_showing(tier, &trace, "<span id=\"w\">0|0</span>");
    }
    builds_for_the_browser(&source);
}

#[test]
fn a_store_call_inside_a_while_is_heard_once_per_turn() {
    let source = page(
        "        While n is less than 2:\n            Set n to n + 1.\n            Call S's bump.",
    );
    let (_, lowered) = traced(&source);
    for (tier, trace) in lowered {
        settles_showing(tier, &trace, "<span id=\"w\">1|2</span>");
    }
    builds_for_the_browser(&source);
}

/// One turn, three moves, one round: the watcher hears that v moved, once. The reference runs a
/// watcher once per CALL that moved its field (three here) — an AST-runtime difference that closes
/// when the lowered page is the reference.
#[test]
fn a_store_call_inside_a_loop_wakes_its_watcher_once_per_turn() {
    let source = page("        Repeat for i from 1 to 3:\n            Call S's bump.");
    let (_, lowered) = traced(&source);
    for (tier, trace) in lowered {
        settles_showing(tier, &trace, "<span id=\"w\">1|3</span>");
    }
    builds_for_the_browser(&source);
}

/// Two calls that move one field are one move to its watcher, not one per call site.
#[test]
fn two_calls_that_move_one_field_wake_its_watcher_once() {
    let source = page("        Call S's bump.\n        Call S's bump.");
    let (_, lowered) = traced(&source);
    for (tier, trace) in lowered {
        settles_showing(tier, &trace, "<span id=\"w\">1|2</span>");
    }
    builds_for_the_browser(&source);
}

const STORE_CALLS_STORE: &str = r##"## Store B
State:
    Let b be 0.
Actions:
    To bump:
        Set b to b + 1.

## Store A
State:
    Let a be 0.
Actions:
    To poke:
        Set a to a + 1.
        Call B's bump.

## Component W
Use B.
State:
    Let heard be 0.
Actions:
    When B's b changes:
        Set heard to heard + 1.
View:
    <span id="w">{heard}|{B's b}</span>

## Page /
Use A.
Actions:
    To go:
        Call A's poke.
View:
    <main><W/><button id="x" on-click={go}>x</button></main>

## Test: a store's action moves another store
    Click "#x".
"##;

/// A store action that calls another store's action moves that store as surely as a component's
/// turn does, and the watchers of the store it moved hear it.
#[test]
fn a_store_that_calls_a_watched_store_wakes_its_watchers() {
    let (reference, lowered) = traced(STORE_CALLS_STORE);
    settles_showing("reference", &reference, "<span id=\"w\">1|1</span>");
    for (tier, trace) in lowered {
        settles_showing(tier, &trace, "<span id=\"w\">1|1</span>");
    }
    builds_for_the_browser(STORE_CALLS_STORE);
}

const PAGE_LOAD_MOVES: &str = r##"## Store S
State:
    Let v be 0.
Actions:
    To bump:
        Set v to v + 1.

## Component W
Use S.
State:
    Let heard be 0.
Actions:
    When S's v changes:
        Set heard to heard + 1.
View:
    <span id="w">{heard}|{S's v}</span>

## Page /
Use S.
State:
    Let n be 0.
Actions:
    When the page loads:
        Call S's bump.
    To go:
        Set n to n + 1.
View:
    <main><W/><p id="n">{n}</p><button id="x" on-click={go}>x</button></main>

## Test: a turn after the page loaded
    Click "#x".
"##;

/// `When the page loads:` moves a watched store before anything is drawn: the watcher that the
/// first paint places sees the store already moved, and hears nothing — then or at any later turn
/// that moves no store.
#[test]
fn a_page_load_that_moves_a_watched_store_leaves_later_turns_whole() {
    let (reference, lowered) = traced(PAGE_LOAD_MOVES);
    settles_showing("reference", &reference, "<span id=\"w\">0|1</span>");
    for (tier, trace) in lowered {
        settles_showing(tier, &trace, "<span id=\"w\">0|1</span>");
        assert!(
            trace
                .steps
                .last()
                .unwrap()
                .snapshot
                .document
                .contains("<p id=\"n\">1</p>"),
            "{tier}: the turn ran"
        );
    }
    builds_for_the_browser(PAGE_LOAD_MOVES);
}

const TRAPS_AFTER_A_MOVE: &str = r##"## Store S
State:
    Let v be 0.
Actions:
    To bump:
        Set v to v + 1.

## Component W
Use S.
State:
    Let heard be 0.
Actions:
    When S's v changes:
        Set heard to heard + 1.
View:
    <span id="w">{heard}|{S's v}</span>

## Page /
Use S.
State:
    Let n be 0.
Actions:
    To go:
        Call S's bump.
        Set n to item 9 of [1].
    To calm:
        Set n to 2.
View:
    <main><W/><p id="n">{n}</p><button id="x" on-click={go}>x</button><button id="c" on-click={calm}>c</button></main>

## Test: a turn traps after its store call committed
    Click "#x".
    Click "#c".
"##;

/// A store action commits when it returns, so a turn that traps after calling it has still moved
/// the store: the watcher hears it at that turn, not at whichever turn happens to come next.
#[test]
fn a_turn_that_traps_after_moving_a_store_still_wakes_its_watchers() {
    let (_, lowered) = traced(TRAPS_AFTER_A_MOVE);
    for (tier, trace) in lowered {
        assert!(trace.stopped.is_none(), "{tier}: {:?}", trace.stopped);
        let first = &trace.steps[1].snapshot;
        assert!(
            first.failure.is_some(),
            "{tier}: the trap is the page's unhandled failure"
        );
        assert!(
            first.document.contains("<span id=\"w\">1|1</span>"),
            "{tier}: heard at the trapping turn:\n{}",
            first.document
        );
        let second = &trace.steps[2].snapshot.document;
        assert!(
            second.contains("<span id=\"w\">1|1</span>"),
            "{tier}: a later turn moves nothing:\n{second}"
        );
        assert!(
            second.contains("<p id=\"n\">2</p>"),
            "{tier}: the page still answers:\n{second}"
        );
    }
    builds_for_the_browser(TRAPS_AFTER_A_MOVE);
}

const TRAP_AFTER_A_COMMIT: &str = r##"## Store S
State:
    Let v be 0.
Actions:
    To bump:
        Set v to v + 1.

## Page /
Use S.
State:
    Let n be 0.
Actions:
    To go:
        Call S's bump.
        Set n to item 9 of [1].
View:
    <main><p id="v">{S's v}</p><p id="n">{n}</p><button id="x" on-click={go}>x</button></main>

## Test: a turn traps after its store call committed
    Click "#x".
"##;

/// A store action commits when it returns, so the page shows the store as committed even when the
/// turn that called it trapped before its own paint — with nobody watching, and the turn's own
/// state rolled back.
#[test]
fn a_store_a_trapping_turn_committed_is_drawn() {
    let (reference, lowered) = traced(TRAP_AFTER_A_COMMIT);
    for (tier, trace) in std::iter::once(("reference", reference)).chain(lowered) {
        assert!(trace.stopped.is_none(), "{tier}: {:?}", trace.stopped);
        let after = &trace.steps[1].snapshot;
        assert!(
            after.failure.is_some(),
            "{tier}: the trap is the page's unhandled failure"
        );
        assert!(
            after.document.contains("<p id=\"v\">1</p>"),
            "{tier}: the committed store is drawn:\n{}",
            after.document
        );
        assert!(
            after.document.contains("<p id=\"n\">0</p>"),
            "{tier}: the turn's own state rolled back:\n{}",
            after.document
        );
    }
    builds_for_the_browser(TRAP_AFTER_A_COMMIT);
}
