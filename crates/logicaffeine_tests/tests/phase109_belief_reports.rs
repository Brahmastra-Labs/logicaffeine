//! Phase 109 — §3.5 Belief reports / opaque complements (work/MISSING_ENGLISH.md).
//!
//! Attitude verbs create intensional contexts. A finite clausal complement is a
//! STRUCTURED PROPOSITION (P3), not an extensional object:
//!   "John believes Mary left." → Believe(John, ⟨Left(Mary)⟩)   (rendered [Left(Mary)])
//! Previously the embedded clause was mis-parsed — "Mary" was grabbed as a plain
//! Theme and the embedded verb "left" was dropped entirely.
//!
//! De re / de dicto for NP complements ("seeks a unicorn") is covered by
//! intensionality_tests.rs and must keep working (regression guard here).

use logicaffeine_language::{compile, compile_all_scopes, compile_kripke, compile_simple};

#[test]
fn belief_clausal_complement_is_structured_proposition() {
    let out = compile("John believes Mary left.").unwrap();
    eprintln!("believe-mary-left: {out}");
    assert!(out.contains("Believe"), "matrix attitude verb: {out}");
    assert!(out.contains("John"), "the believer: {out}");
    // The embedded clause must survive as a proposition, NOT be reduced to the
    // bare individual "Mary" with the verb dropped.
    assert!(out.contains("Leave") || out.contains("Left"), "the embedded predicate (left⇒Leave) must survive: {out}");
    assert!(out.contains("Mary"), "the embedded subject: {out}");
    assert!(
        out.contains('[') || out.contains('⟨'),
        "the complement is a structured proposition ⟨…⟩: {out}"
    );
}

#[test]
fn think_clausal_complement() {
    let out = compile("Mary thinks John runs.").unwrap();
    eprintln!("think-john-runs: {out}");
    assert!(out.contains("Think"), "matrix verb: {out}");
    assert!(out.contains("Run"), "embedded predicate must survive: {out}");
    assert!(out.contains("John"), "embedded subject: {out}");
}

#[test]
fn know_clausal_complement() {
    let out = compile("John knows Mary left.").unwrap();
    eprintln!("know-mary-left: {out}");
    assert!(out.contains("Know"), "matrix verb: {out}");
    assert!(out.contains("Leave") || out.contains("Left"), "embedded predicate (left⇒Leave) must survive: {out}");
}

#[test]
fn extensional_object_of_attitude_verb_unchanged() {
    // "John seeks Mary." — a plain NP object, no embedded clause: stays extensional.
    let out = compile("John seeks Mary.").unwrap();
    eprintln!("seek-mary: {out}");
    assert!(out.contains("Seek"), "matrix verb: {out}");
    assert!(out.contains("Mary"), "object: {out}");
    // No spurious embedded proposition for a bare NP object.
    assert!(!out.contains("[Mary"), "a bare NP object is not a proposition: {out}");
}

#[test]
fn seek_unicorn_keeps_two_readings_regression() {
    // De re / de dicto for an NP complement must still yield two readings.
    let readings = compile_all_scopes("John seeks a unicorn.").unwrap();
    eprintln!("seek-unicorn: {readings:?}");
    assert_eq!(readings.len(), 2, "de re + de dicto: {readings:?}");
    assert!(readings.iter().any(|r| r.contains('^')), "one reading is de dicto (^): {readings:?}");
}

#[test]
fn belief_renders_in_simple_and_kripke() {
    let simple = compile_simple("John believes Mary left.").unwrap();
    eprintln!("believe(simple): {simple}");
    assert!(simple.contains("Believe") && (simple.contains("Leave") || simple.contains("Left")), "SimpleFOL: {simple}");

    let kripke = compile_kripke("John believes Mary left.").unwrap();
    eprintln!("believe(kripke): {kripke}");
    assert!(kripke.contains("Believe"), "Kripke: {kripke}");
}
