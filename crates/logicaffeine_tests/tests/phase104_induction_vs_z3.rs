//! =============================================================================
//! PHASE 104: INDUCTION Z3 CANNOT DO
//! =============================================================================
//!
//! An SMT solver has no induction principle. A universally-quantified statement
//! over an unbounded inductive domain (`∀n:Nat. …`) is, in general, beyond it —
//! it can check instances, never the schema. LOGOS proves such statements by
//! `StructuralInduction`, and the certifier turns that proof into a `Term::Fix`
//! the kernel audits via its termination guard (a recursive call is only allowed
//! on a structurally-smaller argument).
//!
//! This is the side-by-side: LOGOS certifies the induction to a kernel-checked
//! recursive term; the Z3 oracle *declines the very same goal*.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, Context, Term, Universe};
use logicaffeine_proof::certifier::{certify, CertificationContext};
use logicaffeine_proof::{
    BackwardChainer, DerivationTree, InferenceRule, ProofExpr, ProofGoal, ProofTerm,
};

// -----------------------------------------------------------------------------
// Helpers for the deep-embedded arithmetic goal `∀n:Nat. Add(n, Zero) = n`.
// -----------------------------------------------------------------------------

fn zero() -> ProofTerm {
    ProofTerm::Function("Zero".into(), vec![])
}
fn succ(n: ProofTerm) -> ProofTerm {
    ProofTerm::Function("Succ".into(), vec![n])
}
/// A `Nat`-typed variable, encoded as `name:Type` — the signal the prover reads
/// to attempt induction, and (because it carries inductive structure) exactly
/// what the SMT translation layer refuses.
fn nat_var(name: &str) -> ProofTerm {
    ProofTerm::Variable(format!("{}:Nat", name))
}
fn add(a: ProofTerm, b: ProofTerm) -> ProofTerm {
    ProofTerm::Function("Add".into(), vec![a, b])
}
fn var(name: &str) -> ProofTerm {
    ProofTerm::Variable(name.into())
}

/// `Add(n, Zero) = n` — true for all naturals, but only provable by induction.
fn right_identity_goal() -> ProofExpr {
    ProofExpr::Identity(add(nat_var("n"), zero()), nat_var("n"))
}

/// The two defining equations of addition, as axioms.
fn addition_axioms() -> Vec<ProofExpr> {
    vec![
        ProofExpr::Identity(add(zero(), var("m")), var("m")),
        ProofExpr::Identity(add(succ(var("k")), var("m")), succ(add(var("k"), var("m")))),
    ]
}

/// Does a kernel term contain a `Match` anywhere? (Induction-as-recursion emits
/// a `Fix` wrapping a `Match` on the inductive argument.)
fn contains_match(term: &Term) -> bool {
    match term {
        Term::Match { .. } => true,
        Term::App(f, a) => contains_match(f) || contains_match(a),
        Term::Lambda { param_type, body, .. } => contains_match(param_type) || contains_match(body),
        Term::Pi { param_type, body_type, .. } => contains_match(param_type) || contains_match(body_type),
        Term::Fix { body, .. } => contains_match(body),
        _ => false,
    }
}

// =============================================================================
// B1 — LOGOS certifies structural induction to a kernel-checked recursive term.
// =============================================================================
#[test]
fn logos_certifies_induction_to_kernel_checked_fix() {
    // ∀n:Nat. P(n) from base_hyp : P(Zero) and step_hyp : ∀k. P(k) → P(Succ k).
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    let nat = Term::Global("Nat".to_string());
    ctx.add_declaration(
        "P",
        Term::Pi {
            param: "_".into(),
            param_type: Box::new(nat.clone()),
            body_type: Box::new(Term::Sort(Universe::Prop)),
        },
    );
    ctx.add_declaration(
        "base_hyp",
        Term::App(Box::new(Term::Global("P".into())), Box::new(Term::Global("Zero".into()))),
    );
    ctx.add_declaration(
        "step_hyp",
        Term::Pi {
            param: "k".into(),
            param_type: Box::new(nat.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".into(),
                param_type: Box::new(Term::App(
                    Box::new(Term::Global("P".into())),
                    Box::new(Term::Var("k".into())),
                )),
                body_type: Box::new(Term::App(
                    Box::new(Term::Global("P".into())),
                    Box::new(Term::App(
                        Box::new(Term::Global("Succ".into())),
                        Box::new(Term::Var("k".into())),
                    )),
                )),
            }),
        },
    );

    let base = DerivationTree::leaf(ProofExpr::Atom("base_hyp".into()), InferenceRule::PremiseMatch);
    let step_hyp_k = DerivationTree::new(
        ProofExpr::Implies(
            Box::new(ProofExpr::Predicate {
                name: "P".into(),
                args: vec![ProofTerm::Variable("k".into())],
                world: None,
            }),
            Box::new(ProofExpr::Predicate {
                name: "P".into(),
                args: vec![ProofTerm::Function("Succ".into(), vec![ProofTerm::Variable("k".into())])],
                world: None,
            }),
        ),
        InferenceRule::UniversalInst("k".into()),
        vec![DerivationTree::leaf(ProofExpr::Atom("step_hyp".into()), InferenceRule::PremiseMatch)],
    );
    let ih = DerivationTree::leaf(
        ProofExpr::Predicate { name: "P".into(), args: vec![ProofTerm::Variable("k".into())], world: None },
        InferenceRule::PremiseMatch,
    );
    let step = DerivationTree::new(
        ProofExpr::Predicate {
            name: "P".into(),
            args: vec![ProofTerm::Function("Succ".into(), vec![ProofTerm::Variable("k".into())])],
            world: None,
        },
        InferenceRule::ModusPonens,
        vec![step_hyp_k, ih],
    );
    let tree = DerivationTree::new(
        ProofExpr::ForAll {
            variable: "n".into(),
            body: Box::new(ProofExpr::Predicate {
                name: "P".into(),
                args: vec![ProofTerm::Variable("n".into())],
                world: None,
            }),
        },
        InferenceRule::StructuralInduction {
            variable: "n".into(),
            ind_type: "Nat".into(),
            step_var: "k".into(),
        },
        vec![base, step],
    );

    let cert_ctx = CertificationContext::new(&ctx);
    let term = certify(&tree, &cert_ctx).expect("induction must certify");

    // The certificate is a recursive term: a Fix wrapping a Match on the Nat.
    assert!(matches!(term, Term::Fix { .. }), "expected a Fix term, got {:?}", term);
    assert!(contains_match(&term), "induction certificate must Match on the inductive argument");

    // And it kernel-type-checks to `Π(n:Nat). P n` — the kernel audited the recursion.
    let inferred = infer_type(&ctx, &term).expect("induction certificate must type-check");
    assert!(matches!(inferred, Term::Pi { .. }), "expected Π type, got {:?}", inferred);
}

// =============================================================================
// B2 — LOGOS proves `∀n:Nat. Add(n,Zero)=n` by induction over real arithmetic.
// =============================================================================
#[test]
fn logos_proves_right_identity_by_induction() {
    let mut engine = BackwardChainer::new();
    engine.set_max_depth(20);
    for ax in addition_axioms() {
        engine.add_axiom(ax);
    }
    let proof = engine
        .prove(right_identity_goal())
        .expect("LOGOS should prove Add(n,Zero)=n by induction");
    assert!(
        matches!(proof.rule, InferenceRule::StructuralInduction { .. }),
        "expected StructuralInduction, got {:?}",
        proof.rule
    );
}

// =============================================================================
// B3 — The same goal is beyond the Z3 oracle: it declines (Ok(None)).
//      Gated on `verification` since the oracle module requires it.
// =============================================================================
#[cfg(feature = "verification")]
#[test]
fn z3_oracle_declines_the_inductive_goal() {
    use logicaffeine_proof::oracle::try_oracle;

    let goal = ProofGoal::new(right_identity_goal());
    let result = try_oracle(&goal, &addition_axioms());

    // The oracle must not claim a proof of an inductive goal. It declines —
    // SMT has no induction principle, and the goal carries Nat's inductive
    // structure (Zero/Succ, a `:Nat` variable) the SMT translation refuses.
    assert!(
        matches!(result, Ok(None)),
        "Z3 oracle must decline the inductive goal, got {:?}",
        result
    );
}
