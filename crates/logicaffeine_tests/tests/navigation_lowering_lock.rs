//! LOCK — `Navigate to` another page file loads that page; a target this page serves stays in it.
//!
//! A module with a `## Page` emits one predicate, `__lgx_owns(path)`: whether its declared route
//! serves `path`, query and fragment set aside. `Navigate to` branches on it — a pushState and a
//! delivery to the page's route listeners when it does, `assignLocation` (a document load) when it
//! does not — and a host's link-following asks the same function, so the rule has one copy.

use logicaffeine_compile::interpreter::RuntimeValue;
use logicaffeine_compile::page_host::{compare_tests, PageTierKind, PageTrace, TierRun};
use logicaffeine_compile::semantics::dom;
use logicaffeine_compile::ui_bridge::with_parsed_program;
use logicaffeine_compile::vm::session::Session;
use logicaffeine_compile::vm::{Compiler, Vm};
use logicaffeine_language::ast::stmt::Stmt;
use logicaffeine_language::parser::lgx_lower::owns_fn_name;
use std::rc::Rc;

/// The one `## Test` in `source`, traced on the lowered page hosted by the tree-walker and on the VM.
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

/// `__lgx_owns(path)` as the lowered VM answers it for `source`.
fn owns(source: &str, path: &str) -> bool {
    dom::reset();
    with_parsed_program(source, |parsed, interner| {
        let (stmts, types, _) = parsed.unwrap_or_else(|e| panic!("parses: {e}\n{source}"));
        let program = Compiler::compile_with_types(stmts, interner, Some(types))
            .unwrap_or_else(|e| panic!("compiles: {e}\n{source}"));
        let mut session = Session::new(Vm::new(&program), interner);
        session
            .run_main()
            .unwrap_or_else(|e| panic!("main runs: {e}"));
        match session.invoke(
            owns_fn_name(),
            vec![RuntimeValue::Text(Rc::new(path.to_string()))],
        ) {
            Ok(RuntimeValue::Bool(b)) => b,
            other => panic!("`{}` answers a Bool: {other:?}", owns_fn_name()),
        }
    })
}

const LEAVES: &str = r##"## Page /

State:
    Let taps be 0.
Actions:
    When the route changes:
        Set taps to taps + 100.
    To leave:
        Set taps to taps + 1.
        Navigate to "/about?from=home".
View:
    <button id="go" on-click={leave}>{taps}</button>

## Test: leaving for another page file
    Click "#go".
"##;

#[test]
fn navigating_to_another_page_file_loads_it() {
    for (tier, trace) in in_process(LEAVES) {
        assert!(trace.stopped.is_none(), "{tier}: {:?}", trace.stopped);
        let after = &trace.steps[1].snapshot;
        assert_eq!(
            after.url, "/about?from=home",
            "{tier}: the browser went where the page sent it"
        );
        assert_eq!(
            after.route_ops,
            ["load /about?from=home"],
            "{tier}: by loading the document that serves it"
        );
        assert_eq!(
            after.document, "<body></body>",
            "{tier}: the page that asked is gone"
        );
        assert_eq!(
            after.journal,
            ["leave"],
            "{tier}: and no route listener of the page that left ran"
        );
    }
}

/// A page with no state repaints whole after a turn, so the load must not take the paint with it:
/// the page that asked is gone, and nothing it draws after asking lands in the next document.
const LEAVES_AND_REPAINTS: &str = r##"## Page /

Actions:
    To leave:
        Navigate to "/about".
View:
    <button id="leave" on-click={leave}>About</button>

## Test: leaving from a page that repaints whole
    Click "#leave".
"##;

#[test]
fn a_turn_that_leaves_draws_nothing_into_the_next_document() {
    for (tier, trace) in in_process(LEAVES_AND_REPAINTS) {
        assert!(trace.stopped.is_none(), "{tier}: {:?}", trace.stopped);
        let after = &trace.steps[1].snapshot;
        assert_eq!(
            after.route_ops,
            ["load /about"],
            "{tier}: another page file is a document load"
        );
        assert_eq!(
            after.document, "<body></body>",
            "{tier}: the page that asked is gone, paint and all"
        );
    }
}

const MOVES_WITHIN: &str = r##"## Page /posts/:id

State:
    Let moves be 0.
Actions:
    When the route changes:
        Set moves to moves + 1.
    To next:
        Navigate to "/posts/9#comments".
View:
    <button id="go" on-click={next}>{moves}</button>

## Test: moving to another post in the same page
    Click "#go".
"##;

#[test]
fn navigating_within_the_page_route_pushes_and_is_heard_once() {
    for (tier, trace) in in_process(MOVES_WITHIN) {
        assert!(trace.stopped.is_none(), "{tier}: {:?}", trace.stopped);
        let after = &trace.steps[1].snapshot;
        assert_eq!(after.url, "/posts/9#comments", "{tier}");
        assert_eq!(
            after.route_ops,
            ["push /posts/9#comments"],
            "{tier}: a target this page serves is a pushState"
        );
        assert!(
            after.document.contains("<button id=\"go\">1</button>"),
            "{tier}: the page stays and its route listener heard the move exactly once:\n{}",
            after.document
        );
    }
}

const STORE_NAVIGATES: &str = r##"## Store Nav

State:
    Let hops be 0.
Actions:
    To hop:
        Set hops to hops + 1.
        Navigate to "/next".

## Page /next

Use Nav.
State:
    Let heard be 0.
Actions:
    When the route changes:
        Set heard to heard + 1.
    To go:
        Call Nav's hop.
View:
    <button id="go" on-click={go}>{heard}</button>

## Test: a store action's own navigation reaches the page's listener
    Click "#go".
"##;

#[test]
fn a_store_actions_navigation_is_delivered_by_the_turn_that_called_it() {
    for (tier, trace) in in_process(STORE_NAVIGATES) {
        assert!(trace.stopped.is_none(), "{tier}: {:?}", trace.stopped);
        let after = &trace.steps[1].snapshot;
        assert_eq!(after.route_ops, ["push /next"], "{tier}");
        assert!(
            after.document.contains(">1</button>"),
            "{tier}: heard once:\n{}",
            after.document
        );
    }
}

#[test]
fn the_page_owns_exactly_the_paths_its_route_serves() {
    let literal = "## Page /\n\nView:\n    <p>home</p>\n";
    for (path, want) in [
        ("/", true),
        ("/?q=1", true),
        ("/#top", true),
        ("/about", false),
        ("", false),
    ] {
        assert_eq!(owns(literal, path), want, "`## Page /` owns {path:?}");
    }
    let pattern = "## Page /posts/:id\n\nView:\n    <p>post</p>\n";
    for (path, want) in [
        ("/posts/7", true),
        ("/posts/7?x=1#f", true),
        ("/posts/7#a?b", true),
        ("/posts", false),
        ("/posts/7/edit", false),
        ("/about", false),
    ] {
        assert_eq!(
            owns(pattern, path),
            want,
            "`## Page /posts/:id` owns {path:?}"
        );
    }
}

#[test]
fn a_module_with_no_page_emits_no_route_predicate() {
    let src = "## Component App\nState:\n    Let n be 0.\nActions:\n    To go:\n        Navigate to \"/x\".\n\
               View:\n    <button id=\"b\" on-click={go}>b</button>\n\n## Main\nMount App.\n";
    let defined = with_parsed_program(src, |parsed, interner| {
        let (stmts, _, _) = parsed.expect("parses");
        stmts.iter().any(|s| matches!(s, Stmt::FunctionDef { name, .. } if interner.resolve(*name) == owns_fn_name()))
    });
    assert!(
        !defined,
        "only a module with a `## Page` has a route to own paths by"
    );
}
