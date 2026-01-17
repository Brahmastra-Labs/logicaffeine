//! =============================================================================
//! PHASE 76: CERTIFYING INDUCTION (THE RECURSIVE BRIDGE)
//! =============================================================================
//!
//! StructuralInduction: P(0), ∀k(P(k)→P(Sk)) ⊢ ∀n.P(n)
//!
//! The Final Bridge: Induction proofs become Fixpoint terms.
//! The Kernel audits recursive reasoning over inductive types.
//!
//! Curry-Howard Correspondence:
//! - Induction Principle  <->  Dependent Eliminator
//! - Base Case + Step Case  <->  Match Cases
//! - Inductive Hypothesis  <->  Recursive Call

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, Context, Term, Universe};
use logicaffeine_proof::certifier::{certify, CertificationContext};
use logicaffeine_proof::{DerivationTree, InferenceRule, ProofExpr, ProofTerm};

// =============================================================================
// TRIVIAL INDUCTION - NO IH USAGE
// =============================================================================

#[test]
fn test_certify_induction_trivial() {
    // Goal: ∀n:Nat. True (trivial - doesn't use IH)
    // Base: I proves True
    // Step: I proves True (IH unused)
    // Result: fix rec. λn. match n { Zero => I, Succ k => I }

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // Base case: I proves True (I : True in StandardLibrary)
    // Conclusion is the proof term name, which has type True
    let base_proof = DerivationTree::leaf(
        ProofExpr::Atom("I".to_string()),
        InferenceRule::PremiseMatch,
    );

    // Step case: I proves True (not using IH)
    let step_proof = DerivationTree::leaf(
        ProofExpr::Atom("I".to_string()),
        InferenceRule::PremiseMatch,
    );

    // Induction tree
    // Type info is now explicit in the InferenceRule struct
    let tree = DerivationTree::new(
        ProofExpr::ForAll {
            variable: "n".to_string(),
            body: Box::new(ProofExpr::Atom("True".to_string())),
        },
        InferenceRule::StructuralInduction {
            variable: "n".to_string(),
            ind_type: "Nat".to_string(),
            step_var: "k".to_string(), // Not used for IH in this trivial test
        },
        vec![base_proof, step_proof],
    );

    let cert_ctx = CertificationContext::new(&ctx);
    let term = certify(&tree, &cert_ctx).expect("Certification should succeed");

    println!("Certifier: Trivial induction ∀n:Nat. True");
    println!("Certified term: {}", term);

    // Verify structure: Fix { Lambda { Match { ... } } }
    assert!(
        matches!(term, Term::Fix { .. }),
        "Expected Fix, got {:?}",
        term
    );

    // Type-check: should be Π(n:Nat). True
    let inferred = infer_type(&ctx, &term).expect("Should type-check");
    println!("Inferred type: {}", inferred);

    assert!(matches!(inferred, Term::Pi { .. }));
}

// =============================================================================
// INDUCTION WITH IH - THE REAL TEST
// =============================================================================

#[test]
fn test_certify_induction_with_ih() {
    // Goal: ∀n:Nat. P(n) given:
    //   base_hyp : P(Zero)
    //   step_hyp : ∀k:Nat. P(k) → P(Succ(k))
    //
    // This is the REAL test: step case uses IH
    // Step proof: Apply step_hyp to k, then apply to IH
    //
    // Kernel: fix rec. λn. match n {
    //   Zero => base_hyp,
    //   Succ k => step_hyp k (rec k)  // IH becomes (rec k)
    // }

    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    let nat = Term::Global("Nat".to_string());
    let prop = Term::Sort(Universe::Prop);

    // P : Nat → Prop
    ctx.add_declaration(
        "P",
        Term::Pi {
            param: "_".to_string(),
            param_type: Box::new(nat.clone()),
            body_type: Box::new(prop.clone()),
        },
    );

    // base_hyp : P(Zero)
    ctx.add_declaration(
        "base_hyp",
        Term::App(
            Box::new(Term::Global("P".to_string())),
            Box::new(Term::Global("Zero".to_string())),
        ),
    );

    // step_hyp : Π(k:Nat). P(k) → P(Succ(k))
    ctx.add_declaration(
        "step_hyp",
        Term::Pi {
            param: "k".to_string(),
            param_type: Box::new(nat.clone()),
            body_type: Box::new(Term::Pi {
                param: "_".to_string(),
                param_type: Box::new(Term::App(
                    Box::new(Term::Global("P".to_string())),
                    Box::new(Term::Var("k".to_string())),
                )),
                body_type: Box::new(Term::App(
                    Box::new(Term::Global("P".to_string())),
                    Box::new(Term::App(
                        Box::new(Term::Global("Succ".to_string())),
                        Box::new(Term::Var("k".to_string())),
                    )),
                )),
            }),
        },
    );

    // Base case proof: base_hyp
    let base_proof = DerivationTree::leaf(
        ProofExpr::Atom("base_hyp".to_string()),
        InferenceRule::PremiseMatch,
    );

    // Step case proof: (step_hyp k) IH
    // Where IH should become (rec k)

    // First: step_hyp reference
    let step_hyp_ref = DerivationTree::leaf(
        ProofExpr::Atom("step_hyp".to_string()),
        InferenceRule::PremiseMatch,
    );

    // Apply to k: (step_hyp k) : P(k) → P(Succ(k))
    let step_hyp_k = DerivationTree::new(
        ProofExpr::Implies(
            Box::new(ProofExpr::Predicate {
                name: "P".to_string(),
                args: vec![ProofTerm::Variable("k".to_string())],
                world: None,
            }),
            Box::new(ProofExpr::Predicate {
                name: "P".to_string(),
                args: vec![ProofTerm::Function(
                    "Succ".to_string(),
                    vec![ProofTerm::Variable("k".to_string())],
                )],
                world: None,
            }),
        ),
        InferenceRule::UniversalInst("k".to_string()),
        vec![step_hyp_ref],
    );

    // IH: P(k) - this is what should become (rec k)
    let ih_ref = DerivationTree::leaf(
        ProofExpr::Predicate {
            name: "P".to_string(),
            args: vec![ProofTerm::Variable("k".to_string())],
            world: None,
        },
        InferenceRule::PremiseMatch, // IH from induction context
    );

    // Modus ponens: (step_hyp k) IH : P(Succ(k))
    let step_proof = DerivationTree::new(
        ProofExpr::Predicate {
            name: "P".to_string(),
            args: vec![ProofTerm::Function(
                "Succ".to_string(),
                vec![ProofTerm::Variable("k".to_string())],
            )],
            world: None,
        },
        InferenceRule::ModusPonens,
        vec![step_hyp_k, ih_ref],
    );

    // Induction tree - type info explicit in InferenceRule
    let tree = DerivationTree::new(
        ProofExpr::ForAll {
            variable: "n".to_string(),
            body: Box::new(ProofExpr::Predicate {
                name: "P".to_string(),
                args: vec![ProofTerm::Variable("n".to_string())],
                world: None,
            }),
        },
        InferenceRule::StructuralInduction {
            variable: "n".to_string(),
            ind_type: "Nat".to_string(),
            step_var: "k".to_string(), // Must match the "k" used in step case tree
        },
        vec![base_proof, step_proof],
    );

    let cert_ctx = CertificationContext::new(&ctx);
    let term = certify(&tree, &cert_ctx).expect("Certification should succeed");

    println!("Certifier: Induction with IH ∀n:Nat. P(n)");
    println!("Certified term: {}", term);

    // Verify the IH became a recursive call
    // Term should contain: App(Var("rec_n"), Var("k"))
    let term_str = format!("{}", term);
    assert!(
        term_str.contains("rec") && term_str.contains("k"),
        "Expected recursive call in step case: {}",
        term_str
    );

    // Type-check
    let inferred = infer_type(&ctx, &term).expect("Should type-check");
    println!("Inferred type: {}", inferred);
}
