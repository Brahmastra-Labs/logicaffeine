//! ★ LOCK — an act that takes the browser to another document leaves the harness AT that
//! document, every time; an act that does not, does not wait for one.
//!
//! Chrome replaces a document on its own schedule: a followed link, a submitted form and
//! `location.assign` are asked for inside the page's own task and committed whenever the network
//! answers. The harness has to KNOW, for every act, whether the page asked to leave — and when it
//! did, read nothing until the next document has arrived, and lose nothing the leaving document
//! reported on its way out. Each row drives one kind of navigation; the kinds whose ordering Chrome
//! does not fix run in a loop, and the rows that force the late ordering (a turn that keeps working
//! after it navigated, a navigation a request's completion starts) run a few times.
//!
//! | test | property |
//! |---|---|
//! | `a_followed_link_leaves_the_page_for_the_document_it_names` | a trusted click on a link arrives at its document |
//! | `a_click_whose_turn_assigns_the_location_leaves_the_page` | `location.assign` inside a click's turn |
//! | `a_click_on_a_submit_button_leaves_the_page_for_the_forms_answer` | a submit button's default action |
//! | `enter_in_a_field_submits_its_form_and_leaves_the_page` | implicit submission |
//! | `a_submitted_form_leaves_the_page_for_its_answer` | `requestSubmit`, a POST |
//! | `a_form_submitted_while_a_request_is_unanswered_leaves_the_page` | a held request does not hold the page |
//! | `a_change_whose_turn_assigns_the_location_leaves_the_page` | a navigation from typing's `change` |
//! | `a_turn_that_keeps_working_after_it_navigated_leaves_and_is_journaled` | the leaving task ends long after the navigation began |
//! | `a_navigation_a_completed_request_starts_is_followed_before_the_act_ends` | a navigation started while the act settles |
//! | `an_answer_whose_completion_navigates_leaves_the_page` | a navigation started by the answer the test gave |
//! | `a_timer_that_navigates_is_followed_when_the_clock_reaches_it` | a navigation from a timer the clock ran |
//! | `a_read_after_the_page_asked_to_leave_reads_the_document_it_went_to` | a reader arrives before it reads |
//! | `a_read_while_the_page_is_leaving_reads_a_whole_document_and_then_the_one_it_went_to` | a read racing a navigation nothing drove sees no half-replaced document |
//! | `an_act_after_the_page_asked_to_leave_acts_on_the_document_it_went_to` | an act arrives before it acts |
//! | `settling_a_page_that_asked_to_leave_arrives_where_it_went` | a settle ends in the document the page went to |
//! | `a_link_to_another_window_leaves_the_page_where_it_is` | a navigation of another window is not this page's |
//! | `a_link_opened_in_a_new_tab_leaves_the_page_where_it_is` | nor one the page asks for in a new tab |
//! | `a_frame_inside_the_page_navigating_leaves_the_page_where_it_is` | nor is a navigation of a frame inside it |
//! | `a_submission_answered_with_no_content_stays_on_the_page` | a navigation the browser abandons leaves the page in place |
//! | `an_act_that_does_not_navigate_reads_the_page_it_acted_on` | no navigation, no wait |
//! | `a_page_that_navigates_away_leaves_it_on_the_browser_tier_every_time` | a LOGOS page's `Navigate to` another page file |
//! | `a_page_followed_by_its_link_leaves_it_on_the_browser_tier_every_time` | a LOGOS page's link to another page file |
//! | `a_page_whose_form_nothing_handles_is_replaced_on_the_browser_tier_every_time` | a LOGOS page's unhandled form |
//! | `a_page_turn_that_keeps_working_after_navigating_is_journaled` | the leaving turn's note survives the document |

#[path = "cli_common/mod.rs"]
mod common;

use logicaffeine_cli::browser::{self, Answer, Browser, OpenOptions, Page, PageBundle};
use logicaffeine_compile::page_host::{compare_tests, PageTierKind, PageTrace, TierRun};

/// How often a row whose ordering Chrome leaves open is driven.
const TIMES: usize = 20;

/// How often a row that forces the late ordering is driven.
const FORCED: usize = 3;

/// The browser this binary shares, or `None` (recorded as a skip) on a machine without Chrome.
/// `LOGOS_REQUIRE_TOOLS=chrome` makes a missing browser a failure instead.
fn browser() -> Option<&'static Browser> {
    let Some(chrome) = browser::locate_chrome() else {
        let spec = std::env::var("LOGOS_REQUIRE_TOOLS").unwrap_or_default();
        common::require_tool_strict("chrome", &spec);
        return None;
    };
    Some(browser::shared(&chrome).unwrap_or_else(|e| {
        panic!(
            "Chrome is installed at {} and must start: {e}",
            chrome.display()
        )
    }))
}

/// A document whose body is `body`, titled `here`, with an icon of its own — so the browser asks
/// the network for nothing the page did not.
fn document(body: &str) -> String {
    format!("<!doctype html>\n<html>\n<head><title>here</title><link rel=\"icon\" href=\"data:,\"></head>\n<body>\n{body}\n</body>\n</html>\n")
}

/// What the server answers at `/next`, however it is asked for.
const NEXT: &str = "<!doctype html><html><head><title>next</title><link rel=\"icon\" href=\"data:,\"></head><body><p id=\"arrived\">next</p></body></html>";

fn answer(method: &str, path: &str, status: u16, content_type: &str, body: &[u8]) -> Answer {
    Answer {
        method: method.to_string(),
        path: path.to_string(),
        status,
        content_type: content_type.to_string(),
        body: body.to_vec(),
    }
}

/// `/next` answered for both methods, plus `extra`.
fn answers(extra: &[Answer]) -> Vec<Answer> {
    let mut all = vec![
        answer(
            "GET",
            "/next",
            200,
            "text/html; charset=utf-8",
            NEXT.as_bytes(),
        ),
        answer(
            "POST",
            "/next",
            200,
            "text/html; charset=utf-8",
            NEXT.as_bytes(),
        ),
    ];
    all.extend_from_slice(extra);
    all
}

fn open(b: &Browser, body: &str, extra: &[Answer]) -> Page {
    let options = OpenOptions {
        answers: answers(extra),
        ..OpenOptions::default()
    };
    b.open_with(&PageBundle::new("/", document(body)), &options)
        .unwrap_or_else(|e| panic!("the page must open: {e}"))
}

/// The path and query of `url`.
fn path_of(url: &str) -> String {
    let rest = url.split_once("://").map_or(url, |(_, r)| r);
    rest.find('/')
        .map_or("/".to_string(), |i| rest[i..].to_string())
}

/// The page is at `/next` (with `query`), showing the document the server answered there, having
/// loaded it itself.
fn arrived_at_next(p: &Page, query: &str, kind: &str, round: usize) {
    let snap = p
        .snapshot()
        .unwrap_or_else(|e| panic!("{kind}, round {round}: the page could not be read: {e}"));
    let at = format!("/next{query}");
    assert_eq!(
        (path_of(&snap.url), snap.title.as_str(), snap.body_markup()),
        (at.clone(), "next", "<body><p id=\"arrived\">next</p></body>".to_string()),
        "{kind}, round {round}: the act took the browser to {at}, so the page read after it is the \
         document there — never the one that left, never one half-replaced"
    );
    assert_eq!(
        p.route_ops()
            .unwrap_or_else(|e| panic!("{kind}, round {round}: {e}")),
        vec![format!("load {at}")],
        "{kind}, round {round}: the page loaded this document itself"
    );
}

/// The page is still the one it opened on, at `/`, showing `shows`.
fn still_here(p: &Page, shows: &str, kind: &str) {
    let snap = p
        .snapshot()
        .unwrap_or_else(|e| panic!("{kind}: the page could not be read: {e}"));
    assert_eq!(path_of(&snap.url), "/", "{kind}: the page did not leave");
    assert_eq!(
        snap.title, "here",
        "{kind}: the document is the one the page opened on"
    );
    assert!(
        snap.body_markup().contains(shows),
        "{kind}: it shows {shows:?}:\n{}",
        snap.body_markup()
    );
    assert_eq!(
        p.route_ops().unwrap(),
        Vec::<String>::new(),
        "{kind}: nothing moved the location"
    );
}

// ---- Each kind of navigation a visitor's act can start ----

#[test]
fn a_followed_link_leaves_the_page_for_the_document_it_names() {
    let Some(b) = browser() else { return };
    for round in 0..TIMES {
        let p = open(
            b,
            r#"<a id="away" href="/next"><span id="label">away</span></a>"#,
            &[],
        );
        p.click("#label")
            .unwrap_or_else(|e| panic!("round {round}: {e}"));
        arrived_at_next(&p, "", "a followed link", round);
    }
}

#[test]
fn a_click_whose_turn_assigns_the_location_leaves_the_page() {
    let Some(b) = browser() else { return };
    for round in 0..TIMES {
        let p = open(
            b,
            r#"<button id="go" onclick="location.assign('/next')">go</button>"#,
            &[],
        );
        p.click("#go")
            .unwrap_or_else(|e| panic!("round {round}: {e}"));
        arrived_at_next(&p, "", "location.assign in a click", round);
    }
}

#[test]
fn a_click_on_a_submit_button_leaves_the_page_for_the_forms_answer() {
    let Some(b) = browser() else { return };
    for round in 0..TIMES {
        let p = open(
            b,
            r#"<form id="f" action="/next"><input name="q" value="1"><button id="go">go</button></form>"#,
            &[],
        );
        p.click("#go")
            .unwrap_or_else(|e| panic!("round {round}: {e}"));
        arrived_at_next(&p, "?q=1", "a submit button's click", round);
        assert_eq!(
            p.submissions(),
            vec![(Some("/html[1]/body[1]/form[1]".to_string()), false)],
            "round {round}: the leaving document's report of its submission reached the harness"
        );
    }
}

#[test]
fn enter_in_a_field_submits_its_form_and_leaves_the_page() {
    let Some(b) = browser() else { return };
    for round in 0..TIMES {
        let p = open(
            b,
            r#"<form id="f" action="/next"><input id="q" name="q" value="2"></form>"#,
            &[],
        );
        p.press_in("#q", "Enter")
            .unwrap_or_else(|e| panic!("round {round}: {e}"));
        arrived_at_next(&p, "?q=2", "Enter in a field", round);
        assert_eq!(
            p.submissions(),
            vec![(Some("/html[1]/body[1]/form[1]".to_string()), false)],
            "round {round}"
        );
    }
}

#[test]
fn a_submitted_form_leaves_the_page_for_its_answer() {
    let Some(b) = browser() else { return };
    for round in 0..TIMES {
        let p = open(
            b,
            r#"<form id="f" method="post" action="/next"><input name="q" value="3"><button>go</button></form>"#,
            &[],
        );
        p.submit("#f")
            .unwrap_or_else(|e| panic!("round {round}: {e}"));
        arrived_at_next(&p, "", "a submitted POST", round);
        assert_eq!(
            p.submissions(),
            vec![(Some("/html[1]/body[1]/form[1]".to_string()), false)],
            "round {round}"
        );
    }
}

#[test]
fn a_form_submitted_while_a_request_is_unanswered_leaves_the_page() {
    let Some(b) = browser() else { return };
    for round in 0..TIMES {
        let p = open(
            b,
            r#"<button id="ask" onclick="fetch('/api/never')">ask</button><form id="f" action="/next"><input name="q" value="4"></form>"#,
            &[],
        );
        p.click("#ask")
            .unwrap_or_else(|e| panic!("round {round}: {e}"));
        assert_eq!(
            p.pending_requests().len(),
            1,
            "round {round}: the request is held, unanswered"
        );
        p.submit("#f").unwrap_or_else(|e| {
            panic!("round {round}: a request left unanswered does not stop the page leaving: {e}")
        });
        arrived_at_next(
            &p,
            "?q=4",
            "a form submitted while a request is unanswered",
            round,
        );
    }
}

#[test]
fn a_change_whose_turn_assigns_the_location_leaves_the_page() {
    let Some(b) = browser() else { return };
    for round in 0..TIMES {
        let p = open(
            b,
            r#"<input id="t" onchange="location.assign('/next')">"#,
            &[],
        );
        p.type_into("#t", "x")
            .unwrap_or_else(|e| panic!("round {round}: {e}"));
        arrived_at_next(&p, "", "location.assign in a change", round);
    }
}

// ---- The late orderings, forced ----

/// A click whose turn asks for `/next`, then keeps the renderer busy far longer than the network
/// takes to answer, then reports itself — the report is made after the navigation began, in the
/// same task.
const WORKS_AFTER_LEAVING: &str = r#"<button id="go" onclick="location.assign('/next'); let x = 0; for (let i = 0; i < 4e8; i++) x = (x + i) | 0; window.__worked = x; window.__logosHarnessReport(JSON.stringify({ turn: { action: 'leave', instance: '', changes: [], failure: '' } }))">go</button>"#;

#[test]
fn a_turn_that_keeps_working_after_it_navigated_leaves_and_is_journaled() {
    let Some(b) = browser() else { return };
    for round in 0..FORCED {
        let p = open(b, WORKS_AFTER_LEAVING, &[]);
        p.click("#go")
            .unwrap_or_else(|e| panic!("round {round}: {e}"));
        arrived_at_next(&p, "", "a turn that works on after navigating", round);
        let journal: Vec<String> = p.journal().into_iter().map(|e| e.action).collect();
        assert_eq!(
            journal,
            ["leave"],
            "round {round}: what the leaving document reported in the task that navigated reached \
             the harness before that document was replaced"
        );
    }
}

#[test]
fn a_navigation_a_completed_request_starts_is_followed_before_the_act_ends() {
    let Some(b) = browser() else { return };
    let api = answer("GET", "/api/x", 200, "application/json", b"{}");
    for round in 0..FORCED {
        let p = open(
            b,
            r#"<button id="go" onclick="fetch('/api/x').then((r) => r.text()).then(() => location.assign('/next'))">go</button>"#,
            std::slice::from_ref(&api),
        );
        p.click("#go")
            .unwrap_or_else(|e| panic!("round {round}: {e}"));
        arrived_at_next(&p, "", "a navigation from a request's completion", round);
    }
}

#[test]
fn an_answer_whose_completion_navigates_leaves_the_page() {
    let Some(b) = browser() else { return };
    for round in 0..FORCED {
        let p = open(
            b,
            r#"<button id="go" onclick="fetch('/api/later').then((r) => r.text()).then(() => location.assign('/next'))">go</button>"#,
            &[],
        );
        p.click("#go")
            .unwrap_or_else(|e| panic!("round {round}: {e}"));
        let pending = p.pending_requests();
        assert!(
            pending.len() == 1 && pending[0].0 == "GET" && pending[0].1.ends_with("/api/later"),
            "round {round}: the page waits on its request: {pending:?}"
        );
        p.answer("GET", "/api/later", 200, "application/json", b"{}")
            .unwrap_or_else(|e| panic!("round {round}: {e}"));
        arrived_at_next(&p, "", "a navigation from the answer's completion", round);
    }
}

#[test]
fn a_timer_that_navigates_is_followed_when_the_clock_reaches_it() {
    let Some(b) = browser() else { return };
    for round in 0..FORCED {
        let p = open(
            b,
            r#"<p id="p">waiting</p><script>setTimeout(() => location.assign('/next'), 100)</script>"#,
            &[],
        );
        p.advance(50)
            .unwrap_or_else(|e| panic!("round {round}: {e}"));
        still_here(&p, "waiting", "a timer not yet due");
        p.advance(50)
            .unwrap_or_else(|e| panic!("round {round}: {e}"));
        arrived_at_next(&p, "", "a timer that navigates", round);
    }
}

#[test]
fn a_read_after_the_page_asked_to_leave_reads_the_document_it_went_to() {
    let Some(b) = browser() else { return };
    for round in 0..TIMES {
        let p = open(b, r#"<p id="p">here</p>"#, &[]);
        p.eval("location.assign('/next')")
            .unwrap_or_else(|e| panic!("round {round}: {e}"));
        arrived_at_next(&p, "", "a read after the page asked to leave", round);
    }
}

/// A click whose turn asks the network, and whose completion asks for `/next` and then keeps the
/// renderer busy before it reports itself — so the page leaves while nothing drives it, long after
/// the act that started it all returned.
const LEAVES_ON_COMPLETION_WORKING_ON: &str = r#"<button id="go" onclick="fetch('/api/x').then((r) => r.text()).then(() => { location.assign('/next'); let x = 0; for (let i = 0; i < 4e8; i++) x = (x + i) | 0; window.__worked = x; window.__logosHarnessReport(JSON.stringify({ turn: { action: 'leave', instance: '', changes: [], failure: '' } })) })">go</button>"#;

#[test]
fn a_read_while_the_page_is_leaving_reads_a_whole_document_and_then_the_one_it_went_to() {
    let Some(b) = browser() else { return };
    let api = answer("GET", "/api/x", 200, "application/json", b"{}");
    for round in 0..FORCED {
        let p = open(
            b,
            LEAVES_ON_COMPLETION_WORKING_ON,
            std::slice::from_ref(&api),
        );
        p.eval("document.getElementById('go').click()")
            .unwrap_or_else(|e| panic!("round {round}: {e}"));
        let snap = p.snapshot().unwrap_or_else(|e| {
            panic!(
                "round {round}: a read while the page may be leaving reads a whole document: {e}"
            )
        });
        let whole_old =
            path_of(&snap.url) == "/" && snap.title == "here" && snap.by_id("go").len() == 1;
        let whole_new = path_of(&snap.url) == "/next"
            && snap.title == "next"
            && snap.body_markup() == "<body><p id=\"arrived\">next</p></body>";
        assert!(
            whole_old || whole_new,
            "round {round}: a read made while the page may be leaving sees the document the page was in, or the \
             one it went to — whole either way, never one half-replaced: {} {:?} {}",
            snap.url,
            snap.title,
            snap.body_markup()
        );
        p.settle().unwrap_or_else(|e| panic!("round {round}: {e}"));
        arrived_at_next(&p, "", "a read while the page is leaving", round);
        let journal: Vec<String> = p.journal().into_iter().map(|e| e.action).collect();
        assert_eq!(
            journal,
            ["leave"],
            "round {round}: the leaving document's report, made after its navigation began, reached the harness \
             before the document was replaced"
        );
    }
}

#[test]
fn an_act_after_the_page_asked_to_leave_acts_on_the_document_it_went_to() {
    let Some(b) = browser() else { return };
    for round in 0..TIMES {
        let p = open(b, r#"<p id="p">here</p>"#, &[]);
        p.eval("location.assign('/next')")
            .unwrap_or_else(|e| panic!("round {round}: {e}"));
        p.click("#arrived").unwrap_or_else(|e| {
            panic!("round {round}: the click lands on the document the page went to: {e}")
        });
        arrived_at_next(&p, "", "an act after the page asked to leave", round);
    }
}

#[test]
fn settling_a_page_that_asked_to_leave_arrives_where_it_went() {
    let Some(b) = browser() else { return };
    for round in 0..TIMES {
        let p = open(b, r#"<p id="p">here</p>"#, &[]);
        p.eval("location.assign('/next')")
            .unwrap_or_else(|e| panic!("round {round}: {e}"));
        p.settle().unwrap_or_else(|e| panic!("round {round}: {e}"));
        let committed = p.navigations();
        assert!(
            committed.last().is_some_and(|u| path_of(u) == "/next"),
            "round {round}: a settled page is at rest in the document it went to, committed before \
             the settle returned: {committed:?}"
        );
        arrived_at_next(&p, "", "settling after the page asked to leave", round);
    }
}

// ---- Acts that leave the page where it is ----

#[test]
fn a_link_to_another_window_leaves_the_page_where_it_is() {
    let Some(b) = browser() else { return };
    let p = open(
        b,
        r#"<a id="out" href="/next" target="_blank">out</a><p id="p">stay</p>"#,
        &[],
    );
    p.click("#out").unwrap_or_else(|e| {
        panic!(
            "a link to another window is followed there, and this page does not wait for it: {e}"
        )
    });
    still_here(&p, "stay", "a link to another window");
}

#[test]
fn a_link_opened_in_a_new_tab_leaves_the_page_where_it_is() {
    let Some(b) = browser() else { return };
    let p = open(
        b,
        r#"<a id="away" href="/next">away</a><button id="go" onclick="document.getElementById('away').dispatchEvent(new MouseEvent('click', { bubbles: true, cancelable: true, ctrlKey: true }))">go</button><p id="p">stay</p>"#,
        &[],
    );
    p.click("#go").unwrap_or_else(|e| {
        panic!("a link opened in a new tab loads there, and this page does not wait for it: {e}")
    });
    still_here(&p, "stay", "a link opened in a new tab");
}

#[test]
fn a_frame_inside_the_page_navigating_leaves_the_page_where_it_is() {
    let Some(b) = browser() else { return };
    let inner = answer(
        "GET",
        "/inner",
        200,
        "text/html; charset=utf-8",
        b"<!doctype html><p>inner</p>",
    );
    let p = open(
        b,
        r#"<iframe id="f" src="/inner"></iframe><button id="go" onclick="document.getElementById('f').src = '/next'">go</button><p id="p">stay</p>"#,
        &[inner],
    );
    for _ in 0..TIMES {
        p.click("#go").unwrap_or_else(|e| panic!("a frame inside the page navigates on its own, and the page does not wait for it: {e}"));
        still_here(&p, "stay", "a frame inside the page navigating");
    }
}

#[test]
fn a_submission_answered_with_no_content_stays_on_the_page() {
    let Some(b) = browser() else { return };
    let p = open(
        b,
        r#"<form id="f" method="post" action="/quiet"><button id="go">go</button></form><p id="p">stay</p>"#,
        &[answer("POST", "/quiet", 204, "text/plain", b"")],
    );
    p.submit("#f").unwrap_or_else(|e| {
        panic!("a 204 answers the submission with no document, and the page stays: {e}")
    });
    still_here(&p, "stay", "a submission answered 204");
    assert_eq!(
        p.submissions(),
        vec![(Some("/html[1]/body[1]/form[1]".to_string()), false)]
    );
    p.click("#go")
        .unwrap_or_else(|e| panic!("the page that stayed still answers a visitor: {e}"));
    still_here(&p, "stay", "a second submission answered 204");
}

#[test]
fn an_act_that_does_not_navigate_reads_the_page_it_acted_on() {
    let Some(b) = browser() else { return };
    let p = open(
        b,
        r#"<button id="go" onclick="this.textContent = 'went nowhere'">go</button>"#,
        &[],
    );
    for _ in 0..TIMES {
        p.click("#go").unwrap();
        still_here(&p, "went nowhere", "a click that changes the page in place");
    }
    assert!(
        p.navigation_requests().is_empty(),
        "nothing asked to leave: {:?}",
        p.navigation_requests()
    );
}

// ---- LOGOS pages on the browser tier ----

/// The Chrome trace of the ONE `## Test` in `source`.
fn on_chrome(source: &str) -> PageTrace {
    let mut all = compare_tests(
        source,
        &[PageTierKind::Browser],
        0,
        50_000_000,
        Some(&browser::ChromeLauncher::default()),
    )
    .unwrap_or_else(|e| panic!("the page must parse: {e}\n{source}"));
    assert_eq!(all.len(), 1, "exactly one page scenario:\n{source}");
    let c = all.remove(0);
    match c
        .candidates
        .into_iter()
        .find(|(k, _)| *k == PageTierKind::Browser)
    {
        Some((_, TierRun::Traced(t))) => t,
        other => panic!("Chrome must trace the scenario, not {other:?}"),
    }
}

const NAVIGATES_AWAY: &str = r##"## Page /

State:
    Let taps be 0.
Actions:
    To leave:
        Set taps to taps + 1.
        Navigate to "/about?from=home".
Head:
    <title>Home</title>
View:
    <button id="go" on-click={leave}>{taps}</button>

## Test: leaving for another page file
    Click "#go".
"##;

#[test]
fn a_page_that_navigates_away_leaves_it_on_the_browser_tier_every_time() {
    let Some(_) = browser() else { return };
    for round in 0..TIMES {
        let t = on_chrome(NAVIGATES_AWAY);
        assert!(t.stopped.is_none(), "round {round}: {:?}", t.stopped);
        let after = &t.steps[1].snapshot;
        assert_eq!(
            (
                after.document.as_str(),
                after.url.as_str(),
                after.route_ops.clone(),
                after.title.as_str(),
                after.head.clone()
            ),
            (
                "<body></body>",
                "/about?from=home",
                vec!["load /about?from=home".to_string()],
                "",
                None
            ),
            "round {round}: the browser is at the document the page sent it to"
        );
        assert_eq!(
            after.journal,
            ["leave"],
            "round {round}: the turn that navigated is journaled"
        );
    }
}

const FOLLOWS_A_LINK: &str = r##"## Page /

State:
    Let taps be 0.
Actions:
    To tap:
        Set taps to taps + 1.
View:
    <a id="away" href="/elsewhere"><span id="away-label">away</span></a>
    <button id="b" on-click={tap}>{taps}</button>

## Test: a link to another page file loads its document
    Click "#b".
    Click "#away-label".
"##;

#[test]
fn a_page_followed_by_its_link_leaves_it_on_the_browser_tier_every_time() {
    let Some(_) = browser() else { return };
    for round in 0..TIMES {
        let t = on_chrome(FOLLOWS_A_LINK);
        assert!(t.stopped.is_none(), "round {round}: {:?}", t.stopped);
        let after = &t.steps[2].snapshot;
        assert_eq!(
            (
                after.document.as_str(),
                after.url.as_str(),
                after.route_ops.clone()
            ),
            (
                "<body></body>",
                "/elsewhere",
                vec!["load /elsewhere".to_string()]
            ),
            "round {round}: the link loaded the document it names"
        );
        assert_eq!(
            after.journal,
            ["tap"],
            "round {round}: the visit's journal crossed the load"
        );
    }
}

const POSTS_ITSELF: &str = r##"## Page /

View:
    <form id="f" method="post" action="/save"><input type="hidden" name="_csrf" value="t"><input name="note" value="hi"><button id="go" type="submit">save</button></form>

## Test: a form nothing handles is answered by the server
    Given that POST "/save" answers 200 with "{{\"saved\": true}}".
    Click "#go".
"##;

#[test]
fn a_page_whose_form_nothing_handles_is_replaced_on_the_browser_tier_every_time() {
    let Some(_) = browser() else { return };
    for round in 0..TIMES {
        let t = on_chrome(POSTS_ITSELF);
        assert!(t.stopped.is_none(), "round {round}: {:?}", t.stopped);
        let after = &t.steps[2].snapshot;
        assert_eq!(
            after.url, "/save",
            "round {round}: the browser is at the address it posted to"
        );
        assert!(
            after.document.contains("{\"saved\": true}") && !after.document.contains("<form"),
            "round {round}: the document is the server's answer:\n{}",
            after.document
        );
    }
}

const WORKS_ON: &str = r##"## Page /

State:
    Let taps be 0.
    Let work be 0.
Actions:
    To leave:
        Set taps to taps + 1.
        Navigate to "/about".
        Let k be 0.
        Repeat for i from 1 to 400000000:
            Set k to k + 1.
        Set work to k.
View:
    <button id="go" on-click={leave}>{taps}</button>
"##;

#[test]
fn a_page_turn_that_keeps_working_after_navigating_is_journaled() {
    let Some(b) = browser() else { return };
    let bundle = PageBundle::page(WORKS_ON).unwrap_or_else(|e| panic!("{e}\n{WORKS_ON}"));
    for round in 0..FORCED {
        let p = b
            .open(&bundle)
            .unwrap_or_else(|e| panic!("round {round}: the page must open: {e}"));
        p.click("#go")
            .unwrap_or_else(|e| panic!("round {round}: {e}"));
        let snap = p
            .snapshot()
            .unwrap_or_else(|e| panic!("round {round}: the page could not be read: {e}"));
        assert_eq!(
            (path_of(&snap.url), snap.body_markup()),
            ("/about".to_string(), "<body></body>".to_string()),
            "round {round}: the page is at the document its turn sent it to"
        );
        let journal: Vec<String> = p.journal().into_iter().map(|e| e.action).collect();
        assert_eq!(journal, ["leave"], "round {round}: the turn noted itself after navigating, and the note reached the harness");
    }
}
