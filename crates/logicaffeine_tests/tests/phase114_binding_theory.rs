//! Phase 114 — §8.1 Binding theory (work/MISSING_ENGLISH.md).
//!
//! Structural constraints on anaphora:
//!   Principle A — a reflexive is bound in its local clause:
//!     "John saw himself." → See(john, john)
//!   Principle B — a plain pronoun is FREE locally (not the local subject):
//!     "John saw him."     → See(john, y), y ≠ john
//!
//! Previously a plain object pronoun wrongly corefered with the local subject
//! ("John saw him." gave Theme = John).

use logicaffeine_language::compile;

#[test]
fn principle_a_reflexive_binds_subject() {
    let out = compile("John saw himself.").unwrap();
    eprintln!("himself: {out}");
    assert!(out.contains("See"), "the verb: {out}");
    assert!(out.contains("Agent(e, John)"), "John is the agent: {out}");
    assert!(out.contains("Theme(e, John)"), "reflexive bound to the subject: {out}");
}

#[test]
fn principle_b_pronoun_not_local_subject() {
    let out = compile("John saw him.").unwrap();
    eprintln!("him: {out}");
    assert!(out.contains("See"), "the verb: {out}");
    assert!(out.contains("Agent(e, John)"), "John is the agent: {out}");
    assert!(!out.contains("Theme(e, John)"), "Principle B: 'him' must NOT be John: {out}");
    assert!(out.contains("Theme"), "the object still has a Theme (a distinct individual): {out}");
}

#[test]
fn principle_a_with_love() {
    let out = compile("John loves himself.").unwrap();
    eprintln!("loves-himself: {out}");
    assert!(out.contains("Theme(e, John)"), "reflexive bound: {out}");
}

#[test]
fn principle_b_with_love() {
    let out = compile("John loves him.").unwrap();
    eprintln!("loves-him: {out}");
    assert!(!out.contains("Theme(e, John)"), "Principle B: 'him' ≠ John: {out}");
    assert!(out.contains("Theme"), "distinct Theme present: {out}");
}

#[test]
fn reflexive_feminine_binds() {
    let out = compile("Mary saw herself.").unwrap();
    eprintln!("herself: {out}");
    assert!(out.contains("Theme(e, Mary)"), "herself bound to Mary: {out}");
}

// ============================================================================
// Entailment spec (§8.1): binding has semantic teeth — the reflexive IS the
// subject; the free pronoun is NOT provably the subject.
// ============================================================================
#[cfg(feature = "verification")]
mod verification_spec {
    use logicaffeine_compile::{check_theorem_premises_consistent, check_theorem_smt};
    use logicaffeine_proof::oracle::{SmtConsistency, SmtVerdict};

    fn theorem(premises: &[&str], goal: &str) -> String {
        let givens: String = premises.iter().map(|p| format!("Given: {p}\n")).collect();
        format!("## Theorem: Phase114V\n{givens}Prove: {goal}\nProof: Auto.\n")
    }

    #[test]
    fn reflexive_entails_subject_identity() {
        let src = theorem(&["John saw himself."], "John saw John.");
        assert_eq!(
            check_theorem_smt(&src).expect("must parse"),
            SmtVerdict::Entailed,
            "Principle A: See(j, j) ⊢ See(j, j)"
        );
    }

    #[test]
    fn free_pronoun_does_not_entail_subject_identity() {
        let src = theorem(&["John saw him."], "John saw John.");
        assert_eq!(
            check_theorem_premises_consistent(&src).expect("must parse"),
            SmtConsistency::Consistent,
            "the free-pronoun premise is consistent"
        );
        assert_eq!(
            check_theorem_smt(&src).expect("must parse"),
            SmtVerdict::NotEntailed,
            "Principle B: 'him' may be someone else — no forced identity"
        );
    }
}
