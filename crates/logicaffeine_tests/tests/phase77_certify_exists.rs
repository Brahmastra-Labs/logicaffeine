//! =============================================================================
//! PHASE 77: CERTIFYING EXISTENTIALS (THE HIDDEN BRIDGE)
//! =============================================================================
//!
//! ExistentialIntro: P(w) ⊢ ∃x.P(x)
//!
//! The Hidden Bridge: Existential proofs become witness applications.
//! The Kernel validates that the witness satisfies the predicate.
//!
//! Curry-Howard Correspondence:
//! - Existential Type  <->  Sigma Type (Ex)
//! - Witness + Proof   <->  Constructor Application
//! - ∃x.P(x)           <->  Ex A (λx.P(x))

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, Context, Term, Universe};
use logicaffeine_proof::certifier::{certify, CertificationContext};
use logicaffeine_proof::{DerivationTree, InferenceRule, ProofExpr, ProofTerm};

// =============================================================================
// PRELUDE TEST - Ex and witness must exist
// =============================================================================

#[test]
fn test_ex_type_in_prelude() {
    // Verify Ex and witness are properly registered
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // Ex : Π(A:Type 0). (A → Prop) → Prop
    let ex_type = infer_type(&ctx, &Term::Global("Ex".to_string()));
    assert!(ex_type.is_ok(), "Ex should be in prelude: {:?}", ex_type);

    // witness : Π(A:Type 0). Π(P:A→Prop). Π(x:A). P(x) → Ex A P
    let witness_type = infer_type(&ctx, &Term::Global("witness".to_string()));
    assert!(
        witness_type.is_ok(),
        "witness should be in prelude: {:?}",
        witness_type
    );

    println!("Certifier: Ex type in prelude");
    println!("Ex type: {}", ex_type.unwrap());
    println!("witness type: {}", witness_type.unwrap());
}

// =============================================================================
// SIMPLE EXISTENTIAL INTRODUCTION
// =============================================================================

#[test]
fn test_certify_existential_intro_simple() {
    // Goal: ∃x:Nat. P(x) given P(c) where c is a witness
    // Proof: ExistentialIntro("c") with premise proving P(c)
    // Kernel: witness Nat P c h

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    let nat = Term::Global("Nat".to_string());
    let prop = Term::Sort(Universe::Prop);

    // P : Nat → Prop (a predicate)
    ctx.add_declaration(
        "P",
        Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(nat.clone()),
            body_type: Box::new(prop.clone()),
        },
    );

    // c : Nat (the witness)
    ctx.add_declaration("c", nat.clone());

    // h : P(c) (proof that predicate holds for witness)
    ctx.add_declaration(
        "h",
        Term::App(
            Box::new(Term::Global("P".to_string())),
            Box::new(Term::Global("c".to_string())),
        ),
    );

    // Premise: h proves P(c) - use the hypothesis name, not the proposition
    let premise = DerivationTree::leaf(
        ProofExpr::Atom("h".to_string()),
        InferenceRule::PremiseMatch,
    );

    // Conclusion: ∃x:Nat. P(x)
    let tree = DerivationTree::new(
        ProofExpr::Exists {
            variable: "x".to_string(),
            body: Box::new(ProofExpr::Predicate {
                name: "P".to_string(),
                args: vec![ProofTerm::Variable("x".to_string())],
                world: None,
            }),
        },
        InferenceRule::ExistentialIntro {
            witness: "c".to_string(),
            witness_type: "Nat".to_string(),
        },
        vec![premise],
    );

    let cert_ctx = CertificationContext::new(&ctx);
    let term = certify(&tree, &cert_ctx).expect("Certification should succeed");

    println!("Certifier: Existential intro ∃x:Nat. P(x)");
    println!("Certified term: {}", term);

    // Type-check: should be Ex Nat P
    let inferred = infer_type(&ctx, &term).expect("Should type-check");
    println!("Inferred type: {}", inferred);

    // Verify type is an application of Ex
    let type_str = format!("{}", inferred);
    assert!(
        type_str.contains("Ex"),
        "Expected Ex type, got: {}",
        type_str
    );
}

// =============================================================================
// EXISTENTIAL WITH CONCRETE WITNESS
// =============================================================================

#[test]
fn test_certify_existential_intro_concrete_witness() {
    // Goal: ∃n:Nat. n = Zero (there exists a natural number equal to Zero)
    // Witness: Zero itself
    // This tests that we can use a constructor as a witness

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    let nat = Term::Global("Nat".to_string());
    let zero = Term::Global("Zero".to_string());

    // refl_zero : Eq Nat Zero Zero (proof that Zero equals Zero)
    // This is: refl Nat Zero applied
    let eq_zero_zero = Term::App(
        Box::new(Term::App(
            Box::new(Term::App(
                Box::new(Term::Global("Eq".to_string())),
                Box::new(nat.clone()),
            )),
            Box::new(zero.clone()),
        )),
        Box::new(zero.clone()),
    );
    ctx.add_declaration("refl_zero", eq_zero_zero);

    // Premise: refl_zero proves Zero = Zero
    let premise = DerivationTree::leaf(
        ProofExpr::Atom("refl_zero".to_string()),
        InferenceRule::PremiseMatch,
    );

    // Conclusion: ∃n:Nat. n = Zero
    let tree = DerivationTree::new(
        ProofExpr::Exists {
            variable: "n".to_string(),
            body: Box::new(ProofExpr::Identity(
                ProofTerm::Variable("n".to_string()),
                ProofTerm::Function("Zero".to_string(), vec![]),
            )),
        },
        InferenceRule::ExistentialIntro {
            witness: "Zero".to_string(),
            witness_type: "Nat".to_string(),
        },
        vec![premise],
    );

    let cert_ctx = CertificationContext::new(&ctx);
    let term = certify(&tree, &cert_ctx).expect("Certification should succeed");

    println!("Certifier: Existential intro with Zero witness");
    println!("Certified term: {}", term);

    // The term should contain the witness constructor
    let term_str = format!("{}", term);
    assert!(
        term_str.contains("witness") && term_str.contains("Zero"),
        "Expected witness application with Zero: {}",
        term_str
    );
}
