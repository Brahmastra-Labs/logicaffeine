//! W7 — `obs_emission_lock`: observation statements emit structured events to
//! the SEPARATE observation channel.
//!
//! A `Note`/`Warn`/`Record` executes into an [`ObsEvent`] carrying its category,
//! that-clause message, and captured payload values — while program output
//! (`Show`) stays byte-identical (the two-channel contract). Sampling is
//! deterministic and per-site. When no sink is capturing, emission is a no-op.

use logicaffeine_compile::compile::tw_outcome;
use logicaffeine_compile::observe::{self, EventKind, ObsCat};

/// The message category of an event, or panic if it is a span boundary.
fn cat(e: &observe::ObsEvent) -> ObsCat {
    match e.kind {
        EventKind::Message(c) => c,
        other => panic!("expected a message event, got {other:?}"),
    }
}

/// Run `src` on the tree-walker with observation capture installed; return
/// (stdout, events).
fn run_capturing(src: &str) -> (String, Vec<observe::ObsEvent>) {
    observe::install();
    let out = tw_outcome(src);
    let events = observe::drain();
    assert_eq!(out.error, None, "the program runs cleanly: {:?}", out.error);
    (out.output, events)
}

/// A `Note` emits one structured event; stdout is untouched.
#[test]
fn note_emits_a_structured_event() {
    let (stdout, events) = run_capturing(
        "## Main\n\x20   Let n be 5.\n\
         \x20   Note that the cache missed with n.\n\x20   Show 1.\n",
    );
    assert_eq!(
        stdout, "1",
        "program output is on its own channel, untouched"
    );
    assert_eq!(events.len(), 1, "exactly one observation event");
    assert_eq!(cat(&events[0]), ObsCat::Note);
    assert_eq!(events[0].message, "the cache missed");
    assert_eq!(
        events[0].payloads,
        vec![("n".to_string(), "5".to_string())],
        "the payload value is captured at emission"
    );
}

/// The three categories are distinguished on the wire.
#[test]
fn categories_are_distinguished() {
    let (_stdout, events) = run_capturing(
        "## Main\n\
         \x20   Note that a thing happened.\n\
         \x20   Warn that a worse thing happened.\n\
         \x20   Record that the audit thing happened.\n",
    );
    let cats: Vec<ObsCat> = events.iter().map(cat).collect();
    assert_eq!(cats, vec![ObsCat::Note, ObsCat::Warn, ObsCat::Record]);
    assert_eq!(events[0].seq, 0, "sequence numbers are the emission order");
    assert_eq!(events[2].seq, 2);
}

/// `once in every N times` keeps 1-in-N deterministically (the first, then
/// every Nth) — sampling is a rendering decision, not evidence loss.
#[test]
fn sampling_keeps_one_in_n() {
    let (_stdout, events) = run_capturing(
        "## Main\n\
         \x20   Repeat for i from 1 to 6:\n\
         \x20       Note once in every 3 times that the packet arrived.\n",
    );
    assert_eq!(events.len(), 2, "6 reaches, 1-in-3 kept ⇒ 2 events");
}

/// No sink installed ⇒ emission is a silent no-op (no capture, no crash).
#[test]
fn no_capture_is_a_no_op() {
    // Deliberately NOT installing a buffer.
    let out = tw_outcome("## Main\n\x20   Note that nobody is listening.\n\x20   Show 7.\n");
    assert_eq!(out.error, None);
    assert_eq!(out.output, "7", "the program runs; the note goes nowhere");
    // A subsequent capturing run is unaffected by the un-captured one.
    let (_stdout, events) = run_capturing("## Main\n\x20   Note that now someone listens.\n");
    assert_eq!(events.len(), 1);
}
