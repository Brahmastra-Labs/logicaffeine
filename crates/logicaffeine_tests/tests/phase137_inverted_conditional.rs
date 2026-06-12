//! Phase 137 — §4.1 Inverted conditionals (MISSING_ENGLISH.md).
//!
//! Subject-aux inversion of the auxiliary stands in for "if" and yields a
//! counterfactual:
//!   "Had I known, I would have left." → Know(I) □→ Leave(I)
//! Equivalent to "If I had known, I would have left."

use logicaffeine_language::compile;

#[test]
fn had_fronting_is_counterfactual() {
    let out = compile("Had I known, I would have left.").unwrap();
    eprintln!("had: {out}");
    assert!(out.contains("□→"), "an inverted conditional is a counterfactual: {out}");
    assert!(out.contains("Know"), "the antecedent: {out}");
    assert!(out.contains("Leave"), "the consequent: {out}");
}

#[test]
fn inverted_matches_explicit_if() {
    let inverted = compile("Had I known, I would have left.").unwrap();
    let explicit = compile("If I had known, I would have left.").unwrap();
    eprintln!("inverted: {inverted}\nexplicit: {explicit}");
    assert_eq!(inverted, explicit, "inverted conditional must match the explicit-if reading");
}

#[test]
fn had_fronting_with_multiword_subject() {
    // The un-inversion must move the aux past the FULL subject NP, not just a single
    // pronoun token — "the soldiers" is a two-token subject.
    let out = compile("Had the soldiers known, they would have retreated.").unwrap();
    eprintln!("had-multiword: {out}");
    assert!(out.contains("□→"), "inverted conditional is a counterfactual: {out}");
    assert!(out.contains("Know") || out.contains("Knew"), "the antecedent verb: {out}");
    assert!(out.contains("Retreat"), "the consequent verb: {out}");
}

#[test]
fn should_fronting_is_a_conditional() {
    // "Should it rain, …" is an inverted conditional too — `Should`-fronting, not just
    // `Had`/`Were`. Must read as the explicit "If it should rain, …".
    // §4.1 is about un-inversion: the inverted form must read identically to its
    // explicit-"if" counterpart. (Whether "should…would" is counterfactual vs material
    // is §4.5's concern, not §4.1's — here we only require the inversion to be undone.)
    let inverted = compile("Should it rain, the match would be cancelled.").unwrap();
    let explicit = compile("If it should rain, the match would be cancelled.").unwrap();
    eprintln!("should-inverted: {inverted}\nshould-explicit: {explicit}");
    assert!(inverted != "O_{0.6} HAB(∃e(Rain(e) ∧ Agent(e, It)))", "Should-fronting must NOT parse as a bare deontic modal: {inverted}");
    assert_eq!(inverted, explicit, "Should-fronting must match the explicit-if reading");
}
