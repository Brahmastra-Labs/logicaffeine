//! ★ LOCK — `Return.`/`Break` inside an `If` (or any nested block) LEAVES the body it is
//! written in, on every door a page has for running one — a component action, a `When … :` event
//! handler, `When the page loads:`, `When this component disappears:`, `When an action fails …:`,
//! a `When <field> changes:` watcher (component-local and store), a store's own action, and a
//! `## Directive` body — on the tree-walker and the bytecode VM alike. ★
//!
//! A loop that checks each statement only for `Err` lets an early `Return.`/`Break` come back
//! `Ok(..)` and fall straight through to the body's NEXT statement, so `If cond: Return.` guards
//! nothing. Every one of these bodies is a lowered LOGOS function, run by the interpreter's own
//! statement loop, which honors `ControlFlow`; the census at the bottom keeps the raw shape out.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome, RunOutcome};
use logicaffeine_compile::semantics::dom;

/// `src` run on the tree-walker and on the bytecode VM, each from a fresh document: both must run
/// it cleanly and leave the same page — its document and its title.
fn run_on_both_tiers(src: &str) -> (String, String) {
    let run = |tier: &str, outcome: fn(&str) -> RunOutcome| {
        dom::reset();
        let out = outcome(src);
        assert_eq!(
            out.error, None,
            "the {tier} must run the program cleanly:\n{src}\n{out:?}"
        );
        (dom::render(), dom::title())
    };
    let tree_walker = run("tree-walker", tw_outcome);
    let vm = run("bytecode VM", vm_outcome);
    assert_eq!(
        tree_walker, vm,
        "the tree-walker and the bytecode VM leave different pages:\n{src}"
    );
    tree_walker
}

/// The body of the document `src` leaves on every tier ([`run_on_both_tiers`]).
fn rendered(src: &str) -> String {
    let (html, _) = run_on_both_tiers(src);
    html.strip_prefix("<body>")
        .and_then(|h| h.strip_suffix("</body>"))
        .unwrap_or(&html)
        .to_string()
}

/// The exact repro from the LGX ease-of-use audit: `largo run --interpret` printed `1`
/// (expected `0`) because `Set n to n + 1.` ran after the guarded `Return.` anyway.
#[test]
fn an_early_return_inside_an_if_leaves_a_component_action() {
    let src = "\
## Page /
State:
    Let n be 0.
Actions:
    To add:
        If n is equal to 0:
            Return.
        Set n to n + 1.
View:
    <button id=\"go\" on-click={add}>go</button>
    <p id=\"n\">{n}</p>

## Main
Click \"#go\".
";
    assert!(
        rendered(src).contains("<p id=\"n\">0</p>"),
        "`Return.` must leave `add` before `Set n to n + 1.` runs: {}",
        rendered(src)
    );
}

/// Same shape, nested one level deeper: `Return.` inside an `If` inside a `Repeat` inside an
/// action — the loop propagates `ControlFlow::Return` correctly (it always has); what matters
/// here is that the ACTION's own body loop, one level up, still honors it.
#[test]
fn an_early_return_inside_a_repeat_leaves_the_enclosing_action() {
    let src = "\
## Component App
State:
    Let n be 0.
Actions:
    To add:
        Repeat for each x in [1, 2, 3]:
            If x is equal to 2:
                Return.
            Set n to n + x.
        Set n to 999.
View:
    <p id=\"n\">{n}</p>
    <button id=\"go\" on-click={add}>go</button>

## Main
Mount App.
Click \"#go\".
";
    // 1 is added, then x=2 returns — `n` is 1, and the trailing `Set n to 999.` never runs.
    assert!(
        rendered(src).contains("<p id=\"n\">1</p>"),
        "`Return.` inside the `Repeat` must leave `add` entirely: {}",
        rendered(src)
    );
}

/// A stray top-level `Break.` (not inside any loop) ends the action exactly as reaching its
/// last statement would — the same rule a bare `Break` follows in an ordinary function body.
#[test]
fn a_top_level_break_leaves_a_component_action_like_falling_off_its_end() {
    let src = "\
## Component App
State:
    Let n be 0.
Actions:
    To add:
        If n is equal to 0:
            Break.
        Set n to n + 1.
View:
    <p id=\"n\">{n}</p>
    <button id=\"go\" on-click={add}>go</button>

## Main
Mount App.
Click \"#go\".
";
    assert!(
        rendered(src).contains("<p id=\"n\">0</p>"),
        "`Break.` must leave `add` before `Set n to n + 1.` runs: {}",
        rendered(src)
    );
}

/// `When "#sel" is clicked:` — a selector-triggered event handler (`Handler::Event`), the other
/// door a click can run through besides a named action.
#[test]
fn an_early_return_inside_an_if_leaves_a_selector_event_handler() {
    let src = "\
## Component App
State:
    Let n be 0.
Actions:
    When \"#go\" is clicked:
        If n is equal to 0:
            Return.
        Set n to n + 1.
View:
    <p id=\"n\">{n}</p>
    <button id=\"go\">go</button>

## Main
Mount App.
Click \"#go\".
";
    assert!(
        rendered(src).contains("<p id=\"n\">0</p>"),
        "`Return.` must leave the selector handler: {}",
        rendered(src)
    );
}

/// `When the page loads:` — runs once at first render, before the component's own turn model
/// exists to roll anything back.
#[test]
fn an_early_return_inside_an_if_leaves_the_page_load_handler() {
    let src = "\
## Component App
State:
    Let n be 0.
Actions:
    When the page loads:
        If 1 is equal to 1:
            Return.
        Set n to 99.
View:
    <p id=\"n\">{n}</p>

## Main
Mount App.
";
    assert!(
        rendered(src).contains("<p id=\"n\">0</p>"),
        "`Return.` must leave the load handler before `Set n to 99.` runs: {}",
        rendered(src)
    );
}

/// `When this component disappears:` — the cleanup handler the instance sweep runs for an
/// instance the last paint did not reach. Its own state is gone by the time it runs, so the
/// guarded write has to land somewhere else observable: the document title.
#[test]
fn an_early_return_leaves_the_disappearing_handler() {
    let src = "\
## Component Row
Actions:
    When this component disappears:
        If 1 is equal to 1:
            Return.
        Set the title to \"gone\".
View:
    <li id=\"r\">row</li>

## Component App
State:
    Let show be true.
Actions:
    To hide:
        Set show to false.
View:
    <ul>{if show:}<Row/>{end}</ul><button id=\"b\" on-click={hide}>b</button>

## Main
Mount App.
Click \"#b\".
";
    let (_, title) = run_on_both_tiers(src);
    assert_ne!(
        title, "gone",
        "`Return.` must leave the disappearing handler before it sets the title"
    );
}

/// `When an action fails (e: Text):` — the failure observer, run against the record BEFORE the
/// failing turn's rollback.
#[test]
fn an_early_return_leaves_the_action_fails_handler() {
    let src = "\
## Component App
State:
    Let why be \"none\".
Actions:
    When an action fails (e: Text):
        If 1 is equal to 1:
            Return.
        Set why to e.
    To boom:
        Let bad be item 9 of [1, 2].
View:
    <p id=\"w\">{why}</p><button id=\"b\" on-click={boom}>b</button>

## Main
Mount App.
Click \"#b\".
";
    assert!(
        rendered(src).contains("<p id=\"w\">none</p>"),
        "`Return.` must leave the failure handler before `Set why to e.` runs: {}",
        rendered(src)
    );
}

/// `When a fetch fails (url: Text) and (why: Text):` — the page-wide fetch observer, reached when
/// a typed request decodes to `Err`.
#[test]
fn an_early_return_leaves_a_fetch_failure_handler() {
    let src = "\
## A Todo has:
    an id, which is Int.

## Route GET /api/other -> Text:
    Return \"hi\".

## Component App
State:
    Let problem be \"\".
    Let failed be \"no\".
Actions:
    When the page loads:
        Get \"/api/missing\" as Todo, then loaded.
    To loaded (r: Result of Todo and HttpError):
        Inspect r:
            When Ok (v):
                Set problem to \"ok\".
            When Err (e):
                Set problem to e's message.
    When a fetch fails (url: Text) and (why: Text):
        If 1 is equal to 1:
            Return.
        Set failed to \"yes\".
View:
    <p id=\"failed\">{failed}</p>

## Main
Serve fetches with routes.
Mount App.
";
    assert!(
        rendered(src).contains("<p id=\"failed\">no</p>"),
        "`Return.` must leave the fetch-failure handler before `Set failed to \"yes\".` runs: {}",
        rendered(src)
    );
}

/// `When <field> changes:` — a component's own watcher, run inside the turn that moved its
/// field.
#[test]
fn an_early_return_leaves_a_field_changes_watcher() {
    let src = "\
## Component App
State:
    Let a be 0.
    Let seen be 0.
Actions:
    To bump:
        Set a to a + 1.
    When a changes:
        If 1 is equal to 1:
            Return.
        Set seen to seen + 1.
View:
    <p id=\"a\">{a}</p><p id=\"seen\">{seen}</p><button id=\"b\" on-click={bump}>b</button>

## Main
Mount App.
Click \"#b\".
";
    let html = rendered(src);
    assert!(
        html.contains("<p id=\"a\">1</p>"),
        "the watched field still moved: {html}"
    );
    assert!(
        html.contains("<p id=\"seen\">0</p>"),
        "`Return.` must leave the watcher before `Set seen to seen + 1.` runs: {html}"
    );
}

/// `Call <Store>'s <action>.` — a store's own action body, the only door through which a store's
/// state changes.
#[test]
fn an_early_return_leaves_a_store_action() {
    let src = "\
## Store Counter
State:
    Let n be 0.
Actions:
    To add:
        If n is equal to 0:
            Return.
        Set n to n + 1.

## Component App
Use Counter.
Actions:
    To bump:
        Call Counter's add.
View:
    <p id=\"n\">{Counter's n}</p>
    <button id=\"go\" on-click={bump}>go</button>

## Main
Mount App.
Click \"#go\".
";
    assert!(
        rendered(src).contains("<p id=\"n\">0</p>"),
        "`Return.` must leave the store action: {}",
        rendered(src)
    );
}

/// A store's own `When the page loads:` — run at the store's first touch, a separate door from the
/// component load handler above.
#[test]
fn an_early_return_leaves_a_store_page_load_handler() {
    let src = "\
## Store Counter
State:
    Let n be 0.
Actions:
    When the page loads:
        If 1 is equal to 1:
            Return.
        Set n to 99.

## Component App
Use Counter.
View:
    <p id=\"n\">{Counter's n}</p>

## Main
Mount App.
";
    assert!(
        rendered(src).contains("<p id=\"n\">0</p>"),
        "`Return.` must leave the store's load handler: {}",
        rendered(src)
    );
}

/// `When <Store>'s <field> changes:` — a component watching a STORE, distinct from the
/// component-local watchers above.
#[test]
fn an_early_return_leaves_a_store_field_changes_watcher() {
    let src = "\
## Store Counter
State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.

## Component App
Use Counter.
State:
    Let seen be 0.
Actions:
    To go:
        Call Counter's bump.
    When Counter's n changes:
        If 1 is equal to 1:
            Return.
        Set seen to seen + 1.
View:
    <p id=\"seen\">{seen}</p><button id=\"b\" on-click={go}>b</button>

## Main
Mount App.
Click \"#b\".
";
    assert!(
        rendered(src).contains("<p id=\"seen\">0</p>"),
        "`Return.` must leave the store watcher: {}",
        rendered(src)
    );
}

/// A `## Directive` body — the same shape, on the one construct that is neither an action nor an
/// event handler.
#[test]
fn an_early_return_leaves_a_directive_body() {
    let src = "\
## Directive tooltip (el: Element) and (text: Text):
    If 1 is equal to 1:
        Return.
    Call setAttribute with el and \"title\" and text.

## Component App
View:
    <p id=\"p\" tooltip={\"hint\"}>y</p>

## Main
Mount App.
";
    let html = rendered(src);
    assert!(
        !html.contains("title="),
        "`Return.` must leave the directive before `Call setAttribute …` runs: {html}"
    );
}

/// A sibling action called by its bare name (`Call inner.`, run nested in the caller's turn) that
/// leaves early with `Return.` leaves the SIBLING only: the rest of its body is skipped and the
/// caller's turn goes on.
#[test]
fn a_return_in_a_sibling_action_leaves_the_sibling_not_the_callers_turn() {
    let src = "\
## Component App
State:
    Let n be 0.
Actions:
    To inner:
        If n is 0:
            Set n to 1.
            Return.
        Set n to 99.
    To outer:
        Call inner.
        Set n to n + 10.
View:
    <p id=\"n\">{n}</p>
    <button id=\"go\" on-click={outer}>go</button>

## Main
Mount App.
Click \"#go\".
";
    let html = rendered(src);
    assert!(
        html.contains("<p id=\"n\">11</p>"),
        "the sibling returned early and the caller went on: {html}"
    );
}

/// ★ CENSUS — a page's stored bodies (action/handler/directive/watcher) are lowered LOGOS
/// functions, and the tree-walker runs every body through its own statement loop, which honors
/// `ControlFlow::Return`/`Break`. A raw `for stmt in … { if let Err(e) =
/// self.execute_stmt_sync(stmt) { … } }` silently drops an early exit — exactly the defect every
/// test above is pinned against — so this reads the SOURCE rather than trusting that the shape
/// never comes back.
#[test]
fn every_turn_body_runs_through_the_control_flow_aware_loop() {
    let src = include_str!("../../logicaffeine_compile/src/interpreter.rs");
    assert_eq!(
        src.matches("if let Err(e) = self.execute_stmt_sync(stmt)")
            .count(),
        0,
        "a body must not loop over its own statements checking only `Err` — that shape is \
         exactly what dropped `Return.`/`Break`. Run it as a block, whose `ControlFlow` the \
         caller honors."
    );
}
