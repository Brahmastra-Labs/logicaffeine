//! ★ LOCK — a typed fetch COMPLETES on every in-process page tier. ★
//!
//! `Get URL as T, then action.` hands the host a completion closure. A browser calls it when the
//! answer arrives; the in-process bytecode runner never could, because nothing outside a running
//! program could call back into it — so on that tier the request went out and the page sat on its
//! initial state forever, and no fetch behaviour had a cross-tier proof.
//!
//! The page host now completes it the way `runtime.js` does: after every statement it settles the
//! page, calling each answered request's completion closure through the VM session. Every test here
//! runs a `## Test` on the reference tier AND on the lowered VM, and asserts both that the lowered
//! page reached the answered state and that its page equals the reference's after every
//! statement.

use logicaffeine_compile::page_host::{
    compare_tests, Axis, PageTierKind, PageTrace, TestTierComparison, TierRun,
};

const BUDGET: u64 = 50_000_000;

/// Every in-process tier a page runs on besides the reference.
const TIERS: [PageTierKind; 1] = [PageTierKind::LoweredVm];

const PAGE: &str = "\
## A Todo has:
    an id, which is Int.
    a text, which is Text.

## Component App
State:
    Let label be \"...\".
    Let shout always be label + \"!\".
    Let problem be \"-\".
    Let failed be \"-\".
Actions:
    When the page loads:
        Get \"/api/todo\" as Todo, then loaded.
    To loaded (r: Result of Todo and HttpError):
        Inspect r:
            When Ok (v):
                Set label to v's text.
            When Err (e):
                Set problem to e's message.
    When a fetch fails (url: Text) and (why: Text):
        Set failed to url.
View:
    <p id=\"label\">{label}</p><p id=\"shout\">{shout}</p><p id=\"problem\">{problem}</p><p id=\"failed\">{failed}</p>

";

/// A JSON body as a LOGOS text literal: `{{`/`}}` are literal braces, `\"` a quote.
fn json(s: &str) -> String {
    s.replace('{', "{{").replace('}', "}}").replace('"', "\\\"")
}

fn run(test_body: &str) -> TestTierComparison {
    let src = format!("{PAGE}## Test: fetch\n{test_body}");
    let mut all = compare_tests(&src, &TIERS, 0, BUDGET, None)
        .unwrap_or_else(|e| panic!("the page must parse:\n{src}\n=> {e}"));
    assert_eq!(all.len(), 1);
    all.remove(0)
}

/// The lowered run, which must be a trace, agreeing with the reference on every page axis except
/// the journal (the lowering records no journal yet — `lowered_vm_tier_lock`'s ledger holds that).
fn lowered_agrees(c: &TestTierComparison) -> &PageTrace {
    for (kind, run) in &c.candidates {
        let TierRun::Traced(t) = run else {
            panic!("{kind:?} must trace a typed fetch: {run:?}")
        };
        let d = c.reference.compare(t);
        assert!(
            d.axes.iter().all(|a| a.axis == Axis::Journal),
            "{kind:?} diverges from the reference:\n{}",
            d.render()
        );
    }
    match &c.candidates[0].1 {
        TierRun::Traced(t) => t,
        TierRun::Refused { .. } => unreachable!(),
    }
}

fn final_doc(t: &PageTrace) -> &str {
    &t.steps.last().expect("a trace has steps").snapshot.document
}

#[test]
fn an_answered_fetch_completes_on_the_lowered_vm() {
    let c = run(&format!(
        "    Given that GET \"/api/todo\" answers 200 with \"{}\".\n    Mount App.\n",
        json(r#"{"id":1,"text":"milk"}"#)
    ));
    let t = lowered_agrees(&c);
    let doc = final_doc(t);
    assert!(
        doc.contains("<p id=\"label\">milk</p>"),
        "the completion ran in-process: {doc}"
    );
    assert!(
        doc.contains("<p id=\"shout\">milk!</p>"),
        "as a turn — the derived field recomputed: {doc}"
    );
    assert!(
        doc.contains("<p id=\"failed\">-</p>"),
        "a request that worked tells no failure handler: {doc}"
    );
    assert!(
        final_doc(&c.reference).contains("milk!"),
        "and the reference agrees"
    );
}

#[test]
fn a_server_error_reaches_the_action_and_the_page_wide_handler() {
    let c = run("    Given that GET \"/api/todo\" answers 404 with \"missing\".\n    Mount App.\n");
    let doc = final_doc(lowered_agrees(&c));
    assert!(
        doc.contains("<p id=\"problem\">404: missing</p>"),
        "the action hears the status: {doc}"
    );
    assert!(
        doc.contains("<p id=\"failed\">/api/todo</p>"),
        "and the page-wide handler the URL: {doc}"
    );
    assert!(
        doc.contains("<p id=\"label\">...</p>"),
        "the value it never got is untouched: {doc}"
    );
}

#[test]
fn a_body_that_will_not_decode_is_an_error_on_every_tier() {
    let c =
        run("    Given that GET \"/api/todo\" answers 200 with \"not json\".\n    Mount App.\n");
    let doc = final_doc(lowered_agrees(&c));
    assert!(
        !doc.contains("<p id=\"problem\">-</p>"),
        "the decode failure reached the action: {doc}"
    );
    assert!(
        doc.contains("<p id=\"label\">...</p>"),
        "no value was invented: {doc}"
    );
}

#[test]
fn an_unanswered_fetch_leaves_the_page_as_it_was_on_every_tier() {
    let c = run("    Mount App.\n");
    let t = lowered_agrees(&c);
    assert!(final_doc(t).contains("<p id=\"label\">...</p>"));
    let fetches = &t.steps.last().unwrap().snapshot.fetches;
    assert_eq!(fetches.len(), 1, "the request went out: {fetches:?}");
    assert!(fetches[0].starts_with("GET /api/todo "), "{fetches:?}");
}

#[test]
fn a_stub_arranged_after_the_request_went_out_does_not_answer_it() {
    // An answer is decided when the request is SENT — a request nothing answered stays
    // unanswered, on every tier, rather than being reached back into by a later arrangement.
    let c = run(&format!(
        "    Mount App.\n    Given that GET \"/api/todo\" answers 200 with \"{}\".\n",
        json(r#"{"id":1,"text":"milk"}"#)
    ));
    let t = lowered_agrees(&c);
    assert!(
        final_doc(t).contains("<p id=\"label\">...</p>"),
        "{}",
        final_doc(t)
    );
}

#[test]
fn the_stub_is_matched_by_method_and_url_together_on_every_tier() {
    let c = run(&format!(
        "    Given that POST \"/api/todo\" answers 200 with \"{}\".\n    Mount App.\n",
        json(r#"{"id":1,"text":"milk"}"#)
    ));
    assert!(final_doc(lowered_agrees(&c)).contains("<p id=\"label\">...</p>"));
}

#[test]
fn a_completion_that_sends_another_request_settles_all_the_way() {
    let src = format!(
        "## A Todo has:\n    an id, which is Int.\n    a text, which is Text.\n\n\
         ## Component App\nState:\n    Let note be \"start\".\n\
         Actions:\n    When the page loads:\n        Get \"/one\" as Todo, then first.\n\
         \x20   To first (r: Result of Todo and HttpError):\n        Get \"/two\" as Todo, then second.\n\
         \x20   To second (r: Result of Todo and HttpError):\n        Inspect r:\n\
         \x20           When Ok (v):\n                Set note to v's text.\n\
         \x20           When Err (e):\n                Set note to \"failed\".\n\
         View:\n    <p id=\"s\">{{note}}</p>\n\n\
         ## Test: chained\n\
         \x20   Given that GET \"/one\" answers 200 with \"{}\".\n\
         \x20   Given that GET \"/two\" answers 200 with \"{}\".\n\
         \x20   Mount App.\n",
        json(r#"{"id":1,"text":"first"}"#),
        json(r#"{"id":2,"text":"second"}"#)
    );
    let c = compare_tests(&src, &TIERS, 0, BUDGET, None)
        .unwrap()
        .remove(0);
    let doc = final_doc(lowered_agrees(&c)).to_string();
    assert!(
        doc.contains("<p id=\"s\">second</p>"),
        "the settle loop drained the chained completion: {doc}"
    );
}

#[test]
fn a_fetch_driven_by_a_click_completes_after_the_click() {
    let src = format!(
        "## A Todo has:\n    an id, which is Int.\n    a text, which is Text.\n\n\
         ## Component App\nState:\n    Let label be \"...\".\n\
         Actions:\n    To load:\n        Get \"/api/todo\" as Todo, then loaded.\n\
         \x20   To loaded (r: Result of Todo and HttpError):\n        Inspect r:\n\
         \x20           When Ok (v):\n                Set label to v's text.\n\
         \x20           When Err (e):\n                Set label to \"err\".\n\
         View:\n    <p id=\"l\">{{label}}</p><button id=\"go\" on-click={{load}}>go</button>\n\n\
         ## Test: on demand\n\
         \x20   Given that GET \"/api/todo\" answers 200 with \"{}\".\n\
         \x20   Mount App.\n\
         \x20   Expect that the text of \"#l\" is equal to \"...\".\n\
         \x20   Click \"#go\".\n\
         \x20   Expect that the text of \"#l\" is equal to \"milk\".\n",
        json(r#"{"id":1,"text":"milk"}"#)
    );
    let c = compare_tests(&src, &TIERS, 0, BUDGET, None)
        .unwrap()
        .remove(0);
    let t = lowered_agrees(&c);
    assert!(
        t.steps[2].snapshot.document.contains(">...<"),
        "nothing before the click"
    );
    assert!(
        t.steps[4].snapshot.document.contains(">milk<"),
        "the completion after it"
    );
}
