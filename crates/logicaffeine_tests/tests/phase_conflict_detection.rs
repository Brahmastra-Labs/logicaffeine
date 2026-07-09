//! =============================================================================
//! VERIFIED CONFLICT DETECTION — kernel-checked proofs of ⊥
//! =============================================================================
//!
//! A competitor's pitch is "Z3 says these rules are jointly unsat → conflict".
//! That returns a *verdict*. LOGOS returns a **certificate**: when a rule set is
//! inconsistent, the engine derives ⊥ and the certifier turns that derivation
//! into a kernel term of type `False` that re-checks under `infer_type` against
//! the prelude axioms. These tests pin that guarantee at the shared-core layer.

use logicaffeine_kernel::{infer_type, Term};
use logicaffeine_proof::verify::prove_certify_check;
use logicaffeine_proof::{ProofExpr, ProofTerm};

/// `P(c)` as a unary predicate application.
fn pred(name: &str, c: &str) -> ProofExpr {
    ProofExpr::Predicate {
        name: name.to_string(),
        args: vec![ProofTerm::Constant(c.to_string())],
        world: None,
    }
}

fn not(p: ProofExpr) -> ProofExpr {
    ProofExpr::Not(Box::new(p))
}

fn falsum() -> ProofExpr {
    ProofExpr::Atom("⊥".to_string())
}

/// Assert that a verified proof's term genuinely has type `False` in its own
/// kernel context — the certificate is real, not a bare boolean.
fn assert_proves_false(premises: &[ProofExpr]) -> Term {
    let outcome = prove_certify_check(premises, &falsum());
    assert!(
        outcome.verified,
        "expected a kernel-checked proof of ⊥, got error: {:?}",
        outcome.verification_error
    );
    let term = outcome
        .proof_term
        .expect("verified proof must carry a term");
    let ty = infer_type(&outcome.kernel_ctx, &term)
        .expect("verified term must type-check");
    assert_eq!(
        ty,
        Term::Global("False".to_string()),
        "conflict certificate must have type False, got {:?}",
        ty
    );
    term
}

fn implies(p: ProofExpr, q: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(p), Box::new(q))
}

fn forall(var: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::ForAll {
        variable: var.to_string(),
        body: Box::new(body),
    }
}

fn pred_var(name: &str, v: &str) -> ProofExpr {
    ProofExpr::Predicate {
        name: name.to_string(),
        args: vec![ProofTerm::Variable(v.to_string())],
        world: None,
    }
}

/// The simplest conflict: a fact and its direct negation.
#[test]
fn direct_contradiction_certifies_to_false() {
    let premises = vec![pred("taxed", "Alice"), not(pred("taxed", "Alice"))];
    assert_proves_false(&premises);
}

/// A fact that triggers an implication deriving its own negation: P, P→¬P ⊢ ⊥.
/// The derived ¬P must be backed by a real modus-ponens sub-proof, not an
/// unjustified leaf — otherwise the kernel rejects the certificate.
#[test]
fn implication_contradiction_certifies_to_false() {
    let p = pred("taxed", "Alice");
    let premises = vec![p.clone(), implies(p.clone(), not(p))];
    assert_proves_false(&premises);
}

/// A genuine two-rule eligibility/tax conflict over a universal domain:
///   resident(Alice);  ∀x. resident(x) → taxed(x);  ∀x. resident(x) → ¬taxed(x)
/// Instantiating both rules at Alice yields taxed(Alice) and ¬taxed(Alice).
#[test]
fn eligibility_rule_conflict_certifies_to_false() {
    let premises = vec![
        pred("resident", "Alice"),
        forall("x", implies(pred_var("resident", "x"), pred_var("taxed", "x"))),
        forall("x", implies(pred_var("resident", "x"), not(pred_var("taxed", "x")))),
    ];
    assert_proves_false(&premises);
}

fn pred2(name: &str, a: ProofTerm, b: ProofTerm) -> ProofExpr {
    ProofExpr::Predicate {
        name: name.to_string(),
        args: vec![a, b],
        world: None,
    }
}

fn and(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::And(Box::new(l), Box::new(r))
}

/// The Barber paradox stated with *simple* predicates (no event semantics, no
/// definite descriptions): the barber shaves exactly those who don't shave
/// themselves. Instantiating the rules at the barber and splitting on whether he
/// shaves himself drives both cases to ⊥ — a certified `CaseAnalysis`. This is
/// the self-referential heart of the paradox, kernel-checked.
#[test]
fn clean_barber_paradox_certifies_to_false() {
    let var_x = || ProofTerm::Variable("x".to_string());
    let barber = || ProofTerm::Constant("TheBarber".to_string());

    let man_x = pred_var("man", "x");
    let shaves_xx = pred2("shaves", var_x(), var_x());
    let shaves_bx = pred2("shaves", barber(), var_x());

    let premises = vec![
        pred("man", "TheBarber"),
        // ∀x. (man(x) ∧ ¬shaves(x,x)) → shaves(barber, x)
        forall(
            "x",
            implies(and(man_x.clone(), not(shaves_xx.clone())), shaves_bx.clone()),
        ),
        // ∀x. (man(x) ∧ shaves(x,x)) → ¬shaves(barber, x)
        forall(
            "x",
            implies(and(man_x.clone(), shaves_xx.clone()), not(shaves_bx.clone())),
        ),
    ];
    assert_proves_false(&premises);
}

fn exists(var: &str, body: ProofExpr) -> ProofExpr {
    ProofExpr::Exists {
        variable: var.to_string(),
        body: Box::new(body),
    }
}

/// A conflict that requires *eliminating an existential*: from `∃x. evil(x)`
/// plus two rules that make any evil thing both vile and not-vile, derive ⊥.
/// Certifying this means skolemizing the existential (a `Match` on `Ex`) and
/// reasoning under the fresh witness — the foundation for the definite
/// descriptions in the full Barber.
#[test]
fn existential_conflict_certifies_to_false() {
    let premises = vec![
        exists("x", pred_var("evil", "x")),
        forall("x", implies(pred_var("evil", "x"), pred_var("vile", "x"))),
        forall("x", implies(pred_var("evil", "x"), not(pred_var("vile", "x")))),
    ];
    assert_proves_false(&premises);
}

/// A consistent rule set must NOT be reported as a conflict — no false alarms.
#[test]
fn consistent_rules_report_no_conflict() {
    use logicaffeine_proof::verify::detect_conflict;
    let premises = vec![
        pred("resident", "Alice"),
        forall("x", implies(pred_var("resident", "x"), pred_var("taxed", "x"))),
    ];
    let report = detect_conflict(&premises);
    assert!(
        !report.inconsistent,
        "consistent rules were wrongly flagged as a conflict"
    );
    assert!(report.proof_term.is_none());
    assert!(report.conflicting_premises.is_empty());
}

/// Conflict detection must name *which* premises clash, backed by a
/// kernel-checked proof of ⊥ — the certificate, not just a verdict.
#[test]
fn detect_conflict_reports_clashing_premises() {
    use logicaffeine_proof::verify::detect_conflict;
    let premises = vec![
        pred("resident", "Alice"),                                                  // 0
        forall("x", implies(pred_var("resident", "x"), pred_var("taxed", "x"))),     // 1
        forall("x", implies(pred_var("resident", "x"), not(pred_var("taxed", "x")))),// 2
    ];
    let report = detect_conflict(&premises);
    assert!(report.inconsistent, "failed to detect the conflict: {:?}", report.error);

    let term = report.proof_term.expect("a detected conflict must carry a proof term");
    assert_eq!(
        infer_type(&report.kernel_ctx, &term).expect("term must type-check"),
        Term::Global("False".to_string())
    );

    // All three premises participate: the fact and both clashing rules.
    let mut clashing = report.conflicting_premises.clone();
    clashing.sort_unstable();
    assert_eq!(
        clashing,
        vec![0, 1, 2],
        "conflict report should name the fact and both clashing rules"
    );
}
