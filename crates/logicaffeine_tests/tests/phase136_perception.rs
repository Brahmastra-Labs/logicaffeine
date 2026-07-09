//! Phase 136 — §3.2 Perception complements (work/MISSING_ENGLISH.md).
//!
//! A perception verb takes a small-clause complement (NP + bare VP) describing the
//! PERCEIVED EVENT:
//!   "Mary heard the bell ring." → ∃e(Hear ∧ Ag=mary ∧ Th=⟨∃e'(Ring(e') ∧ Ag=bell)⟩)
//!   "John saw Mary leave."      → See(john, ⟨Leave(mary)⟩)
//! The bare-VP event (the bell's ringing) is the theme, not a bare object.

use logicaffeine_language::compile;

#[test]
fn perception_takes_event_complement() {
    let out = compile("Mary heard the bell ring.").unwrap();
    eprintln!("hear: {out}");
    assert!(out.contains("Hear"), "the perception verb: {out}");
    assert!(out.contains("Mary"), "the perceiver: {out}");
    // The perceived event: the bell ringing — BOTH must survive.
    assert!(out.contains("Ring"), "the perceived event predicate: {out}");
    assert!(out.contains("Bell"), "the entity in the perceived event: {out}");
    // "ring" must be the perceived event, not a bare Theme object.
    assert!(!out.contains("Theme(e, Ring)"), "ring is an event, not a theme object: {out}");
}

#[test]
fn perception_event_survives_trailing_adverb() {
    // A trailing adverb on the perceived event must not break the small-clause parse.
    let out = compile("Mary heard the bell ring loudly.").unwrap();
    eprintln!("hear-adv: {out}");
    assert!(out.contains("Hear"), "the perception verb: {out}");
    assert!(out.contains("Ring"), "the perceived event predicate survives: {out}");
    assert!(out.contains("Bell"), "the entity in the perceived event: {out}");
    assert!(!out.contains("Theme(e, Ring)"), "ring is an event, not a theme object: {out}");
}

#[test]
fn see_np_bare_vp() {
    let out = compile("John saw Mary leave.").unwrap();
    eprintln!("see: {out}");
    assert!(out.contains("See"), "the perception verb: {out}");
    assert!(out.contains("Leave"), "the perceived event must NOT be dropped: {out}");
    assert!(out.contains("Mary"), "the entity in the perceived event: {out}");
}
