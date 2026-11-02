//! ★ LOCK — a LOGOS page in a real browser does what the reference tier says it does.
//!
//! Every page here is compiled to WebAssembly, served over loopback HTTP with the real host
//! runtime (`runtime.js`), and driven in headless Chrome through trusted input events. The oracle
//! is the tree-walker: the same page, the same steps, rendered by `semantics::dom`. A browser that
//! paints something else — twice, stale, or not at all — fails here with both documents printed.
//!
//! Each test names one property of the browser host:
//!
//! | test | property |
//! |---|---|
//! | `a_counter_page_traces_identically` | the document after every step equals the reference render |
//! | `a_prerendered_page_shows_its_markup_once` | hydration adopts the pre-rendered markup instead of painting beside it |
//! | `hydration_keeps_the_field_that_had_focus` | an element focused before the module ran is the element focused after |
//! | `a_prevent_form_does_not_navigate_on_first_paint` | `.prevent` holds on the first paint |
//! | `a_prevent_form_does_not_navigate_after_a_repaint` | …and after the form is repainted |
//! | `a_prevent_registered_twice_outside_a_paint_is_one_listener` | outside a paint it registers once |
//! | `a_text_site_updates_after_a_structural_repaint` | a site id recorded during a repaint names the reused node |
//! | `a_child_turn_after_its_parents_repaint_does_not_throw` | a mount id recorded during a repaint names the reused node |
//! | `a_child_turn_repaints_the_document_the_reference_tier_shows` | a child's own turn leaves one copy of the child |
//! | `the_host_node_table_stays_bounded` | removed nodes are released; repaints do not grow host tables |
//! | `a_response_body_over_64_kib_reaches_the_page` | host text of any length reaches the module |
//! | `a_blur_on_a_removed_node_calls_nothing` | a removed node's listeners are gone before it leaves |
//! | `typing_and_choosing_are_the_browsers_to_answer_on_every_tier` | a control's own default action answers typing and choosing |
//! | `typing_into_a_read_only_field_raises_nothing_and_is_refused_on_every_tier` | no default action, no events: the verb is refused |
//! | `a_page_scrolled_to_its_end_runs_its_scroll_trigger_on_every_tier` | the page's own scroll is heard where a browser fires it |
//! | `a_focused_element_that_leaves_the_page_takes_focus_with_it_on_every_tier` | focus leaves with the element a repaint removes |
//! | `a_focused_element_a_reorder_moves_loses_focus_and_hears_its_blur_on_every_lowered_tier` | a move is a removal: focus goes, `blur` reaches the moved element |
//! | `a_page_key_trigger_answers_in_a_browser` | a listener on a live node registered during a paint is kept |
//! | `a_selector_trigger_answers_on_first_paint_and_after_a_repaint` | a selector resolved during a paint sees what the paint draws |
//! | `a_window_trigger_runs_once_however_often_the_page_repainted` | window listeners do not stack per paint |
//! | `a_message_handler_runs_once_however_often_the_page_repainted` | relay subscriptions do not stack per paint |
//! | `the_page_relay_codec_is_the_system_relay_frame_byte_for_byte` | `runtime.js` reads and writes `RelayFrame` exactly as the system does |
//! | `two_pages_on_one_relay_hear_each_other_once` | a publish reaches the other page once and is not echoed to its publisher |
//! | `two_pages_publishing_at_once_both_hear_everything` | interleaved publishes all reach both pages |
//! | `a_click_meets_the_page_after_what_reached_it_is_handled` | a driving verb acts on the page at rest: a message written to it first is handled first |
//! | `a_relay_connection_opened_on_the_paused_clock_opens_without_moving_it` | a connection opened mid-test opens, its frame arrives, the clock stays |
//! | `chrome_journals_every_turn_as_the_reference_does` | `noteTurn`'s four arguments reach the harness journal as the reference's entries |
//! | `a_finished_fetch_leaves_no_host_state_behind` | the fetch table is empty once completions have run |
//! | `a_later_failure_reports_its_own_message_in_one_dismissible_panel` | one panel, current message, dismissible |

#[path = "cli_common/mod.rs"]
mod common;

use logicaffeine_cli::browser::{self, Browser, CallKind, OpenOptions, Page, PageBundle, RelayHub};
use logicaffeine_compile::semantics::dom;
use logicaffeine_compile::semantics::world as dom_journal;
use logicaffeine_system::relay_proto::RelayFrame;

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

fn open(b: &Browser, bundle: PageBundle) -> Page {
    b.open(&bundle)
        .unwrap_or_else(|e| panic!("the page must open: {e}"))
}

fn page(source: &str) -> PageBundle {
    PageBundle::page(source).unwrap_or_else(|e| panic!("{e}\n{source}"))
}

/// The reference tier's document after `steps` (top-level statements) run against `source`.
fn reference(source: &str, steps: &[&str]) -> String {
    dom::reset();
    let program = if steps.is_empty() {
        source.to_string()
    } else {
        format!("{source}\n## Main\n{}\n", steps.join("\n"))
    };
    let out = logicaffeine_compile::compile::tw_outcome(&program);
    assert_eq!(
        out.error, None,
        "the reference tier must run the page:\n{program}"
    );
    dom::render()
}

fn body(page: &Page) -> String {
    page.snapshot()
        .unwrap_or_else(|e| panic!("snapshot: {e}"))
        .body_markup()
}

/// The host → module entries (depth 0 exports) in a trace slice — the page's own code the host
/// runs, not the functions the module mints for the host to build its arguments with
/// (`__logos_text_new`, …).
fn entries(page: &Page, from: usize) -> Vec<String> {
    let minted = logicaffeine_compile::vm::wasm::runtime_export_names();
    let trace = page.import_trace().unwrap_or_else(|e| panic!("trace: {e}"));
    trace[from..]
        .iter()
        .filter(|c| {
            c.kind == CallKind::Export && c.depth == 0 && !minted.contains(&c.name.as_str())
        })
        .map(|c| format!("{}({})", c.name, c.args.join(", ")))
        .collect()
}

fn no_failures(page: &Page) {
    let errors: Vec<String> = page
        .console()
        .into_iter()
        .filter(|c| c.level == "error")
        .map(|c| c.text)
        .collect();
    assert!(
        page.exceptions().is_empty() && errors.is_empty(),
        "the page must not fail:\nexceptions: {:#?}\nconsole errors: {errors:#?}",
        page.exceptions()
    );
}

const COUNTER: &str = r#"## Page /

State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
View:
    <main>
      <p id="out">Count: {n}</p>
      <button id="go" on-click={bump}>go</button>
    </main>
"#;

#[test]
fn a_counter_page_traces_identically() {
    let Some(b) = browser() else { return };
    let expected = [
        reference(COUNTER, &[]),
        reference(COUNTER, &["Click \"#go\"."]),
        reference(COUNTER, &["Click \"#go\".", "Click \"#go\"."]),
    ];
    assert!(
        expected[2].contains("Count: 2"),
        "the oracle must count, or this proves nothing: {expected:?}"
    );

    let p = open(b, page(COUNTER));
    let mut seen = vec![body(&p)];
    p.click("#go").unwrap();
    seen.push(body(&p));
    p.click("#go").unwrap();
    seen.push(body(&p));
    no_failures(&p);
    assert_eq!(
        seen, expected,
        "after the mount and after each click the browser's document must be exactly the page \
         the reference tier renders — nothing beside it, nothing missing"
    );
}

const CHOSEN: &str = r#"## Page /

State:
    Let size be "m".
View:
    <main>
      <label>Size <select id="size" bind={size}><option value="s">S</option><option value="m">M</option><option value="l">L</option></select></label>
    </main>
"#;

#[test]
fn a_bound_select_shows_its_value_on_the_first_paint() {
    let Some(b) = browser() else { return };
    let p = open(b, page(CHOSEN));
    no_failures(&p);
    assert_eq!(
        p.eval("document.querySelector('#size').value").unwrap(),
        serde_json::json!("m"),
        "a select takes only a value one of its options offers, so the page's value must reach it after them"
    );
    assert_eq!(body(&p), reference(CHOSEN, &[]));
}

#[test]
fn a_prerendered_page_shows_its_markup_once() {
    let Some(b) = browser() else { return };
    let bundle = PageBundle::prerendered(COUNTER).unwrap_or_else(|e| panic!("{e}"));
    let shipped = String::from_utf8_lossy(bundle.file("/").expect("the document")).to_string();
    assert!(
        shipped.contains("Count: 0"),
        "the shipped document must already hold the markup:\n{shipped}"
    );

    let p = open(b, bundle);
    no_failures(&p);
    assert_eq!(
        body(&p),
        reference(COUNTER, &[]),
        "after hydration the page must appear ONCE — the module adopts the pre-rendered markup \
         rather than painting a second copy beside it"
    );
    let (total, replaced) = p.prerendered_fate().unwrap();
    assert!(
        total >= 3,
        "the document arrived with its elements (saw {total})"
    );
    assert!(
        replaced.is_empty(),
        "every pre-rendered element must be KEPT by the first paint — replacing it is a flash and \
         loses whatever the visitor was doing to it. Replaced: {replaced:?}"
    );

    p.click("#go").unwrap();
    assert_eq!(
        body(&p),
        reference(COUNTER, &["Click \"#go\"."]),
        "and the adopted page still counts"
    );
}

const FIELD: &str = r#"## Page /

State:
    Let name be "".
View:
    <main>
      <h1>Hello</h1>
      <input id="name" bind={name}>
      <p id="echo">{name}</p>
    </main>
"#;

#[test]
fn hydration_keeps_the_field_that_had_focus() {
    let Some(b) = browser() else { return };
    let bundle = PageBundle::prerendered(FIELD).unwrap_or_else(|e| panic!("{e}"));
    // The visitor puts the cursor in the field while the module is still downloading: parsing is
    // done (`interactive`), no script of the page has run yet.
    let focus_early = "document.addEventListener('readystatechange', () => {\n\
        if (document.readyState === 'interactive') document.querySelector('#name').focus();\n\
    });";
    let p = b
        .open_with(
            &bundle,
            &OpenOptions {
                init_scripts: vec![focus_early.to_string()],
                ..OpenOptions::default()
            },
        )
        .unwrap_or_else(|e| panic!("the page must open: {e}"));
    no_failures(&p);
    assert_eq!(
        body(&p),
        reference(FIELD, &[]),
        "the page appears once, adopted"
    );
    let snap = p.snapshot().unwrap();
    assert_eq!(
        snap.focus.as_deref(),
        Some("/html[1]/body[1]/main[1]/input[1]"),
        "the field focused before the module ran must still have focus after the first paint"
    );
    let (_, replaced) = p.prerendered_fate().unwrap();
    assert!(
        replaced.is_empty(),
        "and it is the same element, not a replacement: {replaced:?}"
    );
    p.type_into("#name", "Ada").unwrap();
    assert_eq!(
        p.snapshot().unwrap().by_id("echo")[0].text_content(),
        "Ada",
        "the adopted field is bound"
    );
}

const PREVENT: &str = r#"## Page /

State:
    Let saved be 0.
    Let on be false.
Actions:
    To save:
        Set saved to saved + 1.
    To flip:
        Set on to not on.
View:
    <main>
      <form id="f" on-submit.prevent={save}>
        <input id="name" name="name">
        {if on:}<span id="flag">on</span>{end}
        <button id="send" type="submit">send</button>
      </form>
      <p id="out">{saved}</p>
      <button id="flip" on-click={flip}>flip</button>
    </main>
"#;

fn submit_is_handled_in_place(p: &Page) {
    let before = p.navigations().len();
    p.click("#send").unwrap();
    let submits = p.submissions();
    assert_eq!(
        submits,
        vec![(Some("/html[1]/body[1]/main[1]/form[1]".to_string()), true)],
        "the form was submitted once and `.prevent` must have stopped the browser's own submission"
    );
    assert!(
        p.navigation_requests().is_empty() && p.navigations().len() == before,
        "a handled form must not ALSO post itself — the page navigated to {:?}",
        p.navigation_requests()
    );
}

#[test]
fn a_prevent_form_does_not_navigate_on_first_paint() {
    let Some(b) = browser() else { return };
    let p = open(b, page(PREVENT));
    submit_is_handled_in_place(&p);
    no_failures(&p);
    assert_eq!(
        body(&p),
        reference(PREVENT, &["Submit \"#f\"."]),
        "and the handler ran"
    );
}

#[test]
fn a_prevent_form_does_not_navigate_after_a_repaint() {
    let Some(b) = browser() else { return };
    let p = open(b, page(PREVENT));
    p.click("#flip").unwrap();
    assert!(
        body(&p).contains("id=\"flag\""),
        "the flip repainted the form"
    );
    submit_is_handled_in_place(&p);
    no_failures(&p);
    assert_eq!(
        body(&p),
        reference(PREVENT, &["Click \"#flip\".", "Submit \"#f\"."])
    );
}

/// `preventDefault` called by imperative code, outside any paint.
fn imperative_form(registrations: usize) -> String {
    format!(
        "## Main\nLet form be createElement(\"form\").\nCall setAttribute with form and \"id\" and \"f\".\n\
         Call appendChild with body() and form.\n{}",
        "Call preventDefault with form and \"submit\".\n".repeat(registrations)
    )
}

#[test]
fn a_prevent_registered_twice_outside_a_paint_is_one_listener() {
    let Some(b) = browser() else { return };
    // Count every `preventDefault` a submission receives: one per listener that prevents it.
    let count = "window.__prevents = 0;\n\
        const preventDefault = Event.prototype.preventDefault;\n\
        Event.prototype.preventDefault = function () { if (this.type === 'submit') window.__prevents++; return preventDefault.call(this); };";
    let p = b
        .open_with(
            &page(&imperative_form(2)),
            &OpenOptions {
                init_scripts: vec![count.to_string()],
                ..OpenOptions::default()
            },
        )
        .unwrap_or_else(|e| panic!("the page must open: {e}"));
    p.submit("#f").unwrap();
    assert_eq!(
        p.submissions(),
        vec![(Some("/html[1]/body[1]/form[1]".to_string()), true)]
    );
    assert_eq!(
        p.eval("window.__prevents").unwrap().as_u64(),
        Some(1),
        "a modifier is a property of the registration: asking twice must leave one listener, not two"
    );
}

const SITE_AFTER_REPAINT: &str = r#"## Page /

State:
    Let n be 0.
    Let on be false.
Actions:
    To flip:
        Set on to not on.
    To bump:
        Set n to n + 1.
View:
    <main>
      <p id="out">{n}</p>
      {if on:}<span id="flag">on</span>{end}
      <button id="flip" on-click={flip}>flip</button>
      <button id="go" on-click={bump}>go</button>
    </main>
"#;

#[test]
fn a_text_site_updates_after_a_structural_repaint() {
    let Some(b) = browser() else { return };
    let steps = ["Click \"#go\".", "Click \"#flip\".", "Click \"#go\"."];
    let expected = reference(SITE_AFTER_REPAINT, &steps);
    assert!(
        expected.contains("<p id=\"out\">2</p>"),
        "the oracle shows both bumps: {expected}"
    );
    let p = open(b, page(SITE_AFTER_REPAINT));
    p.click("#go").unwrap();
    p.click("#flip").unwrap();
    p.click("#go").unwrap();
    no_failures(&p);
    assert_eq!(
        body(&p),
        expected,
        "the second bump patches the text site the flip's repaint recorded — that id must name \
         the node the repaint kept, or the update is written nowhere"
    );
}

const CHILD_AFTER_REPAINT: &str = r#"## Component Counter

State:
    Let c be 0.
Actions:
    To inc:
        Set c to c + 1.
View:
    <div class="counter"><span id="c">{c}</span>{if c > 1:}<b>big</b>{end}<button id="inc" on-click={inc}>+</button></div>

## Page /

State:
    Let on be false.
Actions:
    To flip:
        Set on to not on.
View:
    <main>
      <Counter/>
      {if on:}<p id="flag">on</p>{end}
      <button id="flip" on-click={flip}>flip</button>
    </main>
"#;

#[test]
fn a_child_turn_after_its_parents_repaint_does_not_throw() {
    let Some(b) = browser() else { return };
    let p = open(b, page(CHILD_AFTER_REPAINT));
    p.click("#flip").unwrap();
    p.click("#inc").unwrap();
    no_failures(&p);
    let snap = p.snapshot().unwrap();
    let shown: Vec<String> = snap.by_id("c").iter().map(|n| n.text_content()).collect();
    assert!(
        shown.iter().any(|t| t == "1"),
        "the child's own turn must reach the document after its parent repainted; `#c` reads {shown:?}"
    );
}

#[test]
fn a_child_turn_repaints_the_document_the_reference_tier_shows() {
    let Some(b) = browser() else { return };
    let steps = ["Click \"#inc\".", "Click \"#inc\"."];
    let expected = reference(CHILD_AFTER_REPAINT, &steps);
    let p = open(b, page(CHILD_AFTER_REPAINT));
    p.click("#inc").unwrap();
    p.click("#inc").unwrap();
    no_failures(&p);
    assert_eq!(
        body(&p),
        expected,
        "a child component's own turn must leave exactly one copy of the child, beside its \
         siblings, as the reference tier does"
    );
}

const TOGGLE: &str = r#"## Page /

State:
    Let on be false.
Actions:
    To flip:
        Set on to not on.
View:
    <main>
      {if on:}<ul><li>a</li><li>b</li></ul>{end}
      <button id="flip" on-click={flip}>flip</button>
    </main>
"#;

#[test]
fn the_host_node_table_stays_bounded() {
    let Some(b) = browser() else { return };
    let p = open(b, page(TOGGLE));
    for _ in 0..10 {
        p.click("#flip").unwrap();
    }
    let early = p.dom_counters().unwrap();
    let early_census = p
        .eval("globalThis.__logosHostCensus ? __logosHostCensus() : null")
        .unwrap();
    for _ in 0..50 {
        p.click("#flip").unwrap();
    }
    let late = p.dom_counters().unwrap();
    let late_census = p
        .eval("globalThis.__logosHostCensus ? __logosHostCensus() : null")
        .unwrap();
    no_failures(&p);
    assert_eq!(
        body(&p),
        reference(TOGGLE, &[]),
        "an even number of flips leaves the page as it began"
    );
    assert_eq!(
        late.nodes, early.nodes,
        "fifty repaints later the renderer must hold the same number of nodes — a node the page \
         removed is still reachable from the host (early {early:?}, late {late:?})"
    );
    assert!(
        !early_census.is_null(),
        "the host exposes a census of its tables (`__logosHostCensus`) so their size can be held"
    );
    assert_eq!(
        late_census, early_census,
        "the host's own tables (nodes, listeners, fetches, subscriptions) must not grow with turns"
    );
}

const BIG_BODY: &str = r#"## Main
Let target be createElement("p").
Call setAttribute with target and "id" and "body".
Call appendChild with body() and target.
Let onDone be (reqId: Int) -> setText(target, responseBody(reqId)).
Let req be webGet("/api/big", onDone).
"#;

#[test]
fn a_response_body_over_64_kib_reaches_the_page() {
    let Some(b) = browser() else { return };
    // 65_520 is the largest body the first host page could hold (a page less the 16-byte header);
    // everything above needs more than one page.
    for size in [65_520usize, 65_521, 100_000, 300_000] {
        let p = open(b, page(BIG_BODY));
        assert_eq!(
            p.pending_requests().len(),
            1,
            "the page asked for its data: {:?}",
            p.requests()
        );
        p.answer("GET", "/api/big", 200, "text/plain", &vec![b'y'; size])
            .unwrap();
        no_failures(&p);
        let shown = p
            .eval("document.querySelector('#body').textContent.length")
            .unwrap();
        assert_eq!(
            shown.as_u64(),
            Some(size as u64),
            "a {size}-byte response body must reach the page whole"
        );
    }
}

const BLUR_ON_REMOVAL: &str = r#"## Page /

State:
    Let show be true.
    Let blurs be 0.
Actions:
    To hide:
        Set show to false.
    To left:
        Set blurs to blurs + 1.
View:
    <main>
      {if show:}<input id="field" on-keydown.enter={hide} on-blur={left}>{end}
      <p id="out">{blurs}</p>
    </main>
"#;

#[test]
fn a_blur_on_a_removed_node_calls_nothing() {
    let Some(b) = browser() else { return };
    let p = open(b, page(BLUR_ON_REMOVAL));
    p.focus("#field").unwrap();
    assert_eq!(
        p.snapshot().unwrap().focus.as_deref(),
        Some("/html[1]/body[1]/main[1]/input[1]")
    );
    let mark = p.trace_len().unwrap();
    // Enter hides the field WHILE it has focus, so the browser raises `blur` on it as it leaves.
    p.press("Enter").unwrap();
    let calls = entries(&p, mark);
    assert_eq!(
        calls.len(),
        1,
        "exactly one entry into the module — the keydown. A listener on a node the paint removed \
         must find nothing to call, not re-enter the module in the middle of a commit: {calls:#?}"
    );
    no_failures(&p);
    assert_eq!(
        body(&p),
        reference(BLUR_ON_REMOVAL, &["Press \"Enter\" in \"#field\"."])
    );
}

const FOCUS_LEAVES: &str = r##"## Page /

State:
    Let show be true.
    Let blurs be 0.
Actions:
    To hide:
        Set show to false.
    To left:
        Set blurs to blurs + 1.
View:
    <main>
      {if show:}<input id="field" on-keydown.enter={hide} on-blur={left}>{end}
      <p id="out">{blurs}</p>
    </main>

## Test: the focused field that goes away takes focus with it
    Press "Enter" in "#field".
    Expect that the text of "#out" is equal to "0".
"##;

#[test]
fn a_focused_element_that_leaves_the_page_takes_focus_with_it_on_every_tier() {
    use logicaffeine_compile::page_host::Axis;
    let Some((reference, lowered, chrome)) = three_tiers(FOCUS_LEAVES) else {
        return;
    };
    for (tier, trace) in [
        ("the reference", &reference),
        ("the lowered VM", &lowered),
        ("Chrome", &chrome),
    ] {
        assert_eq!(
            trace.steps[1].snapshot.focus, None,
            "{tier}: the field the key hid had focus, and focus left the page with it"
        );
    }
    for (tier, candidate) in [("the lowered VM", &lowered), ("Chrome", &chrome)] {
        let off = disagreements(
            &reference,
            candidate,
            &[Axis::Focus, Axis::Document, Axis::Journal],
        );
        assert!(off.is_empty(), "{tier}: {off}");
    }
}

const FOCUS_MOVES: &str = r##"## Page /

State:
    Let order be [1, 2, 3].
    Let blurs be 0.
Actions:
    To rotate:
        Set order to [2, 3, 1].
    To left:
        Set blurs to blurs + 1.
View:
    <main>
      <ul>
        {for each n in order, keyed by n:}
          <li><input id="in-{n}" on-keydown.enter={rotate} on-blur={left}></li>
        {end}
      </ul>
      <p id="out">{blurs}</p>
    </main>

## Test: the focused row that moves loses focus
    Press "Enter" in "#in-1".
"##;

/// A keyed reorder MOVES the focused row, and a move is a removal and an insertion: the row loses
/// focus and hears its `blur` (Chrome). The lowered tiers agree once the lowering repaints in
/// process through vdom.lg's reconciler — its `insertBefore` moves are what `semantics::dom`
/// models — rather than by clearing and rebuilding, which keeps focus by id.
#[test]
fn a_focused_element_a_reorder_moves_loses_focus_and_hears_its_blur_on_every_lowered_tier() {
    use logicaffeine_compile::page_host::Axis;
    let Some((here, vm, chrome)) = lowered_tiers(FOCUS_MOVES) else {
        return;
    };
    let out = |t: &logicaffeine_compile::page_host::PageTrace| {
        t.steps[1].snapshot.document.contains("<p id=\"out\">1</p>")
    };
    assert!(
        chrome.steps[1].snapshot.focus.is_none() && out(&chrome),
        "Chrome is the oracle: the reorder moved the focused row, which lost focus and heard its blur:\n{:?}\n{}",
        chrome.steps[1].snapshot.focus,
        chrome.steps[1].snapshot.document
    );
    for (tier, candidate) in [("the lowered tree-walker", &here), ("the lowered VM", &vm)] {
        let off = disagreements(
            &chrome,
            candidate,
            &[Axis::Focus, Axis::Document, Axis::Journal],
        );
        assert!(
            off.is_empty(),
            "{tier} moves focus and raises blur as Chrome does:\n{off}"
        );
    }
}

const SCROLLS_TO_END: &str = r##"## Page /

State:
    Let loads be 0.
Actions:
    When the user scrolls to the bottom:
        Set loads to loads + 1.
View:
    <main><p id="loads">{loads}</p></main>

## Test: the page scrolled to its end
    Raise "scrolledtobottom".
    Expect that the text of "#loads" is equal to "1".
"##;

#[test]
fn a_page_scrolled_to_its_end_runs_its_scroll_trigger_on_every_tier() {
    use logicaffeine_compile::page_host::Axis;
    let Some((reference, lowered, chrome)) = three_tiers(SCROLLS_TO_END) else {
        return;
    };
    assert!(
        chrome.steps[1].snapshot.document.contains("<p id=\"loads\">1</p>"),
        "the viewport's scroll reaches the page's trigger in Chrome, where a browser fires it — at the \
         document, never at body:\n{}",
        chrome.steps[1].snapshot.document
    );
    for (tier, candidate) in [("the lowered VM", &lowered), ("Chrome", &chrome)] {
        let off = disagreements(&reference, candidate, &[Axis::Document, Axis::Journal]);
        assert!(off.is_empty(), "{tier}: {off}");
    }
}

const TYPED_UNHEARD: &str = r##"## Page /

View:
    <main>
      <input id="name" required>
      <select id="size"><option value="s">small</option><option value="m">medium</option></select>
    </main>

## Test: the browser answers typing and choosing where nothing listens
    Type "Ada" into "#name".
    Choose "m" in "#size".
"##;

#[test]
fn typing_and_choosing_are_the_browsers_to_answer_on_every_tier() {
    use logicaffeine_compile::page_host::Axis;
    let Some((reference, lowered, chrome)) = three_tiers(TYPED_UNHEARD) else {
        return;
    };
    for (tier, trace) in [
        ("the reference", &reference),
        ("the lowered VM", &lowered),
        ("Chrome", &chrome),
    ] {
        assert!(
            trace.stopped.is_none(),
            "{tier}: every act is answered: {:?}",
            trace.stopped
        );
    }
    let last = &chrome.steps[2].snapshot.document;
    assert!(
        last.contains("value=\"Ada\"") && last.contains("<select id=\"size\" value=\"m\">"),
        "Chrome is the oracle: the typed text and the choice are entered:\n{last}"
    );
    for (tier, candidate) in [("the lowered VM", &lowered), ("Chrome", &chrome)] {
        let off = disagreements(&reference, candidate, &[Axis::Document, Axis::Journal]);
        assert!(off.is_empty(), "{tier}: {off}");
    }
}

const TYPED_READ_ONLY: &str = r##"## Page /

State:
    Let heard be 0.
Actions:
    To hear:
        Set heard to heard + 1.
View:
    <main>
      <input id="locked" readonly value="kept" on-input={hear} on-change={hear}>
      <p id="heard">{heard}</p>
    </main>

## Test: typing into a read-only field
    Type "x" into "#locked".
"##;

#[test]
fn typing_into_a_read_only_field_raises_nothing_and_is_refused_on_every_tier() {
    let Some((reference, lowered, chrome)) = three_tiers(TYPED_READ_ONLY) else {
        return;
    };
    for (tier, trace) in [
        ("the reference", &reference),
        ("the lowered VM", &lowered),
        ("Chrome", &chrome),
    ] {
        let stop = trace
            .stopped
            .as_ref()
            .unwrap_or_else(|| panic!("{tier}: typing into a read-only field is refused"));
        assert!(
            stop.message
                .contains("Typing into \"#locked\" enters nothing"),
            "{tier}: {}",
            stop.message
        );
        assert!(
            trace
                .steps
                .last()
                .expect("the arrival")
                .snapshot
                .document
                .contains("<p id=\"heard\">0</p>"),
            "{tier}: no `input` or `change` reached the field's listeners"
        );
    }
}

const KEY_TRIGGER: &str = r#"## Page /

State:
    Let closed be 0.
Actions:
    When the user presses "Escape":
        Set closed to closed + 1.
View:
    <main>
      <p id="out">{closed}</p>
    </main>
"#;

#[test]
fn a_page_key_trigger_answers_in_a_browser() {
    let Some(b) = browser() else { return };
    let p = open(b, page(KEY_TRIGGER));
    p.press("Escape").unwrap();
    no_failures(&p);
    assert_eq!(
        body(&p),
        reference(KEY_TRIGGER, &["Press \"Escape\"."]),
        "`When the user presses \"Escape\":` listens on the page, a node no paint creates — the \
         registration made during the first paint must reach it"
    );
}

const SELECTOR_TRIGGER: &str = r##"## Page /

State:
    Let hits be 0.
    Let on be false.
Actions:
    When "#b" is clicked:
        Set hits to hits + 1.
    To flip:
        Set on to not on.
View:
    <main>
      <p id="out">{hits}</p>
      {if on:}<span>on</span>{end}
      <button id="b">+</button>
      <button id="flip" on-click={flip}>flip</button>
    </main>
"##;

#[test]
fn a_selector_trigger_answers_on_first_paint_and_after_a_repaint() {
    let Some(b) = browser() else { return };
    let p = open(b, page(SELECTOR_TRIGGER));
    p.click("#b").unwrap();
    assert_eq!(
        body(&p),
        reference(SELECTOR_TRIGGER, &["Click \"#b\"."]),
        "`When \"#b\" is clicked:` resolves its selector while the first paint is being drawn — \
         against the elements that paint is drawing, not a document that does not hold them yet"
    );
    p.click("#flip").unwrap();
    p.click("#b").unwrap();
    no_failures(&p);
    assert_eq!(
        body(&p),
        reference(
            SELECTOR_TRIGGER,
            &["Click \"#b\".", "Click \"#flip\".", "Click \"#b\"."]
        ),
        "and after a repaint the trigger still answers, once per click"
    );
}

const WINDOW_TRIGGER: &str = r#"## Page /

State:
    Let drops be 0.
    Let on be false.
Actions:
    When the app goes offline:
        Set drops to drops + 1.
    To flip:
        Set on to not on.
View:
    <main>
      <p id="out">{drops}</p>
      {if on:}<span>on</span>{end}
      <button id="flip" on-click={flip}>flip</button>
    </main>
"#;

#[test]
fn a_window_trigger_runs_once_however_often_the_page_repainted() {
    let Some(b) = browser() else { return };
    let p = open(b, page(WINDOW_TRIGGER));
    for _ in 0..4 {
        p.click("#flip").unwrap();
    }
    p.eval("window.dispatchEvent(new Event('offline'))")
        .unwrap();
    no_failures(&p);
    assert_eq!(
        p.snapshot().unwrap().by_id("out")[0].text_content(),
        "1",
        "one `offline` event is one turn — a window listener registered by each paint must replace \
         the previous paint's, not stack beside it"
    );
}

const MESSAGES: &str = r#"## Page /

State:
    Let heard be 0.
    Let on be false.
Actions:
    When a message arrives on "chat" (m: Text):
        Set heard to heard + 1.
    To flip:
        Set on to not on.
View:
    <main>
      <p id="out">{heard}</p>
      {if on:}<span>on</span>{end}
      <button id="flip" on-click={flip}>flip</button>
    </main>
"#;

const WRITES_SHARED: &str = r#"## Main
Call storageSet with "shared" and "room.count" and "5".
Call storageSet with "shared" and "room.count" and "6".
Let seen be storageGetOr("shared", "room.count", "none").
Call storageSet with "local" and "seen" and seen.
"#;

/// A page opened on `hub`: its `/_relay` is that relay.
fn open_on(b: &Browser, bundle: PageBundle, hub: &RelayHub) -> Page {
    b.open_with(
        &bundle,
        &OpenOptions {
            relay: hub.clone(),
            ..OpenOptions::default()
        },
    )
    .unwrap_or_else(|e| panic!("the page must open: {e}"))
}

/// The frames a relay read from its pages that subscribe to `topic`.
fn subscribes(hub: &RelayHub, topic: &str) -> usize {
    hub.received()
        .iter()
        .filter(|f| matches!(f, RelayFrame::Subscribe { topic: t } if t == topic))
        .count()
}

/// The payloads a relay read from its pages published on `topic`, as text.
fn published_on(hub: &RelayHub, topic: &str) -> Vec<String> {
    hub.received()
        .into_iter()
        .filter_map(|f| match f {
            RelayFrame::Publish { topic: t, data } if t == topic => {
                Some(String::from_utf8(data).expect("a page publishes UTF-8"))
            }
            _ => None,
        })
        .collect()
}

#[test]
fn a_shared_write_is_kept_and_published_on_its_topic_in_chrome() {
    let Some(b) = browser() else { return };
    let hub = RelayHub::default();
    let p = open_on(b, page(WRITES_SHARED), &hub);
    no_failures(&p);
    let received = hub.received();
    assert_eq!(
        subscribes(&hub, "room"),
        1,
        "the first touch of a topic joins it, once: {received:?}"
    );
    assert_eq!(
        published_on(&hub, "room"),
        ["room.count\u{0}5", "room.count\u{0}6"],
        "every write to the shared area is a frame on its topic — the key, a NUL, the value: {received:?}"
    );
    assert!(
        p.storage()
            .unwrap()
            .contains(&("local".to_string(), "seen".to_string(), "6".to_string())),
        "the page reads back the last value it wrote"
    );
    hub.deliver("room", b"room.count\x009");
    let untouched = p.settle();
    assert!(
        untouched.is_ok(),
        "a frame for a page that merges nothing is left alone: {untouched:?}"
    );
    no_failures(&p);
}

#[test]
fn a_message_handler_runs_once_however_often_the_page_repainted() {
    let Some(b) = browser() else { return };
    let hub = RelayHub::default();
    let p = open_on(b, page(MESSAGES), &hub);
    for _ in 0..4 {
        p.click("#flip").unwrap();
    }
    hub.deliver("chat", b"hi");
    p.settle().unwrap();
    no_failures(&p);
    assert_eq!(
        p.snapshot().unwrap().by_id("out")[0].text_content(),
        "1",
        "one message is one turn — a subscription made by each paint must replace the previous \
         paint's, not add another handler to the topic"
    );
    assert_eq!(
        subscribes(&hub, "chat"),
        1,
        "and the relay is asked for the topic once"
    );
}

/// Every shape of `RelayFrame` the census speaks: each variant, over topics and payloads that
/// cross every length-prefix and UTF-8 boundary a codec could get wrong.
fn census_frames() -> Vec<RelayFrame> {
    let topics = ["", "chat", "café ☕ 𝄞", "a\u{0}b", &"t".repeat(300)];
    let payloads: Vec<Vec<u8>> = vec![
        Vec::new(),
        b"hi".to_vec(),
        "room.count\u{0}9".as_bytes().to_vec(),
        (0..=255u8).collect(),
        vec![0xFF, 0xFE, 0x80],
        vec![b'x'; 70_000],
    ];
    let mut frames = Vec::new();
    for t in topics {
        let topic = t.to_string();
        frames.push(RelayFrame::Subscribe {
            topic: topic.clone(),
        });
        frames.push(RelayFrame::Unsubscribe {
            topic: topic.clone(),
        });
        frames.push(RelayFrame::SubAck {
            topic: topic.clone(),
        });
        for data in &payloads {
            frames.push(RelayFrame::Publish {
                topic: topic.clone(),
                data: data.clone(),
            });
            frames.push(RelayFrame::Event {
                topic: topic.clone(),
                data: data.clone(),
            });
        }
    }
    frames
}

/// A frame as the page's codec writes it: `{ Variant: { topic, data: [bytes] } }`.
fn frame_json(f: &RelayFrame) -> serde_json::Value {
    let (kind, topic, data) = match f {
        RelayFrame::Subscribe { topic } => ("Subscribe", topic, None),
        RelayFrame::Unsubscribe { topic } => ("Unsubscribe", topic, None),
        RelayFrame::Publish { topic, data } => ("Publish", topic, Some(data)),
        RelayFrame::SubAck { topic } => ("SubAck", topic, None),
        RelayFrame::Event { topic, data } => ("Event", topic, Some(data)),
    };
    let mut fields = serde_json::json!({ "topic": topic });
    if let Some(d) = data {
        fields["data"] = serde_json::json!(d);
    }
    serde_json::json!({ kind: fields })
}

/// Each of `inputs` decoded by the page's own codec, as JSON with its payload as an array — `null`
/// where it refuses the bytes.
fn js_decode_all(p: &Page, inputs: &[Vec<u8>]) -> Vec<serde_json::Value> {
    let got = p
        .eval(&format!(
            "{}.map((b) => {{ const f = __logosRelayCodec.decode(new Uint8Array(b)); if (!f) return null; \
               const k = Object.keys(f)[0]; const o = {{ topic: f[k].topic }}; \
               if (f[k].data) o.data = Array.from(f[k].data); return {{ [k]: o }}; }})",
            serde_json::json!(inputs)
        ))
        .unwrap_or_else(|e| panic!("the page's codec reads {} input(s): {e}", inputs.len()));
    serde_json::from_value(got).expect("one answer per input")
}

/// `f` encoded by the page's own codec.
fn js_encode(p: &Page, f: &RelayFrame) -> Vec<u8> {
    let json = frame_json(f);
    let got = p
        .eval(&format!(
            "(() => {{ const f = {json}; const k = Object.keys(f)[0]; \
               if (f[k].data) f[k].data = new Uint8Array(f[k].data); \
               return Array.from(__logosRelayCodec.encode(f)); }})()"
        ))
        .unwrap_or_else(|e| panic!("the page's codec writes {f:?}: {e}"));
    serde_json::from_value(got).expect("bytes")
}

/// The system's reader on `bytes`, in the page codec's JSON shape.
fn rust_decode(bytes: &[u8]) -> serde_json::Value {
    RelayFrame::from_bytes(bytes).map_or(serde_json::Value::Null, |f| frame_json(&f))
}

#[test]
fn the_page_relay_codec_is_the_system_relay_frame_byte_for_byte() {
    let Some(b) = browser() else { return };
    let p = open(b, page(MESSAGES));
    let frames = census_frames();
    assert_eq!(
        frames.len(),
        5 * (3 + 2 * 6),
        "every variant over every topic and payload"
    );
    let written: Vec<Vec<u8>> = frames.iter().map(RelayFrame::to_bytes).collect();
    let read = js_decode_all(&p, &written);
    for (f, got) in frames.iter().zip(&read) {
        assert_eq!(
            got,
            &frame_json(f),
            "the page reads the frame the system wrote, {f:?}"
        );
        assert_eq!(
            js_encode(&p, f),
            f.to_bytes(),
            "the page writes the bytes the system writes, {f:?}"
        );
    }
    let mut malformed: Vec<Vec<u8>> = Vec::new();
    for bytes in written.iter().filter(|b| b.len() < 400) {
        malformed.extend((0..bytes.len()).map(|n| bytes[..n].to_vec()));
        let mut trailing = bytes.clone();
        trailing.extend_from_slice(b"\x00tail");
        malformed.push(trailing);
    }
    let mut bad_topic = RelayFrame::Subscribe { topic: "ab".into() }.to_bytes();
    let n = bad_topic.len();
    bad_topic[n - 1] = 0xFF;
    malformed.push(bad_topic);
    for variant in [5u32, 6, 0xFFFF_FFFF] {
        let mut unknown = variant.to_le_bytes().to_vec();
        unknown.extend_from_slice(&RelayFrame::SubAck { topic: "x".into() }.to_bytes()[4..]);
        malformed.push(unknown);
    }
    let mut overlong = 0u32.to_le_bytes().to_vec();
    overlong.extend_from_slice(&u64::MAX.to_le_bytes());
    overlong.extend_from_slice(b"x");
    malformed.push(overlong);
    let page_read = js_decode_all(&p, &malformed);
    let (mut refused, mut accepted) = (0, 0);
    for (bytes, got) in malformed.iter().zip(&page_read) {
        let system = rust_decode(bytes);
        if system.is_null() {
            refused += 1;
        } else {
            accepted += 1;
        }
        assert_eq!(
            got, &system,
            "the page refuses exactly what the system refuses, and reads the rest alike: {bytes:?}"
        );
    }
    assert!(
        refused > 100 && accepted > 10,
        "the malformed rows exercise both answers: {refused} refused, {accepted} read"
    );
    no_failures(&p);
}

const CHAT: &str = r#"## Page /

State:
    Let heard be 0.
    Let last be "".
Actions:
    When a message arrives on "chat" (m: Text):
        Set heard to heard + 1.
        Set last to m.
    To send:
        Publish "hi" on "chat".
View:
    <main>
      <p id="heard">{heard}</p>
      <p id="last">{last}</p>
      <button id="send" on-click={send}>send</button>
    </main>
"#;

fn text_of(p: &Page, id: &str) -> String {
    p.snapshot().unwrap().by_id(id)[0].text_content()
}

#[test]
fn two_pages_on_one_relay_hear_each_other_once() {
    let Some(b) = browser() else { return };
    let hub = RelayHub::default();
    let a = open_on(b, page(CHAT), &hub);
    let other = open_on(b, page(CHAT), &hub);
    assert_eq!(
        hub.subscribers("chat"),
        2,
        "each page joined the topic over its own connection"
    );
    a.click("#send").unwrap();
    other.settle().unwrap();
    assert_eq!(
        (text_of(&a, "heard"), text_of(&other, "heard"), text_of(&other, "last")),
        ("1".to_string(), "1".to_string(), "hi".to_string()),
        "the publisher hears itself once, in compiled LOGOS, and the relay does not echo it back; the \
         other page hears it once, through the relay"
    );
    other.click("#send").unwrap();
    a.settle().unwrap();
    assert_eq!(
        (text_of(&a, "heard"), text_of(&other, "heard")),
        ("2".to_string(), "2".to_string())
    );
    assert_eq!(
        published_on(&hub, "chat"),
        ["hi", "hi"],
        "one frame per publish"
    );
    no_failures(&a);
    no_failures(&other);
}

#[test]
fn two_pages_publishing_at_once_both_hear_everything() {
    let Some(b) = browser() else { return };
    let hub = RelayHub::default();
    let a = open_on(b, page(CHAT), &hub);
    let other = open_on(b, page(CHAT), &hub);
    for _ in 0..3 {
        a.click("#send").unwrap();
        other.click("#send").unwrap();
    }
    a.settle().unwrap();
    other.settle().unwrap();
    assert_eq!(
        (text_of(&a, "heard"), text_of(&other, "heard")),
        ("6".to_string(), "6".to_string()),
        "every publish of either page reaches both, however the two pages' turns interleave"
    );
    no_failures(&a);
    no_failures(&other);
}

/// A page whose button a message moves: the message fills the tall paragraph above it.
const MOVED_BY_A_MESSAGE: &str = r#"## Page /

State:
    Let news be "".
    Let pressed be 0.
Actions:
    When a message arrives on "news" (m: Text):
        Set news to m.
    To press:
        Set pressed to pressed + 1.
View:
    <main>
      <p id="news" class="news">{news}</p>
      <p id="pressed">{pressed}</p>
      <button id="go" on-click={press}>go</button>
    </main>
Style:
    .news { margin: 0; font-size: 64px; line-height: 96px; }
"#;

/// The `top` and `height` of the element `sel` names, in the page's viewport.
fn rect_of(p: &Page, sel: &str) -> (f64, f64) {
    let r = p
        .eval(&format!("(() => {{ const r = document.querySelector({sel:?}).getBoundingClientRect(); return [r.top, r.height]; }})()"))
        .unwrap();
    (r[0].as_f64().unwrap(), r[1].as_f64().unwrap())
}

/// How many iterations of `occupier`'s loop the page runs in about `ms` milliseconds.
fn iterations_in(p: &Page, ms: u64) -> u64 {
    const SAMPLE: u64 = 20_000_000;
    let started = std::time::Instant::now();
    p.eval(&format!("(() => {{ let x = 0; for (let i = 0; i < {SAMPLE}; i++) x = (x * 31 + i) | 0; return x; }})()"))
        .unwrap();
    let took = started.elapsed().as_millis().max(1) as u64;
    (SAMPLE * ms / took).clamp(SAMPLE, SAMPLE * 1000)
}

/// A relay connection of the test's own on the page, joined to `busy`: each message on it tells
/// the relay it began (a `Publish` on `busy-began`, which leaves the page while the page is still
/// occupied) and then runs `iterations` of arithmetic on the page's main thread. A message's task
/// is one the paused clock does not hold, so the page is occupied from the moment the message runs
/// until the arithmetic ends, and whatever reaches the page meanwhile waits in its queue, in the
/// order it arrived.
fn occupier(p: &Page, hub: &RelayHub, iterations: u64) {
    p.eval(&format!(
        "(() => {{ const s = new WebSocket(location.origin.replace('http', 'ws') + '/_relay'); \
         s.binaryType = 'arraybuffer'; \
         s.onopen = () => s.send(__logosRelayCodec.encode({{ Subscribe: {{ topic: 'busy' }} }})); \
         s.onmessage = (ev) => {{ \
           const f = __logosRelayCodec.decode(new Uint8Array(ev.data)); \
           if (!f || !f.Event) return; \
           s.send(__logosRelayCodec.encode({{ Publish: {{ topic: 'busy-began', data: new Uint8Array() }} }})); \
           let x = 0; for (let i = 0; i < {iterations}; i++) x = (x * 31 + i) | 0; \
           window.__occupied = x; }}; \
         return 0; }})()"
    ))
    .unwrap();
    p.settle().unwrap();
    assert_eq!(
        hub.subscribers("busy"),
        1,
        "the test's own connection joined `busy`"
    );
}

/// Occupy the page through its `occupier`, and return once the page has begun.
fn occupy(hub: &RelayHub) {
    let began = || {
        hub.received()
            .iter()
            .filter(|f| matches!(f, RelayFrame::Publish { topic, .. } if topic == "busy-began"))
            .count()
    };
    let before = began();
    hub.deliver("busy", b"");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    while began() == before {
        assert!(
            std::time::Instant::now() < deadline,
            "the page never began the task that occupies it"
        );
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

#[test]
fn a_click_meets_the_page_after_what_reached_it_is_handled() {
    let Some(b) = browser() else { return };
    let hub = RelayHub::default();
    let p = open_on(b, page(MOVED_BY_A_MESSAGE), &hub);
    let (top, height) = rect_of(&p, "#go");
    occupier(&p, &hub, iterations_in(&p, 2000));
    // The page is occupied when the click begins, so the click's first look into it waits in the
    // page's queue, and the message reaches the page after that look is queued and before the page
    // is free. A click that looks once and presses aims at the button where it stood before the
    // message moved it: the message's turn runs between the look and the press.
    occupy(&hub);
    let deliverer = {
        let hub = hub.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(20));
            hub.deliver("news", b"moved");
        })
    };
    p.click("#go").unwrap();
    deliverer.join().unwrap();
    p.settle().unwrap();
    assert_eq!(text_of(&p, "news"), "moved", "the message reached the page");
    let (moved_top, _) = rect_of(&p, "#go");
    assert!(
        moved_top - top >= height,
        "the message must move the button past where a press aimed before it would land, or this proves \
         nothing: top {top} → {moved_top}, height {height}"
    );
    let journal = p.journal();
    let turn_changing = |field: &str| {
        journal
            .iter()
            .position(|e| e.changes.iter().any(|c| c.0 == field))
    };
    assert_eq!(
        text_of(&p, "pressed"),
        "1",
        "the click pressed the button where the message left it: {journal:?}"
    );
    assert!(
        matches!((turn_changing("news"), turn_changing("pressed")), (Some(m), Some(c)) if m < c),
        "the message written to the page before the click looked is handled before the click: {journal:?}"
    );
    no_failures(&p);
}

const PUBLISHES_ON_A_CLOCK: &str = r#"## Page /

State:
    Let ticks be 0.
Actions:
    Every 1 second:
        Set ticks to ticks + 1.
    To send:
        Publish "hello" on "room".
View:
    <main>
      <p id="ticks">{ticks}</p>
      <button id="go" on-click={send}>send</button>
    </main>
"#;

#[test]
fn a_relay_connection_opened_on_the_paused_clock_opens_without_moving_it() {
    let Some(b) = browser() else { return };
    let hub = RelayHub::default();
    let p = open_on(b, page(PUBLISHES_ON_A_CLOCK), &hub);
    assert!(
        hub.received().is_empty(),
        "nothing opened the relay before the click: {:?}",
        hub.received()
    );
    let before = p.eval("Date.now()").unwrap();
    p.click("#go").unwrap();
    assert_eq!(
        published_on(&hub, "room"),
        ["hello"],
        "the connection the click opened carried its frame to the relay"
    );
    assert_eq!(
        p.eval("Date.now()").unwrap(),
        before,
        "opening it moved the page's clock"
    );
    assert_eq!(text_of(&p, "ticks"), "0", "opening it ran a timer");
    p.advance(1000).unwrap();
    assert_eq!(
        text_of(&p, "ticks"),
        "1",
        "the clock runs when a test moves it, so the zero above is the clock held"
    );
    no_failures(&p);
}

/// A click arms a fuse that lands one second later, in the page's own timer table: each `Fuse` a
/// click mounts lands itself `After 1 second`.
const FUSE_ON_EACH_CLICK: &str = r#"## A Shot has:
    a number, which is Int.

## Store Shots

State:
    Let live be a new Seq of Shot.
    Let nextNumber be 1.
    Let fired be 0.

Actions:
    To fire:
        Push (a new Shot with number nextNumber) to live.
        Set nextNumber to nextNumber + 1.

    To land (number: Int):
        Let mutable kept be a new Seq of Shot.
        Repeat for s in live:
            If s's number is not equal to number:
                Push s to kept.
        Set live to kept.
        Set fired to fired + 1.

## Component Fuse (number: Int)

Use Shots.

Actions:
    After 1 second:
        Call Shots's land with number.

View:
    <i class="fuse">{number}</i>

## Page /

Use Shots.

Actions:
    To go:
        Call Shots's fire.

View:
    <main>
      <p id="fired">{Shots's fired}</p>
      <button id="go" on-click={go}>go</button>
      <div>{for each s in Shots's live, keyed by s's number:}<Fuse number={s's number}/>{end}</div>
    </main>
"#;

/// A page timer lands when the time it waits for has passed, whatever the page's clock reads: advancing
/// the clock by exactly a fuse's second lands it. Chrome's `performance.now()` is coarsened with jitter
/// and reads up to a tenth of a millisecond either side of the time the browser has run its timers to, so
/// a page that counted whole milliseconds off it could wake for its fuse a millisecond short by its own
/// count and wait another, past the end of the advance — about one page in sixty, by the draw of its
/// clock's first reading. Here the reading is held short on purpose, and further short at the wake than
/// at the click, which is that case every time.
#[test]
fn a_page_timer_lands_when_its_time_has_passed() {
    let Some(b) = browser() else { return };
    let hub = RelayHub::default();
    let p = open_on(b, page(FUSE_ON_EACH_CLICK), &hub);
    p.eval(
        "{ const read = performance.now.bind(performance); let reads = 0; \
         performance.now = () => read() - (reads++ === 0 ? 0.5 : 1.9); } 1",
    )
    .unwrap();
    p.click("#go").unwrap();
    p.advance(1000).unwrap();
    assert_eq!(
        text_of(&p, "fired"),
        "1",
        "the fuse the click armed lands when its second has passed, read short or not"
    );
    no_failures(&p);
}

/// A page with nothing on its own clock: the timers in the test below are the test's own.
const QUIET_PAGE: &str = "## Page /\n\nState:\n    Let n be 0.\nView:\n    <main><p id=\"n\">{n}</p></main>\n";

/// Every task that falls due by the end of a clock advance has run when the advance returns. Chrome
/// reports the budget spent as its clock reaches the end and pauses the clock there, fencing the
/// page's timer queues, so work that falls due at that instant can land behind the fence and wait for
/// the next advance: a follow-up a timer queues at the deadline is held there every time, and a timer
/// due at the deadline now and then. Each round names its own tasks, so a task held from one round
/// cannot pass for the next round's. A zero advance runs what is due now and returns.
#[test]
fn every_task_due_by_the_end_of_an_advance_has_run_when_it_returns() {
    let Some(b) = browser() else { return };
    let hub = RelayHub::default();
    let p = open_on(b, page(QUIET_PAGE), &hub);
    p.eval("window.__now = 0; setTimeout(() => { window.__now += 1; }, 0); 1")
        .unwrap();
    p.advance(0).unwrap();
    assert_eq!(
        p.eval("window.__now").unwrap(),
        serde_json::json!(1),
        "a zero advance runs the timer due now, which the paused clock held"
    );
    let rounds = 2000;
    let mut left = Vec::new();
    for round in 0..rounds {
        p.eval(&format!(
            "setTimeout(() => {{ window.__due = {round}; \
             setTimeout(() => {{ window.__after = {round}; }}, 0); }}, 7); 1"
        ))
        .unwrap();
        p.advance(7).unwrap();
        let ran = p.eval("[window.__due, window.__after]").unwrap();
        if ran != serde_json::json!([round, round]) {
            left.push(format!("round {round}: {ran}"));
        }
    }
    assert!(
        left.is_empty(),
        "{} of {rounds} advances returned with a task due by their end not run (`[timer, its follow-up]`):\n{}",
        left.len(),
        left.join("\n")
    );
    no_failures(&p);
}

const JOURNALLED: &str = r#"## Component App
State:
    Let n be 0.
    Let label be "zero".
    Let why be "".
Actions:
    To bump:
        Set n to n + 1.
        Set label to "more".
    To half:
        Set n to 5.
        Set n to item 9 of [1].
    When an action fails (e: Text):
        Set why to e.
View:
    <p id="n">{n} {label} {why}</p>
    <button id="go" on-click={bump}>go</button>
    <button id="bad" on-click={half}>bad</button>

## Main
Mount App.
"#;

type Turn = (
    String,
    String,
    Vec<(String, String, String)>,
    Option<String>,
);

fn turns(journal: Vec<dom_journal::JournalEntry>) -> Vec<Turn> {
    journal
        .into_iter()
        .map(|e| (e.instance, e.action, e.changes, e.failed))
        .collect()
}

#[test]
fn chrome_journals_every_turn_as_the_reference_does() {
    let Some(b) = browser() else { return };
    dom::reset();
    logicaffeine_compile::semantics::world::reset();
    let out = logicaffeine_compile::compile::tw_outcome(&format!(
        "{JOURNALLED}Click \"#go\".\nClick \"#bad\".\n"
    ));
    assert_eq!(out.error, None, "the reference runs the page");
    let expected = turns(dom_journal::journal());
    assert!(
        expected.iter().any(|t| !t.2.is_empty()) && expected.iter().any(|t| t.3.is_some()),
        "the oracle must journal a change and a failure, or this proves nothing: {expected:?}"
    );
    let p = open(b, page(JOURNALLED));
    p.click("#go").unwrap();
    p.click("#bad").unwrap();
    assert_eq!(
        turns(p.journal()),
        expected,
        "every turn Chrome ran is journalled with the instance, the changed fields and the failure the reference journals"
    );
}

#[test]
fn a_finished_fetch_leaves_no_host_state_behind() {
    let Some(b) = browser() else { return };
    let p = open(b, page(BIG_BODY));
    p.answer("GET", "/api/big", 200, "text/plain", b"done")
        .unwrap();
    no_failures(&p);
    let census = p.eval("__logosHostCensus()").unwrap();
    assert_eq!(
        census["fetches"].as_u64(),
        Some(0),
        "a request whose completion turn has run is finished; the host keeps nothing for it: {census}"
    );
}

const FETCH_ON_CLICK: &str = r#"## A Todo has:
    an id, which is Int.
    a text, which is Text.

## Page /

State:
    Let label be "...".
Actions:
    To load:
        Get "/api/todo" as Todo, then loaded.
    To loaded (r: Result of Todo and HttpError):
        Inspect r:
            When Ok (v):
                Set label to v's text.
            When Err (e):
                Set label to e's message.
View:
    <p id="label">{label}</p>
    <button id="go" on-click={load}>load</button>
"#;

/// A network that answers late: every response reaches the page only after forty more round trips
/// to the page's own server — real network time, which paused virtual time does not stop, and
/// nothing the page's host-call trace records while it passes.
const SLOW_RESPONSES: &str = r#"(() => {
  const fetchNow = window.fetch;
  window.fetch = (...args) => fetchNow(...args).then(async (response) => {
    for (let trip = 0; trip < 40; trip++) await (await fetchNow('/app.js')).text();
    return response;
  });
})();"#;

#[test]
fn a_statement_ends_only_after_every_answered_request_completes() {
    let Some(b) = browser() else { return };
    let p = b
        .open_with(
            &page(FETCH_ON_CLICK),
            &OpenOptions {
                init_scripts: vec![SLOW_RESPONSES.to_string()],
                ..OpenOptions::default()
            },
        )
        .unwrap_or_else(|e| panic!("the page must open: {e}"));
    p.answer(
        "GET",
        "/api/todo",
        200,
        "application/json",
        br#"{"id": 1, "text": "milk"}"#,
    )
    .unwrap();
    p.click("#go").unwrap();
    let noted: Vec<String> = p
        .import_trace()
        .unwrap()
        .into_iter()
        .filter(|c| c.kind == CallKind::Import && c.name == "dom_note_turn")
        .filter_map(|c| c.texts.into_iter().next())
        .collect();
    assert_eq!(
        noted,
        ["load", "loaded"],
        "a click whose request the test already answered ends when that answer has been delivered \
         and its completion turn has run — however long the response takes to arrive — never when \
         the page merely went quiet for a moment"
    );
    assert!(
        body(&p).contains("<p id=\"label\">milk</p>"),
        "{}",
        body(&p)
    );
    no_failures(&p);
}

const FAILURES: &str = r#"## Page /

State:
    Let xs be [1, 2].
    Let n be 0.
Actions:
    To first:
        Set n to item 99 of xs.
    To second:
        Call applyPatches with [238].
View:
    <main>
      <p id="out">{n}</p>
      <button id="a" on-click={first}>a</button>
      <button id="b" on-click={second}>b</button>
    </main>
"#;

#[test]
fn a_later_failure_reports_its_own_message_in_one_dismissible_panel() {
    let Some(b) = browser() else { return };
    let p = open(b, page(FAILURES));
    let _ = p.click("#a");
    let panels = || {
        p.eval("Array.from(document.querySelectorAll('[role=alert]')).map((e) => e.textContent)")
            .unwrap()
    };
    let first = panels();
    assert_eq!(
        first.as_array().map(Vec::len),
        Some(1),
        "one failure, one panel: {first}"
    );
    assert!(
        first[0]
            .as_str()
            .unwrap_or("")
            .contains("Index 99 out of bounds"),
        "it says why: {first}"
    );

    let _ = p.click("#b");
    let second = panels();
    assert_eq!(
        second.as_array().map(Vec::len),
        Some(1),
        "a second failure replaces the panel rather than stacking another over the page: {second}"
    );
    let text = second[0].as_str().unwrap_or("");
    assert!(
        text.contains("unknown patch opcode") && !text.contains("Index 99"),
        "and it reports ITS OWN cause — the message captured for the first failure belongs to \
         the first failure: {text}"
    );

    p.click("[role=alert] button").unwrap();
    assert_eq!(
        panels().as_array().map(Vec::len),
        Some(0),
        "the panel can be dismissed"
    );
}

// ---- `page_host::RemotePage` — the same `## Test` driver `largo test --browser` uses. ----
//
// Everything above drives `Page` directly, statement by statement, by hand. The tests below go
// through `page_host::compare_tests` exactly as `largo test --browser` and the tree-walker/VM/
// Chrome corpus lock do: the SAME `## Test` body walked on the reference tier, the lowered VM, and
// a real page in this binary's shared Chrome, page-tier by page-tier, axis by axis.

#[path = "support/rust_literals.rs"]
mod rust_literals;
use rust_literals::read_stable;

/// One divergence this lock has already found and named, read from the ledger every
/// `browser:<axis>` row of `lgx_known_divergences.txt` — shared with `lowered_vm_tier_lock`, which
/// owns every OTHER row. Parsing and the `(scenario, axis, statement) -> cause` map are
/// `page_host`'s own (`parse_ledger`/`ledger_map`), so the two locks read one file one way.
fn known_divergences() -> std::collections::BTreeMap<(String, String, usize), String> {
    let path =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/lgx_known_divergences.txt");
    let rows = logicaffeine_compile::page_host::parse_ledger(&read_stable(&path));
    logicaffeine_compile::page_host::ledger_map(
        &rows,
        logicaffeine_compile::page_host::LedgerTier::Browser,
    )
}

/// What running `source`'s page scenario(s) on the reference tier, the lowered VM, and a real
/// Chrome found, on the BROWSER candidate only (`LoweredVm`'s own agreement is
/// `lowered_vm_tier_lock`'s job; its ledger rows already cover it — this lock only ever names a
/// browser-only key, so the two never fight over the same row).
#[derive(Default)]
struct TierScan {
    /// Every axis divergence NOT already in the shared ledger, formatted to report.
    unlisted: Vec<String>,
    /// Every browser-tier key this run actually reached, keyed as the ledger keys it — feeds
    /// [`logicaffeine_compile::page_host::stale_rows`].
    found: std::collections::BTreeMap<(String, String, usize), String>,
    /// This page's own id, if its source did not parse at all.
    unparsed_page: Option<String>,
    /// Scenarios (`id::test`) refused at statement 0 — before any axis was ever compared.
    refused_at_zero: std::collections::BTreeSet<String>,
}

/// The launcher for the page `id` names (a path from the repository root): as its project serves
/// it, with the project's `static/` beside it, when the page lives in one.
fn launcher_for(id: &str) -> browser::ChromeLauncher {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let statics = root
        .join(id)
        .parent()
        .and_then(|pages| pages.parent())
        .map(|project| project.join("static"));
    match statics {
        Some(dir) if dir.is_dir() => browser::ChromeLauncher::for_project(&dir),
        _ => browser::ChromeLauncher::default(),
    }
}

fn tier_divergences(
    id: &str,
    source: &str,
    known: &std::collections::BTreeMap<(String, String, usize), String>,
) -> TierScan {
    let Some(_) = browser() else {
        return TierScan::default();
    };
    let launcher = launcher_for(id);
    let results = match logicaffeine_compile::page_host::compare_tests(
        source,
        &[
            logicaffeine_compile::page_host::PageTierKind::LoweredVm,
            logicaffeine_compile::page_host::PageTierKind::Browser,
        ],
        0,
        50_000_000,
        Some(&launcher),
    ) {
        Ok(r) => r,
        Err(e) => {
            return TierScan {
                unlisted: vec![format!(
                    "{id}: does not parse: {}",
                    e.lines().next().unwrap_or("")
                )],
                unparsed_page: Some(id.to_string()),
                ..Default::default()
            }
        }
    };
    let mut scan = TierScan::default();
    for comparison in &results {
        let scenario = format!("{id}::{}", comparison.test);
        for (kind, run) in &comparison.candidates {
            if *kind != logicaffeine_compile::page_host::PageTierKind::Browser {
                continue;
            }
            match run {
                logicaffeine_compile::page_host::TierRun::Traced(t) => {
                    for a in comparison.reference.compare(t).axes {
                        let key = (scenario.clone(), a.axis.name().to_string(), a.statement);
                        let detail = logicaffeine_compile::page_host::describe_difference(
                            &a.reference,
                            &a.candidate,
                        );
                        if !known.contains_key(&key) {
                            scan.unlisted.push(format!(
                                "{}::browser:{} @ {} | <cause>\n{detail}",
                                scenario, key.1, key.2
                            ));
                        }
                        scan.found.insert(key, detail);
                    }
                }
                logicaffeine_compile::page_host::TierRun::Refused { statement, reason } => {
                    let key = (scenario.clone(), "refused".to_string(), *statement);
                    if *statement == 0 {
                        scan.refused_at_zero.insert(scenario.clone());
                    }
                    if !known.contains_key(&key) {
                        scan.unlisted.push(format!(
                            "{scenario}::browser:refused @ {statement} | <cause>\n{reason}"
                        ));
                    }
                    scan.found.insert(key, reason.clone());
                }
            }
        }
    }
    scan
}

// ---- Arrival: every tier visits the page where a visitor would, with the scene already set ----

/// The ONE `## Test` in `source`, run through `page_host::compare_tests` on the reference tier, the
/// lowered VM and Chrome: the three traces, in that order.
fn three_tiers(
    source: &str,
) -> Option<(
    logicaffeine_compile::page_host::PageTrace,
    logicaffeine_compile::page_host::PageTrace,
    logicaffeine_compile::page_host::PageTrace,
)> {
    use logicaffeine_compile::page_host::{compare_tests, PageTierKind, TierRun};
    let Some(_) = browser() else { return None };
    let mut all = compare_tests(
        source,
        &[PageTierKind::LoweredVm, PageTierKind::Browser],
        0,
        50_000_000,
        Some(&browser::ChromeLauncher::default()),
    )
    .unwrap_or_else(|e| panic!("the page must parse: {e}\n{source}"));
    assert_eq!(all.len(), 1, "exactly one page scenario:\n{source}");
    let c = all.remove(0);
    let traced = |kind: PageTierKind| match c.candidates.iter().find(|(k, _)| *k == kind) {
        Some((_, TierRun::Traced(t))) => t.clone(),
        other => panic!(
            "the {} tier must trace the scenario, not {other:?}",
            kind.label()
        ),
    };
    Some((
        c.reference.clone(),
        traced(PageTierKind::LoweredVm),
        traced(PageTierKind::Browser),
    ))
}

/// Where `candidate` disagrees with `reference` on `axes`, rendered.
fn disagreements(
    reference: &logicaffeine_compile::page_host::PageTrace,
    candidate: &logicaffeine_compile::page_host::PageTrace,
    axes: &[logicaffeine_compile::page_host::Axis],
) -> String {
    let d = reference.compare(candidate);
    let kept: Vec<_> = d
        .axes
        .into_iter()
        .filter(|a| axes.contains(&a.axis))
        .collect();
    logicaffeine_compile::page_host::Divergence { axes: kept }.render()
}

const AT_ABOUT: &str = r##"## Page /about

State:
    Let visits be 0.
Actions:
    To visit:
        Set visits to visits + 1.
View:
    <p id="where">{routePath()}</p>
    <button id="go" on-click={visit}>{visits}</button>

## Test: the page reads the URL it was visited at
    Expect that the text of "#where" is equal to "/about".
"##;

#[test]
fn a_page_is_visited_at_its_own_route() {
    use logicaffeine_compile::page_host::Axis;
    let Some((reference, lowered, chrome)) = three_tiers(AT_ABOUT) else {
        return;
    };
    for (tier, trace) in [
        ("the reference", &reference),
        ("the lowered VM", &lowered),
        ("Chrome", &chrome),
    ] {
        let arrived = &trace.steps[0].snapshot;
        assert_eq!(
            arrived.url, "/about",
            "{tier} must visit a `## Page /about` at /about"
        );
        assert!(
            arrived.document.contains("<p id=\"where\">/about</p>"),
            "{tier}: `routePath()` reads the URL the page was visited at:\n{}",
            arrived.document
        );
    }
    let off = disagreements(&reference, &chrome, &[Axis::Url, Axis::Document]);
    assert!(
        off.is_empty(),
        "Chrome must arrive where the reference does:\n{off}"
    );
}

const ONLY_THE_ROUTE: &str = r##"## Page /

View:
    <p id="where">{routePath()}</p>
"##;

#[test]
fn a_page_whose_only_text_comes_from_its_host_starts() {
    let Some(b) = browser() else { return };
    let p = b.open(&page(ONLY_THE_ROUTE)).unwrap_or_else(|e| {
        panic!(
            "a page that shows nothing but `routePath()` has no Text of its own, yet the host hands \
             it one — the module must carry the allocator the host writes that Text through: {e}"
        )
    });
    no_failures(&p);
    assert_eq!(body(&p), reference(ONLY_THE_ROUTE, &[]));
}

const MOUNTED_LATER: &str = r##"## Component Tally
State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
View:
    <p id="n">{n}</p>
    <button id="go" on-click={bump}>go</button>

## Test: mounted at a statement of its own
    Mount Tally.
    Click "#go".
    Expect that the text of "#n" is equal to "1".
"##;

#[test]
fn an_inline_mount_mounts_at_its_own_statement() {
    use logicaffeine_compile::page_host::Axis;
    let Some((reference, lowered, chrome)) = three_tiers(MOUNTED_LATER) else {
        return;
    };
    for (tier, trace) in [
        ("the reference", &reference),
        ("the lowered VM", &lowered),
        ("Chrome", &chrome),
    ] {
        assert_eq!(
            trace.steps[0].snapshot.document, "<body></body>",
            "{tier}: before the test's own `Mount Tally.` the page holds nothing — a component a \
             test mounts is mounted at that statement, never baked into the page's load"
        );
        assert!(
            trace.steps[1]
                .snapshot
                .document
                .contains("<p id=\"n\">0</p>"),
            "{tier}: `Mount Tally.` mounts it: {}",
            trace.steps[1].snapshot.document
        );
        assert!(
            trace.steps[2]
                .snapshot
                .document
                .contains("<p id=\"n\">1</p>"),
            "{tier}: and it counts"
        );
    }
    let off = disagreements(
        &reference,
        &chrome,
        &[Axis::Document, Axis::Journal, Axis::Head, Axis::Title],
    );
    assert!(
        off.is_empty(),
        "Chrome mounts where the reference mounts:\n{off}"
    );
}

const LOADS_ON_ARRIVAL: &str = r##"## A Todo has:
    an id, which is Int.
    a text, which is Text.

## Page /

State:
    Let label be "...".
Actions:
    When the page loads:
        Get "/api/todo" as Todo, then loaded.
    To loaded (r: Result of Todo and HttpError):
        Inspect r:
            When Ok (v):
                Set label to v's text.
            When Err (e):
                Set label to e's message.
View:
    <p id="label">{label}</p>

## Test: the page shows what the server said when it loaded
    Given that GET "/api/todo" answers 200 with "{{\"id\": 1, \"text\": \"milk\"}}".
    Expect that the text of "#label" is equal to "milk".
"##;

#[test]
fn a_pages_scene_is_set_before_it_arrives() {
    use logicaffeine_compile::page_host::Axis;
    let Some((reference, lowered, chrome)) = three_tiers(LOADS_ON_ARRIVAL) else {
        return;
    };
    for (tier, trace) in [
        ("the reference", &reference),
        ("the lowered VM", &lowered),
        ("Chrome", &chrome),
    ] {
        let arrived = &trace.steps[0].snapshot;
        assert!(
            arrived.document.contains("<p id=\"label\">milk</p>"),
            "{tier}: a `## Page` arrives with its `Given that …` answers already in place, so the \
             request it makes on load is answered before anything is observed:\n{}",
            arrived.document
        );
        assert_eq!(
            arrived.journal,
            ["loaded"],
            "{tier}: the completion turn ran on arrival"
        );
    }
    let off = disagreements(
        &reference,
        &chrome,
        &[Axis::Document, Axis::Fetches, Axis::Journal],
    );
    assert!(
        off.is_empty(),
        "every tier arrives at the same page:\n{off}"
    );
}

const SAVES: &str = r##"## Page /

State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
        Save n as "count".
View:
    <button id="go" on-click={bump}>{n}</button>

## Test: a click saves the count
    Click "#go".
    Expect that the text of "#go" is equal to "1".
"##;

#[test]
fn storage_written_by_a_page_is_observed_in_chrome() {
    use logicaffeine_compile::page_host::Axis;
    let Some((reference, lowered, chrome)) = three_tiers(SAVES) else {
        return;
    };
    let written = &reference.steps[1].snapshot.storage;
    assert!(
        written
            .iter()
            .any(|(area, key, _)| area == "local" && key == "count"),
        "the oracle must see the write, or this proves nothing: {written:?}"
    );
    assert_eq!(
        &lowered.steps[1].snapshot.storage, written,
        "the lowered VM stores what the reference stores"
    );
    assert_eq!(
        &chrome.steps[1].snapshot.storage, written,
        "Chrome's own localStorage holds exactly what the reference's storage holds, key for key and \
         byte for byte"
    );
    let off = disagreements(&reference, &chrome, &[Axis::Storage]);
    assert!(off.is_empty(), "{off}");
}

const PUBLISHES: &str = r##"## Page /

Actions:
    To send:
        Publish "hello" on "room".
View:
    <button id="go" on-click={send}>send</button>

## Test: a click publishes
    Click "#go".
"##;

#[test]
fn a_publish_is_observed_in_chrome() {
    let Some((reference, lowered, chrome)) = three_tiers(PUBLISHES) else {
        return;
    };
    let sent = vec![("room".to_string(), "hello".to_string())];
    assert_eq!(
        reference.steps[1].snapshot.published, sent,
        "the oracle records the frame"
    );
    assert_eq!(
        lowered.steps[1].snapshot.published, sent,
        "the lowered VM hands the transport the same frame"
    );
    assert_eq!(
        chrome.steps[1].snapshot.published, sent,
        "the frame a real page hands its relay is the frame the reference recorded"
    );
}

const HEADED: &str = r##"## Page /

Head:
    <title>Hello, head</title>
    <meta name="description" content="A page with a head.">
Actions:
    To rename:
        Set the title to "Renamed".
View:
    <button id="go" on-click={rename}>rename</button>

## Test: the head's title is the document's title until the page renames it
    Click "#go".
"##;

#[test]
fn a_head_title_is_the_document_title_on_every_tier() {
    use logicaffeine_compile::page_host::Axis;
    let Some((reference, lowered, chrome)) = three_tiers(HEADED) else {
        return;
    };
    for (tier, trace) in [
        ("the reference", &reference),
        ("the lowered VM", &lowered),
        ("Chrome", &chrome),
    ] {
        assert_eq!(
            trace.steps[0].snapshot.title, "Hello, head",
            "{tier}: `document.title` is the head's first `<title>`, whoever put it there"
        );
        assert_eq!(
            trace.steps[1].snapshot.title, "Renamed",
            "{tier}: `Set the title to` renames it"
        );
    }
    for (tier, candidate) in [("the lowered VM", &lowered), ("Chrome", &chrome)] {
        let off = disagreements(&reference, candidate, &[Axis::Head, Axis::Title]);
        assert!(
            off.is_empty(),
            "{tier} must carry the reference's head, title included:\n{off}"
        );
    }
}

const BLURS: &str = r##"## Page /

State:
    Let draft be "".
    Let blurs be 0.
    Let taps be 0.
Actions:
    To left:
        Set blurs to blurs + 1.
    To tap:
        Set taps to taps + 1.
View:
    <input id="field" bind={draft} on-blur={left}>
    <button id="b" on-click={tap}>b</button>
    <p id="n">{blurs}</p>

## Test: clicking away from a field blurs it
    Type "x" into "#field".
    Click "#b".
    Expect that the text of "#n" is equal to "1".
"##;

#[test]
fn a_click_blurs_the_field_that_had_focus_on_every_tier() {
    use logicaffeine_compile::page_host::Axis;
    let Some((reference, lowered, chrome)) = three_tiers(BLURS) else {
        return;
    };
    for (tier, trace) in [
        ("the reference", &reference),
        ("the lowered VM", &lowered),
        ("Chrome", &chrome),
    ] {
        let after = &trace.steps[2].snapshot;
        assert!(
            after.document.contains("<p id=\"n\">1</p>"),
            "{tier}: typing focused the field and the click's press moved focus to the button, so \
             the field's `on-blur` ran its turn — a browser runs it, so every tier must:\n{}",
            after.document
        );
        assert_eq!(
            after.focus.as_deref(),
            Some("button#b"),
            "{tier}: the pressed button has focus"
        );
    }
    for (tier, candidate) in [("the lowered VM", &lowered), ("Chrome", &chrome)] {
        let off = disagreements(
            &reference,
            candidate,
            &[Axis::Focus, Axis::Document, Axis::Journal],
        );
        assert!(off.is_empty(), "{tier}:\n{off}");
    }
}

const EVERY_VERB: &str = r##"## Page /

State:
    Let draft be "".
    Let blurs be 0.
    Let focuses be 0.
    Let hits be 0.
Actions:
    To left:
        Set blurs to blurs + 1.
    To arrived:
        Set focuses to focuses + 1.
    To hit:
        Set hits to hits + 1.
    To send:
        Set hits to hits + 10.
View:
    <input id="field" bind={draft} on-blur={left} on-focus={arrived} on-keydown.enter={hit}>
    <button id="b" on-click={hit}>b</button>
    <p id="plain" on-click={hit}>plain</p>
    <select id="s" on-change={hit}><option value="a">a</option><option value="b">b</option></select>
    <form id="f" on-submit.prevent={send}><button id="go" type="submit">go</button></form>
    <p id="counts">{blurs} {focuses} {hits}</p>

## Test: focus goes where a visitor's input sends it
    Type "x" into "#field".
    Click "#b".
    Type "y" into "#field".
    Click "#plain".
    Choose "b" in "#s".
    Press "Enter" in "#field".
    Submit "#f".
"##;

#[test]
fn focus_follows_every_driving_verb_as_in_a_browser() {
    use logicaffeine_compile::page_host::Axis;
    let Some((reference, lowered, chrome)) = three_tiers(EVERY_VERB) else {
        return;
    };
    let focus: Vec<Option<String>> = chrome
        .steps
        .iter()
        .map(|s| s.snapshot.focus.clone())
        .collect();
    assert_eq!(
        focus,
        [
            None,
            Some("input#field".to_string()),
            Some("button#b".to_string()),
            Some("input#field".to_string()),
            None,
            Some("select#s".to_string()),
            Some("input#field".to_string()),
            Some("input#field".to_string()),
        ],
        "Chrome is the oracle for where each verb leaves focus: typing and choosing focus the \
         control, a press focuses what was pressed (nothing, for plain text), a key pressed in an \
         element focuses it, and submitting moves nothing"
    );
    for (tier, candidate) in [("the lowered VM", &lowered), ("Chrome", &chrome)] {
        let off = disagreements(
            &reference,
            candidate,
            &[Axis::Focus, Axis::Document, Axis::Journal],
        );
        assert!(
            off.is_empty(),
            "{tier} moves focus, and raises blur and focus, where the reference does:\n{off}"
        );
    }
}

const DROP_ZONE: &str = r##"## Page /

State:
    Let last be "".
    Let drops be 0.
Actions:
    When a file is dropped on "#zone" (name: Text):
        Set drops to drops + 1.
        Set last to name.
View:
    <div id="zone">drop here</div>
    <p id="last">{drops}: {last}</p>

## Test: a dropped file reaches the page with its name
    Raise "filedropped" on "#zone" with "photo.png".
    Expect that the text of "#last" is equal to "1: photo.png".
"##;

#[test]
fn a_file_dropped_in_chrome_is_the_file_the_reference_drops() {
    use logicaffeine_compile::page_host::Axis;
    let Some((reference, _lowered, chrome)) = three_tiers(DROP_ZONE) else {
        return;
    };
    assert!(
        chrome.steps[1].snapshot.document.contains("<p id=\"last\">1: photo.png</p>"),
        "a raised `filedropped` is a real drop in a browser — the file in the drop's transfer, read \
         by the page's own drop listener:\n{}",
        chrome.steps[1].snapshot.document
    );
    let off = disagreements(&reference, &chrome, &[Axis::Document, Axis::Journal]);
    assert!(off.is_empty(), "{off}");
}

const DROPPED_CONTENT: &str = r##"## Page /

State:
    Let fname be "none".
    Let fsize be 0.
    Let ftype be "".
    Let fcount be 0.
    Let b1 be 0.
    Let b2 be 0.
Actions:
    When a file is dropped on "#zone" (file: File):
        Let content be file's bytes.
        Set fname to file's name.
        Set fsize to file's size.
        Set ftype to file's type.
        Set fcount to length of content.
        Set b1 to item 1 of content.
        Set b2 to item 2 of content.
View:
    <div id="zone">drop here</div>
    <p id="file">{fname} {fsize} {ftype}</p>
    <p id="bytes">{fcount}: {b1} {b2}</p>

## Test: a dropped file arrives with its name, size, type and content
    Raise "filedropped" on "#zone" with a new File with name "photo.png" and type "image/png" and bytes [137, 80, 78, 71].
"##;

/// The document fragment `DROPPED_CONTENT`/`TWO_DROPS` show once a file of `name`, `mime` and
/// `bytes` has been dropped.
fn shows_file(name: &str, mime: &str, bytes: &[u8]) -> String {
    format!(
        "<p id=\"file\">{name} {} {mime}</p><p id=\"bytes\">{}: {} {}</p>",
        bytes.len(),
        bytes.len(),
        bytes[0],
        bytes[1]
    )
}

#[test]
fn a_dropped_file_arrives_with_its_declared_size_type_and_content_on_every_tier() {
    use logicaffeine_compile::page_host::Axis;
    let Some((reference, lowered, chrome)) = three_tiers(DROPPED_CONTENT) else {
        return;
    };
    let dropped = shows_file("photo.png", "image/png", &[137, 80, 78, 71]);
    for (tier, trace) in [
        ("the reference", &reference),
        ("the lowered VM", &lowered),
        ("Chrome", &chrome),
    ] {
        let after = &trace.steps[1].snapshot.document;
        assert!(
            after.contains(&dropped),
            "{tier}: a `(file: File)` handler reads the dropped file itself — its name, its size (the \
             count of its bytes, never a literal), the type it was declared with, and its content — \
             where Chrome reads a real file dropped by a real drag:\n{after}"
        );
    }
    for (tier, candidate) in [("the lowered VM", &lowered), ("Chrome", &chrome)] {
        let off = disagreements(&reference, candidate, &[Axis::Document, Axis::Journal]);
        assert!(off.is_empty(), "{tier}:\n{off}");
    }
}

const TWO_DROPS: &str = r##"## Page /

State:
    Let fname be "none".
    Let fsize be 0.
    Let ftype be "".
    Let fcount be 0.
    Let b1 be 0.
    Let b2 be 0.
Actions:
    When a file is dropped on "#zone" (file: File):
        Let content be file's bytes.
        Set fname to file's name.
        Set fsize to file's size.
        Set ftype to file's type.
        Set fcount to length of content.
        Set b1 to item 1 of content.
        Set b2 to item 2 of content.
View:
    <div id="zone">drop here</div>
    <p id="file">{fname} {fsize} {ftype}</p>
    <p id="bytes">{fcount}: {b1} {b2}</p>

## Test: each drop carries its own file
    Raise "filedropped" on "#zone" with a new File with name "hi.txt" and type "text/plain" and bytes [104, 105].
    Raise "filedropped" on "#zone" with a new File with name "photo.png" and type "image/png" and bytes [137, 80, 78, 71].
"##;

#[test]
fn each_drop_delivers_its_own_file_on_every_tier() {
    use logicaffeine_compile::page_host::Axis;
    let Some((reference, lowered, chrome)) = three_tiers(TWO_DROPS) else {
        return;
    };
    let first = shows_file("hi.txt", "text/plain", &[104, 105]);
    let second = shows_file("photo.png", "image/png", &[137, 80, 78, 71]);
    for (tier, trace) in [
        ("the reference", &reference),
        ("the lowered VM", &lowered),
        ("Chrome", &chrome),
    ] {
        let (one, two) = (
            &trace.steps[1].snapshot.document,
            &trace.steps[2].snapshot.document,
        );
        assert!(
            one.contains(&first),
            "{tier}: the first drop delivers the first file, not whichever file the test declares \
             last:\n{one}"
        );
        assert!(
            two.contains(&second),
            "{tier}: the second drop delivers the second file:\n{two}"
        );
    }
    for (tier, candidate) in [("the lowered VM", &lowered), ("Chrome", &chrome)] {
        let off = disagreements(&reference, candidate, &[Axis::Document, Axis::Journal]);
        assert!(off.is_empty(), "{tier}:\n{off}");
    }
}

// ---- Propagation: an event travels the page as Chrome sends it ----
//
// The in-process tiers deliver an event the way a browser does: capture-phase listeners from the
// root down, the target, then bubbling listeners back up. Chrome is the oracle.

/// The ONE `## Test` in `source`, on the lowered page hosted by the tree-walker, the lowered page on
/// the VM, and Chrome: the three traces, in that order.
fn lowered_tiers(
    source: &str,
) -> Option<(
    logicaffeine_compile::page_host::PageTrace,
    logicaffeine_compile::page_host::PageTrace,
    logicaffeine_compile::page_host::PageTrace,
)> {
    use logicaffeine_compile::page_host::{compare_tests, PageTierKind, TierRun};
    let Some(_) = browser() else { return None };
    let mut all = compare_tests(
        source,
        &[
            PageTierKind::LoweredHere,
            PageTierKind::LoweredVm,
            PageTierKind::Browser,
        ],
        0,
        50_000_000,
        Some(&browser::ChromeLauncher::default()),
    )
    .unwrap_or_else(|e| panic!("the page must parse: {e}\n{source}"));
    assert_eq!(all.len(), 1, "exactly one page scenario:\n{source}");
    let c = all.remove(0);
    let traced = |kind: PageTierKind| match c.candidates.iter().find(|(k, _)| *k == kind) {
        Some((_, TierRun::Traced(t))) => t.clone(),
        other => panic!(
            "the {} tier must trace the scenario, not {other:?}",
            kind.label()
        ),
    };
    Some((
        traced(PageTierKind::LoweredHere),
        traced(PageTierKind::LoweredVm),
        traced(PageTierKind::Browser),
    ))
}

const NESTED_HANDLERS: &str = r##"## Page /

State:
    Let log be "".
Actions:
    To inner:
        Set log to "{log}i".
    To outer:
        Set log to "{log}o".
View:
    <div id="card" on-click={outer}><button id="b" on-click={inner}><span id="label">press</span></button><em id="note">note</em></div>
    <p id="log">{log}</p>

## Test: a click travels from its target up through every listener above it
    Click "#label".
    Click "#b".
    Click "#note".
"##;

#[test]
fn a_click_bubbles_from_its_target_through_every_listener_above_it_on_every_tier() {
    use logicaffeine_compile::page_host::Axis;
    let Some((here, vm, chrome)) = lowered_tiers(NESTED_HANDLERS) else {
        return;
    };
    let logs: Vec<&str> = ["io", "ioio", "ioioo"].to_vec();
    for (tier, trace) in [
        ("the lowered tree-walker", &here),
        ("the lowered VM", &vm),
        ("Chrome", &chrome),
    ] {
        assert!(
            trace.stopped.is_none(),
            "{tier}: every click lands somewhere that answers: {:?}",
            trace.stopped
        );
        for (step, log) in logs.iter().enumerate() {
            let doc = &trace.steps[step + 1].snapshot.document;
            assert!(
                doc.contains(&format!("<p id=\"log\">{log}</p>")),
                "{tier}, after click {}: a click on an element with no listener of its own is heard \
                 by the listeners of its ancestors, the nearest first — the button's `inner`, then \
                 the card's `outer` — as a browser bubbles it:\n{doc}",
                step + 1
            );
        }
    }
    for (tier, candidate) in [("the lowered tree-walker", &here), ("the lowered VM", &vm)] {
        let off = disagreements(
            &chrome,
            candidate,
            &[Axis::Document, Axis::Journal, Axis::Url],
        );
        assert!(
            off.is_empty(),
            "{tier} must propagate a click as Chrome does:\n{off}"
        );
    }
}

const LINK_WITH_A_LABEL: &str = r##"## Page /

State:
    Let taps be 0.
Actions:
    To tap:
        Set taps to taps + 1.
View:
    <a id="go" href="/elsewhere" on-click.prevent={tap}><span id="label">go</span></a>
    <p id="taps">{taps}</p>

## Test: a click inside a handled link runs the handler and stays on the page
    Click "#label".
"##;

#[test]
fn a_click_inside_a_link_is_the_links_click_on_every_tier() {
    use logicaffeine_compile::page_host::Axis;
    let Some((here, vm, chrome)) = lowered_tiers(LINK_WITH_A_LABEL) else {
        return;
    };
    for (tier, trace) in [
        ("the lowered tree-walker", &here),
        ("the lowered VM", &vm),
        ("Chrome", &chrome),
    ] {
        assert!(
            trace.stopped.is_none(),
            "{tier}: the link's listener answers a click on its label: {:?}",
            trace.stopped
        );
        let after = &trace.steps[1].snapshot;
        assert!(
            after.document.contains("<p id=\"taps\">1</p>"),
            "{tier}: the click on the label bubbles to the link's `on-click`:\n{}",
            after.document
        );
        assert_eq!(
            after.url, "/",
            "{tier}: the link's `.prevent` listener was on the click's path, so the link is not followed"
        );
    }
    for (tier, candidate) in [("the lowered tree-walker", &here), ("the lowered VM", &vm)] {
        let off = disagreements(
            &chrome,
            candidate,
            &[Axis::Document, Axis::Url, Axis::Journal],
        );
        assert!(off.is_empty(), "{tier}:\n{off}");
    }
}

const EVERY_CONTROL: &str = r##"## Page /

State:
    Let draft be "".
Actions:
    To keep:
        Set draft to draft.
View:
    <div id="outer">
      <button id="b">b</button>
      <input id="field" bind={draft}>
      <select id="s"><option value="a">a</option><option value="b">b</option></select>
      <form id="f" on-submit.prevent={keep}><button id="go" type="submit">go</button></form>
    </div>
"##;

/// Records, at the document, every event dispatched below it — seen on its way down, in the
/// capture phase — and every one that came back up in the BUBBLING phase. An event that does not
/// bubble is seen going down and never coming back.
const RECORD_BUBBLING: &str = r#"window.__captured = []; window.__bubbled = [];
for (const ev of ['click', 'submit', 'keydown', 'keyup', 'input', 'change', 'mouseover', 'focus', 'blur']) {
  document.addEventListener(ev, (e) => { if (e.eventPhase === Event.CAPTURING_PHASE) window.__captured.push(ev); }, true);
  document.addEventListener(ev, (e) => { if (e.eventPhase === Event.BUBBLING_PHASE) window.__bubbled.push(ev); });
}"#;

#[test]
fn every_event_a_driving_verb_raises_bubbles_in_process_exactly_when_it_bubbles_in_chrome() {
    use logicaffeine_compile::semantics::drive::Verb;
    let Some(b) = browser() else { return };
    let p = b
        .open_with(
            &page(EVERY_CONTROL),
            &OpenOptions {
                init_scripts: vec![RECORD_BUBBLING.to_string()],
                ..OpenOptions::default()
            },
        )
        .unwrap_or_else(|e| panic!("{e}"));
    let bubbled = |act: &dyn Fn(&Page)| -> (Vec<String>, Vec<String>) {
        p.eval("window.__captured = []; window.__bubbled = []")
            .unwrap();
        act(&p);
        let read = |name: &str| serde_json::from_value(p.eval(name).unwrap()).unwrap();
        (read("window.__captured"), read("window.__bubbled"))
    };
    let mut wrong = Vec::new();
    let mut check = |what: &str, events: &[&str], (captured, seen): (Vec<String>, Vec<String>)| {
        for event in events {
            if !captured.iter().any(|s| s == event) {
                wrong.push(format!(
                    "{what} never raised `{event}` in Chrome, so this proves nothing about it"
                ));
                continue;
            }
            let chrome = seen.iter().any(|s| s == event);
            if dom::event_bubbles(event) != chrome {
                wrong.push(format!(
                    "{what} raises `{event}`: Chrome {} it, and `semantics::dom::event_bubbles` says it {}",
                    if chrome { "bubbles" } else { "does not bubble" },
                    if chrome { "does not" } else { "does" }
                ));
            }
        }
    };
    for verb in Verb::ALL {
        let seen = match verb {
            Verb::Click => bubbled(&|p| p.click("#b").unwrap()),
            Verb::Submit => bubbled(&|p| p.submit("#f").unwrap()),
            Verb::Press => bubbled(&|p| p.press_in("#field", "a").unwrap()),
            Verb::Type => bubbled(&|p| p.type_into("#field", "hi").unwrap()),
            Verb::Choose => bubbled(&|p| p.choose("#s", "b").unwrap()),
            Verb::Hover => bubbled(&|p| p.hover("#b").unwrap()),
        };
        check(&format!("{verb:?}"), verb.events(), seen);
    }
    p.eval("document.activeElement && document.activeElement.blur()")
        .unwrap();
    let seen = bubbled(&|p| p.focus("#field").unwrap());
    check("Focus", &["focus"], seen);
    let seen = bubbled(&|p| p.blur("#field").unwrap());
    check("Blur", &["blur"], seen);
    assert!(
        wrong.is_empty(),
        "the in-process tiers must bubble exactly the events Chrome bubbles:\n{}",
        wrong.join("\n")
    );
}

const SCOPED_OVER_THEME: &str = r##"## Page /

View:
    <main>
      <button id="b">b</button>
      <section id="s">s</section>
      <a id="a" href="/x">a</a>
    </main>
Style:
    button { background: rgb(1, 2, 3); }
    section { padding: 7px; }
    a { color: rgb(4, 5, 6); }
"##;

#[test]
fn a_components_own_style_wins_over_the_built_in_theme() {
    let Some(b) = browser() else { return };
    let p = open(b, page(SCOPED_OVER_THEME));
    no_failures(&p);
    for (sel, property, want) in [
        ("#b", "background-color", "rgb(1, 2, 3)"),
        ("#s", "padding-top", "7px"),
        ("#a", "color", "rgb(4, 5, 6)"),
    ] {
        let got = p
            .eval(&format!(
                "getComputedStyle(document.querySelector({sel:?})).getPropertyValue({property:?})"
            ))
            .unwrap_or_else(|e| panic!("{sel}: {e}"));
        assert_eq!(
            got.as_str(),
            Some(want),
            "{sel}: a component's `Style:` rule for an element the built-in theme also styles must win \
             — the theme is the base layer a page's own design sits on"
        );
    }
}

// ---- Native constraint validation: one rule in process, Chrome as the witness ----

/// What the visitor does to the control before its validity is read.
#[derive(Clone, Copy, Debug)]
enum Act {
    Nothing,
    Type(&'static str),
    Tick,
}

/// One control in a form: its tag, its attributes, its `<option>`s as `(value, text)`, and the act.
struct ConstraintRow {
    what: &'static str,
    tag: &'static str,
    attrs: &'static [(&'static str, &'static str)],
    options: &'static [(&'static str, &'static str)],
    act: Act,
}

const fn row(
    what: &'static str,
    tag: &'static str,
    attrs: &'static [(&'static str, &'static str)],
    act: Act,
) -> ConstraintRow {
    ConstraintRow {
        what,
        tag,
        attrs,
        options: &[],
        act,
    }
}

/// Every constraint kind a form control can carry, each on both sides of its line.
const CONSTRAINT_ROWS: &[ConstraintRow] = &[
    row(
        "required, empty",
        "input",
        &[("required", "")],
        Act::Nothing,
    ),
    row(
        "required, typed",
        "input",
        &[("required", "")],
        Act::Type("a"),
    ),
    row(
        "required, typed then cleared",
        "input",
        &[("required", "")],
        Act::Type(""),
    ),
    row(
        "required textarea, empty",
        "textarea",
        &[("required", "")],
        Act::Nothing,
    ),
    row(
        "required checkbox, unticked",
        "input",
        &[("type", "checkbox"), ("required", "")],
        Act::Nothing,
    ),
    row(
        "required checkbox, ticked",
        "input",
        &[("type", "checkbox"), ("required", "")],
        Act::Tick,
    ),
    row(
        "required radio, unticked",
        "input",
        &[("type", "radio"), ("name", "g"), ("required", "")],
        Act::Nothing,
    ),
    row(
        "required radio, ticked",
        "input",
        &[("type", "radio"), ("name", "g"), ("required", "")],
        Act::Tick,
    ),
    ConstraintRow {
        what: "required select on its placeholder",
        tag: "select",
        attrs: &[("required", "")],
        options: &[("", "Pick one"), ("a", "A")],
        act: Act::Nothing,
    },
    ConstraintRow {
        what: "required select on a real option",
        tag: "select",
        attrs: &[("required", "")],
        options: &[("a", "A"), ("", "none")],
        act: Act::Nothing,
    },
    row(
        "required and disabled",
        "input",
        &[("required", ""), ("disabled", "")],
        Act::Nothing,
    ),
    row(
        "required and readonly",
        "input",
        &[("required", ""), ("readonly", "")],
        Act::Nothing,
    ),
    row(
        "email, not one",
        "input",
        &[("type", "email")],
        Act::Type("ada"),
    ),
    row(
        "email, one",
        "input",
        &[("type", "email")],
        Act::Type("ada@example.com"),
    ),
    row(
        "email, spaces around",
        "input",
        &[("type", "email")],
        Act::Type("  ada@example.com  "),
    ),
    row("email, empty", "input", &[("type", "email")], Act::Nothing),
    row(
        "email, a list of two",
        "input",
        &[("type", "email"), ("multiple", "")],
        Act::Type("a@b.co, c@d.co"),
    ),
    row(
        "email, a list with a bad one",
        "input",
        &[("type", "email"), ("multiple", "")],
        Act::Type("a@b.co, nope"),
    ),
    row(
        "email, a label ending in a hyphen",
        "input",
        &[("type", "email")],
        Act::Type("a@b-.co"),
    ),
    row(
        "url, not one",
        "input",
        &[("type", "url")],
        Act::Type("not a url"),
    ),
    row(
        "url, one",
        "input",
        &[("type", "url")],
        Act::Type("https://example.com/x?y=1"),
    ),
    row(
        "url, a scheme of its own",
        "input",
        &[("type", "url")],
        Act::Type("mailto:ada@example.com"),
    ),
    row(
        "url, http with no host",
        "input",
        &[("type", "url")],
        Act::Type("http://"),
    ),
    row(
        "number, not one",
        "input",
        &[("type", "number")],
        Act::Type("abc"),
    ),
    row(
        "number, letters among digits",
        "input",
        &[("type", "number")],
        Act::Type("12a"),
    ),
    row(
        "number, a sign in the middle",
        "input",
        &[("type", "number")],
        Act::Type("1-2"),
    ),
    row(
        "number, one",
        "input",
        &[("type", "number")],
        Act::Type("42"),
    ),
    row(
        "number, a fraction",
        "input",
        &[("type", "number")],
        Act::Type("-4.5"),
    ),
    row(
        "number, an exponent",
        "input",
        &[("type", "number")],
        Act::Type("1e3"),
    ),
    row(
        "number under its min",
        "input",
        &[("type", "number"), ("min", "5")],
        Act::Type("3"),
    ),
    row(
        "number at its min",
        "input",
        &[("type", "number"), ("min", "5")],
        Act::Type("5"),
    ),
    row(
        "number over its max",
        "input",
        &[("type", "number"), ("max", "10")],
        Act::Type("12"),
    ),
    row(
        "number off its step",
        "input",
        &[("type", "number"), ("step", "2")],
        Act::Type("3"),
    ),
    row(
        "number on its step",
        "input",
        &[("type", "number"), ("step", "2")],
        Act::Type("4"),
    ),
    row(
        "number off a step based on its min",
        "input",
        &[("type", "number"), ("min", "1"), ("step", "2")],
        Act::Type("4"),
    ),
    row(
        "number on a step based on its min",
        "input",
        &[("type", "number"), ("min", "1"), ("step", "2")],
        Act::Type("5"),
    ),
    row(
        "number, any step",
        "input",
        &[("type", "number"), ("step", "any")],
        Act::Type("3.25"),
    ),
    row(
        "number, fractional default step",
        "input",
        &[("type", "number")],
        Act::Type("3.5"),
    ),
    row(
        "minlength, short",
        "input",
        &[("minlength", "3")],
        Act::Type("ab"),
    ),
    row(
        "minlength, enough",
        "input",
        &[("minlength", "3")],
        Act::Type("abc"),
    ),
    row(
        "minlength, untouched",
        "input",
        &[("minlength", "3"), ("value", "ab")],
        Act::Nothing,
    ),
    row(
        "maxlength, typed past it",
        "input",
        &[("maxlength", "3")],
        Act::Type("abcdef"),
    ),
    row(
        "textarea minlength, short",
        "textarea",
        &[("minlength", "4")],
        Act::Type("abc"),
    ),
    row(
        "pattern, off",
        "input",
        &[("pattern", "[0-9]{3}")],
        Act::Type("12a"),
    ),
    row(
        "pattern, on",
        "input",
        &[("pattern", "[0-9]{3}")],
        Act::Type("123"),
    ),
    row(
        "pattern, the whole value",
        "input",
        &[("pattern", "[0-9]{3}")],
        Act::Type("1234"),
    ),
    row(
        "pattern, an alternation anchored whole",
        "input",
        &[("pattern", "cat|dog")],
        Act::Type("cats"),
    ),
    row(
        "pattern, password lookaheads, weak",
        "input",
        &[
            ("type", "password"),
            ("pattern", "(?=.*\\d)(?=.*[A-Z]).{8,}"),
        ],
        Act::Type("password1"),
    ),
    row(
        "pattern, password lookaheads, strong",
        "input",
        &[
            ("type", "password"),
            ("pattern", "(?=.*\\d)(?=.*[A-Z]).{8,}"),
        ],
        Act::Type("Password1"),
    ),
    row(
        "pattern, \\d is ASCII",
        "input",
        &[("pattern", "\\d+")],
        Act::Type("١٢"),
    ),
    row(
        "pattern, \\w",
        "input",
        &[("pattern", "\\w+")],
        Act::Type("é"),
    ),
    row(
        "pattern, \\s",
        "input",
        &[("pattern", "a\\sb")],
        Act::Type("a b"),
    ),
    row(
        "pattern, a backreference",
        "input",
        &[("pattern", "(a)\\1")],
        Act::Type("aa"),
    ),
    row(
        "pattern, a Unicode property",
        "input",
        &[("pattern", "[\\p{L} ]+")],
        Act::Type("Ada Lovelace"),
    ),
    row(
        "pattern, a Unicode property, off",
        "input",
        &[("pattern", "[\\p{L} ]+")],
        Act::Type("Ada 1"),
    ),
    row(
        "pattern, not a valid expression",
        "input",
        &[("pattern", "[a-z(]")],
        Act::Type("zzz!"),
    ),
    row(
        "pattern, a class minus a class",
        "input",
        &[("pattern", "[\\w--\\d]+")],
        Act::Type("abc"),
    ),
    row(
        "pattern, a class minus a class, off",
        "input",
        &[("pattern", "[\\w--\\d]+")],
        Act::Type("ab1"),
    ),
    row(
        "pattern, nested classes intersected",
        "input",
        &[("pattern", "[[a-z]&&[aeiou]]+")],
        Act::Type("aei"),
    ),
    row(
        "pattern, nested classes intersected, off",
        "input",
        &[("pattern", "[[a-z]&&[aeiou]]+")],
        Act::Type("abc"),
    ),
    row(
        "pattern, empty value",
        "input",
        &[("pattern", "[0-9]+")],
        Act::Nothing,
    ),
];

#[test]
fn every_constraint_kind_validates_in_process_as_chrome_validates_it() {
    use logicaffeine_compile::semantics::drive::Verb;
    let Some(b) = browser() else { return };
    let p = open(b, page(HEADLESS));
    let mut wrong = Vec::new();
    for r in CONSTRAINT_ROWS {
        let attrs = serde_json::to_string(&r.attrs).unwrap();
        let options = serde_json::to_string(&r.options).unwrap();
        p.eval(&format!(
            "(() => {{ document.getElementById('vf')?.remove(); const f = document.createElement('form'); f.id = 'vf'; \
             const c = document.createElement({tag:?}); c.id = 'c'; for (const [k, v] of {attrs}) c.setAttribute(k, v); \
             for (const [v, t] of {options}) {{ const o = document.createElement('option'); o.setAttribute('value', v); o.textContent = t; c.appendChild(o); }} \
             f.appendChild(c); document.body.appendChild(f); }})()",
            tag = r.tag
        ))
        .unwrap_or_else(|e| panic!("{}: {e}", r.what));
        match r.act {
            Act::Nothing => {}
            Act::Type(text) => p
                .type_into("#c", text)
                .unwrap_or_else(|e| panic!("{}: {e}", r.what)),
            Act::Tick => p.click("#c").unwrap_or_else(|e| panic!("{}: {e}", r.what)),
        }
        let chrome = p
            .eval(
                "(() => { const c = document.getElementById('c'); const v = c.validity; return [v.valueMissing, \
                 v.typeMismatch, v.tooShort, v.tooLong, v.patternMismatch, v.rangeUnderflow, v.rangeOverflow, \
                 v.stepMismatch, v.badInput, c.value]; })()",
            )
            .unwrap_or_else(|e| panic!("{}: {e}", r.what));

        dom::reset();
        let body = dom::body();
        let form = dom::create_element("form");
        dom::append_child(body, form);
        let c = dom::create_element(r.tag);
        dom::set_attribute(c, "id", "c");
        for (k, v) in r.attrs {
            dom::set_attribute(c, k, v);
        }
        for (v, t) in r.options {
            let o = dom::create_element("option");
            dom::set_attribute(o, "value", v);
            let text = dom::create_text_node(t);
            dom::append_child(o, text);
            dom::append_child(c, o);
        }
        dom::append_child(form, c);
        match r.act {
            Act::Nothing => {}
            Act::Type(text) => dom::enter_value(Verb::Type, "#c", text),
            Act::Tick => dom::set_checked(c, true),
        }
        let v = dom::validity(c).unwrap_or_else(|e| panic!("{}: refused: {e}", r.what));
        let here = serde_json::json!([
            v.value_missing,
            v.type_mismatch,
            v.too_short,
            v.too_long,
            v.pattern_mismatch,
            v.range_underflow,
            v.range_overflow,
            v.step_mismatch,
            v.bad_input,
            dom::control_value(c)
        ]);
        if here != chrome {
            wrong.push(format!(
                "{}\n  Chrome:     {chrome}\n  in process: {here}\n  (valueMissing, typeMismatch, tooShort, tooLong, patternMismatch, rangeUnderflow, rangeOverflow, stepMismatch, badInput, value)",
                r.what
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "the in-process tiers must validate every control exactly as Chrome does — the rule `Submit` \
         applies before a form is submitted:\n{}",
        wrong.join("\n")
    );
}

#[test]
fn a_pattern_neither_engine_can_express_is_refused_by_name() {
    for pattern in [
        "\\p{RGI_Emoji}",
        "[\\p{RGI_Emoji_Flag_Sequence}a]",
        "[\\q{abc}]",
        "[\\q{abc|d}x]",
    ] {
        let why = dom::pattern_matches(pattern, "x").expect_err(pattern);
        assert!(
            why.contains(pattern),
            "the refusal names the pattern it cannot check: {why}"
        );
    }
}

const VALIDATES: &str = r##"## Page /

State:
    Let name be "".
    Let mail be "".
    Let sent be 0.
    Let invalids be 0.
Actions:
    To send:
        Set sent to sent + 1.
    To noted:
        Set invalids to invalids + 1.
View:
    <form id="f" on-submit.prevent={send}>
      <input id="name" name="name" required bind={name} on-invalid={noted}>
      <input id="mail" name="mail" type="email" bind={mail} on-invalid={noted}>
      <button id="go" type="submit">go</button>
    </form>
    <form id="nv" novalidate on-submit.prevent={send}>
      <input id="nv-name" name="name" required>
    </form>
    <p id="counts">{sent} {invalids}</p>

## Test: an invalid form is not submitted
    Type "nope" into "#mail".
    Submit "#f".
    Submit "#nv".
    Type "Ada" into "#name".
    Type "ada@example.com" into "#mail".
    Submit "#go".
"##;

#[test]
fn submitting_validates_the_form_as_request_submit_does_on_every_tier() {
    use logicaffeine_compile::page_host::Axis;
    let Some((here, vm, chrome)) = lowered_tiers(VALIDATES) else {
        return;
    };
    for (tier, trace) in [
        ("the lowered tree-walker", &here),
        ("the lowered VM", &vm),
        ("Chrome", &chrome),
    ] {
        assert!(trace.stopped.is_none(), "{tier}: {:?}", trace.stopped);
        let counts = |k: usize| trace.steps[k].snapshot.document.clone();
        assert!(
            counts(2).contains("<p id=\"counts\">0 2</p>"),
            "{tier}: an invalid form fires `invalid` at each invalid control and no `submit`:\n{}",
            counts(2)
        );
        assert_eq!(
            trace.steps[2].snapshot.focus.as_deref(),
            Some("input#name"),
            "{tier}: the first invalid control takes focus"
        );
        assert!(
            counts(3).contains("<p id=\"counts\">1 2</p>"),
            "{tier}: a `novalidate` form submits as it is:\n{}",
            counts(3)
        );
        assert!(
            counts(6).contains("<p id=\"counts\">2 2</p>"),
            "{tier}: a valid form submits, through its submit button:\n{}",
            counts(6)
        );
    }
    for (tier, candidate) in [("the lowered tree-walker", &here), ("the lowered VM", &vm)] {
        let off = disagreements(
            &chrome,
            candidate,
            &[Axis::Document, Axis::Focus, Axis::Journal],
        );
        assert!(off.is_empty(), "{tier}:\n{off}");
    }
}

const TWO_ROUTE_HANDLERS: &str = r##"## Main
Call onRoute with (tick: Int) -> storageSet("local", "first", routePath()).
Call onRoute with (tick: Int) -> storageSet("local", "second", routePath()).
"##;

#[test]
fn a_popstate_reaches_every_route_handler_the_page_registered_in_chrome() {
    let Some(b) = browser() else { return };
    let p = open(b, page(TWO_ROUTE_HANDLERS));
    p.eval("history.pushState(null, '', '/elsewhere'); window.dispatchEvent(new PopStateEvent('popstate'))").unwrap();
    let stored = p.storage().unwrap();
    let heard = |key: &str| {
        stored
            .iter()
            .any(|(area, k, v)| area == "local" && k == key && v == "/elsewhere")
    };
    assert!(
        heard("first") && heard("second"),
        "a back/forward reaches every handler the page registered with `onRoute` — a second \
         registration never silences the first: {stored:?}"
    );
}

const LOADS_ANOTHER_DOCUMENT: &str = r##"## Page /

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

const LINKS: &str = r##"## Page /

State:
    Let taps be 0.
Actions:
    To tap:
        Set taps to taps + 1.
View:
    <a id="away" href="/elsewhere"><span id="away-label">away</span></a>
    <a id="again" href="/?tab=2"><span id="again-label">again</span></a>
    <a id="out" href="/other" target="_blank">out</a>
    <button id="b" on-click={tap}>{taps}</button>

## Test: a link to the page's own path navigates in place; any other loads its document
    Click "#b".
    Click "#again-label".
    Click "#out".
    Click "#away-label".
"##;

/// The document, URL and navigation log after each step of `trace`.
fn visited(
    trace: &logicaffeine_compile::page_host::PageTrace,
) -> Vec<(String, String, Vec<String>)> {
    trace
        .steps
        .iter()
        .map(|s| {
            (
                s.snapshot.document.clone(),
                s.snapshot.url.clone(),
                s.snapshot.route_ops.clone(),
            )
        })
        .collect()
}

#[test]
fn a_page_that_loads_another_document_leaves_it_on_every_tier() {
    use logicaffeine_compile::page_host::Axis;
    let Some((here, vm, chrome)) = lowered_tiers(LOADS_ANOTHER_DOCUMENT) else {
        return;
    };
    for (tier, trace) in [
        ("the lowered tree-walker", &here),
        ("the lowered VM", &vm),
        ("Chrome", &chrome),
    ] {
        assert!(trace.stopped.is_none(), "{tier}: {:?}", trace.stopped);
        let after = &trace.steps[1].snapshot;
        assert_eq!(
            (after.document.as_str(), after.url.as_str(), after.route_ops.clone()),
            ("<body></body>", "/about?from=home", vec!["load /about?from=home".to_string()]),
            "{tier}: the browser is at the address the page sent it to, having LOADED a document there \
             — another page file's, outside the test"
        );
        assert_eq!(
            (after.title.as_str(), after.head.as_deref()),
            ("", None),
            "{tier}: the page's head left with it"
        );
        assert_eq!(after.journal, ["leave"], "{tier}");
    }
    for (tier, candidate) in [("the lowered tree-walker", &here), ("the lowered VM", &vm)] {
        let off = disagreements(
            &chrome,
            candidate,
            &[
                Axis::Document,
                Axis::Url,
                Axis::RouteOps,
                Axis::Head,
                Axis::Title,
                Axis::Journal,
            ],
        );
        assert!(off.is_empty(), "{tier}:\n{off}");
    }
}

#[test]
fn a_link_is_followed_in_place_when_the_page_owns_its_path_and_by_a_load_otherwise_on_every_tier() {
    use logicaffeine_compile::page_host::Axis;
    let Some((here, vm, chrome)) = lowered_tiers(LINKS) else {
        return;
    };
    for (tier, trace) in [
        ("the lowered tree-walker", &here),
        ("the lowered VM", &vm),
        ("Chrome", &chrome),
    ] {
        assert!(trace.stopped.is_none(), "{tier}: {:?}", trace.stopped);
        let steps = visited(trace);
        assert!(
            steps[1].0.contains("<button id=\"b\">1</button>"),
            "{tier}: the page counted:\n{}",
            steps[1].0
        );
        assert!(
            steps[2].0.contains("<button id=\"b\">1</button>"),
            "{tier}: a link to a path this page answers itself moves the location in place — the page, and \
             what it counted, stay:\n{}",
            steps[2].0
        );
        assert_eq!(
            (steps[2].1.as_str(), steps[2].2.clone()),
            ("/?tab=2", vec!["push /?tab=2".to_string()]),
            "{tier}"
        );
        assert_eq!(
            (steps[3].0.as_str(), steps[3].1.as_str()),
            (steps[2].0.as_str(), "/?tab=2"),
            "{tier}: a link to another window leaves this document where it is"
        );
        assert_eq!(
            (steps[4].0.as_str(), steps[4].1.as_str(), steps[4].2.clone()),
            (
                "<body></body>",
                "/elsewhere",
                vec!["load /elsewhere".to_string()]
            ),
            "{tier}: a link to a path the page does not answer loads that page file's document"
        );
    }
    for (tier, candidate) in [("the lowered tree-walker", &here), ("the lowered VM", &vm)] {
        let off = disagreements(
            &chrome,
            candidate,
            &[Axis::Document, Axis::Url, Axis::RouteOps, Axis::Journal],
        );
        assert!(off.is_empty(), "{tier}:\n{off}");
    }
}

const GOES_BACK: &str = r##"## Page /

State:
    Let moves be 0.
Actions:
    When the route changes:
        Set moves to moves + 1.
    To back:
        Go back.
View:
    <p id="moves">{moves}</p>
    <button id="back" on-click={back}>back</button>

## Test: going back is one popstate
    Navigate to "/second".
    Click "#back".
"##;

#[test]
fn going_back_raises_one_popstate_every_route_listener_hears_once_on_every_tier() {
    use logicaffeine_compile::page_host::Axis;
    let Some((here, vm, chrome)) = lowered_tiers(GOES_BACK) else {
        return;
    };
    for (tier, trace) in [
        ("the lowered tree-walker", &here),
        ("the lowered VM", &vm),
        ("Chrome", &chrome),
    ] {
        assert!(trace.stopped.is_none(), "{tier}: {:?}", trace.stopped);
        let after = &trace.steps[2].snapshot;
        assert_eq!(
            after.url, "/",
            "{tier}: Back returned to where the visit began"
        );
        assert!(
            after.document.contains("<p id=\"moves\">2</p>"),
            "{tier}: the test's move and the page's own Back are one route change each — Back raises one \
             `popstate`, heard once:\n{}",
            after.document
        );
    }
    for (tier, candidate) in [("the lowered tree-walker", &here), ("the lowered VM", &vm)] {
        let off = disagreements(
            &chrome,
            candidate,
            &[Axis::Document, Axis::Url, Axis::Journal],
        );
        assert!(off.is_empty(), "{tier}:\n{off}");
    }
}

const POSTS_ITSELF: &str = r##"## Page /

View:
    <form id="f" method="post" action="/save"><input type="hidden" name="_csrf" value="t"><input name="note" value="hi"><button id="go" type="submit">save</button></form>

## Test: a form nothing handles is answered by the server
    Given that POST "/save" answers 200 with "{{\"saved\": true}}".
    Submit "#f".
"##;

#[test]
fn a_form_nothing_handles_is_replaced_by_the_servers_answer_on_every_tier() {
    use logicaffeine_compile::page_host::Axis;
    let Some((here, vm, chrome)) = lowered_tiers(POSTS_ITSELF) else {
        return;
    };
    for (tier, trace) in [
        ("the lowered tree-walker", &here),
        ("the lowered VM", &vm),
        ("Chrome", &chrome),
    ] {
        assert!(trace.stopped.is_none(), "{tier}: {:?}", trace.stopped);
        let after = &trace.steps[2].snapshot;
        assert_eq!(
            after.url, "/save",
            "{tier}: the browser is at the address it posted to"
        );
        assert!(
            after.document.contains("{\"saved\": true}") && !after.document.contains("<form"),
            "{tier}: the page is gone and the document is the server's answer:\n{}",
            after.document
        );
    }
    for (tier, candidate) in [("the lowered tree-walker", &here), ("the lowered VM", &vm)] {
        let off = disagreements(&chrome, candidate, &[Axis::Document, Axis::Head, Axis::Url]);
        assert!(off.is_empty(), "{tier}:\n{off}");
    }
}

const TWO_ROUTE_LISTENERS: &str = r##"## Component Left
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

## Test: every component listening for the route hears it
    Mount Page.
    Mount Left at "#a".
    Mount Right at "#b".
    Navigate to "/elsewhere".
    Click "#go".
"##;

#[test]
fn every_mounted_route_listener_hears_every_move_once_on_every_tier() {
    use logicaffeine_compile::page_host::Axis;
    let Some((here, vm, chrome)) = lowered_tiers(TWO_ROUTE_LISTENERS) else {
        return;
    };
    for (tier, trace) in [
        ("the lowered tree-walker", &here),
        ("the lowered VM", &vm),
        ("Chrome", &chrome),
    ] {
        assert!(trace.stopped.is_none(), "{tier}: {:?}", trace.stopped);
        let moved = &trace.steps[4].snapshot.document;
        assert!(
            moved.contains("<p id=\"left\">1</p>") && moved.contains("<p id=\"right\">1</p>"),
            "{tier}: a move of the location reaches BOTH components that listen for it, not only the \
             one registered last:\n{moved}"
        );
        let own = &trace.steps[5].snapshot.document;
        assert!(
            own.contains("<p id=\"left\">2</p>") && own.contains("<p id=\"right\">2</p>"),
            "{tier}: the page's own `Navigate to` reaches each listener exactly once:\n{own}"
        );
    }
    for (tier, candidate) in [("the lowered tree-walker", &here), ("the lowered VM", &vm)] {
        let off = disagreements(
            &chrome,
            candidate,
            &[Axis::Document, Axis::Url, Axis::Journal],
        );
        assert!(off.is_empty(), "{tier}:\n{off}");
    }
}

const A_LISTENER_LEAVES: &str = r##"## Component Child
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
    Click "#hide".
    Navigate to "/two".
"##;

#[test]
fn a_route_listener_hears_moves_while_on_the_page_and_none_after_it_leaves_on_every_tier() {
    use logicaffeine_compile::page_host::Axis;
    let Some((here, vm, chrome)) = lowered_tiers(A_LISTENER_LEAVES) else {
        return;
    };
    for (tier, trace) in [
        ("the lowered tree-walker", &here),
        ("the lowered VM", &vm),
        ("Chrome", &chrome),
    ] {
        assert!(trace.stopped.is_none(), "{tier}: {:?}", trace.stopped);
        let heard = &trace.steps[2].snapshot.document;
        assert!(
            heard.contains("<p id=\"child\">1</p>") && heard.contains("<p id=\"page\">1</p>"),
            "{tier}: a move of the location reaches every component on the page listening for it, a \
             placed one as much as the one mounted:\n{heard}"
        );
        let (gone, after) = (&trace.steps[3].snapshot, &trace.steps[4].snapshot);
        assert!(
            !gone.document.contains("id=\"child\""),
            "{tier}: the child left:\n{}",
            gone.document
        );
        assert!(
            after.document.contains("<p id=\"page\">2</p>"),
            "{tier}: the page still hears it:\n{}",
            after.document
        );
        assert_eq!(
            after.journal.len(),
            gone.journal.len() + 1,
            "{tier}: exactly one turn answers the second move — the page's — and none runs for the \
             child that left: {:?} → {:?}",
            gone.journal,
            after.journal
        );
    }
    for (tier, candidate) in [("the lowered tree-walker", &here), ("the lowered VM", &vm)] {
        let off = disagreements(&chrome, candidate, &[Axis::Document, Axis::Journal]);
        assert!(off.is_empty(), "{tier}:\n{off}");
    }
}

const CARRIES_A_TOKEN: &str = r##"## A Note has:
    a text, which is Text.

## Page /

State:
    Let got be "".
Actions:
    To save:
        Post "x" to "/api/notes" as Note, then saved.
    To read:
        Get "/api/notes" as Note, then saved.
    To leave:
        Post "x" to "https://elsewhere.example/api" as Note, then saved.
    To saved (r: Result of Note and HttpError):
        Set got to "done".
Head:
    <meta name="csrf-token" content="t0k3n">
View:
    <button id="save" on-click={save}>save</button>
    <button id="read" on-click={read}>read</button>
    <button id="leave" on-click={leave}>leave</button>

## Test: a page's state-changing requests to its own server carry its token
    Click "#save".
    Click "#read".
    Click "#leave".
"##;

#[test]
fn a_pages_state_changing_requests_to_its_own_server_carry_its_csrf_token_on_every_tier() {
    use logicaffeine_compile::page_host::Axis;
    let Some((reference, lowered, chrome)) = three_tiers(CARRIES_A_TOKEN) else {
        return;
    };
    for (tier, trace) in [
        ("the reference", &reference),
        ("the lowered VM", &lowered),
        ("Chrome", &chrome),
    ] {
        let fetches = &trace.steps[3].snapshot.fetches;
        assert_eq!(fetches.len(), 3, "{tier}: three requests: {fetches:#?}");
        assert!(
            fetches[0].starts_with("POST /api/notes ")
                && fetches[0].contains("x-csrf-token: t0k3n"),
            "{tier}: a POST to the page's own server carries the token its document's \
             `<meta name=\"csrf-token\">` holds: {}",
            fetches[0]
        );
        assert!(
            fetches[1].starts_with("GET /api/notes ") && !fetches[1].contains("x-csrf-token"),
            "{tier}: a GET changes nothing and carries no token: {}",
            fetches[1]
        );
        assert!(
            fetches[2].starts_with("POST https://elsewhere.example/api ")
                && !fetches[2].contains("x-csrf-token"),
            "{tier}: a token is never handed to another origin: {}",
            fetches[2]
        );
    }
    for (tier, candidate) in [("the lowered VM", &lowered), ("Chrome", &chrome)] {
        let off = disagreements(&reference, candidate, &[Axis::Fetches]);
        assert!(off.is_empty(), "{tier}:\n{off}");
    }
}

/// Head elements whose identity — what a page's own version of it replaces — must be ONE rule in a
/// browser and in process: every kind a `Head:` or a shell carries, and the two a substring reading
/// gets wrong (`charset=` inside another attribute's value, `name` as the tail of `data-name`).
const HEAD_ELEMENTS: &[&str] = &[
    "<title>a page</title>",
    r#"<meta charset="utf-8">"#,
    r#"<meta http-equiv="Content-Type" content="text/html; charset=utf-8">"#,
    r#"<meta name="Description" content="what the page is">"#,
    r##"<meta name="theme-color" content="#faf9f5" media="(prefers-color-scheme: light)">"##,
    r##"<meta name="theme-color" content="#1f1e1d" media="(prefers-color-scheme: dark)">"##,
    r#"<meta data-name="description" content="not a name">"#,
    r#"<meta property="og:title" content="a page">"#,
    r#"<meta content="nothing names it">"#,
    r#"<link rel="icon" href="/favicon.svg">"#,
    r#"<link rel="Stylesheet" href="/site.css">"#,
    r#"<link href="/no-rel.css">"#,
    r#"<base href="/app/">"#,
    "<style>p > a { color: red; }</style>",
    r#"<script src="/extra.js"></script>"#,
];

#[test]
fn the_head_identity_is_one_rule_in_chrome_and_in_process() {
    let Some(b) = browser() else { return };
    let p = open(b, page(COUNTER));
    let mut differ = Vec::new();
    for tag in HEAD_ELEMENTS {
        let markup = serde_json::to_string(tag).unwrap();
        let chrome = p
            .eval(&format!(
                "(() => {{ const t = document.createElement('template'); t.innerHTML = {markup}; \
                 return __logosHeadIdentity(t.content.firstElementChild); }})()"
            ))
            .unwrap_or_else(|e| panic!("{tag}: {e}"));
        let parsed = dom::parse_head(tag);
        assert_eq!(parsed.len(), 1, "{tag} is one head element: {parsed:?}");
        let in_process = parsed[0].identity();
        if chrome.as_str() != in_process.as_deref() {
            differ.push(format!(
                "{tag}\n  Chrome:     {chrome}\n  in process: {in_process:?}"
            ));
        }
    }
    assert!(
        differ.is_empty(),
        "`runtime.js`'s `headIdentity` and `semantics::dom::HeadTag::identity` must be one rule — the \
         server's composition and the in-process document merge a `Head:` by the latter:\n{}",
        differ.join("\n")
    );
}

const HEADLESS: &str = r##"## Page /

View:
    <p id="p">plain</p>

## Test: nothing in the page names a head
    Expect that the text of "#p" is equal to "plain".
"##;

#[test]
fn every_tier_arrives_at_the_same_document() {
    use logicaffeine_compile::page_host::Axis;
    let Some((reference, lowered, chrome)) = three_tiers(HEADLESS) else {
        return;
    };
    let arrived = &reference.steps[0].snapshot;
    assert!(
        arrived
            .head
            .as_deref()
            .is_some_and(|h| h.contains("<meta charset=\"utf-8\">")),
        "the reference arrives in the shell a page ships in, not in an empty document: {:?}",
        arrived.head
    );
    for (tier, candidate) in [("the lowered VM", &lowered), ("Chrome", &chrome)] {
        let off = disagreements(
            &reference,
            candidate,
            &[Axis::Head, Axis::Title, Axis::Url, Axis::Document],
        );
        assert!(
            off.is_empty(),
            "{tier} arrives at a different document:\n{off}"
        );
    }
}

/// An inline scenario's corpus lock: every divergence its run finds is ledgered, and every row the
/// ledger keeps under its own namespace (`inline::<owner>::…`) is one the run still finds.
fn inline_agrees(owner: &str, source: &str) {
    let id = format!("inline::{owner}");
    let known = known_divergences();
    let scan = tier_divergences(&id, source, &known);
    let own: std::collections::BTreeMap<(String, String, usize), String> = known
        .iter()
        .filter(|(k, _)| k.0.starts_with(&format!("{id}::")))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let unparsed: Vec<String> = scan.unparsed_page.iter().cloned().collect();
    let stale: Vec<String> = logicaffeine_compile::page_host::stale_rows(
        &own,
        &scan.found,
        &unparsed,
        &scan.refused_at_zero,
    )
    .into_iter()
    .map(|(id, kind, at)| format!("{id} | browser:{kind} @ {at}"))
    .collect();
    assert!(
        scan.unlisted.is_empty() && stale.is_empty(),
        "UNLISTED:\n{}\n\nSTALE (the tiers agree there now — delete them):\n{}",
        scan.unlisted.join("\n\n"),
        stale.join("\n")
    );
}

#[test]
fn a_component_test_agrees_on_the_tree_walker_the_vm_and_chrome() {
    inline_agrees(
        "a_component_test_agrees_on_the_tree_walker_the_vm_and_chrome",
        "## Component App\n\
         State:\n    Let n be 0.\n\
         Actions:\n    To bump:\n        Set n to n + 1.\n\
         View:\n    <p id=\"n\">{n}</p>\n    <button id=\"go\" on-click={bump}>go</button>\n\n\
         ## Test: bumps twice\n\
         \x20   Mount App.\n\
         \x20   Click \"#go\".\n\
         \x20   Click \"#go\".\n\
         \x20   Expect that the text of \"#n\" is equal to \"2\".\n",
    );
}

#[test]
fn a_typed_control_agrees_on_the_tree_walker_the_vm_and_chrome() {
    inline_agrees(
        "a_typed_control_agrees_on_the_tree_walker_the_vm_and_chrome",
        "## Component App\n\
         State:\n    Let draft be \"\".\n\
         View:\n    <input id=\"draft\" bind={draft}>\n    <p id=\"echo\">{draft}</p>\n\n\
         ## Test: typing echoes\n\
         \x20   Mount App.\n\
         \x20   Type \"hi\" into \"#draft\".\n\
         \x20   Expect that the text of \"#echo\" is equal to \"hi\".\n",
    );
}

/// Every page file under `examples/*/pages/` — mirrors `lowered_vm_tier_lock.rs`'s own
/// `example_pages()` (each `tests/*.rs` file is its own binary, unreachable from here), reading
/// each page through its own project closure ([`logicaffeine_compile::page_host::assemble_stable`]
/// — the ONE function both readers call, so they cannot resolve `Import`s two different ways) the
/// same way `largo build --static` resolves them — a page that imports a project module (`Import
/// Shared.`) must compile identically on every tier, this corpus included, not only under the
/// CLI's own build. A `## Main` script becomes a `## Test` of its own; a page with no scenario at
/// all is traced as it renders.
fn example_pages() -> Vec<(String, String)> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let mut out = Vec::new();
    let mut apps: Vec<std::path::PathBuf> = std::fs::read_dir(root.join("examples"))
        .unwrap()
        .flatten()
        .map(|e| e.path().join("pages"))
        .filter(|p| p.is_dir())
        .collect();
    apps.sort();
    for pages in apps {
        let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(&pages)
            .unwrap()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "lg"))
            .collect();
        files.sort();
        for f in files {
            let text = logicaffeine_compile::page_host::assemble_stable(&f)
                .unwrap_or_else(|e| panic!("could not stably assemble {}: {e}", f.display()));
            let mut source = text.replace("\n## Main\n", "\n## Test: main\n");
            if !source.contains("## Test") {
                source.push_str("\n## Test: the page renders\n    Expect that 1 is equal to 1.\n");
            }
            let id = f
                .strip_prefix(&root)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            out.push((id, source));
        }
    }
    out
}

/// `fullstack_todo/pages/app.lg` `Import Shared.` — the corpus's only page that imports another
/// module, named on its own so a regression in import resolution (`example_pages`'s
/// `assemble_stable`, or the WASM backend's field-slot handling of an imported type) fails HERE by
/// name instead of being lost in the general corpus sweep below. Every remaining divergence is one
/// this page shares with the rest of the corpus (the shell's head/title, a real click or keystroke's
/// focus, the journal gap) or a named gap in this tier's own snapshot (`fetches`) — none of them the
/// refusal this test exists to catch a return of.
#[test]
fn fullstack_todo_agrees_on_the_tree_walker_the_vm_and_chrome_or_is_ledgered() {
    let Some(_) = browser() else { return };
    let pages = example_pages();
    let (id, source) = pages
        .iter()
        .find(|(id, _)| id.starts_with("examples/fullstack_todo/"))
        .unwrap_or_else(|| panic!("fullstack_todo/pages/app.lg must be in the example corpus"));
    let scan = tier_divergences(id, source, &known_divergences());
    assert!(
        scan.unlisted.is_empty(),
        "fullstack_todo must compile and agree on every tier, or ledger the divergence with its \
         cause as `<scenario> | browser:<axis> @ <statement> | <cause>`:\n{}",
        scan.unlisted.join("\n\n")
    );
}

#[test]
fn every_example_page_agrees_on_the_tree_walker_the_vm_and_chrome_or_is_ledgered() {
    let Some(_) = browser() else { return };
    let known = known_divergences();
    let pages = example_pages();
    assert!(
        pages.len() >= 4,
        "only {} example pages found — the reader is broken",
        pages.len()
    );
    let mut unlisted = Vec::new();
    let mut found = std::collections::BTreeMap::new();
    let mut unparsed_pages = Vec::new();
    let mut refused_at_zero = std::collections::BTreeSet::new();
    for (id, source) in &pages {
        let scan = tier_divergences(id, source, &known);
        unlisted.extend(scan.unlisted);
        found.extend(scan.found);
        unparsed_pages.extend(scan.unparsed_page);
        refused_at_zero.extend(scan.refused_at_zero);
    }
    // Only this test's OWN namespace (`examples/…`) can go stale from its own run: the ledger's
    // `inline::…` rows belong to the two tests above, which read the same shared file but never
    // run this corpus, so they are never "found" here and must not be flagged as if they were.
    let known_examples: std::collections::BTreeMap<(String, String, usize), String> = known
        .iter()
        .filter(|(k, _)| k.0.starts_with("examples/"))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    let stale: Vec<String> = logicaffeine_compile::page_host::stale_rows(
        &known_examples,
        &found,
        &unparsed_pages,
        &refused_at_zero,
    )
    .into_iter()
    .map(|(id, kind, at)| format!("{id} | browser:{kind} @ {at}"))
    .collect();
    assert!(
        unlisted.is_empty() && stale.is_empty(),
        "UNLISTED browser divergences over {} example pages (fix the tier, or ledger each with \
         its cause as `<scenario> | browser:<axis> @ <statement> | <cause>`):\n{}\n\n\
         STALE browser ledger entries (name no page this corpus still discovers — delete them):\n{}",
        pages.len(),
        unlisted.join("\n\n"),
        stale.join("\n")
    );
}

/// A backdrop holding a dialog holding a button, each centred in the one around it: the centre of
/// the backdrop is the dialog and the centre of the dialog is the button.
const NESTED_TARGETS: &str = r#"## Page /

State:
    Let log be "".
Actions:
    To outside:
        Set log to log + "o".
    To inside:
        Set log to log + "i".
    To press:
        Set log to log + "b".
View:
    <main>
      <p id="log">[{log}]</p>
      <div id="backdrop" class="backdrop" on-click={outside}>
        <div id="dialog" class="dialog" on-click.stop={inside}>
          <button id="b" on-click.stop={press}>b</button>
        </div>
      </div>
    </main>
Style:
    .backdrop { position: fixed; inset: 0; display: flex; align-items: center; justify-content: center; }
    .dialog { padding: 48px; }
"#;

#[test]
fn a_click_lands_on_the_element_it_names_not_on_what_is_drawn_at_its_centre() {
    let Some(b) = browser() else { return };
    let steps = [
        "Click \"#dialog\".",
        "Click \"#backdrop\".",
        "Click \"#b\".",
    ];
    let expected: Vec<String> = (1..=steps.len())
        .map(|n| reference(NESTED_TARGETS, &steps[..n]))
        .collect();
    assert!(
        expected[2].contains("[iob]"),
        "the oracle must tell the three apart: {expected:?}"
    );

    let p = open(b, page(NESTED_TARGETS));
    let mut seen = Vec::new();
    for sel in ["#dialog", "#backdrop", "#b"] {
        p.click(sel).unwrap();
        seen.push(body(&p));
    }
    no_failures(&p);
    assert_eq!(
        seen, expected,
        "`Click` names the element that is clicked, on every tier"
    );
}

/// A `## Store` field holding a `Seq` of records, pushed by a store action and drawn by a PLACED
/// component's loop — the shape examples/modal_toast's toast tray has. Every tier draws each record.
const STORE_RECORDS: &str = r#"## A Toast has:
    a number, which is Int.
    a message, which is Text.

## Store Toasts

State:
    Let toasts be a new Seq of Toast.
    Let next be 1.

Actions:
    To post (text: Text):
        Push (a new Toast with number next and message text) to toasts.
        Set next to next + 1.

## Component Tray

Use Toasts.

View:
    <ul id="tray">{for each t in Toasts's toasts, keyed by t's number:}<li>{t's number}: {t's message}</li>{end}</ul>

## Page /

Use Toasts.

Actions:
    To add:
        Call Toasts's post with "hi".

View:
    <main><button id="add" on-click={add}>add</button><Tray/></main>
"#;

#[test]
fn a_store_list_of_records_pushed_by_its_action_is_drawn_by_a_placed_component_on_every_tier() {
    let steps = ["Click \"#add\".", "Click \"#add\"."];
    let expected: Vec<String> = (1..=steps.len())
        .map(|n| reference(STORE_RECORDS, &steps[..n]))
        .collect();
    assert!(
        expected[1].contains("<li>2: hi</li>"),
        "the oracle must draw both toasts: {expected:?}"
    );
    let lowered: Vec<String> = (1..=steps.len())
        .map(|n| {
            dom::reset();
            let program = format!("{STORE_RECORDS}\n## Main\n{}\n", steps[..n].join("\n"));
            let out = logicaffeine_compile::compile::vm_outcome(&program);
            assert_eq!(out.error, None, "the lowered VM runs the page:\n{program}");
            dom::render()
        })
        .collect();
    assert_eq!(
        lowered, expected,
        "the lowered VM draws what the reference tier draws"
    );
    let Some(b) = browser() else { return };
    let p = open(b, page(STORE_RECORDS));
    let mut seen = Vec::new();
    for _ in steps {
        p.click("#add").unwrap();
        seen.push(body(&p));
    }
    no_failures(&p);
    assert_eq!(
        seen, expected,
        "the browser draws every toast the store holds, after each click"
    );
}
