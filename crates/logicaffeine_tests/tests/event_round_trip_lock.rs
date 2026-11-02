//! ★ LOCK — every event a handler can WAIT FOR, a test can CAUSE. ★
//!
//! `SELECTOR_EVENTS` lists the six English predicates a handler may name — `clicked`, `submitted`,
//! `focused`, `blurred`, `hovered`, `changed` — and nothing anywhere said which of them a `## Test`
//! can actually make happen. Three could not:
//!
//! | event | declare | drive |
//! |---|---|---|
//! | `clicked` | yes | `Click` |
//! | `submitted` | yes | `Submit` |
//! | `changed` | yes | `Choose` / `Type into` |
//! | `focused` | **REFUSED** | — |
//! | `blurred` | yes | **nothing** |
//! | `hovered` | yes | **nothing** |
//!
//! ★ `focused` COULD NOT BE DECLARED AT ALL, and the reason is a grammar collision: `"#b" is
//! focused` is also a page QUESTION (`DomQuery::Focused`), so the trigger's selector — parsed as a
//! full expression — swallowed the whole clause and `When "#b" is focused:` read as an expression
//! followed by a stray colon. `blurred` and `hovered` name no page question, which is the only
//! reason they escaped it.
//!
//! ★ AND `Hover` WAS ALLOWLISTED WITH NOTHING BEHIND IT. Two hand-maintained lists describe the
//! driving verbs — the gate in `parse_statement` that decides whether a sentence may open a UI
//! statement, and the verb table in `parse_ui_statement` that reads one — and they had drifted in
//! BOTH directions: `Hover` was in the gate and had no branch, `Focus` had a branch and was not in
//! the gate. Each half looked complete on its own.
//!
//! So a handler waiting on a hover, a blur or a focus was unreachable from any test — the page
//! could be wrong in that path forever and every suite stayed green. This file is the round trip:
//! for each event, DECLARE a handler, DRIVE it, and require the action to have run.

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::semantics::dom;

/// Every declarable event, with the sentence that causes it.
///
/// ★ THE CATALOGUE, and the reason it is a table rather than six tests: an event added to
/// `SELECTOR_EVENTS` with no way to cause it is exactly the hole this file exists to close, and
/// `every_declarable_event_is_in_this_table` re-reads the parser's own list to prove none is
/// missing here.
const ROUND_TRIP: &[(&str, &str)] = &[
    ("clicked", "Click \"#b\"."),
    ("submitted", "Submit \"#b\"."),
    ("changed", "Type \"x\" into \"#b\"."),
    ("focused", "Focus \"#b\"."),
    // Blurring takes focus away, so it raises `blur` only where there was focus to take — as a
    // browser's `element.blur()` does.
    ("blurred", "Focus \"#b\".\nBlur \"#b\"."),
    ("hovered", "Hover \"#b\"."),
    // ★ THESE TWO ARRIVED WITH THE VOCABULARY MERGE. There were two English→event maps and they
    // DISAGREED: the trigger parser knew `typed` and `keyed`, the selector parser knew `focused`
    // and `blurred`, and each was the only reader of its own copy. Merging them into one
    // `SELECTOR_EVENTS` is what made this file demand a driver for the two that had never been
    // asked for one — which is the anti-staleness half doing precisely its job.
    ("typed", "Type \"x\" into \"#b\"."),
    ("keyed", "Press \"a\" in \"#b\"."),
];

/// Declare a handler for `event`, cause it with `verb`, and report what the page shows.
fn round_trip(event: &str, verb: &str) -> String {
    let src = format!(
        "## Component App\nState:\n\x20   Let n be 0.\n\
         Actions:\n\x20   When \"#b\" is {event}:\n\x20       Set n to n + 1.\n\
         View:\n\x20   <input id=\"b\">\n\x20   <p id=\"c\">{{n}}</p>\n\n\
         ## Main\nMount App.\n{verb}\nShow the text of \"#c\".\n"
    );
    dom::reset();
    let out = tw_outcome(&src);
    assert_eq!(
        out.error, None,
        "`{event}` driven by `{verb}` must run:\n{src}"
    );
    out.output.trim().to_string()
}

#[test]
fn every_event_a_handler_can_wait_for_can_be_caused() {
    for (event, verb) in ROUND_TRIP {
        assert_eq!(
            round_trip(event, verb),
            "1",
            "a handler waiting for `{event}` must run when a test writes `{verb}` — an event that \
             can be declared and not caused is a code path no suite can ever reach"
        );
    }
}

#[test]
fn every_declarable_event_is_in_this_table() {
    // ★ THE ANTI-STALENESS HALF. This file would still pass if the parser grew a seventh event
    // nobody could drive, so the parser's own list is read back and compared. It is the same
    // discipline `action_verb_tier_lock` holds over `UiStmt`: silently absent is not an option.
    let src = std::fs::read_to_string(
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../logicaffeine_language/src/parser/component.rs"),
    )
    .expect("the parser source is readable");
    let table = src
        .split("const SELECTOR_EVENTS")
        .nth(1)
        .and_then(|s| s.split("];").next())
        .expect("SELECTOR_EVENTS is declared there");
    let declared: Vec<String> = table
        .match_indices("(\"")
        .filter_map(|(at, _)| table[at + 2..].split('"').next().map(str::to_string))
        .collect();
    assert!(
        declared.len() >= 6,
        "the event table scan found only {declared:?} — it has stopped matching rather than the \
         parser having stopped declaring events"
    );
    for event in &declared {
        assert!(
            ROUND_TRIP.iter().any(|(e, _)| e == event),
            "`{event}` can be waited for and this file names no way to cause it. Add the driving \
             sentence here — an event with no driver is a handler no test can ever reach:\n\
             known: {ROUND_TRIP:?}"
        );
    }
}

#[test]
fn the_round_trip_can_fail() {
    // ★ ANTI-VACUITY. If the handler never ran, or `the text of` always answered "1", every
    // assertion above would pass. Prove the un-driven page shows 0.
    let src = "## Component App\nState:\n\x20   Let n be 0.\n\
               Actions:\n\x20   When \"#b\" is clicked:\n\x20       Set n to n + 1.\n\
               View:\n\x20   <input id=\"b\">\n\x20   <p id=\"c\">{n}</p>\n\n\
               ## Main\nMount App.\nShow the text of \"#c\".\n";
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "{src}");
    assert_eq!(
        out.output.trim(),
        "0",
        "an undriven page must show 0, or `1` proves nothing"
    );
}

fn undriven(verb: &str) -> Option<String> {
    let src = format!(
        "## Component App\nState:\n\x20   Let n be 0.\n\
         View:\n\x20   <input id=\"b\">\n\n## Main\nMount App.\n{verb}\nShow \"ok\".\n"
    );
    dom::reset();
    tw_outcome(&src).error
}

#[test]
fn hovering_something_that_answers_nothing_is_refused() {
    // ★ A HOVER IS ONLY EVER A TEST SPEAKING. A page cannot hover over itself, so the sentence
    // means "the visitor's pointer went here" — and an element that answers nothing means the test
    // named the wrong one. That is the rule `Click` already follows.
    let err = undriven("Hover \"#b\".")
        .unwrap_or_else(|| panic!("hovering an element that answers nothing must say so"));
    assert!(err.contains("#b"), "and name the element: {err}");
}

#[test]
fn focusing_and_blurring_are_actions_and_need_no_answer() {
    // ★ THE OTHER HALF OF THE SAME DISTINCTION, and it is not a preference. `Focus` is a DOM
    // ACTION a page performs on itself — a form putting the cursor in its own first field answers
    // nothing, and `element_state_lock` writes exactly that. Requiring an answer would turn every
    // existing use of it into an error. Blur is its complement.
    for verb in ["Focus \"#b\".", "Blur \"#b\"."] {
        assert_eq!(
            undriven(verb),
            None,
            "`{verb}` is something a page does, so an element with no handler is not a mistake"
        );
    }
}

#[test]
fn focus_moves_the_cursor_and_blur_takes_it_away() {
    // The action half, asserted on its own: raising the event must not have replaced the effect.
    let src = "## Component App\nState:\n\x20   Let n be 0.\n\
               View:\n\x20   <input id=\"b\">\n\n## Main\nMount App.\n\
               Show \"#b\" is focused.\nFocus \"#b\".\nShow \"#b\" is focused.\n\
               Blur \"#b\".\nShow \"#b\" is focused.\n";
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "{src}");
    assert_eq!(out.output.trim(), "false\ntrue\nfalse", "{src}");
}

#[test]
fn the_page_question_and_the_trigger_do_not_collide() {
    // `"#b" is focused` is BOTH a question a test asks and an event a handler waits for. Fixing
    // one must not take the other: this is the pair that collided.
    let src = "## Component App\nState:\n\x20   Let n be 0.\n\
               Actions:\n\x20   When \"#b\" is focused:\n\x20       Set n to n + 1.\n\
               View:\n\x20   <input id=\"b\">\n\n\
               ## Main\nMount App.\nShow \"#b\" is focused.\nFocus \"#b\".\nShow \"#b\" is focused.\n";
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "{src}");
    assert_eq!(
        out.output.trim(),
        "false\ntrue",
        "the QUESTION must still answer, and focusing must change its answer:\n{src}"
    );
}

/// ★ CHOOSING IS A CHANGE TOO, on the control a browser lets a visitor choose in: a `<select>` that
/// offers the option. (`#b` in the table is a text `<input>`, whose change is caused by typing —
/// choosing in it selects nothing, and the driver refuses it by name.)
#[test]
fn choosing_an_offered_option_is_a_change() {
    let src = "## Component App\nState:\n    Let n be 0.\n\
        Actions:\n    When \"#b\" is changed:\n        Set n to n + 1.\n\
        View:\n    <select id=\"b\"><option>w</option><option>x</option></select>\n    <p id=\"c\">{n}</p>\n\n\
        ## Main\nMount App.\nChoose \"x\" in \"#b\".\nShow the text of \"#c\".\n";
    dom::reset();
    let out = tw_outcome(src);
    assert_eq!(
        out.error, None,
        "choosing an offered option must run:\n{src}"
    );
    assert_eq!(out.output.trim(), "1");
}
