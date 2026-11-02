//! W13 — `obs_vm_emission_lock`: the register VM EMITS observation events, and
//! its stream is byte-for-byte the tree-walker's.
//!
//! This is the Wave-13 emission floor RISING: the differential floor
//! (`obs_tier_differential`) proved the VM *elides* observation without dropping
//! the program's work; here the VM compiles in the OBSERVED profile
//! (`vm_outcome_observed`) and emits the SAME event stream the tree-walker does —
//! same categories, same that-clauses, same payload values, same balanced spans.
//! Now two tiers agree on the observation channel, not just program output. The
//! seq counter (a per-tier cursor) is the one field allowed to differ; the
//! observed CONTENT must be identical.

use logicaffeine_compile::compile::{tw_outcome, vm_outcome, vm_outcome_observed};
use logicaffeine_compile::observe::{self, EventKind, ObsEvent};

/// Nested spans, observation inside a loop, payloads over mutable state — the
/// demanding shape.
const OBSERVED: &str = "## Main\n\
    \x20   Let mutable total be 0.\n\
    \x20   Note that the run began with total.\n\
    \x20   During accumulating the values:\n\
    \x20       Repeat for i from 1 to 4:\n\
    \x20           Set total to total + i.\n\
    \x20           During folding one step:\n\
    \x20               Record that a step folded with i and total.\n\
    \x20   Warn that the fold is complete with total.\n\
    \x20   Show total.\n";

/// The identity-comparable content of an event (everything but the per-tier seq).
fn fingerprint(e: &ObsEvent) -> (String, String, Vec<(String, String)>) {
    let tag = match e.kind {
        EventKind::Message(cat) => format!("Message::{cat:?}"),
        EventKind::SpanEnter => "SpanEnter".to_string(),
        EventKind::SpanExit => "SpanExit".to_string(),
    };
    (tag, e.message.clone(), e.payloads.clone())
}

fn tw_events(src: &str) -> Vec<ObsEvent> {
    observe::install();
    let out = tw_outcome(src);
    assert_eq!(out.error, None, "tree-walker runs cleanly: {:?}", out.error);
    observe::drain()
}

fn vm_events(src: &str) -> Vec<ObsEvent> {
    observe::install();
    let out = vm_outcome_observed(src);
    assert_eq!(out.error, None, "observed VM runs cleanly: {:?}", out.error);
    observe::drain()
}

/// The headline: the VM's observed event stream is identical to the
/// tree-walker's, event for event (modulo the per-tier seq cursor).
#[test]
fn vm_emits_the_tree_walkers_full_event_stream() {
    let tw: Vec<_> = tw_events(OBSERVED).iter().map(fingerprint).collect();
    let vm: Vec<_> = vm_events(OBSERVED).iter().map(fingerprint).collect();
    assert_eq!(
        vm, tw,
        "the VM must emit exactly the tree-walker's observation stream"
    );

    // And it is the substantive stream we expect, not two empty vecs agreeing.
    let messages = tw
        .iter()
        .filter(|(t, _, _)| t.starts_with("Message"))
        .count();
    let enters = tw.iter().filter(|(t, _, _)| t == "SpanEnter").count();
    assert_eq!(messages, 6, "1 Note + 4 Records (loop) + 1 Warn");
    assert_eq!(enters, 5, "outer During + 4 inner During");
}

/// The payload VALUES the VM captured are the live runtime values, matching the
/// tree-walker's — proof the VM evaluated the payload expressions, not just the
/// names. The final `Warn … with total` carries total = 10.
#[test]
fn vm_captures_live_payload_values() {
    let vm = vm_events(OBSERVED);
    let warn = vm
        .iter()
        .find(|e| matches!(e.kind, EventKind::Message(observe::ObsCat::Warn)))
        .expect("the Warn fired");
    assert_eq!(warn.message, "the fold is complete");
    assert_eq!(warn.payloads, vec![("total".to_string(), "10".to_string())]);
}

/// Spans are balanced on the VM (every `SpanEnter` has its `SpanExit`), exactly
/// as on the tree-walker — the `During` bracket survives compilation.
#[test]
fn vm_spans_are_balanced() {
    let vm = vm_events(OBSERVED);
    let enters = vm
        .iter()
        .filter(|e| matches!(e.kind, EventKind::SpanEnter))
        .count();
    let exits = vm
        .iter()
        .filter(|e| matches!(e.kind, EventKind::SpanExit))
        .count();
    assert_eq!(enters, exits, "balanced spans on the VM");
    assert_eq!(enters, 5);
}

/// The two-channel contract holds under observed emission: turning observation
/// ON in the VM does not change the program-output channel by a single byte
/// (observed output == plain-VM output).
#[test]
fn observed_emission_never_touches_program_output() {
    let observed = vm_outcome_observed(OBSERVED);
    let plain = vm_outcome(OBSERVED);
    assert_eq!(
        observed.output, plain.output,
        "emission is on a separate channel"
    );
    assert_eq!(observed.output.trim_end(), "10");
}
