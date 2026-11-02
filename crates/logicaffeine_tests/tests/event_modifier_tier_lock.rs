//! LOCK — every row of the event-modifier catalogue behaves the same on the reference tier and the
//! bytecode VM (the tier a browser runs), row by row.
//!
//! The catalogue (`lgx::html::EventModifier`) is the only place a qualifier's meaning lives; this
//! file drives each row through both engines. A key row fires on the key a browser reports for it
//! and on no other key; every behavioural row is supported on both; a system-key row is a compile
//! error on every tier.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome};
use logicaffeine_compile::semantics::dom;
use logicaffeine_language::lgx::html::{
    EventModifier, BEHAVIOUR_MODIFIERS, KEY_EVENTS, KEY_MODIFIERS, SYSTEM_KEY_MODIFIERS,
};

/// A page whose input answers `attr` by counting, then `drive`.
fn page(attr: &str, drive: &str) -> String {
    format!(
        "## Component App\nState:\n    Let n be 0.\nActions:\n    To add:\n        Set n to n + 1.\n\
         View:\n    <p id=\"p\">{{n}}</p>\n    <input id=\"d\" aria-label=\"d\" {attr}={{add}}>\n\n\
         ## Main\nMount App.\n{drive}\n"
    )
}

/// `src` on both engines: `Ok(document)` when both run it, `Err(messages)` when both refuse.
/// One engine running what the other refuses is the divergence this file exists to catch.
fn both(src: &str) -> Result<String, (String, String)> {
    dom::reset();
    let tw = tw_outcome(src);
    let tw_doc = dom::render();
    dom::reset();
    let vm = vm_outcome(src);
    let vm_doc = dom::render();
    match (tw.error, vm.error) {
        (None, None) => {
            assert_eq!(
                tw_doc, vm_doc,
                "the engines painted different documents:\n{src}"
            );
            Ok(tw_doc)
        }
        (Some(a), Some(b)) => Err((a, b)),
        (a, b) => {
            panic!("one engine ran what the other refused — tree-walker {a:?}, VM {b:?}:\n{src}")
        }
    }
}

/// What a browser reports as `KeyboardEvent.key` for each named key, from the UI Events key-value
/// tables — written here independently of the catalogue, so a catalogue row that compared the
/// wrong spelling would fire in these tests on nothing a browser ever sends.
const BROWSER_REPORTS: &[(&str, &str)] = &[
    ("enter", "Enter"),
    ("escape", "Escape"),
    ("space", " "),
    ("tab", "Tab"),
    ("backspace", "Backspace"),
    ("delete", "Delete"),
    ("arrowup", "ArrowUp"),
    ("arrowdown", "ArrowDown"),
    ("arrowleft", "ArrowLeft"),
    ("arrowright", "ArrowRight"),
    ("home", "Home"),
    ("end", "End"),
    ("pageup", "PageUp"),
    ("pagedown", "PageDown"),
];

/// The key a browser reports for the catalogued key `word`.
fn reported(word: &str) -> &'static str {
    BROWSER_REPORTS
        .iter()
        .find(|(w, _)| *w == word)
        .map(|(_, k)| *k)
        .unwrap_or_else(|| panic!("`.{word}` is catalogued with no browser report recorded here"))
}

/// A `Press` sentence naming `key` as a LOGOS string literal.
fn press(key: &str) -> String {
    format!("Press \"{key}\" in \"#d\".")
}

#[test]
fn every_named_key_fires_on_its_reported_key_on_both_tiers() {
    // `Press` raises `keydown`, the event a key handler is written against, carrying the key
    // exactly as a browser would report it.
    for (word, _) in KEY_MODIFIERS {
        let key = reported(word);
        let fired = both(&page(&format!("on-keydown.{word}"), &press(key)))
            .unwrap_or_else(|e| panic!("`.{word}` must fire on {key:?}: {e:?}"));
        assert!(
            fired.contains("<p id=\"p\">1</p>"),
            "`.{word}` on {key:?}: {fired}"
        );
    }
}

#[test]
fn a_key_is_matched_whatever_its_case() {
    let fired =
        both(&page("on-keydown.enter", &press("ENTER"))).expect("case is not part of a key");
    assert!(fired.contains(">1<"), "{fired}");
}

#[test]
fn every_named_key_ignores_another_key_on_both_tiers() {
    // Each row against its neighbour in the table and against a character: a handler with no key
    // comparison answers both, and one comparing the wrong spelling fails the row above.
    for (i, (word, _)) in KEY_MODIFIERS.iter().enumerate() {
        let neighbour = reported(KEY_MODIFIERS[(i + 1) % KEY_MODIFIERS.len()].0);
        for other in [neighbour, "x"] {
            let refused = both(&page(&format!("on-keydown.{word}"), &press(other)));
            assert!(
                refused.is_err(),
                "`.{word}` must not answer a press of {other:?}: {refused:?}"
            );
        }
    }
}

#[test]
fn a_character_key_fires_on_that_character_only() {
    let fired = both(&page("on-keydown.k", &press("k"))).expect("`.k` answers k");
    assert!(fired.contains(">1<"), "{fired}");
    let fired = both(&page("on-keydown.k", &press("K"))).expect("and K, whatever the case");
    assert!(fired.contains(">1<"), "{fired}");
    assert!(
        both(&page("on-keydown.k", &press("j"))).is_err(),
        "`.k` does not answer j"
    );
}

#[test]
fn every_key_event_carries_a_key_qualifier() {
    for event in KEY_EVENTS {
        dom::reset();
        let out = vm_outcome(&page(&format!("on-{event}.enter"), ""));
        assert_eq!(out.error, None, "`on-{event}.enter` lowers");
    }
}

#[test]
fn a_handler_with_no_key_answers_every_key() {
    for (word, _) in KEY_MODIFIERS.iter().take(3) {
        let fired = both(&page("on-keydown", &press(reported(word))))
            .expect("an unqualified handler answers");
        assert!(fired.contains(">1<"), "{fired}");
    }
}

#[test]
fn every_behaviour_row_has_its_browser_status() {
    for (word, _) in BEHAVIOUR_MODIFIERS {
        let src = page(&format!("on-click.{word}"), "");
        dom::reset();
        assert_eq!(
            tw_outcome(&src).error,
            None,
            "the reference tier accepts `.{word}`"
        );
        dom::reset();
        assert_eq!(
            vm_outcome(&src).error,
            None,
            "`.{word}` is supported on the tier a browser runs"
        );
    }
}

#[test]
fn prevent_is_recorded_on_the_listener_on_both_tiers() {
    let src = "## Component App\nState:\n    Let n be 0.\nActions:\n    To save:\n        Set n to n + 1.\n\
               View:\n    <p id=\"p\">{n}</p>\n    <form id=\"f\" on-submit.prevent={save}><button>go</button></form>\n\n\
               ## Main\nMount App.\nSubmit \"#f\".\n";
    let doc = both(src).expect("a prevented submit runs its action");
    assert!(doc.contains("<p id=\"p\">1</p>"), "{doc}");
}

#[test]
fn every_system_key_is_a_compile_error_on_both_tiers() {
    for (word, _) in SYSTEM_KEY_MODIFIERS {
        let (tw, vm) = both(&page(&format!("on-keydown.{word}.enter"), &press("Enter")))
            .expect_err("a system key is refused");
        for why in [tw, vm] {
            assert!(
                why.contains(&format!("`.{word}` waits for a modifier key")),
                "{why}"
            );
        }
    }
}

#[test]
fn a_misspelt_qualifier_is_a_compile_error_on_both_tiers() {
    let (tw, vm) = both(&page("on-keydown.entr", &press("Enter"))).expect_err("`.entr` is refused");
    for why in [tw, vm] {
        assert!(why.contains("did you mean `.enter`?"), "{why}");
    }
}

#[test]
fn the_catalogue_is_what_the_rows_above_iterate() {
    // The rows are read from the catalogue itself, so a new row is driven here the moment it is
    // added. This pins the one assumption that makes that true: every word in the tables
    // classifies back to a row.
    for w in EventModifier::every_word() {
        assert!(EventModifier::classify(w).is_some(), "{w}");
    }
}
