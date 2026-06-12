//! Phase 128 — §8.4 Deixis / indexicals (MISSING_ENGLISH.md).
//!
//! Context-dependent reference resolves against the utterance context to a stable
//! anchor: I → Speaker, you → Addressee (in ANY position), here → the place,
//! today/tomorrow → the day. Previously object "you" resolved to a discourse
//! referent ("Someone"/"Them") and place/time indexicals were parsed literally.
//!
//! (Note: collecting an adverbial indexical AFTER a direct object — "saw you HERE"
//! — is a separate, deixis-orthogonal parser-coverage gap and is not exercised
//! here; the deixis RESOLUTION itself is what this phase verifies.)

use logicaffeine_language::compile;

#[test]
fn person_indexicals_resolve_in_any_position() {
    // Subject "I" and object "you" both resolve to discourse roles.
    let out = compile("I saw you.").unwrap();
    eprintln!("i-saw-you: {out}");
    assert!(out.contains("Speaker"), "'I' → Speaker: {out}");
    assert!(out.contains("Addressee"), "object 'you' → Addressee (not a discourse referent): {out}");
    // (Note: "Theme" contains the substring "Them", so check the role argument.)
    assert!(out.contains("Theme(e, Addressee)"), "the object Theme is the Addressee: {out}");
    assert!(!out.contains("Someone"), "no spurious discourse referent: {out}");
}

#[test]
fn person_indexicals_in_future_clause() {
    let out = compile("I will meet you.").unwrap();
    eprintln!("i-meet-you: {out}");
    assert!(out.contains("Speaker") && out.contains("Addressee"), "both person indexicals resolve: {out}");
}

#[test]
fn place_indexical_resolves() {
    let out = compile("John works here.").unwrap();
    eprintln!("here: {out}");
    assert!(out.contains("Here"), "'here' resolves to the utterance-place anchor: {out}");
}

#[test]
fn time_indexical_resolves() {
    let out = compile("Mary arrived today.").unwrap();
    eprintln!("today: {out}");
    assert!(out.contains("Today"), "'today' resolves to the utterance-day anchor: {out}");
}
