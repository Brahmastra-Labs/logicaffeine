//! =============================================================================
//! PHASE 73: THE CERTIFIER (REIFICATION)
//! =============================================================================
//!
//! The Great Bridge. Proof Trees become Lambda Terms.
//! The Kernel audits the Engine.
//!
//! Curry-Howard Correspondence:
//! - A Proposition is a Type
//! - A Proof is a Program (Term)
//! - Verification is Type Checking

use logos::kernel::prelude::StandardLibrary;
use logos::kernel::{infer_type, Context, Term, Universe};
use logos::proof::certifier::{certify, CertificationContext};
use logos::proof::{DerivationTree, InferenceRule, ProofExpr};

// =============================================================================
// AXIOM / HYPOTHESIS
// =============================================================================

#[test]
fn test_certify_axiom() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // P : Prop
    ctx.add_inductive("P", Term::Sort(Universe::Prop));
    // h : P (a hypothesis)
    ctx.add_declaration("h", Term::Global("P".to_string()));

    // Simple axiom: just h : P
    let tree = DerivationTree::leaf(ProofExpr::Atom("h".to_string()), InferenceRule::Axiom);

    // Certify
    let cert_ctx = CertificationContext::new(&ctx);
    let term = certify(&tree, &cert_ctx).expect("Certification should succeed");

    // Verify: term should type-check to P
    let inferred = infer_type(&ctx, &term).expect("Should type-check");
    assert!(
        matches!(&inferred, Term::Global(s) if s == "P"),
        "Expected type P, got {:?}",
        inferred
    );

    println!("Certifier: Axiom h : P");
    println!("Certified term: {}", term);
}

// =============================================================================
// MODUS PONENS
// =============================================================================

#[test]
fn test_certify_modus_ponens() {
    // Setup: P, Q are propositions. h1: P -> Q, h2: P. Prove Q.
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // Register P and Q as propositions
    ctx.add_inductive("P", Term::Sort(Universe::Prop));
    ctx.add_inductive("Q", Term::Sort(Universe::Prop));

    // Declare hypotheses
    // h1 : P -> Q
    let impl_type = Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(Term::Global("P".to_string())),
        body_type: Box::new(Term::Global("Q".to_string())),
    };
    ctx.add_declaration("h1", impl_type);
    // h2 : P
    ctx.add_declaration("h2", Term::Global("P".to_string()));

    // Build DerivationTree for Modus Ponens
    // Tree: ModusPonens -> [Axiom(h1), Axiom(h2)]
    let impl_leaf = DerivationTree::leaf(
        ProofExpr::Atom("h1".to_string()),
        InferenceRule::PremiseMatch,
    );
    let arg_leaf = DerivationTree::leaf(
        ProofExpr::Atom("h2".to_string()),
        InferenceRule::PremiseMatch,
    );
    let tree = DerivationTree::new(
        ProofExpr::Atom("Q".to_string()),
        InferenceRule::ModusPonens,
        vec![impl_leaf, arg_leaf],
    );

    // Certify
    let cert_ctx = CertificationContext::new(&ctx);
    let term = certify(&tree, &cert_ctx).expect("Certification should succeed");

    println!("Certifier: ModusPonens(h1: P->Q, h2: P) proves Q");
    println!("Certified term: {}", term);

    // Verify: term should be App(h1, h2) with type Q
    let inferred = infer_type(&ctx, &term).expect("Should type-check");
    assert!(
        matches!(&inferred, Term::Global(s) if s == "Q"),
        "Expected type Q, got {:?}",
        inferred
    );
}

// =============================================================================
// CONJUNCTION INTRODUCTION
// =============================================================================

#[test]
fn test_certify_conjunction_intro() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // P, Q : Prop
    ctx.add_inductive("P", Term::Sort(Universe::Prop));
    ctx.add_inductive("Q", Term::Sort(Universe::Prop));

    // hp : P, hq : Q
    ctx.add_declaration("hp", Term::Global("P".to_string()));
    ctx.add_declaration("hq", Term::Global("Q".to_string()));

    // Prove P AND Q from hp: P and hq: Q
    let p_proof = DerivationTree::leaf(
        ProofExpr::Atom("hp".to_string()),
        InferenceRule::PremiseMatch,
    );
    let q_proof = DerivationTree::leaf(
        ProofExpr::Atom("hq".to_string()),
        InferenceRule::PremiseMatch,
    );
    let tree = DerivationTree::new(
        ProofExpr::And(
            Box::new(ProofExpr::Atom("P".to_string())),
            Box::new(ProofExpr::Atom("Q".to_string())),
        ),
        InferenceRule::ConjunctionIntro,
        vec![p_proof, q_proof],
    );

    // Certify
    let cert_ctx = CertificationContext::new(&ctx);
    let term = certify(&tree, &cert_ctx).expect("Certification should succeed");

    println!("Certifier: ConjunctionIntro(hp: P, hq: Q) proves P AND Q");
    println!("Certified term: {}", term);

    // Verify: term should type-check to And P Q
    let inferred = infer_type(&ctx, &term).expect("Should type-check");

    // Build expected type: And P Q
    let expected_type = Term::App(
        Box::new(Term::App(
            Box::new(Term::Global("And".to_string())),
            Box::new(Term::Global("P".to_string())),
        )),
        Box::new(Term::Global("Q".to_string())),
    );

    assert_eq!(
        format!("{}", inferred),
        format!("{}", expected_type),
        "Should have type And P Q"
    );
}
