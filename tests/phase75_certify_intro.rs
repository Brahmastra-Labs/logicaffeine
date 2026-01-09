//! =============================================================================
//! PHASE 75: CERTIFYING GENERALIZATION (SCOPES & LAMBDAS)
//! =============================================================================
//!
//! Universal Introduction: Γ, x:T ⊢ P(x) implies Γ ⊢ ∀x:T.P(x)
//!
//! The Bridge opens fully: Proof Trees with universal introduction become Lambda Terms.
//! The Kernel audits predicate logic proofs that CREATE universal truths.
//!
//! Curry-Howard Correspondence:
//! - ∀x:T. P(x)  <->  Π(x:T). P x
//! - Universal Introduction  <->  Lambda Abstraction

use logos::kernel::prelude::StandardLibrary;
use logos::kernel::{infer_type, Context, Term, Universe};
use logos::proof::certifier::{certify, CertificationContext};
use logos::proof::{DerivationTree, InferenceRule, ProofExpr, ProofTerm};

// =============================================================================
// UNIVERSAL INTRODUCTION - TRIVIAL CASE (VACUOUS QUANTIFICATION)
// =============================================================================

#[test]
fn test_certify_universal_intro_trivial() {
    // Goal: ∀x:Nat. True
    // Kernel Term: λ(x:Nat). I
    // Type: Π(x:Nat). True

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // Inner proof: I proves True
    let i_proof = DerivationTree::leaf(
        ProofExpr::Atom("I".to_string()),
        InferenceRule::Axiom,
    );

    // Outer: ∀x:Nat. True via UniversalIntro
    let tree = DerivationTree::new(
        ProofExpr::ForAll {
            variable: "x".to_string(),
            body: Box::new(ProofExpr::Atom("True".to_string())),
        },
        InferenceRule::UniversalIntro {
            variable: "x".to_string(),
            var_type: "Nat".to_string(),
        },
        vec![i_proof],
    );

    let cert_ctx = CertificationContext::new(&ctx);
    let term = certify(&tree, &cert_ctx).expect("Certification should succeed");

    println!("Certifier: UniversalIntro(x:Nat) proves ∀x:Nat. True");
    println!("Certified term: {}", term);

    // term should be: λ(x:Nat). I
    match &term {
        Term::Lambda { param, param_type, body } => {
            assert_eq!(param, "x");
            assert!(
                matches!(param_type.as_ref(), Term::Global(n) if n == "Nat"),
                "Expected Nat, got {:?}",
                param_type
            );
            assert!(
                matches!(body.as_ref(), Term::Global(n) if n == "I"),
                "Expected I, got {:?}",
                body
            );
        }
        _ => panic!("Expected Lambda, got {:?}", term),
    }

    // Type-check: should be Π(x:Nat). True
    let inferred = infer_type(&ctx, &term).expect("Should type-check");
    println!("Inferred type: {}", inferred);

    match &inferred {
        Term::Pi { param, param_type, body_type } => {
            assert_eq!(param, "x");
            assert!(
                matches!(param_type.as_ref(), Term::Global(n) if n == "Nat"),
                "Expected Nat, got {:?}",
                param_type
            );
            assert!(
                matches!(body_type.as_ref(), Term::Global(n) if n == "True"),
                "Expected True, got {:?}",
                body_type
            );
        }
        _ => panic!("Expected Pi, got {:?}", inferred),
    }
}

// =============================================================================
// UNIVERSAL INTRODUCTION - WITH LOCAL VARIABLE REFERENCE
// =============================================================================

#[test]
fn test_certify_universal_intro_with_local_var() {
    // Goal: ∀x:Nat. P(x) given h1 : Π(y:Nat). P(y)
    // Proof: Intro x. Apply h1 to x.
    // Kernel Term: λ(x:Nat). (h1 x)
    // This tests that x is correctly resolved as a local Var, not Global

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    let nat = Term::Global("Nat".to_string());

    // P : Nat -> Prop
    let predicate_type = Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(nat.clone()),
        body_type: Box::new(Term::Sort(Universe::Prop)),
    };
    ctx.add_declaration("P", predicate_type);

    // h1 : Π(y:Nat). (P y)
    let h1_type = Term::Pi {
        param: "y".to_string(),
        param_type: Box::new(nat.clone()),
        body_type: Box::new(Term::App(
            Box::new(Term::Global("P".to_string())),
            Box::new(Term::Var("y".to_string())),
        )),
    };
    ctx.add_declaration("h1", h1_type);

    // Inner proof: (h1 x) proves P(x)
    // Step 1: PremiseMatch h1 (proves ∀y. P(y))
    let h1_leaf = DerivationTree::leaf(
        ProofExpr::Atom("h1".to_string()),
        InferenceRule::PremiseMatch,
    );

    // Step 2: UniversalInst(x) on h1 (proves P(x))
    // CRITICAL: "x" here should become Var("x"), not Global("x")
    let inst_step = DerivationTree::new(
        ProofExpr::Predicate {
            name: "P".to_string(),
            args: vec![ProofTerm::Variable("x".to_string())],
            world: None,
        },
        InferenceRule::UniversalInst("x".to_string()),
        vec![h1_leaf],
    );

    // Outer: ∀x:Nat. P(x)
    let tree = DerivationTree::new(
        ProofExpr::ForAll {
            variable: "x".to_string(),
            body: Box::new(ProofExpr::Predicate {
                name: "P".to_string(),
                args: vec![ProofTerm::Variable("x".to_string())],
                world: None,
            }),
        },
        InferenceRule::UniversalIntro {
            variable: "x".to_string(),
            var_type: "Nat".to_string(),
        },
        vec![inst_step],
    );

    let cert_ctx = CertificationContext::new(&ctx);
    let term = certify(&tree, &cert_ctx).expect("Certification should succeed");

    println!("Certifier: Re-wrapping h1 as λx. (h1 x)");
    println!("Certified term: {}", term);

    // term should be: λ(x:Nat). (h1 x)
    match &term {
        Term::Lambda { param, body, .. } => {
            assert_eq!(param, "x");
            // body should be App(Global("h1"), Var("x"))
            match body.as_ref() {
                Term::App(func, arg) => {
                    assert!(
                        matches!(func.as_ref(), Term::Global(n) if n == "h1"),
                        "Expected Global(h1), got {:?}",
                        func
                    );
                    assert!(
                        matches!(arg.as_ref(), Term::Var(n) if n == "x"),
                        "Expected Var(x), got {:?}",
                        arg
                    );
                }
                _ => panic!("Expected App, got {:?}", body),
            }
        }
        _ => panic!("Expected Lambda, got {:?}", term),
    }

    // Type-check: should be Π(x:Nat). (P x)
    let inferred = infer_type(&ctx, &term).expect("Should type-check");
    println!("Inferred type: {}", inferred);

    assert!(matches!(inferred, Term::Pi { .. }));
}
