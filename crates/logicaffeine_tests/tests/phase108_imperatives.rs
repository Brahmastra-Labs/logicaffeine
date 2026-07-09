//! Phase 108 — §1.4 Imperatives (work/MISSING_ENGLISH.md).
//!
//! Commands with a covert 2nd-person subject:
//!   "Close the door."  → Directive(hearer, ⟨∃e(Close(e)∧Agent(e,hearer)∧Theme(e,door))⟩)
//!   "Don't touch that."→ negated directive
//!   "Let's leave."     → hortative (inclusive agent)
//!
//! The `Imperative{action}` node carries a full neo-Davidsonian event with
//! Agent = hearer and a Theme, handles the negative and hortative forms, and
//! accepts period-terminated commands. It renders as the directive operator
//! `Directive(hearer, [action])` and lowers to a bouletic obligation in the
//! proof IR — the commanded action is never asserted.

use logicaffeine_language::{compile, compile_kripke, compile_simple};

#[test]
fn imperative_has_hearer_agent_and_theme() {
    let out = compile("Close the door.").unwrap();
    eprintln!("close-door: {out}");
    assert!(out.contains("Directive("), "directive operator: {out}");
    assert!(out.contains("Close"), "the command verb event: {out}");
    // Covert 2nd-person subject is the addressee/hearer, as the Agent.
    assert!(
        out.contains("Agent") && out.contains("Addressee"),
        "Agent must be the hearer (Addressee): {out}"
    );
    // The object must survive as a Theme (not be dropped).
    assert!(out.contains("Theme"), "object must be a Theme role: {out}");
    assert!(out.contains("Door") || out.contains("door"), "the object 'door': {out}");
}

#[test]
fn negative_imperative_flips_polarity() {
    let out = compile("Don't touch that.").unwrap();
    eprintln!("dont-touch: {out}");
    assert!(out.contains("Directive("), "directive operator: {out}");
    assert!(out.contains('¬') || out.contains("NOT") || out.contains("\\neg"), "negation: {out}");
    assert!(out.contains("Touch"), "the command verb: {out}");
    assert!(out.contains("Addressee"), "agent is the hearer: {out}");
}

#[test]
fn hortative_lets_has_inclusive_agent() {
    let out = compile("Let's leave.").unwrap();
    eprintln!("lets-leave: {out}");
    assert!(out.contains("Directive("), "directive/hortative operator: {out}");
    assert!(out.contains("Leave"), "the command verb: {out}");
    // Hortative "let's" = inclusive we (speaker + addressee), not a bare addressee.
    assert!(
        out.contains("Us") || out.contains("We"),
        "hortative agent is the inclusive group: {out}"
    );
}

#[test]
fn imperative_renders_in_simple_and_kripke() {
    let simple = compile_simple("Close the door.").unwrap();
    eprintln!("close(simple): {simple}");
    assert!(simple.contains("Close") && simple.contains("Addressee"), "SimpleFOL: {simple}");

    let kripke = compile_kripke("Close the door.").unwrap();
    eprintln!("close(kripke): {kripke}");
    assert!(kripke.contains("Close"), "Kripke: {kripke}");
}

// ============================================================================
// Entailment spec (§1.4): a directive is a bouletic/deontic modal — the
// commanded action is quantified over ideal worlds, never asserted.
// ============================================================================
#[cfg(feature = "verification")]
mod verification_spec {
    use logicaffeine_compile::compile_for_proof;
    use logicaffeine_proof::oracle::{oracle_consistent, oracle_entails, SmtConsistency, SmtVerdict};
    use logicaffeine_proof::ProofExpr;

    #[test]
    fn imperative_lowers_to_deontic_modal_in_proof_ir() {
        let result = compile_for_proof("Close the door.");
        let expr = result
            .proof_expr
            .expect("imperative must convert to a proof expression");
        match &expr {
            ProofExpr::Modal { domain, flavor, .. } => {
                assert_eq!(domain, "Deontic", "directive domain: {expr}");
                assert_eq!(flavor, "Bouletic", "directive flavor: {expr}");
            }
            other => panic!(
                "Directive must lower to a Deontic/Bouletic Modal, got: {other}"
            ),
        }
    }

    #[test]
    fn commanded_action_is_not_entailed() {
        let result = compile_for_proof("Close the door.");
        let expr = result
            .proof_expr
            .expect("imperative must convert to a proof expression");
        let ProofExpr::Modal { body, .. } = &expr else {
            panic!("Directive must lower to a Modal, got: {expr}");
        };
        let premises = vec![expr.clone()];
        assert_eq!(
            oracle_consistent(&premises),
            SmtConsistency::Consistent,
            "a directive premise is consistent"
        );
        assert_eq!(
            oracle_entails(&premises, body),
            SmtVerdict::NotEntailed,
            "Directive(p) must NOT entail p — commanding is not doing"
        );
    }

    #[test]
    fn directive_entails_its_own_obligation() {
        let result = compile_for_proof("Close the door.");
        let expr = result.proof_expr.expect("must convert");
        assert_eq!(
            oracle_entails(&[expr.clone()], &expr),
            SmtVerdict::Entailed,
            "identity: the obligation itself is entailed"
        );
    }
}
