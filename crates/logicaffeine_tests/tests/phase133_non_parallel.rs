//! Phase 133 — §2.2 Non-parallel coordination (work/MISSING_ENGLISH.md).
//!
//! Coordinated copular predicates of different categories (adjective + predicate
//! nominal) are each predicated of the shared subject:
//!   "He is wealthy and a philanthropist." → Wealthy(he) ∧ Philanthropist(he)
//! Previously the second conjunct was mis-reconstructed as a gapped event verb.

use logicaffeine_language::compile;

#[test]
fn adjective_and_nominal_share_subject() {
    let out = compile("He is wealthy and a philanthropist.").unwrap();
    eprintln!("wealthy+phil: {out}");
    assert!(out.contains("Wealthy(Him)"), "first predicate of the subject: {out}");
    assert!(out.contains("Philanthropist(Him)"), "second predicate of the SAME subject: {out}");
    assert!(!out.contains("Agent(e, Philanthropist)"), "philanthropist is a predicate, not an agent: {out}");
}

#[test]
fn nominal_and_adjective_share_subject() {
    let out = compile("Mary is a doctor and wealthy.").unwrap();
    eprintln!("doctor+wealthy: {out}");
    assert!(out.contains("Doctor(Mary)"), "first predicate: {out}");
    assert!(out.contains("Wealthy(Mary)"), "second predicate of the same subject: {out}");
}
