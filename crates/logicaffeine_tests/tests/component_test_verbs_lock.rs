//! ★ LOCK — a component is testable IN LOGOS, through `largo test`. ★
//!
//! The pyramid's crown: a page's behaviour asserted in the language the page is written in, with
//! no Rust harness, no mock DOM assembled by hand, and no wasm host. `Mount`, `Click`,
//! `Type … into` and the page queries all existed; what this pins is that they compose inside a
//! `## Test` block and that the RUNNER reports the truth about them.
//!
//! The load-bearing test in this file is `a_wrong_expectation_actually_fails`. Everything else
//! here would pass just as happily against a runner that treated every `Expect` as decoration —
//! and a test framework whose failures pass is worse than no framework, because it converts every
//! future regression into a green build. It is the same shape as the `bind` listener nothing
//! registered and the handler closure nothing could call: the machinery LOOKED wired up.

use logicaffeine_compile::testrun::{run_tests, TestConfig};

/// Run a source's `## Test` blocks and return (passed, failed).
fn run(src: &str) -> (usize, usize) {
    let report = run_tests(src, &TestConfig::default())
        .unwrap_or_else(|e| panic!("the program must parse:\n{src}\n=> {e}"));
    (report.passed, report.failed)
}

/// Every test in `src` must pass, and there must BE some — a filter typo that runs zero tests
/// reports zero failures, which reads exactly like success.
fn all_pass(src: &str, expected: usize) {
    let (passed, failed) = run(src);
    assert_eq!(failed, 0, "no test may fail:\n{src}");
    assert_eq!(
        passed, expected,
        "exactly {expected} test(s) must RUN:\n{src}"
    );
}

const COUNTER: &str = "## Component Counter\n\
    State:\n    Let n be 0.\n\
    Actions:\n    To bump:\n        Set n to n + 1.\n\
    View:\n    <p id=\"n\">{n}</p>\n    <button id=\"b\" on-click={bump}>+</button>\n\n";

#[test]
fn a_mounted_component_can_be_asserted_on() {
    let src = "## Component App\nView:\n    <p id=\"m\">hi</p>\n\n\
        ## Test: it renders its text\n\
        \x20   Mount App.\n\
        \x20   Expect that the text of \"#m\" is equal to \"hi\".\n";
    all_pass(src, 1);
}

#[test]
fn clicking_drives_a_turn_and_the_page_shows_the_result() {
    let src = format!(
        "{COUNTER}## Test: clicking counts\n\
         \x20   Mount Counter.\n\
         \x20   Expect that the text of \"#n\" is equal to \"0\".\n\
         \x20   Click \"#b\".\n\
         \x20   Expect that the text of \"#n\" is equal to \"1\".\n\
         \x20   Click \"#b\".\n\
         \x20   Expect that the text of \"#n\" is equal to \"2\".\n"
    );
    all_pass(&src, 1);
}

#[test]
fn typing_into_a_bound_input_is_observable() {
    let src = "## Component Form\n\
        State:\n    Let draft be \"\".\n\
        View:\n    <input id=\"d\" type=\"text\" bind={draft}>\n    <p id=\"echo\">{draft}</p>\n\n\
        ## Test: typing updates the echo\n\
        \x20   Mount Form.\n\
        \x20   Type \"milk\" into \"#d\".\n\
        \x20   Expect that the text of \"#echo\" is equal to \"milk\".\n";
    all_pass(src, 1);
}

#[test]
fn the_count_of_a_selector_is_assertable() {
    let src = "## Component List\n\
        State:\n    Let xs be [\"a\", \"b\", \"c\"].\n\
        View:\n    <ul>{for each x in xs:}<li class=\"row\">{x}</li>{end}</ul>\n\n\
        ## Test: it renders one row per item\n\
        \x20   Mount List.\n\
        \x20   Expect that the count of \".row\" is equal to 3.\n";
    all_pass(src, 1);
}

#[test]
fn several_tests_in_one_file_each_get_a_fresh_page() {
    // A page left mounted by one test would make the next one's assertions depend on the order
    // they happen to run in — the failure mode that makes a suite untrustworthy rather than red.
    let src = format!(
        "{COUNTER}## Test: first clicks once\n\
         \x20   Mount Counter.\n\
         \x20   Click \"#b\".\n\
         \x20   Expect that the text of \"#n\" is equal to \"1\".\n\n\
         ## Test: second starts from zero\n\
         \x20   Mount Counter.\n\
         \x20   Expect that the text of \"#n\" is equal to \"0\".\n"
    );
    all_pass(&src, 2);
}

#[test]
fn a_wrong_expectation_actually_fails() {
    // ★ THE test in this file. Everything above passes just as well against a runner that treats
    // `Expect` as decoration, and a framework whose failures pass converts every future
    // regression into a green build.
    let src = "## Component App\nView:\n    <p id=\"m\">hi</p>\n\n\
        ## Test: this expectation is wrong\n\
        \x20   Mount App.\n\
        \x20   Expect that the text of \"#m\" is equal to \"NOT WHAT IT SAYS\".\n";
    let (passed, failed) = run(src);
    assert_eq!(failed, 1, "a wrong expectation MUST fail the test:\n{src}");
    assert_eq!(passed, 0, "and must not also be counted as passing");
}

#[test]
fn a_wrong_count_expectation_actually_fails() {
    let src = "## Component App\nView:\n    <ul><li>a</li></ul>\n\n\
        ## Test: wrong count\n\
        \x20   Mount App.\n\
        \x20   Expect that the count of \"li\" is equal to 99.\n";
    let (passed, failed) = run(src);
    assert_eq!((passed, failed), (0, 1), "a wrong count MUST fail:\n{src}");
}

#[test]
fn a_click_on_a_selector_the_page_does_not_render_fails_loudly() {
    // Driving a page is where a test's own typo hides: a click that silently hits nothing would
    // leave every later assertion describing a page nobody touched.
    let src = format!(
        "{COUNTER}## Test: clicks a button that is not there\n\
         \x20   Mount Counter.\n\
         \x20   Click \"#typo\".\n"
    );
    let (passed, failed) = run(&src);
    assert_eq!(
        (passed, failed),
        (0, 1),
        "clicking a missing element must FAIL the test:\n{src}"
    );
}

#[test]
fn a_test_can_navigate_and_assert_the_url() {
    // A test DRIVES a page, so the driving verbs have to reach it. Outside a component most UI
    // verbs are deliberately not available — `Save`, `Focus` and friends would steal ordinary
    // words from an ordinary program — so the ones a test needs are allowed BY NAME rather than
    // by opening the whole surface.
    let src = "## Component App\nView:\n    <p>x</p>\n\n\
        ## Test: navigating moves the URL\n\
        \x20   Mount App.\n\
        \x20   Navigate to \"/here\".\n\
        \x20   Expect that the URL is equal to \"/here\".\n";
    all_pass(src, 1);
}

#[test]
fn a_wrong_url_expectation_fails() {
    let src = "## Component App\nView:\n    <p>x</p>\n\n\
        ## Test: wrong url\n\
        \x20   Mount App.\n\
        \x20   Navigate to \"/here\".\n\
        \x20   Expect that the URL is equal to \"/elsewhere\".\n";
    let (passed, failed) = run(src);
    assert_eq!(
        (passed, failed),
        (0, 1),
        "a wrong URL expectation must fail:\n{src}"
    );
}

#[test]
fn a_test_can_submit_a_form() {
    // A test that could only CLICK would have to know which button submits a form — the page's
    // business, not the test's. `Submit` names the intent instead.
    let src = "## Component App\n\
        State:\n    Let n be 0.\n\
        Actions:\n    When \"#f\" is submitted:\n        Set n to n + 1.\n\
        View:\n    <form id=\"f\"><p id=\"n\">{n}</p></form>\n\n\
        ## Test: submitting runs the handler\n\
        \x20   Mount App.\n\
        \x20   Expect that the text of \"#n\" is equal to \"0\".\n\
        \x20   Submit \"#f\".\n\
        \x20   Expect that the text of \"#n\" is equal to \"1\".\n";
    all_pass(src, 1);
}

#[test]
fn submitting_a_form_that_answers_nothing_fails_loudly() {
    // The same honesty the driving verbs already have: a `Submit` that quietly hit nothing would
    // leave every later assertion describing a page nobody touched.
    let src = "## Component App\nView:\n    <form id=\"f\"><p>x</p></form>\n\n\
        ## Test: nothing answers\n\
        \x20   Mount App.\n\
        \x20   Submit \"#f\".\n";
    let (passed, failed) = run(src);
    assert_eq!(
        (passed, failed),
        (0, 1),
        "a submit nothing answers must FAIL:\n{src}"
    );
}
