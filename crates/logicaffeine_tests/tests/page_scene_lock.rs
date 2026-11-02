//! ★ LOCK — a page `## Test` sets its scene BEFORE the page loads, and a reload loads it again,
//! on every page tier. ★
//!
//! A `## Page` mounts itself: that mount belongs to the program, and the program runs before any
//! test statement. So a test that says `Given that GET "/api/todos" answers …` arranged its server
//! AFTER the page had already asked it, and the load-time request went unanswered — a page that
//! fetches `When the page loads` could not be tested at all. A browser test sets the scene, THEN
//! opens the page; the runner now does the same: the test's leading scene statements (`Given that …
//! answers`, `Given a file`, `Serve fetches with routes`) run first, then the page's own mount.
//!
//! And `Reload the page.` is the page loading again — in a browser the program runs again and the
//! page mounts itself again. It used to leave a page file's test looking at an empty document.
//!
//! Every test runs the authoritative verdict (`largo test`'s runner, the tree-walker) AND the page
//! host's lowered tier, and asserts the same page on both.

use logicaffeine_compile::page_host::{compare_tests, Axis, PageTierKind, TierRun};
use logicaffeine_compile::testrun::{run_tests, TestConfig, Verdict};

const BUDGET: u64 = 50_000_000;

/// The runner's verdict for every test, by name.
fn verdicts(src: &str) -> Vec<(String, Verdict, Vec<String>)> {
    run_tests(src, &TestConfig::default())
        .unwrap_or_else(|e| panic!("the page must parse:\n{src}\n=> {e}"))
        .outcomes
        .into_iter()
        .map(|o| (o.name, o.verdict, o.failures))
        .collect()
}

fn all_pass(src: &str, count: usize) {
    let v = verdicts(src);
    assert_eq!(v.len(), count, "{v:?}");
    for (name, verdict, failures) in &v {
        assert_eq!(
            *verdict,
            Verdict::Passed,
            "`{name}` must pass on the reference tier: {failures:#?}"
        );
    }
}

/// The lowered tier's last page for the one test named `test`, held equal to the reference's on
/// every axis but the (ledgered) journal.
fn lowered_doc(src: &str, test: &str) -> String {
    let all = compare_tests(src, &[PageTierKind::LoweredVm], 0, BUDGET, None).unwrap();
    let c = all
        .iter()
        .find(|c| c.test == test)
        .unwrap_or_else(|| panic!("no test `{test}`"));
    let TierRun::Traced(t) = &c.candidates[0].1 else {
        panic!("the lowered tier must run `{test}`: {:?}", c.candidates)
    };
    let d = c.reference.compare(t);
    assert!(
        d.axes.iter().all(|a| a.axis == Axis::Journal),
        "`{test}` diverges:\n{}",
        d.render()
    );
    t.steps.last().unwrap().snapshot.document.clone()
}

const LOADING_PAGE: &str = "\
## A Todo has:
    an id, which is Int.
    a text, which is Text.

## Page /
State:
    Let label be \"...\".
Actions:
    When the page loads:
        Get \"/api/todo\" as Todo, then loaded.
    To loaded (r: Result of Todo and HttpError):
        Inspect r:
            When Ok (v):
                Set label to v's text.
            When Err (e):
                Set label to e's message.
View:
    <p id=\"label\">{label}</p>

";

#[test]
fn a_stub_given_in_the_test_answers_the_pages_load_time_request() {
    let src = format!(
        "{LOADING_PAGE}## Test: the page shows what the server says\n\
         Given that GET \"/api/todo\" answers 200 with \"{{{{\\\"id\\\": 1, \\\"text\\\": \\\"milk\\\"}}}}\".\n\
         Expect that the text of \"#label\" is equal to \"milk\".\n"
    );
    all_pass(&src, 1);
    assert!(lowered_doc(&src, "the page shows what the server says").contains(">milk<"));
}

#[test]
fn a_failing_answer_given_in_the_test_reaches_the_page_too() {
    let src = format!(
        "{LOADING_PAGE}## Test: the page shows the server's refusal\n\
         Given that GET \"/api/todo\" answers 503 with \"busy\".\n\
         Expect that the text of \"#label\" is equal to \"503: busy\".\n"
    );
    all_pass(&src, 1);
    assert!(lowered_doc(&src, "the page shows the server's refusal").contains(">503: busy<"));
}

#[test]
fn a_page_with_no_scene_still_loads_before_the_first_statement() {
    // Nothing to arrange: the page is on screen when the test's first statement looks at it.
    let src = format!(
        "{LOADING_PAGE}## Test: nothing answers\n\
         Expect that the text of \"#label\" is equal to \"...\".\n"
    );
    all_pass(&src, 1);
    assert!(lowered_doc(&src, "nothing answers").contains(">...<"));
}

#[test]
fn a_stub_given_later_still_changes_what_later_requests_hear() {
    // Only the LEADING scene is set before the page loads; a stub given after the page is up is
    // a change of scene, answering the requests that follow it.
    let src = "\
## A Todo has:
    an id, which is Int.
    a text, which is Text.

## Page /
State:
    Let label be \"...\".
Actions:
    To load:
        Get \"/api/todo\" as Todo, then loaded.
    To loaded (r: Result of Todo and HttpError):
        Inspect r:
            When Ok (v):
                Set label to v's text.
            When Err (e):
                Set label to e's message.
View:
    <p id=\"label\">{label}</p><button id=\"go\" on-click={load}>go</button>

## Test: the scene changes
Given that GET \"/api/todo\" answers 404 with \"gone\".
Click \"#go\".
Expect that the text of \"#label\" is equal to \"404: gone\".
Given that GET \"/api/todo\" answers 200 with \"{{\\\"id\\\": 1, \\\"text\\\": \\\"back\\\"}}\".
Click \"#go\".
Expect that the text of \"#label\" is equal to \"back\".
";
    all_pass(src, 1);
    assert!(lowered_doc(src, "the scene changes").contains(">back<"));
}

const COUNTER_PAGE: &str = "\
## Page /
State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
View:
    <p id=\"n\">{n}</p><button id=\"b\" on-click={bump}>+</button>

";

#[test]
fn reloading_a_page_loads_it_again() {
    let src = format!(
        "{COUNTER_PAGE}## Test: a reload starts the page over\n\
         Click \"#b\".\n\
         Expect that the text of \"#n\" is equal to \"1\".\n\
         Reload the page.\n\
         Expect that the text of \"#n\" is equal to \"0\".\n\
         Click \"#b\".\n\
         Expect that the text of \"#n\" is equal to \"1\".\n"
    );
    all_pass(&src, 1);
    let doc = lowered_doc(&src, "a reload starts the page over");
    assert!(doc.contains("<p id=\"n\">1</p>"), "{doc}");
}

#[test]
fn a_component_test_that_mounts_for_itself_is_unchanged() {
    // A `## Component` is mounted by the TEST, where the test says — nothing is moved.
    let src = "## Component App\nState:\n    Let n be 0.\nView:\n    <p id=\"n\">{n}</p>\n\n\
        ## Test: mounted here\n\
        \x20   Expect that the count of \"#n\" is equal to 0.\n\
        \x20   Mount App.\n\
        \x20   Expect that the count of \"#n\" is equal to 1.\n";
    all_pass(src, 1);
    assert!(lowered_doc(src, "mounted here").contains("<p id=\"n\">0</p>"));
}

#[test]
fn every_test_starts_with_a_fresh_document() {
    // Each test runs in a fresh world, and the page's world includes its document: the URL, the
    // title, the requests sent, the answers arranged. A test that inherited the previous test's
    // location would pass or fail by the order the tests happened to run in.
    let src = "## Component App\nView:\n    <p>x</p>\n\n\
        ## Test: a first test moves the location\n\
        \x20   Mount App.\n\
        \x20   Navigate to \"/elsewhere\".\n\
        \x20   Expect that the URL is equal to \"/elsewhere\".\n\n\
        ## Test: b second test starts at the root\n\
        \x20   Mount App.\n\
        \x20   Expect that the URL is equal to \"/\".\n";
    all_pass(src, 2);
}
