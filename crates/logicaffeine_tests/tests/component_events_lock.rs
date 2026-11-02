//! ★ LOCK — a mounted component ANSWERS EVENTS: a click runs its action, the action changes
//! the instance's state, and the page shows the new state. ★
//!
//! This is the reactive cycle the whole framework rests on — event → action → state → view —
//! and it is proven on the tree-walker, the reference semantics every other tier is held to.
//! State lives in the INSTANCE (a record keyed by the instance's path), never in a
//! render-order cursor: a second component of the same type keeps its own.

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::semantics::dom;

/// Run `src` and return the rendered page, without the recorder's `<body>` wrapper.
fn rendered(src: &str) -> String {
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "the program must run cleanly:\n{src}");
    let html = dom::render();
    html.strip_prefix("<body>")
        .and_then(|h| h.strip_suffix("</body>"))
        .unwrap_or(&html)
        .to_string()
}

/// Run `src` expecting a clean failure, and return the message.
fn failure(src: &str) -> String {
    dom::reset();
    tw_outcome(src)
        .error
        .unwrap_or_else(|| panic!("expected a failure:\n{src}"))
}

const COUNTER: &str = "\
## Component App
State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
View:
    <p>{n}</p>
    <button id=\"go\" on-click={bump}>+</button>

## Main
Mount App.
";

#[test]
fn a_click_runs_the_action_and_the_page_shows_the_new_state() {
    let mut src = COUNTER.to_string();
    src.push_str("Click \"#go\".\n");
    assert!(
        rendered(&src).contains("<p>1</p>"),
        "one click: {}",
        rendered(&src)
    );
    src.push_str("Click \"#go\".\nClick \"#go\".\n");
    assert!(
        rendered(&src).contains("<p>3</p>"),
        "three clicks: {}",
        rendered(&src)
    );
}

#[test]
fn the_re_render_replaces_the_old_view_rather_than_appending() {
    let mut src = COUNTER.to_string();
    src.push_str("Click \"#go\".\n");
    let out = rendered(&src);
    assert_eq!(
        out.matches("<button").count(),
        1,
        "one button after a re-render, got: {out}"
    );
    assert_eq!(
        out.matches("<p>").count(),
        1,
        "one paragraph after a re-render, got: {out}"
    );
}

#[test]
fn typing_into_a_bound_input_updates_the_field_it_binds() {
    let src = "\
## Component App
State:
    Let draft be \"\".
View:
    <input id=\"draft\" bind={draft}>
    <p>{draft}</p>

## Main
Mount App.
Type \"milk\" into \"#draft\".
";
    assert!(
        rendered(src).contains("<p>milk</p>"),
        "rendered: {}",
        rendered(src)
    );
}

#[test]
fn a_selector_event_sentence_answers_the_same_click() {
    let src = "\
## Component App
State:
    Let n be 0.
Actions:
    When \"#go\" is clicked:
        Set n to n + 5.
View:
    <p>{n}</p>
    <button id=\"go\">+</button>

## Main
Mount App.
Click \"#go\".
";
    assert!(
        rendered(src).contains("<p>5</p>"),
        "rendered: {}",
        rendered(src)
    );
}

#[test]
fn a_handler_can_call_an_action_with_an_argument_from_the_loop_item() {
    let src = "\
## Component App
State:
    Let items be [\"a\", \"b\", \"c\"].
    Let picked be \"none\".
Actions:
    To pick (x: Text):
        Set picked to x.
View:
    <p>{picked}</p>
    <ul>{for each item in items:}<li id={item} on-click={pick(item)}>{item}</li>{end}</ul>

## Main
Mount App.
Click \"#b\".
";
    assert!(
        rendered(src).contains("<p>b</p>"),
        "the clicked item's own value reached the action: {}",
        rendered(src)
    );
}

#[test]
fn two_instances_of_one_component_keep_separate_state() {
    let src = "\
## Component Counter (tag: Text)
State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
View:
    <button id={tag} on-click={bump}>{n}</button>

## Component App
View:
    <Counter tag=\"a\"/>
    <Counter tag=\"b\"/>

## Main
Mount App.
Click \"#a\".
Click \"#a\".
";
    let out = rendered(src);
    assert!(
        out.contains("<button id=\"a\">2</button>"),
        "the clicked counter advanced: {out}"
    );
    assert!(
        out.contains("<button id=\"b\">0</button>"),
        "its sibling did not: {out}"
    );
}

#[test]
fn a_derived_field_recomputes_after_an_action() {
    let src = "\
## Component App
State:
    Let items be a new Seq of Text.
    Let total always be length of items.
Actions:
    To add:
        Push \"x\" to items.
View:
    <p>{total}</p>
    <button id=\"go\" on-click={add}>+</button>

## Main
Mount App.
Click \"#go\".
Click \"#go\".
";
    assert!(
        rendered(src).contains("<p>2</p>"),
        "rendered: {}",
        rendered(src)
    );
}

#[test]
fn clicking_nothing_is_a_clean_error_not_a_silent_no_op() {
    let mut src = COUNTER.to_string();
    src.push_str("Click \"#missing\".\n");
    let msg = failure(&src);
    assert!(
        msg.contains("#missing"),
        "the message names the selector: {msg}"
    );
}

#[test]
fn an_action_that_fails_leaves_the_state_as_it_was() {
    let src = "\
## Component App
State:
    Let n be 1.
Actions:
    To bump:
        Set n to 99.
        Let bad be item 5 of [1, 2].
View:
    <p>{n}</p>
    <button id=\"go\" on-click={bump}>+</button>

## Main
Mount App.
Click \"#go\".
";
    // The turn is a transaction: a failing action rolls its instance back, and the page still
    // shows the state from before the click.
    //
    // ★ AND THE FAILURE IS REPORTED, which is why this cannot use `rendered` — that helper
    // requires a clean run, and it required one HERE while the page's whole point is that the
    // action failed. An unhandled `When an action fails …:` used to be dropped on this tier: the
    // turn rolled back and nothing anywhere said why, so an author saw a button that did nothing.
    // A browser already logged it and raised a panel; this tier now agrees. Both halves are
    // asserted, because rolling back quietly is the bug and rolling back loudly is the contract.
    dom::reset();
    let out = tw_outcome(src);
    let html = dom::render();
    assert!(
        html.contains("<p>1</p>"),
        "the failing turn must roll back: {html}"
    );
    let why = out
        .error
        .expect("an unhandled action failure must be reported, not dropped");
    assert!(
        why.contains("5")
            || why.to_lowercase().contains("index")
            || why.to_lowercase().contains("item"),
        "the report must say what actually went wrong, not merely that something did: {why}"
    );
}
