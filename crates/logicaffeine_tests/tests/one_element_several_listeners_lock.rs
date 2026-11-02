//! ★ LOCK — every listener an event reaches answers it, however many wait on one element. ★
//!
//! One element can wait for one event several times over: a directive on the element and a sentence
//! naming it, two sentences naming it. The reference tier runs every listener the event reaches, in
//! the order it runs them; the lowered page keeps its listeners in one table, where the first
//! listener on an (element, event) slot is found by the slot and each after it spills under a lookup
//! of its own, so none takes another's place. Each row drives the page on the tree-walker (the
//! reference) and the lowered page on the bytecode VM and holds both to the same document; the
//! browser row drives the same pages in Chrome. The table rows hold the lowered page's bookkeeping
//! to a constant size as elements are redrawn and instances leave.

#[path = "cli_common/mod.rs"]
mod common;

use logicaffeine_cli::browser::{self, Browser, PageBundle};
use logicaffeine_compile::compile::{compile_to_wasm, tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;

/// `src`'s document on both tiers, which must agree.
fn drawn(src: &str) -> String {
    dom::reset();
    let tw = tw_outcome(src);
    assert_eq!(tw.error, None, "the tree-walker runs it:\n{src}");
    let tw_doc = dom::render();
    dom::reset();
    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "the VM runs it:\n{src}");
    let vm_doc = dom::render();
    assert_eq!(
        tw_doc, vm_doc,
        "the lowered page answers what the reference answers:\n{src}"
    );
    tw_doc
}

/// A directive on the button and a sentence naming it, both waiting for a click.
const DIRECTIVE_AND_SENTENCE: &str = "\
## Component App
State:
    Let a be 0.
    Let b be 0.
Actions:
    To one:
        Set a to a + 1.
    When \"#go\" is clicked:
        Set b to b + 1.
View:
    <p id=\"n\">{a}-{b}</p>
    <button id=\"go\" on-click={one}>go</button>
";

/// Two sentences naming one button, both waiting for a click.
const TWO_SENTENCES: &str = "\
## Component App
State:
    Let a be 0.
    Let b be 0.
Actions:
    When \"#go\" is clicked:
        Set a to a + 1.
    When \"#go\" is clicked:
        Set b to b + 1.
View:
    <p id=\"n\">{a}-{b}</p>
    <button id=\"go\">go</button>
";

/// Three listeners on one button, each writing its letter, so the document shows the order they ran.
const THREE_IN_ORDER: &str = "\
## Component App
State:
    Let trail be \"\".
Actions:
    To first:
        Set trail to trail + \"d\".
    When \"#go\" is clicked:
        Set trail to trail + \"s\".
    When \"#go\" is clicked:
        Set trail to trail + \"t\".
View:
    <p id=\"n\">[{trail}]</p>
    <button id=\"go\" on-click={first}>go</button>
";

/// Two listeners on a button every click redraws: the `{if}` flips, so each paint draws the button
/// again under a fresh node, and both listeners register on it again.
const REDRAWN_WITH_TWO_LISTENERS: &str = "\
## Component App
State:
    Let flag be true.
    Let heard be 0.
Actions:
    To flip:
        Set flag to not flag.
    When \"#go\" is clicked:
        Set heard to heard + 1.
View:
    <div>{if flag:}<p>even</p>{else:}<p>odd</p>{end}<p id=\"n\">{heard}</p><button id=\"go\" on-click={flip}>x</button></div>
";

/// Every click replaces the one row with a new one under a new key, so each click an instance —
/// and the listener it registered — leaves for good.
const A_ROW_LEAVES_EVERY_CLICK: &str = "\
## Component Row (label: Int)
State:
    Let clicks be 0.
Actions:
    To bump:
        Set clicks to clicks + 1.
View:
    <li class=\"row\" on-click={bump}>{label}:{clicks}</li>

## Component App
State:
    Let n be 0.
    Let items be [0].
Actions:
    To churn:
        Set n to n + 1.
        Set items to [n].
View:
    <ul>{for each x in items, keyed by x:}<Row label={x}/>{end}</ul>
    <button id=\"go\" on-click={churn}>next</button>
";

/// A directive on a paragraph and a sentence naming it by a class it loses: after the first click
/// the sentence names nothing, and the directive goes on answering alone. The two branches draw the
/// same paragraph, so a browser keeps the element and only its class changes.
const ONE_LISTENER_ENDS: &str = "\
## Component App
State:
    Let hot be true.
    Let a be 0.
    Let b be 0.
Actions:
    To one:
        Set a to a + 1.
        Set hot to false.
    When \".hot\" is clicked:
        Set b to b + 1.
View:
    {if hot:}<p id=\"go\" class=\"hot\" on-click={one}>t</p>{else:}<p id=\"go\" class=\"cold\" on-click={one}>t</p>{end}
    <span id=\"n\">{a}-{b}</span>
";

/// A placed component's directive and its own sentence on one element, the sentence naming it by a
/// class the page's button takes away. The component is drawn inside the page's paint, before its
/// markup is in the document, so a browser holds the sentence's listener as a registration of the
/// page's paint beside the element's own; the class is a hole, so the element is kept when it
/// changes — and the element's own listener must outlive the sentence's.
const A_PLACED_SENTENCE_ENDS: &str = "\
## Component Zone (look: Text)
State:
    Let a be 0.
    Let b be 0.
Actions:
    To one:
        Set a to a + 1.
    When \".hot\" is clicked:
        Set b to b + 1.
View:
    <div id=\"zone\" class={look} on-click={one}>z</div>
    <span id=\"n\">{a}-{b}</span>

## Component App
State:
    Let look be \"hot\".
Actions:
    To cool:
        Set look to \"cold\".
View:
    <button id=\"cool\" on-click={cool}>cool</button>
    <main><Zone look={look}/></main>
";

/// Two neighbours drawn one after the other: the input listens for the program's LAST entry and
/// the button right after it for its FIRST, so the input's slot is the one just below the button's
/// — a slot space short of the program's entries would make them one slot.
const NEIGHBOURS: &str = "\
## Component App
State:
    Let a be 0.
    Let b be 0.
Actions:
    To typed:
        Set a to a + 1.
    To clicked:
        Set b to b + 1.
View:
    <p id=\"n\">{a}-{b}</p>
    <button id=\"first\" on-click={clicked}>first</button>
    <input id=\"k\" aria-label=\"k\" on-keydown={typed}><button id=\"go\" on-click={clicked}>go</button>
";

fn clicked(page: &str, target: &str, times: usize) -> String {
    let clicks: String = (0..times)
        .map(|_| format!("Click \"{target}\".\n"))
        .collect();
    drawn(&format!("{page}\n## Main\nMount App.\n{clicks}"))
}

#[test]
fn a_directive_and_a_sentence_on_one_element_both_answer() {
    let doc = clicked(DIRECTIVE_AND_SENTENCE, "#go", 2);
    assert!(
        doc.contains("<p id=\"n\">2-2</p>"),
        "both listeners answered each click: {doc}"
    );
}

#[test]
fn two_sentences_waiting_for_one_event_on_one_element_both_answer() {
    let doc = clicked(TWO_SENTENCES, "#go", 2);
    assert!(
        doc.contains("<p id=\"n\">2-2</p>"),
        "both sentences answered each click: {doc}"
    );
}

#[test]
fn the_listeners_on_one_element_answer_in_the_reference_order() {
    let doc = clicked(THREE_IN_ORDER, "#go", 2);
    let trail = doc
        .split("<p id=\"n\">[")
        .nth(1)
        .and_then(|t| t.split(']').next())
        .unwrap_or("");
    assert_eq!(
        trail.len(),
        6,
        "all three listeners answered both clicks: {doc}"
    );
    assert_eq!(
        &trail[..3],
        &trail[3..],
        "each click ran the three in the same order: {doc}"
    );
}

#[test]
fn both_listeners_on_a_redrawn_element_answer_every_click() {
    let doc = clicked(REDRAWN_WITH_TWO_LISTENERS, "#go", 3);
    assert!(
        doc.contains("<p id=\"n\">3</p>"),
        "the sentence answered every click: {doc}"
    );
    assert!(
        doc.contains("<p>odd</p>"),
        "the directive answered every click: {doc}"
    );
}

#[test]
fn the_listener_that_stays_answers_after_the_other_ends() {
    let doc = clicked(ONE_LISTENER_ENDS, "#go", 2);
    assert!(
        doc.contains("<span id=\"n\">2-1</span>"),
        "the sentence answered once, the directive twice: {doc}"
    );
}

#[test]
fn neighbouring_elements_never_share_a_slot() {
    let doc = clicked(NEIGHBOURS, "#go", 1);
    assert!(
        doc.contains("<p id=\"n\">0-1</p>"),
        "a click on the button ran the button's listener alone: {doc}"
    );
}

#[test]
fn every_page_builds_for_the_browser() {
    for page in [
        NEIGHBOURS,
        DIRECTIVE_AND_SENTENCE,
        TWO_SENTENCES,
        THREE_IN_ORDER,
        REDRAWN_WITH_TWO_LISTENERS,
        A_ROW_LEAVES_EVERY_CLICK,
        ONE_LISTENER_ENDS,
    ] {
        let src = format!("{page}\n## Main\nMount App.\n");
        if let Err(e) = compile_to_wasm(&src) {
            panic!("the page builds for the browser: {e:?}\n{src}");
        }
    }
}

/// How many entries each of `globals` holds after `clicks` clicks on `#go`, on the lowered page the
/// bytecode VM runs.
fn tables_after(page: &str, clicks: usize, globals: &[&str]) -> Vec<usize> {
    use logicaffeine_compile::page_host::{Drive, LoweredPage};
    use logicaffeine_compile::vm::session::Session;
    use logicaffeine_compile::vm::{Compiler, Vm};
    let src = format!("{page}\n## Main\nMount App.\n");
    dom::reset();
    logicaffeine_compile::ui_bridge::with_parsed_program(&src, |parsed, interner| {
        let (stmts, types, policies) = parsed.unwrap_or_else(|e| panic!("must parse: {e}\n{src}"));
        let program = Compiler::compile_with_types(stmts, interner, Some(types))
            .unwrap_or_else(|e| panic!("must compile for the VM: {e}\n{src}"));
        let vm = Vm::new(&program).with_policy_ctx(&policies, interner);
        let mut page = LoweredPage::new(Session::new(vm, interner));
        page.start(None, |s| s.run_main())
            .unwrap_or_else(|e| panic!("main must not trap: {e:?}"));
        for _ in 0..clicks {
            page.drive(&Drive::Click {
                selector: "#go".to_string(),
            })
            .unwrap_or_else(|e| panic!("a click must not trap: {e:?}"));
        }
        globals
            .iter()
            .map(|g| {
                page.calls()
                    .global_len(g)
                    .unwrap_or_else(|| panic!("the page declares `{g}`"))
            })
            .collect()
    })
}

#[test]
fn a_redrawn_elements_spilled_listeners_are_retired_with_it() {
    let tables = ["__LISTENER_FN", "__LISTENER_MORE", "__LISTENER_MORE_AT"];
    let early = tables_after(REDRAWN_WITH_TWO_LISTENERS, 20, &tables);
    let late = tables_after(REDRAWN_WITH_TWO_LISTENERS, 200, &tables);
    assert_eq!(
        early, late,
        "{tables:?} held {early:?} after 20 redraws and {late:?} after 200"
    );
    assert!(
        early[1] >= 2,
        "the second listener spilled — the fixture reaches the spill: {early:?}"
    );
}

#[test]
fn an_instance_that_leaves_takes_its_listeners_with_it() {
    let tables = ["__LISTENER_FN", "__LISTENER_KEY"];
    let early = tables_after(A_ROW_LEAVES_EVERY_CLICK, 20, &tables);
    let late = tables_after(A_ROW_LEAVES_EVERY_CLICK, 200, &tables);
    assert_eq!(
        early, late,
        "{tables:?} held {early:?} after 20 departures and {late:?} after 200"
    );
}

fn chrome() -> Option<&'static Browser> {
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

/// The text `read` evaluates to after clicking `#go` `times` times in Chrome.
fn clicked_in_chrome(b: &Browser, page: &str, times: usize, read: &str) -> String {
    let src = format!("{page}\n## Main\nMount App.\n");
    let bundle = PageBundle::page(&src).unwrap_or_else(|e| panic!("the page bundles: {e}"));
    let p = b
        .open(&bundle)
        .unwrap_or_else(|e| panic!("the page opens: {e}"));
    for _ in 0..times {
        p.click("#go").unwrap_or_else(|e| panic!("click #go: {e}"));
    }
    let text = p.eval(read).unwrap();
    assert!(
        p.exceptions().is_empty(),
        "the page does not throw: {:?}",
        p.exceptions()
    );
    text.as_str().unwrap_or_default().to_string()
}

#[test]
fn every_listener_on_one_element_answers_in_a_browser() {
    let Some(b) = chrome() else { return };
    let read = "document.querySelector('#n').textContent";
    assert_eq!(
        clicked_in_chrome(b, DIRECTIVE_AND_SENTENCE, 2, read),
        "2-2",
        "directive and sentence"
    );
    assert_eq!(
        clicked_in_chrome(b, TWO_SENTENCES, 2, read),
        "2-2",
        "two sentences"
    );
    let reference = clicked(THREE_IN_ORDER, "#go", 2);
    let expected = reference
        .split("<p id=\"n\">")
        .nth(1)
        .and_then(|t| t.split("</p>").next())
        .unwrap();
    assert_eq!(
        clicked_in_chrome(b, THREE_IN_ORDER, 2, read),
        expected,
        "the reference order"
    );
    assert_eq!(
        clicked_in_chrome(b, REDRAWN_WITH_TWO_LISTENERS, 3, read),
        "3",
        "a redrawn element"
    );
    assert_eq!(
        clicked_in_chrome(b, ONE_LISTENER_ENDS, 2, read),
        "2-1",
        "the listener that stays"
    );
    assert_eq!(
        clicked_in_chrome(b, NEIGHBOURS, 1, read),
        "0-1",
        "neighbours"
    );
}

/// A browser only: the in-process tiers register a placed component's own sentences while its
/// markup is detached and hear none of them (`drop_target_lock`'s finding, closed when selector
/// sentences resolve at the paint's commit). The sentence answers the first click; the page's
/// button takes its class away; the directive answers both clicks on the element that stays.
#[test]
fn a_placed_components_own_listener_outlives_its_sentence_in_a_browser() {
    let Some(b) = chrome() else { return };
    let page = format!("{A_PLACED_SENTENCE_ENDS}\n## Main\nMount App.\n");
    let bundle = PageBundle::page(&page).unwrap_or_else(|e| panic!("the page bundles: {e}"));
    let p = b
        .open(&bundle)
        .unwrap_or_else(|e| panic!("the page opens: {e}"));
    for c in ["#zone", "#cool", "#zone"] {
        p.click(c).unwrap_or_else(|e| panic!("click {c}: {e}"));
    }
    let shown = p.eval("document.querySelector('#n').textContent").unwrap();
    assert_eq!(
        shown.as_str(),
        Some("2-1"),
        "the directive answered both clicks and the sentence the first"
    );
    assert!(
        p.exceptions().is_empty(),
        "the page does not throw: {:?}",
        p.exceptions()
    );
}
