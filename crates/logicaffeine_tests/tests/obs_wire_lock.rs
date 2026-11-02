//! W16 — `obs_wire_lock`: the `.logosobs` observation stream round-trips.
//!
//! An
//! emitted event stream serializes to a portable, line-oriented `.logosobs`
//! form (one JSON object per line — appendable, greppable) that parses back
//! byte-faithfully. This is the `largo trace` reader's format; columnar
//! batching (RLE/dict) is the wave's deferred optimization.

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::observe::{self, EventKind, ObsCat, ObsEvent};

/// Emitting a program, serializing, and re-parsing recovers the exact stream.
#[test]
fn logosobs_round_trips() {
    observe::install();
    let out = tw_outcome(
        "## Main\n\
         \x20   Let n be 5.\n\
         \x20   Note that the cache missed with n.\n\
         \x20   During work:\n\
         \x20       Warn that a retry happened.\n\
         \x20   Record that the run finished.\n",
    );
    assert_eq!(out.error, None, "{:?}", out.error);
    let events = observe::drain();
    assert!(!events.is_empty(), "the program emitted events");

    let wire = observe::to_logosobs(&events);
    let back = observe::from_logosobs(&wire).expect("the wire parses back");
    assert_eq!(
        back, events,
        "the .logosobs stream is a lossless round-trip"
    );

    // Line-oriented: one JSON object per event, greppable.
    assert_eq!(
        wire.lines().count(),
        events.len(),
        "one line per event — appendable and greppable"
    );
}

/// The wire form is stable JSON a reader without the program can render.
#[test]
fn wire_carries_kind_message_and_payloads() {
    let events = vec![
        ObsEvent {
            kind: EventKind::Message(ObsCat::Warn),
            message: "the disk is full".to_string(),
            payloads: vec![("free".to_string(), "0".to_string())],
            seq: 0,
        },
        ObsEvent {
            kind: EventKind::SpanEnter,
            message: "flushing".to_string(),
            payloads: vec![],
            seq: 1,
        },
    ];
    let wire = observe::to_logosobs(&events);
    assert!(
        wire.contains("\"warn\""),
        "the category is on the wire: {wire}"
    );
    assert!(
        wire.contains("the disk is full"),
        "the message is on the wire"
    );
    assert!(
        wire.contains("span_enter"),
        "span boundaries are on the wire"
    );
    assert_eq!(observe::from_logosobs(&wire).unwrap(), events);
}
