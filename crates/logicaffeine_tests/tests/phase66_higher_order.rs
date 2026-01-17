// =============================================================================
// PHASE 66: HIGHER-ORDER UNIFICATION (BETA-REDUCTION)
// =============================================================================
//
// "The Crawl Before the Walk"
//
// This phase implements beta-reduction for lambda calculus:
//   (λx. P(x))(a) → P(a)
//
// This is the computational engine that underpins type theory.
// In CIC, computation equals truth: if 2+2 reduces to 4, then P(2+2) = P(4).
//
// We implement NORMALIZATION (deterministic computation), not full
// higher-order unification (which would require Huet's algorithm).

use logicaffeine_proof::{BackwardChainer, ProofExpr, ProofTerm};

// =============================================================================
// BASIC BETA-REDUCTION
// =============================================================================

#[test]
fn test_beta_reduction_basic() {
    // Premise: Run(John)
    let premise = ProofExpr::Predicate {
        name: "run".to_string(),
        args: vec![ProofTerm::Constant("John".to_string())],
        world: None,
    };

    // Goal: (λx. Run(x))(John)
    // Should beta-reduce to: Run(John)
    // Then match the premise via PremiseMatch
    let goal = ProofExpr::App(
        Box::new(ProofExpr::Lambda {
            variable: "x".to_string(),
            body: Box::new(ProofExpr::Predicate {
                name: "run".to_string(),
                args: vec![ProofTerm::Variable("x".to_string())],
                world: None,
            }),
        }),
        Box::new(ProofExpr::Atom("John".to_string())),
    );

    let mut engine = BackwardChainer::new();
    engine.add_axiom(premise);

    let result = engine.prove(goal);
    assert!(
        result.is_ok(),
        "Beta-reduction should enable proving (λx.Run(x))(John) from Run(John)"
    );
    println!("Basic beta-reduction proof:\n{}", result.unwrap());
}

// =============================================================================
// NESTED BETA-REDUCTION
// =============================================================================

#[test]
fn test_beta_reduction_nested() {
    // Premise: P(A, B)
    let premise = ProofExpr::Predicate {
        name: "p".to_string(),
        args: vec![
            ProofTerm::Constant("A".to_string()),
            ProofTerm::Constant("B".to_string()),
        ],
        world: None,
    };

    // Goal: (λx. (λy. P(x, y))(B))(A)
    // Should reduce: (λx. P(x, B))(A) → P(A, B)
    let inner_lambda = ProofExpr::Lambda {
        variable: "y".to_string(),
        body: Box::new(ProofExpr::Predicate {
            name: "p".to_string(),
            args: vec![
                ProofTerm::Variable("x".to_string()),
                ProofTerm::Variable("y".to_string()),
            ],
            world: None,
        }),
    };

    let inner_app = ProofExpr::App(
        Box::new(inner_lambda),
        Box::new(ProofExpr::Atom("B".to_string())),
    );

    let outer_lambda = ProofExpr::Lambda {
        variable: "x".to_string(),
        body: Box::new(inner_app),
    };

    let goal = ProofExpr::App(
        Box::new(outer_lambda),
        Box::new(ProofExpr::Atom("A".to_string())),
    );

    let mut engine = BackwardChainer::new();
    engine.add_axiom(premise);

    let result = engine.prove(goal);
    assert!(
        result.is_ok(),
        "Nested beta-reduction should work: (λx.(λy.P(x,y))(B))(A) → P(A,B)"
    );
    println!("Nested beta-reduction proof:\n{}", result.unwrap());
}

// =============================================================================
// BETA-REDUCTION IN PREMISES
// =============================================================================

#[test]
fn test_lambda_in_premise() {
    // Premise: (λx. Run(x))(John) - a beta-redex
    let premise = ProofExpr::App(
        Box::new(ProofExpr::Lambda {
            variable: "x".to_string(),
            body: Box::new(ProofExpr::Predicate {
                name: "run".to_string(),
                args: vec![ProofTerm::Variable("x".to_string())],
                world: None,
            }),
        }),
        Box::new(ProofExpr::Atom("John".to_string())),
    );

    // Goal: Run(John) - the reduced form
    let goal = ProofExpr::Predicate {
        name: "run".to_string(),
        args: vec![ProofTerm::Constant("John".to_string())],
        world: None,
    };

    let mut engine = BackwardChainer::new();
    engine.add_axiom(premise);

    let result = engine.prove(goal);
    assert!(
        result.is_ok(),
        "Premises should also be beta-reduced before matching"
    );
    println!("Premise beta-reduction proof:\n{}", result.unwrap());
}

// =============================================================================
// NO REDUCTION WITHOUT APPLICATION
// =============================================================================

#[test]
fn test_no_reduction_without_application() {
    // Premise: The lambda itself (not applied)
    let lambda = ProofExpr::Lambda {
        variable: "x".to_string(),
        body: Box::new(ProofExpr::Predicate {
            name: "run".to_string(),
            args: vec![ProofTerm::Variable("x".to_string())],
            world: None,
        }),
    };

    // Goal: Same lambda
    // Should match directly without reduction (nothing to reduce)
    let mut engine = BackwardChainer::new();
    engine.add_axiom(lambda.clone());

    let result = engine.prove(lambda);
    assert!(
        result.is_ok(),
        "Lambda without application should match directly"
    );
    println!("No-reduction proof:\n{}", result.unwrap());
}
