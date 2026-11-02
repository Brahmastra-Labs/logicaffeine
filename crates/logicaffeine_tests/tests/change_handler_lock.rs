//! LOCK — `When <field> changes:` on the lowered page: judged against the action, run once each,
//! and unable to undo the action that woke it.
//!
//! The reference runs a turn's change handlers after the action commits: the CHANGED set is the
//! fields the action moved, every handler for a field in it runs once in declaration order, a
//! handler's own write starts no second round, and a handler that traps is dropped — the action
//! stays committed and on screen, and the page keeps answering. These rows hold the lowered page
//! (hosted by the tree-walker, and on the VM) to exactly that.

use logicaffeine_compile::page_host::{compare_tests, PageTierKind, PageTrace, TierRun};

/// The one `## Test` in `source`, traced on both lowered tiers.
fn in_process(source: &str) -> Vec<(&'static str, PageTrace)> {
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
    // A tier the harness runs as its reference is not among the candidates; its trace is the
    // reference's.
    [PageTierKind::LoweredHere, PageTierKind::LoweredVm]
        .into_iter()
        .map(|kind| match c.candidates.iter().find(|(k, _)| *k == kind) {
            Some((_, TierRun::Traced(t))) => (kind.label(), t.clone()),
            None => (kind.label(), c.reference.clone()),
            Some((_, other)) => panic!(
                "the {} tier must trace the scenario, not {other:?}",
                kind.label()
            ),
        })
        .collect()
}

fn builds_for_the_browser(source: &str) {
    let built = logicaffeine_compile::compile::compile_to_wasm(source);
    assert!(
        built.is_ok(),
        "the page builds for the browser: {:?}",
        built.err()
    );
}

const TWO_WATCHERS: &str = r##"## Page /

State:
    Let a be 0.
    Let b be 0.
Actions:
    To bump:
        Set a to a + 1.
    When a changes:
        Set b to b + 1.
    When b changes:
        Set a to a + 100.
View:
    <p id="p">{a}-{b}</p>
    <button id="go" on-click={bump}>+</button>

## Test: one round of handlers, judged against the action
    Click "#go".
"##;

#[test]
fn change_handlers_are_judged_against_the_actions_writes_alone() {
    for (tier, trace) in in_process(TWO_WATCHERS) {
        assert!(trace.stopped.is_none(), "{tier}: {:?}", trace.stopped);
        let after = &trace.steps[1].snapshot.document;
        assert!(
            after.contains("<p id=\"p\">1-1</p>"),
            "{tier}: `a` moved, so its handler ran; `b` moved only in a handler, so its handler did not:\n{after}"
        );
    }
    builds_for_the_browser(TWO_WATCHERS);
}

const TWO_HANDLERS_ONE_FIELD: &str = r##"## Page /

State:
    Let n be 0.
    Let log be "".
Actions:
    To bump:
        Set n to n + 1.
    When n changes:
        Set log to log + "first ".
    When n changes:
        Set log to log + "second".
View:
    <p id="p">{n}: {log}</p>
    <button id="go" on-click={bump}>+</button>

## Test: every handler for a moved field runs, in declaration order
    Click "#go".
"##;

#[test]
fn every_handler_for_a_moved_field_runs_once_in_declaration_order() {
    for (tier, trace) in in_process(TWO_HANDLERS_ONE_FIELD) {
        let after = &trace.steps[1].snapshot.document;
        assert!(
            after.contains("<p id=\"p\">1: first second</p>"),
            "{tier}:\n{after}"
        );
    }
    builds_for_the_browser(TWO_HANDLERS_ONE_FIELD);
}

const A_TRAPPING_WATCHER: &str = r##"## Page /

State:
    Let n be 0.
    Let seen be 0.
Actions:
    To bump:
        Set n to n + 1.
    When n changes:
        Set seen to item 9 of [1].
View:
    <p id="n">{n}-{seen}</p>
    <button id="go" on-click={bump}>x</button>

## Test: a handler that traps cannot undo the action
    Click "#go".
    Click "#go".
"##;

#[test]
fn a_trap_in_a_change_handler_leaves_the_action_committed_and_the_page_answering() {
    for (tier, trace) in in_process(A_TRAPPING_WATCHER) {
        assert!(
            trace.stopped.is_none(),
            "{tier}: the page keeps answering: {:?}",
            trace.stopped
        );
        let once = &trace.steps[1].snapshot;
        assert!(
            once.document.contains("<p id=\"n\">1-0</p>"),
            "{tier}: the action committed and is shown:\n{}",
            once.document
        );
        assert_eq!(
            once.journal,
            ["bump"],
            "{tier}: the action's turn is journaled as it committed"
        );
        let twice = &trace.steps[2].snapshot;
        assert!(
            twice.document.contains("<p id=\"n\">2-0</p>"),
            "{tier}: and the next click lands:\n{}",
            twice.document
        );
    }
    builds_for_the_browser(A_TRAPPING_WATCHER);
}

const A_STORE_ACTION_TRAPS: &str = r##"## Store Counter

State:
    Let n be 0.
Actions:
    To hit:
        Set n to item 9 of [1].

## Page /

Use Counter.
Actions:
    To go:
        Call Counter's hit.
View:
    <button id="go" on-click={go}>x</button>

## Test: a trap inside a store action called from an action
    Click "#go".
"##;

/// A trap journals every turn it interrupted, innermost first — the store action, then the action
/// that called it — as the reference's nested turns record themselves.
#[test]
fn a_trap_journals_every_turn_it_interrupted_innermost_first() {
    for (tier, trace) in in_process(A_STORE_ACTION_TRAPS) {
        let after = &trace.steps[1].snapshot;
        assert_eq!(after.journal, ["hit", "go"], "{tier}: {:?}", trace.stopped);
    }
}

/// `source`'s `## Test`s pass on the reference and trace identically on the lowered VM, and the page
/// builds for the browser.
fn passes_everywhere(source: &str) {
    use logicaffeine_compile::testrun::{run_tests, TestConfig, Verdict};
    let report = run_tests(source, &TestConfig::default())
        .unwrap_or_else(|e| panic!("the fixture must parse and run: {e}\n{source}"));
    assert!(
        !report.outcomes.is_empty(),
        "the fixture declares at least one ## Test"
    );
    for outcome in &report.outcomes {
        assert_eq!(
            outcome.verdict,
            Verdict::Passed,
            "`{}`: {:?}",
            outcome.name,
            outcome.failures
        );
    }
    builds_for_the_browser(source);
}

/// Change handlers belong to a turn a visitor started — an action, a bind write, a listener
/// delivery. A lifecycle hook is not one: the reference runs `When the page loads:` with no
/// change round after it, so the field it sets wakes nothing.
#[test]
fn a_page_load_write_wakes_no_change_handler() {
    passes_everywhere(
        r##"## Page /

State:
    Let n be 0.
    Let seen be 0.
Actions:
    When the page loads:
        Set n to 5.
    When n changes:
        Set seen to seen + 1.
    To bump:
        Set n to n + 1.
View:
    <p id="p">{n}-{seen}</p>
    <button id="go" on-click={bump}>+</button>

## Test: the load's write wakes nothing; the next action's does
    Expect that the text of "#p" is equal to "5-0".
    Click "#go".
    Expect that the text of "#p" is equal to "6-1".
"##,
    );
}

/// `When an action fails:` is a handler, not a turn of its own: the field it writes wakes no change
/// handler, on the reference or the lowered page.
#[test]
fn a_failure_handlers_write_wakes_no_change_handler() {
    passes_everywhere(
        r##"## Page /

State:
    Let why be "".
    Let seen be 0.
Actions:
    When an action fails (e: Text):
        Set why to "failed".
    When why changes:
        Set seen to seen + 1.
    To trip:
        Set seen to item 9 of [1].
View:
    <p id="p">{why}-{seen}</p>
    <button id="go" on-click={trip}>x</button>

## Test: the failure handler's write wakes nothing
    Click "#go".
    Expect that the text of "#p" is equal to "failed-0".
"##,
    );
}
