//! W8 — `obs_span_balance_lock`: trace spans are lexical and always balanced.
//!
//! `During`
//! opens a lexical span: it emits a `SpanEnter` before its body and a matching
//! `SpanExit` after — on EVERY exit path (normal, early control flow, error),
//! so a span can never leak. Message events emitted inside the span sit between
//! the two boundaries. Program output is untouched (the two-channel contract).

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::observe::{self, EventKind, ObsEvent};

fn run_capturing(src: &str) -> (String, Vec<ObsEvent>) {
    observe::install();
    let out = tw_outcome(src);
    let events = observe::drain();
    (out.output, events)
}

/// Every `SpanEnter` is matched by exactly one `SpanExit`, and message events
/// emitted inside the span fall between them.
#[test]
fn span_brackets_its_body() {
    let (stdout, events) = run_capturing(
        "## Main\n\
         \x20   Note that before the span.\n\
         \x20   During parsing the file:\n\
         \x20       Let x be 1.\n\
         \x20       Note that inside the span.\n\
         \x20   Note that after the span.\n\
         \x20   Show 9.\n",
    );
    assert_eq!(stdout, "9", "program output is untouched by spans");

    let kinds: Vec<EventKind> = events.iter().map(|e| e.kind).collect();
    use logicaffeine_compile::observe::ObsCat::Note;
    assert_eq!(
        kinds,
        vec![
            EventKind::Message(Note), // before
            EventKind::SpanEnter,     // During opens
            EventKind::Message(Note), // inside
            EventKind::SpanExit,      // During closes
            EventKind::Message(Note), // after
        ],
        "the span brackets exactly the events emitted within its body"
    );
    let enters = events
        .iter()
        .filter(|e| e.kind == EventKind::SpanEnter)
        .count();
    let exits = events
        .iter()
        .filter(|e| e.kind == EventKind::SpanExit)
        .count();
    assert_eq!(enters, exits, "every enter has exactly one exit");
    // The enter and exit name the same span.
    let enter = events
        .iter()
        .find(|e| e.kind == EventKind::SpanEnter)
        .unwrap();
    let exit = events
        .iter()
        .find(|e| e.kind == EventKind::SpanExit)
        .unwrap();
    assert_eq!(
        enter.message, exit.message,
        "the span boundaries name the same span"
    );
    assert_eq!(enter.message, "parsing the file");
}

/// A span exits even when its body breaks early — the exit is on every path.
#[test]
fn span_exits_on_early_break() {
    let (_stdout, events) = run_capturing(
        "## Main\n\
         \x20   Repeat for i from 1 to 3:\n\
         \x20       During one iteration:\n\
         \x20           Note that a step ran.\n\
         \x20           Break.\n",
    );
    let enters = events
        .iter()
        .filter(|e| e.kind == EventKind::SpanEnter)
        .count();
    let exits = events
        .iter()
        .filter(|e| e.kind == EventKind::SpanExit)
        .count();
    assert!(enters >= 1, "the span opened");
    assert_eq!(
        enters, exits,
        "an early Break inside a span still closes it"
    );
}

/// Nested spans nest correctly (a stack, not a tangle).
#[test]
fn nested_spans_nest() {
    let (_stdout, events) = run_capturing(
        "## Main\n\
         \x20   During outer work:\n\
         \x20       During inner work:\n\
         \x20           Note that the innermost ran.\n",
    );
    let kinds: Vec<EventKind> = events.iter().map(|e| e.kind).collect();
    use logicaffeine_compile::observe::ObsCat::Note;
    assert_eq!(
        kinds,
        vec![
            EventKind::SpanEnter, // outer
            EventKind::SpanEnter, // inner
            EventKind::Message(Note),
            EventKind::SpanExit, // inner closes first
            EventKind::SpanExit, // outer closes last
        ],
        "nested spans close inner-first (a proper stack)"
    );
}
