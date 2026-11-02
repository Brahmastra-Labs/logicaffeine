//! LOCK — an instance is on the page until a repaint that could have drawn it does not.
//!
//! The lowered page keeps a registry of live instances per component (`__KEYS_<C>`), stamped with
//! the paint pass that last drew each one. A repaint draws one instance's subtree — the page root's,
//! or a placed component's own — so only what that subtree held can leave in it; an instance
//! elsewhere on the page was not in the paint at all, whatever its stamp says. Every reader of
//! liveness (the unmount sweep, the route and message fan-outs, the timers) asks the registry.

use logicaffeine_compile::compile::compile_to_wasm;
use logicaffeine_compile::testrun::{run_tests, TestConfig, Verdict};

fn passes_everywhere(source: &str) {
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
            "`{}` must pass on the reference tier and trace identically on the lowered VM: {:?}",
            outcome.name,
            outcome.failures
        );
    }
    let built = compile_to_wasm(source);
    assert!(
        built.is_ok(),
        "the page must build for the browser: {:?}",
        built.err()
    );
}

/// A tile whose own turn repaints it (the `{if}` inside it has no patch site).
const TILE: &str = r##"## Component Tile (name: Text)

State:
    Let n be 0.

Actions:
    To bump:
        Set n to n + 1.

View:
    <div><button id="{name}" on-click={bump}>{name}</button><p id="{name}-n">{n}</p>{if n is greater than 0:}<em>on</em>{end}</div>
"##;

#[test]
fn a_placed_components_own_repaint_leaves_its_siblings_on_the_page() {
    passes_everywhere(&format!(
        "{TILE}
## Page /

State:
    Let show be true.

View:
    {{if show:}}<Tile name=\"a\"/>{{end}}
    {{if show:}}<Tile name=\"b\"/>{{end}}

## Test: each tile's own repaint leaves the other one alive

Click \"#b\".
Click \"#a\".
Click \"#b\".
Expect that the text of \"#b-n\" is equal to \"2\".
Expect that the text of \"#a-n\" is equal to \"1\".
"
    ));
}

#[test]
fn a_placed_components_own_repaint_still_sweeps_what_it_stops_drawing() {
    passes_everywhere(
        r##"## Component Badge

State:
    Let hits be 0.

Actions:
    To hit:
        Set hits to hits + 1.

View:
    <span><button id="hit" on-click={hit}>hit</button><b id="hits">{hits}</b></span>

## Component Card (name: Text)

State:
    Let open be true.

Actions:
    To toggle:
        Set open to not open.

View:
    <div><button id="toggle" on-click={toggle}>t</button>{if open:}<Badge/>{end}</div>

## Page /

View:
    <main><Card name="c"/></main>

## Test: a badge the card stops drawing leaves, and comes back new

Click "#hit".
Click "#hit".
Expect that the text of "#hits" is equal to "2".
Click "#toggle".
Expect that the count of "#hits" is equal to 0.
Click "#toggle".
Expect that the text of "#hits" is equal to "0".
"##,
    );
}

#[test]
fn a_move_of_the_location_reaches_a_placed_listener_and_none_after_it_leaves() {
    passes_everywhere(
        r##"## Component Child

State:
    Let moves be 0.

Actions:
    When the route changes:
        Set moves to moves + 1.

View:
    <p id="child">{moves}</p>

## Component Page

State:
    Let showing be true.
    Let moves be 0.

Actions:
    When the route changes:
        Set moves to moves + 1.
    To hide:
        Set showing to false.

View:
    {if showing:}<Child/>{end}
    <p id="page">{moves}</p>
    <button id="hide" on-click={hide}>hide</button>

## Test: a route listener hears the location move while it is on the page, and not after

Mount Page.
Navigate to "/one".
Expect that the text of "#child" is equal to "1".
Expect that the text of "#page" is equal to "1".
Click "#hide".
Navigate to "/two".
Expect that the text of "#page" is equal to "2".
"##,
    );
}

/// The page's final document after each statement, on each lowered tier (the page run in-process
/// on the tree-walker, and on the VM).
fn lowered_documents(source: &str) -> Vec<(&'static str, Vec<String>)> {
    use logicaffeine_compile::page_host::{compare_tests, PageTierKind, TierRun};
    let tiers = [PageTierKind::LoweredHere, PageTierKind::LoweredVm];
    let mut all = compare_tests(source, &tiers, 0, 50_000_000, None)
        .unwrap_or_else(|e| panic!("the page must parse: {e}\n{source}"));
    assert_eq!(all.len(), 1, "exactly one page scenario:\n{source}");
    let c = all.remove(0);
    tiers
        .iter()
        .map(|kind| {
            let trace = match c.candidates.iter().find(|(k, _)| k == kind) {
                Some((_, TierRun::Traced(t))) => t,
                None => &c.reference,
                Some((_, other)) => panic!(
                    "the {} tier must trace the scenario, not {other:?}",
                    kind.label()
                ),
            };
            assert!(
                trace.stopped.is_none(),
                "{}: {:?}",
                kind.label(),
                trace.stopped
            );
            (
                kind.label(),
                trace
                    .steps
                    .iter()
                    .map(|s| s.snapshot.document.clone())
                    .collect(),
            )
        })
        .collect()
}

#[test]
fn a_components_own_navigation_reaches_its_own_route_listener_once() {
    let source = r##"## Component Left

State:
    Let moves be 0.

Actions:
    When the route changes:
        Set moves to moves + 1.
    To go:
        Navigate to "/from-left".

View:
    <p id="left">{moves}</p>
    <button id="go" on-click={go}>go</button>

## Component Right

State:
    Let moves be 0.

Actions:
    When the route changes:
        Set moves to moves + 1.

View:
    <p id="right">{moves}</p>

## Component Page

View:
    <div id="a"></div><div id="b"></div>

## Test: every component listening for the route hears every move once

Mount Page.
Mount Left at "#a".
Mount Right at "#b".
Navigate to "/elsewhere".
Click "#go".
"##;
    for (tier, documents) in lowered_documents(source) {
        let moved = &documents[4];
        assert!(
            moved.contains("<p id=\"left\">1</p>") && moved.contains("<p id=\"right\">1</p>"),
            "{tier}: a move of the location reaches both mounted listeners, whichever painted last:\n{moved}"
        );
        let own = &documents[5];
        assert!(
            own.contains("<p id=\"left\">2</p>") && own.contains("<p id=\"right\">2</p>"),
            "{tier}: a component's own `Navigate to` reaches its own listener and the other, once each:\n{own}"
        );
    }
}

/// A turn paints what it committed BEFORE the turns it owes run: its own route listener, answering
/// its own `Navigate to`, writes the same field after it and paints last — never overwritten by the
/// outer turn's paint from the value it held.
#[test]
fn a_turns_own_route_listener_paints_after_the_turn_that_moved() {
    passes_everywhere(
        r##"## Component App

State:
    Let label be "idle".

Actions:
    When the route changes:
        Set label to "arrived".
    To go:
        Set label to "going".
        Navigate to "/x".

View:
    <p id="label">{label}</p>
    <button id="go" on-click={go}>go</button>

## Test: the route listener's write is what the page shows

Mount App.
Click "#go".
Expect that the text of "#label" is equal to "arrived".
"##,
    );
}
