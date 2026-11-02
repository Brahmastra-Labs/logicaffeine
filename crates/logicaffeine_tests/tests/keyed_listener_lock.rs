//! ★ LOCK — one element waits for several keys, and each key answers its own listener. ★
//!
//! A listener waiting for one key is an entry of its own in the page's listener table — its event
//! and its key, next to the element's listener for that event that waits for any key — so an
//! `<input>` can answer Enter one way and Escape another, and a press reaches the listener waiting
//! for that key AND the one waiting for any, as the reference tier runs every listener a press
//! reaches. Each row drives the page on the tree-walker and the bytecode VM and holds both to the
//! same document; the last drives it in Chrome.

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
        "the two tiers answer the same presses:\n{src}"
    );
    tw_doc
}

const TWO_KEY_SENTENCES: &str = "\
## Component App
State:
    Let n be 0.
    Let escaped be 0.
Actions:
    When the user presses Enter in \"#draft\":
        Set n to n + 1.
    When the user presses Escape in \"#draft\":
        Set escaped to escaped + 1.
View:
    <p id=\"n\">{n}-{escaped}</p>
    <input id=\"draft\" aria-label=\"draft\">
";

const TWO_KEY_DIRECTIVES: &str = "\
## Component App
State:
    Let n be 0.
    Let escaped be 0.
Actions:
    To enter:
        Set n to n + 1.
    To leave:
        Set escaped to escaped + 1.
View:
    <p id=\"n\">{n}-{escaped}</p>
    <input id=\"draft\" aria-label=\"draft\" on-keydown.enter={enter} on-keydown.escape={leave}>
";

/// A listener waiting for any key beside one waiting for Enter, on one element.
const ANY_AND_ENTER: &str = "\
## Component App
State:
    Let any be 0.
    Let n be 0.
Actions:
    To anyKey:
        Set any to any + 1.
    When the user presses Enter in \"#draft\":
        Set n to n + 1.
View:
    <p id=\"n\">{any}-{n}</p>
    <input id=\"draft\" aria-label=\"draft\" on-keydown={anyKey}>
";

fn pressed(page: &str, keys: &[&str]) -> String {
    let presses: String = keys
        .iter()
        .map(|k| format!("Press \"{k}\" in \"#draft\".\n"))
        .collect();
    drawn(&format!("{page}\n## Main\nMount App.\n{presses}"))
}

#[test]
fn two_key_sentences_on_one_element_each_answer_their_own_key() {
    assert!(
        pressed(TWO_KEY_SENTENCES, &["Enter", "Escape", "Enter"]).contains("<p id=\"n\">2-1</p>")
    );
}

#[test]
fn two_key_directives_on_one_element_each_answer_their_own_key() {
    assert!(
        pressed(TWO_KEY_DIRECTIVES, &["Escape", "Enter", "Escape"]).contains("<p id=\"n\">1-2</p>")
    );
}

#[test]
fn a_key_answers_its_own_listener_and_the_one_waiting_for_any_key() {
    assert!(pressed(ANY_AND_ENTER, &["Enter", "a"]).contains("<p id=\"n\">2-1</p>"));
}

#[test]
fn every_keyed_page_builds_for_the_browser() {
    for page in [TWO_KEY_SENTENCES, TWO_KEY_DIRECTIVES, ANY_AND_ENTER] {
        let src = format!("{page}\n## Main\nMount App.\n");
        if let Err(e) = compile_to_wasm(&src) {
            panic!("the page builds for the browser: {e:?}\n{src}");
        }
    }
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

#[test]
fn each_key_answers_its_own_listener_in_a_browser() {
    let Some(b) = chrome() else { return };
    for (page, keys, shown) in [
        (TWO_KEY_SENTENCES, ["Enter", "Escape", "Enter"], "2-1"),
        (TWO_KEY_DIRECTIVES, ["Escape", "Enter", "Escape"], "1-2"),
    ] {
        let src = format!("{page}\n## Main\nMount App.\n");
        let bundle = PageBundle::page(&src).unwrap_or_else(|e| panic!("the page bundles: {e}"));
        let p = b
            .open(&bundle)
            .unwrap_or_else(|e| panic!("the page opens: {e}"));
        for key in keys {
            p.press_in("#draft", key)
                .unwrap_or_else(|e| panic!("press {key}: {e}"));
        }
        let text = p.eval("document.querySelector('#n').textContent").unwrap();
        assert_eq!(
            text.as_str(),
            Some(shown),
            "each key answered its own listener in Chrome"
        );
        assert!(
            p.exceptions().is_empty(),
            "the page does not throw: {:?}",
            p.exceptions()
        );
    }
}

#[test]
fn a_key_answers_its_own_listener_and_the_one_waiting_for_any_key_in_a_browser() {
    let Some(b) = chrome() else { return };
    let src = format!("{ANY_AND_ENTER}\n## Main\nMount App.\n");
    let bundle = PageBundle::page(&src).unwrap_or_else(|e| panic!("the page bundles: {e}"));
    let p = b
        .open(&bundle)
        .unwrap_or_else(|e| panic!("the page opens: {e}"));
    for key in ["Enter", "a"] {
        p.press_in("#draft", key)
            .unwrap_or_else(|e| panic!("press {key}: {e}"));
    }
    let text = p.eval("document.querySelector('#n').textContent").unwrap();
    assert_eq!(
        text.as_str(),
        Some("2-1"),
        "Enter answered both listeners and `a` the one waiting for any key"
    );
    assert!(
        p.exceptions().is_empty(),
        "the page does not throw: {:?}",
        p.exceptions()
    );
}
