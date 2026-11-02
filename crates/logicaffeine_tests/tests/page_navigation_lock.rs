//! ★ LOCK — a page that navigates is seen to navigate: within itself, or away to another document.
//!
//! `navigate(url)` moves the location within the page (`history.pushState`): the document stays, the
//! navigation log records `push <url>`. `assignLocation(url)` loads ANOTHER document there
//! (`location.assign`): the page that asked is gone, the location and the back stack move, the new
//! document's navigation log begins with `load <url>`, and its content — another page file's — is
//! outside the test, so the document the test sees there is empty (plan D6). What the test arranged
//! for requests survives the load, as it survives a reload.
//!
//! | test | property |
//! |---|---|
//! | `a_document_load_is_recorded_and_the_test_sees_where_the_browser_went` | url, `load` op, empty document, on every in-process tier |
//! | `navigating_within_the_page_keeps_the_document` | url, `push` op, the page still there |
//! | `a_load_keeps_the_tests_answers_and_the_back_stack` | `semantics::dom::assign_location` directly |
//! | `the_browser_goes_where_the_turn_last_asked_once_the_turn_is_done` | a load is a request: the last ask wins, carried out after the turn |
//! | `a_turn_that_fails_after_asking_to_leave_is_handled_where_it_ran_and_the_browser_still_leaves` | the failure handler runs in the leaving page; the load still happens |

use logicaffeine_compile::page_host::{compare_tests, PageTierKind, PageTrace, TierRun};
use logicaffeine_compile::semantics::dom;

/// The ONE `## Test` in `source` on the lowered page hosted by the tree-walker and on the VM.
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

const LEAVES: &str = r##"## Page /

State:
    Let taps be 0.
Actions:
    To leave:
        Set taps to taps + 1.
        Call assignLocation with "/about?from=home".
Head:
    <title>Home</title>
View:
    <button id="go" on-click={leave}>{taps}</button>

## Test: leaving for another page file
    Click "#go".
"##;

#[test]
fn a_document_load_is_recorded_and_the_test_sees_where_the_browser_went() {
    for (tier, trace) in in_process(LEAVES) {
        assert!(trace.stopped.is_none(), "{tier}: {:?}", trace.stopped);
        let before = &trace.steps[0].snapshot;
        assert!(
            before.document.contains("<button id=\"go\">0</button>"),
            "{tier}: the page arrived:\n{}",
            before.document
        );
        assert_eq!(before.title, "Home", "{tier}");
        let after = &trace.steps[1].snapshot;
        assert_eq!(
            after.url, "/about?from=home",
            "{tier}: the browser went where the page sent it"
        );
        assert_eq!(
            after.route_ops,
            ["load /about?from=home"],
            "{tier}: and it went by loading a document there"
        );
        assert_eq!(
            after.document, "<body></body>",
            "{tier}: the page that asked is gone; what another page file serves is outside the test"
        );
        assert_eq!(
            (after.title.as_str(), after.head.as_deref()),
            ("", None),
            "{tier}: its head went with it"
        );
        assert_eq!(
            after.journal,
            ["leave"],
            "{tier}: the turn that left was the page's last"
        );
    }
}

const MOVES_WITHIN: &str = r##"## Page /

State:
    Let taps be 0.
Actions:
    To move:
        Set taps to taps + 1.
        Call navigate with "/inside".
View:
    <button id="go" on-click={move}>{taps}</button>

## Test: moving within the page
    Click "#go".
"##;

#[test]
fn navigating_within_the_page_keeps_the_document() {
    for (tier, trace) in in_process(MOVES_WITHIN) {
        assert!(trace.stopped.is_none(), "{tier}: {:?}", trace.stopped);
        let after = &trace.steps[1].snapshot;
        assert_eq!(after.url, "/inside", "{tier}");
        assert_eq!(
            after.route_ops,
            ["push /inside"],
            "{tier}: a move within the page is a pushState"
        );
        assert!(
            after.document.contains("<button id=\"go\">1</button>"),
            "{tier}: and the page is still there:\n{}",
            after.document
        );
    }
}

#[test]
fn a_load_keeps_the_tests_answers_and_the_back_stack() {
    dom::reset();
    dom::init_route("/", "");
    dom::stub_fetch("GET", "/api/x", 200, "kept");
    let b = dom::body();
    let el = dom::create_element("p");
    dom::append_child(b, el);
    dom::navigate("/second");
    dom::assign_location("/elsewhere?q=1");
    assert_eq!(dom::route_path(), "/elsewhere?q=1");
    assert_eq!(
        dom::route_query("q"),
        "1",
        "the loaded document's own query string"
    );
    assert_eq!(
        dom::route_ops(),
        vec![dom::RouteOp::Load {
            path: "/elsewhere?q=1".to_string()
        }]
    );
    assert_eq!(dom::render(), "<body></body>");
    let req = dom::fetch_send(
        "GET",
        "/api/x",
        "",
        Vec::new(),
        logicaffeine_compile::interpreter::RuntimeValue::Nothing,
    );
    assert_eq!(
        dom::stubbed_answer(req),
        Some((200, "kept".to_string())),
        "the test's answers survive the load"
    );
    assert_eq!(
        dom::go_back().map(|(path, _)| path),
        Some("/second".to_string()),
        "Back returns to where the page was"
    );
}

const ASKS_TWICE: &str = r##"## Page /

State:
    Let taps be 0.
Actions:
    To leave:
        Call assignLocation with "/first".
        Set taps to taps + 1.
        Call assignLocation with "/second".
View:
    <button id="go" on-click={leave}>{taps}</button>

## Test: asking twice
    Click "#go".
"##;

#[test]
fn the_browser_goes_where_the_turn_last_asked_once_the_turn_is_done() {
    for (tier, trace) in in_process(ASKS_TWICE) {
        assert!(trace.stopped.is_none(), "{tier}: {:?}", trace.stopped);
        let after = &trace.steps[1].snapshot;
        assert_eq!(
            after.url, "/second",
            "{tier}: a later `location.assign` replaces the earlier navigation"
        );
        assert_eq!(
            after.route_ops,
            ["load /second"],
            "{tier}: one document is loaded, where the turn last asked"
        );
        assert_eq!(
            after.document, "<body></body>",
            "{tier}: nothing the leaving turn drew reaches it"
        );
    }
}

const FAILS_AFTER_ASKING: &str = r##"## Page /

State:
    Let why be "".
Actions:
    To leave:
        Call assignLocation with "/away".
        Set why to item 9 of [1].
    When an action fails (e: Text):
        Set why to "caught".
View:
    <button id="go" on-click={leave}>{why}</button>

## Test: failing after asking to leave
    Click "#go".
"##;

#[test]
fn a_turn_that_fails_after_asking_to_leave_is_handled_where_it_ran_and_the_browser_still_leaves() {
    for (tier, trace) in in_process(FAILS_AFTER_ASKING) {
        assert!(
            trace.stopped.is_none(),
            "{tier}: the failure is the page's to handle: {:?}",
            trace.stopped
        );
        let after = &trace.steps[1].snapshot;
        assert_eq!(
            after.url, "/away",
            "{tier}: the navigation the turn started is carried out after its failure"
        );
        assert_eq!(after.route_ops, ["load /away"], "{tier}");
        assert_eq!(
            after.document, "<body></body>",
            "{tier}: the failure handler drew into the page that asked, which is gone"
        );
    }
}
