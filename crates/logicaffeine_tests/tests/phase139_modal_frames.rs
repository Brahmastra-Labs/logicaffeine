#![cfg(feature = "verification")]
//! ============================================================================
//! PHASE 139: P1 MODAL FRAME REASONING — THE ENTAILMENT SPEC
//! ============================================================================
//!
//! Modal operators must reason over Kratzer-style accessibility relations with
//! per-(domain, flavor) frame axioms — not as uninterpreted functions. These
//! tests define the spec for the standard translation behind the Z3 verdict
//! API (`oracle_entails` / `oracle_consistent`) and the English theorem door
//! (`check_theorem_smt`).
//!
//! Frame spec (v1):
//!   - Alethic/Root       : T (reflexive)  ⇒ □P ⊢ P
//!   - Alethic/Epistemic  : D (serial)     ⇒ must(P) ⊬ P
//!   - Alethic/Evidential : D (serial)     ⇒ Seem(P) ⊬ P (non-factive)
//!   - Deontic/* (Bouletic incl.): D       ⇒ O(P) ⊢ ¬O(¬P), O(P) ⊬ P
//!   - Counterfactual     : per-antecedent Closest relation with a success
//!                          axiom and NO weak centering ⇒ (P □→ Q) ⊬ (P → Q)
//!                          and (P → Q) ⊬ (P □→ Q); consequent weakening holds;
//!                          antecedent strengthening fails.
//!
//! Every non-entailment pairs with a consistency check (no vacuous passes) and
//! a positive sibling (a permanently-Unknown stub cannot fake a green file).
//!
//! Verdicts are Z3-side only — never kernel-certified (`SmtVerdict` ≠
//! `VerifiedProof.verified`).

use logicaffeine_compile::{check_theorem_premises_consistent, check_theorem_smt};
use logicaffeine_proof::oracle::{
    oracle_consistent, oracle_entails, SmtConsistency, SmtVerdict,
};
use logicaffeine_proof::{ProofExpr, ProofTerm};

fn atom(p: &str) -> ProofExpr {
    ProofExpr::Atom(p.to_string())
}

fn pred1(name: &str, arg: &str) -> ProofExpr {
    ProofExpr::Predicate {
        name: name.to_string(),
        args: vec![ProofTerm::Constant(arg.to_string())],
        world: None,
    }
}

fn modal(domain: &str, flavor: &str, force: f32, body: ProofExpr) -> ProofExpr {
    ProofExpr::Modal {
        domain: domain.to_string(),
        force,
        flavor: flavor.to_string(),
        body: Box::new(body),
    }
}

fn nec(domain: &str, flavor: &str, body: ProofExpr) -> ProofExpr {
    modal(domain, flavor, 1.0, body)
}

fn poss(domain: &str, flavor: &str, body: ProofExpr) -> ProofExpr {
    modal(domain, flavor, 0.3, body)
}

fn not(e: ProofExpr) -> ProofExpr {
    ProofExpr::Not(Box::new(e))
}

fn implies(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(a), Box::new(b))
}

// ============================================================================
// A. Alethic Root: the T axiom (reflexive accessibility)
// ============================================================================

#[test]
fn alethic_root_box_entails_actual() {
    let premises = vec![nec("Alethic", "Root", atom("P"))];
    assert_eq!(
        oracle_entails(&premises, &atom("P")),
        SmtVerdict::Entailed,
        "T axiom: alethic □P must entail P"
    );
}

#[test]
fn alethic_root_box_entails_diamond() {
    let premises = vec![nec("Alethic", "Root", atom("P"))];
    assert_eq!(
        oracle_entails(&premises, &poss("Alethic", "Root", atom("P"))),
        SmtVerdict::Entailed,
        "D (via T): □P must entail ◇P"
    );
}

#[test]
fn alethic_root_actual_does_not_entail_box() {
    let premises = vec![atom("P")];
    assert_eq!(
        oracle_consistent(&premises),
        SmtConsistency::Consistent,
        "a bare fact is a consistent premise set"
    );
    assert_eq!(
        oracle_entails(&premises, &nec("Alethic", "Root", atom("P"))),
        SmtVerdict::NotEntailed,
        "P must not entail □P (necessitation is not a consequence relation)"
    );
}

// ============================================================================
// B. Modal K: distribution holds in every normal frame
// ============================================================================

#[test]
fn k_axiom_box_distributes_over_implication() {
    let premises = vec![
        nec("Alethic", "Root", implies(atom("P"), atom("Q"))),
        nec("Alethic", "Root", atom("P")),
    ];
    assert_eq!(
        oracle_entails(&premises, &nec("Alethic", "Root", atom("Q"))),
        SmtVerdict::Entailed,
        "K: □(P→Q), □P ⊢ □Q"
    );
}

// ============================================================================
// C. Epistemic: serial but not reflexive — must(P) is not factive
// ============================================================================

#[test]
fn epistemic_must_is_not_factive() {
    let premises = vec![nec("Alethic", "Epistemic", pred1("Guilty", "John"))];
    assert_eq!(
        oracle_consistent(&premises),
        SmtConsistency::Consistent,
        "an epistemic necessity premise is consistent"
    );
    assert_eq!(
        oracle_entails(&premises, &pred1("Guilty", "John")),
        SmtVerdict::NotEntailed,
        "epistemic must(P) must NOT entail P (no T on the epistemic frame)"
    );
}

#[test]
fn epistemic_must_entails_itself() {
    let p = nec("Alethic", "Epistemic", pred1("Guilty", "John"));
    assert_eq!(
        oracle_entails(&[p.clone()], &p),
        SmtVerdict::Entailed,
        "identity: an epistemic premise entails itself"
    );
}

// ============================================================================
// D. Evidential: Seem(⟨P⟩) ⊬ P — the §4.3 non-factivity property
// ============================================================================

#[test]
fn evidential_is_not_factive() {
    let premises = vec![nec("Alethic", "Evidential", pred1("Happy", "John"))];
    assert_eq!(
        oracle_consistent(&premises),
        SmtConsistency::Consistent,
        "an evidential premise is consistent"
    );
    assert_eq!(
        oracle_entails(&premises, &pred1("Happy", "John")),
        SmtVerdict::NotEntailed,
        "Seem(⟨Happy(j)⟩) must NOT entail Happy(j)"
    );
}

#[test]
fn evidential_consistent_with_negated_complement() {
    // "John seems happy, but he is not" must be a satisfiable state of
    // affairs — the whole point of a non-factive evidential.
    let premises = vec![
        nec("Alethic", "Evidential", pred1("Happy", "John")),
        not(pred1("Happy", "John")),
    ];
    assert_eq!(
        oracle_consistent(&premises),
        SmtConsistency::Consistent,
        "Seem(P) ∧ ¬P must be consistent"
    );
}

#[test]
fn evidential_k_still_holds() {
    // Non-factive does not mean non-logical: K must hold on the evidential
    // relation too.
    let premises = vec![
        nec(
            "Alethic",
            "Evidential",
            implies(pred1("Happy", "John"), pred1("Smiling", "John")),
        ),
        nec("Alethic", "Evidential", pred1("Happy", "John")),
    ];
    assert_eq!(
        oracle_entails(
            &premises,
            &nec("Alethic", "Evidential", pred1("Smiling", "John"))
        ),
        SmtVerdict::Entailed,
        "K on the evidential frame: Seem(P→Q), Seem(P) ⊢ Seem(Q)"
    );
}

// ============================================================================
// E. Deontic: serial — obligations are jointly satisfiable, never factive
// ============================================================================

#[test]
fn deontic_d_no_conflicting_obligations() {
    let premises = vec![nec("Deontic", "Root", pred1("Pay", "John"))];
    assert_eq!(
        oracle_entails(
            &premises,
            &not(nec("Deontic", "Root", not(pred1("Pay", "John"))))
        ),
        SmtVerdict::Entailed,
        "D: O(P) ⊢ ¬O(¬P)"
    );
}

#[test]
fn deontic_obligation_is_not_factive() {
    let premises = vec![nec("Deontic", "Root", pred1("Pay", "John"))];
    assert_eq!(
        oracle_consistent(&premises),
        SmtConsistency::Consistent,
        "an obligation premise is consistent"
    );
    assert_eq!(
        oracle_entails(&premises, &pred1("Pay", "John")),
        SmtVerdict::NotEntailed,
        "O(P) must NOT entail P (ought ≠ is)"
    );
}

#[test]
fn bouletic_complement_not_entailed() {
    // The shared spec for imperatives (§1.4) and optatives (§1.2): the
    // directive/wish content is quantified over ideal worlds, not asserted.
    let premises = vec![nec("Deontic", "Bouletic", pred1("Prosper", "Addressee"))];
    assert_eq!(
        oracle_consistent(&premises),
        SmtConsistency::Consistent,
        "a bouletic premise is consistent"
    );
    assert_eq!(
        oracle_entails(&premises, &pred1("Prosper", "Addressee")),
        SmtVerdict::NotEntailed,
        "Wish/Directive(P) must NOT entail P"
    );
}

#[test]
fn bouletic_d_axiom() {
    let premises = vec![nec("Deontic", "Bouletic", pred1("Prosper", "Addressee"))];
    assert_eq!(
        oracle_entails(
            &premises,
            &poss("Deontic", "Bouletic", pred1("Prosper", "Addressee"))
        ),
        SmtVerdict::Entailed,
        "D on the bouletic frame: Wish(P) ⊢ ◇_wish P (wishes are satisfiable)"
    );
}

// ============================================================================
// F. Flavors are distinct relations: no cross-contamination
// ============================================================================

#[test]
fn deontic_does_not_leak_into_alethic() {
    let premises = vec![nec("Deontic", "Root", pred1("Pay", "John"))];
    assert_eq!(
        oracle_entails(&premises, &nec("Alethic", "Root", pred1("Pay", "John"))),
        SmtVerdict::NotEntailed,
        "O(P) must NOT entail □_alethic P — distinct accessibility relations"
    );
}

#[test]
fn evidential_does_not_leak_into_epistemic() {
    let premises = vec![nec("Alethic", "Evidential", pred1("Happy", "John"))];
    assert_eq!(
        oracle_entails(
            &premises,
            &nec("Alethic", "Epistemic", pred1("Happy", "John"))
        ),
        SmtVerdict::NotEntailed,
        "Seem(P) must NOT entail must(P) — evidence ≠ knowledge"
    );
}

// ============================================================================
// F2. Adversarial discriminators: a translation that ignores force, bodies,
// or consistency must fail HERE, not just in the headline tests.
// ============================================================================

#[test]
fn translation_does_not_entail_everything() {
    // The cheapest unsoundness: inconsistent axioms make every goal entailed.
    let premises = vec![nec("Alethic", "Root", atom("P"))];
    assert_eq!(
        oracle_entails(&premises, &atom("Q")),
        SmtVerdict::NotEntailed,
        "□P must not entail an unrelated Q"
    );
}

#[test]
fn modal_bodies_are_not_conflated() {
    let premises = vec![nec("Alethic", "Root", atom("P"))];
    assert_eq!(
        oracle_entails(&premises, &nec("Alethic", "Root", atom("Q"))),
        SmtVerdict::NotEntailed,
        "□P must not entail □Q — the relation must respect bodies"
    );
}

#[test]
fn diamond_does_not_entail_box() {
    // A translation that ignores `force` collapses ◇ into □.
    let premises = vec![poss("Alethic", "Root", atom("P"))];
    assert_eq!(
        oracle_consistent(&premises),
        SmtConsistency::Consistent,
        "◇P is a consistent premise"
    );
    assert_eq!(
        oracle_entails(&premises, &nec("Alethic", "Root", atom("P"))),
        SmtVerdict::NotEntailed,
        "◇P must NOT entail □P — the force split must be real"
    );
}

#[test]
fn nested_box_unwraps_via_t() {
    // T applied at the outer layer: □□P ⊢ □P. A translation that only
    // pattern-matches one modal layer fails this.
    let premises = vec![nec("Alethic", "Root", nec("Alethic", "Root", atom("P")))];
    assert_eq!(
        oracle_entails(&premises, &nec("Alethic", "Root", atom("P"))),
        SmtVerdict::Entailed,
        "T at the outer modal: □□P ⊢ □P"
    );
}

#[test]
fn no_spurious_four_axiom() {
    // The converse needs transitivity (4), which v1 does not assert for the
    // alethic frame.
    let premises = vec![nec("Alethic", "Root", atom("P"))];
    assert_eq!(
        oracle_consistent(&premises),
        SmtConsistency::Consistent,
        "□P is consistent"
    );
    assert_eq!(
        oracle_entails(&premises, &nec("Alethic", "Root", nec("Alethic", "Root", atom("P")))),
        SmtVerdict::NotEntailed,
        "□P must NOT entail □□P without a 4 axiom"
    );
}

#[test]
fn negation_inside_modal_is_respected() {
    // □¬P together with T must yield ¬P, and must be inconsistent with P.
    let premises = vec![nec("Alethic", "Root", not(atom("P")))];
    assert_eq!(
        oracle_entails(&premises, &not(atom("P"))),
        SmtVerdict::Entailed,
        "T: □¬P ⊢ ¬P"
    );
    let clash = vec![nec("Alethic", "Root", not(atom("P"))), atom("P")];
    assert_eq!(
        oracle_consistent(&clash),
        SmtConsistency::Inconsistent,
        "□¬P ∧ P must be INCONSISTENT under T — consistency checking must \
         have teeth, not rubber-stamp"
    );
}

// ============================================================================
// G. Counterfactuals through English (§4.5): □→ is not material implication
// ============================================================================

const CF_PREMISE: &str = "If John had studied, he would have passed.";

fn theorem(premises: &[&str], goal: &str) -> String {
    let givens: String = premises
        .iter()
        .map(|p| format!("Given: {p}\n"))
        .collect();
    format!("## Theorem: Phase139\n{givens}Prove: {goal}\nProof: Auto.\n")
}

#[test]
fn counterfactual_entails_itself() {
    let src = theorem(&[CF_PREMISE], CF_PREMISE);
    assert_eq!(
        check_theorem_smt(&src).expect("counterfactual theorem must parse"),
        SmtVerdict::Entailed,
        "identity: a counterfactual premise entails itself"
    );
}

#[test]
fn counterfactual_does_not_entail_material() {
    let src = theorem(&[CF_PREMISE], "If John studied, John passed.");
    assert_eq!(
        check_theorem_premises_consistent(&src).expect("must parse"),
        SmtConsistency::Consistent,
        "a counterfactual premise is consistent"
    );
    assert_eq!(
        check_theorem_smt(&src).expect("must parse"),
        SmtVerdict::NotEntailed,
        "(P □→ Q) must NOT entail (P → Q): no weak centering in v1"
    );
}

#[test]
fn material_does_not_entail_counterfactual() {
    let src = theorem(&["If John studied, John passed."], CF_PREMISE);
    assert_eq!(
        check_theorem_premises_consistent(&src).expect("must parse"),
        SmtConsistency::Consistent,
        "a material-conditional premise is consistent"
    );
    assert_eq!(
        check_theorem_smt(&src).expect("must parse"),
        SmtVerdict::NotEntailed,
        "(P → Q) must NOT entail (P □→ Q) — the counterfactual is stronger \
         about non-actual worlds"
    );
}

#[test]
fn counterfactual_does_not_entail_antecedent_or_consequent() {
    let src_a = theorem(&[CF_PREMISE], "John studied.");
    assert_eq!(
        check_theorem_smt(&src_a).expect("must parse"),
        SmtVerdict::NotEntailed,
        "(P □→ Q) must NOT entail P"
    );
    let src_c = theorem(&[CF_PREMISE], "John passed.");
    assert_eq!(
        check_theorem_smt(&src_c).expect("must parse"),
        SmtVerdict::NotEntailed,
        "(P □→ Q) must NOT entail Q"
    );
}

#[test]
fn counterfactual_consequent_weakening_holds() {
    let src = theorem(
        &["If John had studied, he would have passed and he would have celebrated."],
        CF_PREMISE,
    );
    assert_eq!(
        check_theorem_smt(&src).expect("conjunctive consequent must parse"),
        SmtVerdict::Entailed,
        "(P □→ Q∧R) ⊢ (P □→ Q): consequent weakening is valid"
    );
}

#[test]
fn counterfactual_antecedent_strengthening_fails() {
    let src = theorem(
        &[CF_PREMISE],
        "If John had studied and John had slept, he would have passed.",
    );
    assert_eq!(
        check_theorem_premises_consistent(&src).expect("must parse"),
        SmtConsistency::Consistent,
        "premise set consistent"
    );
    assert_eq!(
        check_theorem_smt(&src).expect("conjunctive antecedent must parse"),
        SmtVerdict::NotEntailed,
        "(P □→ Q) must NOT entail (P∧R □→ Q): counterfactuals are \
         non-monotonic in the antecedent"
    );
}

// ============================================================================
// H. The English evidential door (parse → Modal{Evidential} → frames)
// ============================================================================

#[test]
fn english_seems_happy_does_not_entail_happy() {
    let src = theorem(&["John seems happy."], "John is happy.");
    assert_eq!(
        check_theorem_premises_consistent(&src).expect("must parse"),
        SmtConsistency::Consistent,
        "evidential premise consistent"
    );
    assert_eq!(
        check_theorem_smt(&src).expect("must parse"),
        SmtVerdict::NotEntailed,
        "English door: 'John seems happy' must NOT entail 'John is happy'"
    );
}

#[test]
fn english_seems_happy_entails_itself() {
    let src = theorem(&["John seems happy."], "John seems happy.");
    assert_eq!(
        check_theorem_smt(&src).expect("must parse"),
        SmtVerdict::Entailed,
        "identity through the English door"
    );
}
