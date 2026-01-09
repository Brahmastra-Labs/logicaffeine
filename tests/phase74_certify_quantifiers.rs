//! =============================================================================
//! PHASE 74: CERTIFYING QUANTIFIERS (THE PI BRIDGE)
//! =============================================================================
//!
//! Universal Elimination: forall x.P(x) |- P(t) via Application
//!
//! The Bridge expands: Proof Trees with quantifiers become Lambda Terms.
//! The Kernel audits predicate logic proofs.
//!
//! Curry-Howard Correspondence:
//! - forall x. P(x)  <->  Pi(x:T). P x
//! - Universal Instantiation  <->  Function Application

use logos::kernel::prelude::StandardLibrary;
use logos::kernel::{infer_type, Context, Term, Universe};
use logos::proof::certifier::{certify, CertificationContext};
use logos::proof::{DerivationTree, InferenceRule, ProofExpr, ProofTerm};

// =============================================================================
// UNIVERSAL INSTANTIATION - SIMPLE CASE
// =============================================================================

#[test]
fn test_certify_universal_inst_simple() {
    // Goal: h1 : forall x:Nat. P(x) |- P(Zero)

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // P : Nat -> Prop (a predicate on naturals)
    let nat = Term::Global("Nat".to_string());
    let predicate_type = Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(nat.clone()),
        body_type: Box::new(Term::Sort(Universe::Prop)),
    };
    ctx.add_declaration("P", predicate_type);

    // h1 : Pi(x:Nat). (P x)
    // This is the kernel representation of "forall x:Nat. P(x)"
    let h1_type = Term::Pi {
        param: "x".to_string(),
        param_type: Box::new(nat.clone()),
        body_type: Box::new(Term::App(
            Box::new(Term::Global("P".to_string())),
            Box::new(Term::Var("x".to_string())),
        )),
    };
    ctx.add_declaration("h1", h1_type);

    // Build DerivationTree:
    // UniversalInst(Zero) with premise h1 proves P(Zero)
    let h1_leaf = DerivationTree::leaf(
        ProofExpr::Atom("h1".to_string()),
        InferenceRule::PremiseMatch,
    );

    let tree = DerivationTree::new(
        ProofExpr::Predicate {
            name: "P".to_string(),
            args: vec![ProofTerm::Constant("Zero".to_string())],
            world: None,
        },
        InferenceRule::UniversalInst("Zero".to_string()),
        vec![h1_leaf],
    );

    // Certify
    let cert_ctx = CertificationContext::new(&ctx);
    let term = certify(&tree, &cert_ctx).expect("Certification should succeed");

    println!("Certifier: UniversalInst(Zero) on h1 proves P(Zero)");
    println!("Certified term: {}", term);

    // Verify: term should type-check to (P Zero)
    let inferred = infer_type(&ctx, &term).expect("Should type-check");

    println!("Inferred type: {}", inferred);

    // Check it's App(P, Zero)
    match &inferred {
        Term::App(func, arg) => {
            assert!(
                matches!(func.as_ref(), Term::Global(n) if n == "P"),
                "Expected P, got {:?}",
                func
            );
            assert!(
                matches!(arg.as_ref(), Term::Global(n) if n == "Zero"),
                "Expected Zero, got {:?}",
                arg
            );
        }
        _ => panic!("Expected App(P, Zero), got {:?}", inferred),
    }
}

// =============================================================================
// UNIVERSAL INSTANTIATION + MODUS PONENS - THE CLASSIC SYLLOGISM
// =============================================================================

#[test]
fn test_certify_universal_inst_with_modus_ponens() {
    // The classic syllogism:
    // h1 : forall x:Nat. P(x) -> Q(x)
    // h2 : P(Zero)
    // Prove: Q(Zero)
    //
    // Tree structure:
    // ModusPonens
    //   UniversalInst(Zero) [proves P(Zero) -> Q(Zero)]
    //     PremiseMatch(h1)
    //   PremiseMatch(h2)

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    let nat = Term::Global("Nat".to_string());
    let prop = Term::Sort(Universe::Prop);

    // P, Q : Nat -> Prop
    let predicate_type = Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(nat.clone()),
        body_type: Box::new(prop.clone()),
    };
    ctx.add_declaration("P", predicate_type.clone());
    ctx.add_declaration("Q", predicate_type);

    // h1 : Pi(x:Nat). (P x) -> (Q x)
    let h1_type = Term::Pi {
        param: "x".to_string(),
        param_type: Box::new(nat.clone()),
        body_type: Box::new(Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(Term::App(
                Box::new(Term::Global("P".to_string())),
                Box::new(Term::Var("x".to_string())),
            )),
            body_type: Box::new(Term::App(
                Box::new(Term::Global("Q".to_string())),
                Box::new(Term::Var("x".to_string())),
            )),
        }),
    };
    ctx.add_declaration("h1", h1_type);

    // h2 : (P Zero)
    let h2_type = Term::App(
        Box::new(Term::Global("P".to_string())),
        Box::new(Term::Global("Zero".to_string())),
    );
    ctx.add_declaration("h2", h2_type);

    // Build tree
    let h1_leaf = DerivationTree::leaf(
        ProofExpr::Atom("h1".to_string()),
        InferenceRule::PremiseMatch,
    );

    // UniversalInst(Zero) on h1 gives: (P Zero) -> (Q Zero)
    let inst_step = DerivationTree::new(
        ProofExpr::Implies(
            Box::new(ProofExpr::Predicate {
                name: "P".to_string(),
                args: vec![ProofTerm::Constant("Zero".to_string())],
                world: None,
            }),
            Box::new(ProofExpr::Predicate {
                name: "Q".to_string(),
                args: vec![ProofTerm::Constant("Zero".to_string())],
                world: None,
            }),
        ),
        InferenceRule::UniversalInst("Zero".to_string()),
        vec![h1_leaf],
    );

    let h2_leaf = DerivationTree::leaf(
        ProofExpr::Atom("h2".to_string()),
        InferenceRule::PremiseMatch,
    );

    // ModusPonens: (P Zero -> Q Zero), (P Zero) |- Q(Zero)
    let tree = DerivationTree::new(
        ProofExpr::Predicate {
            name: "Q".to_string(),
            args: vec![ProofTerm::Constant("Zero".to_string())],
            world: None,
        },
        InferenceRule::ModusPonens,
        vec![inst_step, h2_leaf],
    );

    // Certify
    let cert_ctx = CertificationContext::new(&ctx);
    let term = certify(&tree, &cert_ctx).expect("Certification should succeed");

    println!("Certifier: Syllogism proof");
    println!("  h1 : forall x. P(x) -> Q(x)");
    println!("  h2 : P(Zero)");
    println!("  |- Q(Zero)");
    println!("Certified term: {}", term);

    // Verify: should be ((h1 Zero) h2) : (Q Zero)
    let inferred = infer_type(&ctx, &term).expect("Should type-check");

    println!("Inferred type: {}", inferred);

    // Check it's App(Q, Zero)
    match &inferred {
        Term::App(func, arg) => {
            assert!(
                matches!(func.as_ref(), Term::Global(n) if n == "Q"),
                "Expected Q, got {:?}",
                func
            );
            assert!(
                matches!(arg.as_ref(), Term::Global(n) if n == "Zero"),
                "Expected Zero, got {:?}",
                arg
            );
        }
        _ => panic!("Expected App(Q, Zero), got {:?}", inferred),
    }
}
