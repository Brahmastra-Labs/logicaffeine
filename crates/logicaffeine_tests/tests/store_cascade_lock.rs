//! LOCK — a store-watcher cascade that never settles is refused on the lowered page, in the
//! reference's words, and never takes the tier down with it.
//!
//! `When <Store>'s <field> changes:` runs as a turn of its own, after the turn that moved the store
//! commits; a watcher that writes a store someone watches starts another round. The lowered page
//! counts rounds running inside one another and fails the turn whose round passes
//! `STORE_CASCADE_LIMIT` with `store_cascade_message()` — the budget lands before the stack does.
//! The refusal is that turn's failure like any other: the watcher's own `When an action fails:`
//! absorbs it, and one nothing absorbs is the page's unhandled failure while the page keeps
//! answering, as a browser reports it and carries on. A cascade that settles runs every round, and
//! a refused one leaves no rounds behind: the count starts from nothing at the next turn.

use logicaffeine_compile::page_host::{compare_tests, PageTierKind, PageTrace, TierRun};
use logicaffeine_language::ast::component::store_cascade_message;

/// The one `## Test` in `source`, traced on both lowered tiers.
fn lowered(source: &str) -> Vec<(&'static str, PageTrace)> {
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
    c.candidates
        .into_iter()
        .map(|(kind, run)| match run {
            TierRun::Traced(t) => (kind.label(), t),
            other => panic!(
                "the {} tier must trace the scenario, not {other:?}",
                kind.label()
            ),
        })
        .collect()
}

/// The cascade was refused at `statement` in the reference's words, as the page's unhandled
/// failure, and the run went on.
fn refused_at(tier: &str, trace: &PageTrace, statement: usize) {
    assert!(
        trace.stopped.is_none(),
        "{tier}: the refusal fails a turn, never the run: {:?}",
        trace.stopped
    );
    let failure = trace.steps[statement]
        .snapshot
        .failure
        .as_deref()
        .unwrap_or_else(|| panic!("{tier}: a cascade that never settles is refused"));
    assert!(
        failure.contains(&store_cascade_message()),
        "{tier}: the page is told it never settles, in every tier's one sentence: {failure}"
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

const SELF_FEEDING: &str = r##"## Store S
State:
    Let v be 0.
Actions:
    To bump:
        Set v to v + 1.

## Component W
Use S.
State:
    Let seen be 0.
Actions:
    When S's v changes:
        Set seen to seen + 1.
        Call S's bump.
View:
    <span id="w">{seen}|{S's v}</span>

## Page /
Use S.
Actions:
    To go:
        Call S's bump.
View:
    <main><W/><button id="x" on-click={go}>x</button></main>

## Test: a watcher that feeds the store it watches
    Click "#x".
"##;

#[test]
fn a_self_feeding_watcher_fails_the_page_in_the_references_words() {
    for (tier, trace) in lowered(SELF_FEEDING) {
        refused_at(tier, &trace, 1);
    }
    builds_for_the_browser(SELF_FEEDING);
}

const MUTUAL: &str = r##"## Store A
State:
    Let a be 0.
Actions:
    To bump:
        Set a to a + 1.

## Store B
State:
    Let b be 0.
Actions:
    To bump:
        Set b to b + 1.

## Component WA
Use A.
Use B.
Actions:
    When A's a changes:
        Call B's bump.
View:
    <span id="wa">a</span>

## Component WB
Use A.
Use B.
Actions:
    When B's b changes:
        Call A's bump.
View:
    <span id="wb">b</span>

## Page /
Use A.
Use B.
Actions:
    To go:
        Call A's bump.
View:
    <main><WA/><WB/><button id="x" on-click={go}>x</button></main>

## Test: two stores watching each other
    Click "#x".
"##;

#[test]
fn two_stores_watching_each_other_fail_the_page_the_same_way() {
    for (tier, trace) in lowered(MUTUAL) {
        refused_at(tier, &trace, 1);
    }
    builds_for_the_browser(MUTUAL);
}

/// Sixteen rounds are allowed; the seventeenth is refused. A watcher that stops on its own after
/// `n` rounds settles below the limit and is refused at it.
fn counted(rounds: usize) -> String {
    format!(
        r##"## Store S
State:
    Let v be 0.
Actions:
    To bump:
        Set v to v + 1.

## Component W
Use S.
Actions:
    When S's v changes:
        If S's v is less than {rounds}:
            Call S's bump.
View:
    <span id="w">{{S's v}}</span>

## Page /
Use S.
Actions:
    To go:
        Call S's bump.
View:
    <main><W/><button id="x" on-click={{go}}>x</button></main>

## Test: a cascade {rounds} rounds deep
    Click "#x".
"##
    )
}

#[test]
fn a_cascade_as_deep_as_the_limit_settles_and_one_round_more_is_refused() {
    use logicaffeine_language::ast::component::STORE_CASCADE_LIMIT;
    for (tier, trace) in lowered(&counted(STORE_CASCADE_LIMIT)) {
        assert!(
            trace.stopped.is_none(),
            "{tier}: {STORE_CASCADE_LIMIT} rounds settle: {:?}",
            trace.stopped
        );
        assert!(
            trace.steps[1].snapshot.failure.is_none(),
            "{tier}: a cascade that settles is not refused: {:?}",
            trace.steps[1].snapshot.failure
        );
        let shown = &trace.steps[1].snapshot.document;
        assert!(
            shown.contains(&format!("<span id=\"w\">{STORE_CASCADE_LIMIT}</span>")),
            "{tier}: every round ran:\n{shown}"
        );
    }
    for (tier, trace) in lowered(&counted(STORE_CASCADE_LIMIT + 1)) {
        refused_at(tier, &trace, 1);
    }
}

/// A refused cascade is absorbed by the watcher's own `When an action fails:`; the page keeps
/// answering, and the next turn's cascade counts from nothing — the rounds the refused one entered
/// did not leave it at the limit.
const REFUSED_THEN_SETTLES: &str = r##"## Store S
State:
    Let v be 0.
    Let loops be true.
Actions:
    To bump:
        Set v to v + 1.
    To calm:
        Set loops to false.

## Component W
Use S.
State:
    Let refused be 0.
Actions:
    When S's v changes:
        If S's loops:
            Call S's bump.
    When an action fails (e: Text):
        Set refused to refused + 1.
        Call S's calm.
View:
    <span id="w">{refused}|{S's v}</span>

## Page /
Use S.
Actions:
    To go:
        Call S's bump.
View:
    <main><W/><button id="x" on-click={go}>x</button></main>

## Test: a refused cascade leaves no rounds behind
    Click "#x".
    Click "#x".
"##;

#[test]
fn a_refused_cascade_leaves_the_next_turn_counting_from_nothing() {
    for (tier, trace) in lowered(REFUSED_THEN_SETTLES) {
        assert!(
            trace.stopped.is_none(),
            "{tier}: the watcher's failure handler absorbs the refusal: {:?}",
            trace.stopped
        );
        let first = &trace.steps[1].snapshot.document;
        assert!(
            first.contains("<span id=\"w\">1|"),
            "{tier}: the first cascade was refused once:\n{first}"
        );
        let second = &trace.steps[2].snapshot.document;
        assert!(
            second.contains("<span id=\"w\">1|"),
            "{tier}: the second turn's one round was not refused:\n{second}"
        );
    }
}

const SETTLES: &str = r##"## Store S1
State:
    Let a be 0.
Actions:
    To bump:
        Set a to a + 1.

## Store S2
State:
    Let b be 0.
Actions:
    To bump:
        Set b to b + 1.

## Component Mid
Use S1.
Use S2.
Actions:
    When S1's a changes:
        Call S2's bump.
View:
    <span id="m">m</span>

## Page /
Use S1.
Use S2.
Actions:
    To go:
        Call S1's bump.
View:
    <main><Mid/><span id="s">{S1's a}|{S2's b}</span><button id="x" on-click={go}>x</button></main>

## Test: a cascade that settles
    Click "#x".
    Click "#x".
"##;

#[test]
fn a_cascade_that_settles_runs_every_round_every_time() {
    for (tier, trace) in lowered(SETTLES) {
        assert!(trace.stopped.is_none(), "{tier}: {:?}", trace.stopped);
        assert!(
            trace.steps[1].snapshot.document.contains(">1|1<"),
            "{tier}: {}",
            trace.steps[1].snapshot.document
        );
        assert!(
            trace.steps[2].snapshot.document.contains(">2|2<"),
            "{tier}: {}",
            trace.steps[2].snapshot.document
        );
    }
    builds_for_the_browser(SETTLES);
}
