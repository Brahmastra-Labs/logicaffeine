//! LOCK — `on-<event>.once`: a listener answers the first event that passes its key gate, and
//! nothing after it, however its element is redrawn.
//!
//! `.once` is compiled LOGOS (`lgx_lower`'s spent map, keyed by the listener's identity — its
//! instance, its static site, its row in every enclosing loop), so every host that hands the page an
//! event — a browser, a driving verb — reaches the same dispatcher and gets the same answer. These
//! rows hand the page's dispatcher (`__lgx_dispatch`) the events a click or a press raises, on the
//! lowered VM, and read the answer: 2 answered, 1 nothing answered.

use logicaffeine_compile::compile::compile_to_wasm;
use logicaffeine_compile::interpreter::RuntimeValue;
use logicaffeine_compile::semantics::dom;
use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_compile::vm::session::Session;
use logicaffeine_compile::vm::{Compiler, Vm};
use logicaffeine_language::ast::stmt::Stmt;
use std::rc::Rc;

const ANSWERED: i64 = 2;
const UNANSWERED: i64 = 1;

/// Run `src`'s main on the lowered VM, then hand the live session to `f`.
fn with_page<R>(src: &str, f: impl FnOnce(&mut Session<'_>) -> R) -> R {
    dom::reset();
    with_parsed_program(src, |parsed, interner| {
        let (stmts, types, _policies) =
            parsed.unwrap_or_else(|e| panic!("the page must parse:\n{src}\n=> {e}"));
        let program = Compiler::compile_with_types(stmts, interner, Some(types))
            .unwrap_or_else(|e| panic!("the page must compile:\n{src}\n=> {e}"));
        let mut session = Session::new(Vm::new(&program), interner);
        session
            .run_main()
            .unwrap_or_else(|e| panic!("main runs:\n{src}\n=> {e}"));
        f(&mut session)
    })
}

fn text(s: &str) -> RuntimeValue {
    RuntimeValue::Text(Rc::new(s.to_string()))
}

/// Hand the element `sel` names each of `events`, carrying `value`, through the page's dispatcher,
/// as a driving verb does (`semantics::drive::Verb::events`): answered when any one was.
fn answer(s: &mut Session<'_>, sel: &str, events: &[&str], value: &str) -> i64 {
    let mut answered = false;
    for event in events {
        match s.invoke("__lgx_dispatch", vec![text(sel), text(event), text(value)]) {
            Ok(RuntimeValue::Bool(heard)) => answered |= heard,
            other => {
                panic!("`__lgx_dispatch({sel:?}, {event:?}, {value:?})` answers a Bool: {other:?}")
            }
        }
    }
    if answered {
        ANSWERED
    } else {
        UNANSWERED
    }
}

fn click(s: &mut Session<'_>, sel: &str) -> i64 {
    answer(s, sel, &["click"], "")
}

fn press(s: &mut Session<'_>, sel: &str, key: &str) -> i64 {
    answer(s, sel, &["keydown", "keyup"], key)
}

fn shows(fragment: &str) -> bool {
    dom::render().contains(fragment)
}

fn builds_for_the_browser(src: &str) {
    let built = compile_to_wasm(src);
    assert!(
        built.is_ok(),
        "the page builds for the browser: {:?}",
        built.err()
    );
}

#[test]
fn a_once_listener_answers_its_first_click_and_nothing_after() {
    let src = "## Component App\nState:\n    Let n be 0.\nActions:\n    To hit:\n        Set n to n + 1.\n\
               View:\n    <button id=\"b\" on-click.once={hit}>b</button>\n    <p id=\"n\">{n}</p>\n\n\
               ## Main\nMount App.\n";
    with_page(src, |s| {
        assert_eq!(click(s, "#b"), ANSWERED, "the first click is answered");
        assert_eq!(
            click(s, "#b"),
            UNANSWERED,
            "the second click has nothing left to answer it"
        );
        assert_eq!(click(s, "#b"), UNANSWERED, "and stays unanswered");
        assert!(
            shows("<p id=\"n\">1</p>"),
            "the action ran once: {}",
            dom::render()
        );
    });
    builds_for_the_browser(src);
}

#[test]
fn a_repaint_does_not_rearm_a_spent_listener() {
    let src = "## Component App\nState:\n    Let n be 0.\nActions:\n    To hit:\n        Set n to n + 1.\n\
               View:\n    <div class:hot={n is greater than 0}>\n        \
               <button id=\"b\" on-click.once={hit}>b</button>\n        \
               {if n is greater than 0:}<em id=\"on\">on {n}</em>{end}\n    </div>\n\n\
               ## Main\nMount App.\n";
    with_page(src, |s| {
        assert_eq!(click(s, "#b"), ANSWERED);
        assert!(
            shows("<em id=\"on\">on 1</em>"),
            "the turn redrew the view: {}",
            dom::render()
        );
        assert_eq!(
            click(s, "#b"),
            UNANSWERED,
            "the redrawn button carries no listener"
        );
    });
    builds_for_the_browser(src);
}

#[test]
fn a_key_gated_once_listener_is_spent_only_by_its_own_key() {
    let src = "## Component App\nState:\n    Let n be 0.\nActions:\n    To add:\n        Set n to n + 1.\n\
               View:\n    <input id=\"d\" aria-label=\"d\" on-keydown.enter.once={add}>\n    <p id=\"n\">{n}</p>\n\n\
               ## Main\nMount App.\n";
    with_page(src, |s| {
        assert_eq!(
            press(s, "#d", "x"),
            UNANSWERED,
            "another key is not this listener's event"
        );
        assert_eq!(
            press(s, "#d", "Enter"),
            ANSWERED,
            "so Enter still finds it armed"
        );
        assert_eq!(press(s, "#d", "Enter"), UNANSWERED, "and then it is spent");
        assert!(shows("<p id=\"n\">1</p>"), "{}", dom::render());
    });
    builds_for_the_browser(src);
}

#[test]
fn each_row_of_a_loop_has_its_own_once_listener() {
    let src = "## Component App\nState:\n    Let n be 0.\n    Let items be [1, 2].\nActions:\n    \
               To hit:\n        Set n to n + 1.\n\
               View:\n    {for each t in items, keyed by t:}<button id=\"b{t}\" on-click.once={hit}>{t}</button>{end}\n    \
               <p id=\"n\">{n}</p>\n\n## Main\nMount App.\n";
    with_page(src, |s| {
        assert_eq!(click(s, "#b1"), ANSWERED);
        assert_eq!(
            click(s, "#b2"),
            ANSWERED,
            "the second row's listener is its own"
        );
        assert_eq!(click(s, "#b1"), UNANSWERED);
        assert!(shows("<p id=\"n\">2</p>"), "{}", dom::render());
    });
    builds_for_the_browser(src);
}

#[test]
fn a_row_is_told_apart_from_the_same_row_of_another_outer_row() {
    let src = "## Component App\nState:\n    Let n be 0.\n    Let outer be [1, 2].\n    Let inner be [1, 2].\n\
               Actions:\n    To hit:\n        Set n to n + 1.\n\
               View:\n    {for each o in outer:}{for each i in inner:}\
               <button id=\"b{o}{i}\" on-click.once={hit}>{o}{i}</button>{end}{end}\n    \
               <p id=\"n\">{n}</p>\n\n## Main\nMount App.\n";
    with_page(src, |s| {
        assert_eq!(click(s, "#b11"), ANSWERED);
        assert_eq!(
            click(s, "#b21"),
            ANSWERED,
            "row 1 of the second outer row is a different row"
        );
        assert_eq!(click(s, "#b11"), UNANSWERED);
        assert!(shows("<p id=\"n\">2</p>"), "{}", dom::render());
    });
    builds_for_the_browser(src);
}

#[test]
fn a_remounted_component_starts_with_its_once_listener_armed() {
    let src = "## Component Child (ping: Action)\nView:\n    <button id=\"b\" on-click.once={ping}>b</button>\n\n\
               ## Component App\nState:\n    Let n be 0.\n    Let show be true.\nActions:\n    \
               To hit:\n        Set n to n + 1.\n    To flip:\n        Set show to not show.\n\
               View:\n    {if show:}<Child ping={hit}/>{end}\n    \
               <button id=\"toggle\" on-click={flip}>t</button>\n    <p id=\"n\">{n}</p>\n\n\
               ## Main\nMount App.\n";
    with_page(src, |s| {
        assert_eq!(click(s, "#b"), ANSWERED);
        assert_eq!(click(s, "#b"), UNANSWERED);
        assert_eq!(click(s, "#toggle"), ANSWERED);
        assert!(!shows("id=\"b\""), "the child left: {}", dom::render());
        assert_eq!(click(s, "#toggle"), ANSWERED);
        assert_eq!(
            click(s, "#b"),
            ANSWERED,
            "a new instance's listener is armed"
        );
        assert!(shows("<p id=\"n\">2</p>"), "{}", dom::render());
    });
    builds_for_the_browser(src);
}

/// A page with no `.once` pays nothing: the spent records are declared only when a handler asks.
#[test]
fn a_page_without_once_declares_no_spent_record() {
    let src = "## Component App\nState:\n    Let n be 0.\nActions:\n    To hit:\n        Set n to n + 1.\n\
               View:\n    <button id=\"b\" on-click={hit}>b</button>\n\n## Main\nMount App.\n";
    let lowered = with_parsed_program(src, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("parses");
        stmts
            .iter()
            .filter_map(|s| match s {
                Stmt::Let { var, .. } => Some(interner.resolve(*var).to_string()),
                _ => None,
            })
            .collect::<Vec<_>>()
    });
    for name in ["__LGX_ONCE", "__LISTENER_ONCE"] {
        assert!(
            !lowered.iter().any(|l| l == name),
            "`{name}` is declared only when a handler is `.once`: {lowered:?}"
        );
    }
}
