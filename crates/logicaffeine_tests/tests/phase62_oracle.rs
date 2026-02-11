// =============================================================================
// PHASE 62: THE ORACLE LINK - TEST SUITE
// =============================================================================
// TDD RED: These tests define the specification for Z3 Oracle fallback.
// The proof engine must fall back to Z3 when structural proofs fail.
//
// The Hybrid Architecture:
// - Tier 1 (Prover): Explains "Why" using DerivationTree
// - Tier 2 (Oracle): If Tier 1 fails, checks validity via Z3

#![cfg(feature = "verification")]

use logicaffeine_proof::{BackwardChainer, InferenceRule, ProofExpr, ProofTerm};

// =============================================================================
// HELPER FUNCTIONS
// =============================================================================

/// Create a variable term
fn var(name: &str) -> ProofTerm {
    ProofTerm::Variable(name.into())
}

/// Create a constant term (for numbers)
fn num(n: i64) -> ProofTerm {
    ProofTerm::Constant(n.to_string())
}

/// Create a comparison predicate: Gt(x, y) means x > y
fn gt(left: ProofTerm, right: ProofTerm) -> ProofExpr {
    ProofExpr::Predicate {
        name: "Gt".into(),
        args: vec![left, right],
        world: None,
    }
}

/// Create a comparison predicate: Lt(x, y) means x < y
fn lt(left: ProofTerm, right: ProofTerm) -> ProofExpr {
    ProofExpr::Predicate {
        name: "Lt".into(),
        args: vec![left, right],
        world: None,
    }
}

/// Create an arithmetic function: Add(x, y)
fn add(left: ProofTerm, right: ProofTerm) -> ProofTerm {
    ProofTerm::Function("Add".into(), vec![left, right])
}

/// Create implication: P → Q
fn implies(p: ProofExpr, q: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(p), Box::new(q))
}

/// Create conjunction: P ∧ Q
fn and(p: ProofExpr, q: ProofExpr) -> ProofExpr {
    ProofExpr::And(Box::new(p), Box::new(q))
}

// =============================================================================
// ORACLE FALLBACK TESTS
// =============================================================================

#[test]
fn test_oracle_fallback_for_arithmetic() {
    // Goal: x > 10 → x > 5
    // No axioms provided - structural prover cannot derive this.
    // Z3 knows arithmetic semantics, so oracle should catch it.

    let x = var("x");

    // x > 10
    let gt_10 = gt(x.clone(), num(10));

    // x > 5
    let gt_5 = gt(x, num(5));

    // Goal: (x > 10) → (x > 5)
    let goal = implies(gt_10, gt_5);

    let mut engine = BackwardChainer::new();
    // NO axioms added - prover must fall back to oracle

    let result = engine.prove(goal);

    assert!(result.is_ok(), "Oracle should verify arithmetic implication");

    let proof = result.unwrap();
    assert!(
        matches!(proof.rule, InferenceRule::OracleVerification(_)),
        "Should use OracleVerification rule, got {:?}",
        proof.rule
    );

    println!("✓ Oracle verified: x > 10 → x > 5");
    println!("{}", proof.display_tree());
}

#[test]
fn test_oracle_handles_linear_arithmetic() {
    // Goal: (x + y > 10 ∧ y = 5) → x > 5
    // This requires linear arithmetic reasoning.

    let x = var("x");
    let y = var("y");

    // x + y > 10
    let sum_gt_10 = gt(add(x.clone(), y.clone()), num(10));

    // y = 5 (as identity)
    let y_eq_5 = ProofExpr::Identity(y, num(5));

    // x > 5
    let x_gt_5 = gt(x, num(5));

    // Goal: (x + y > 10 ∧ y = 5) → x > 5
    let goal = implies(and(sum_gt_10, y_eq_5), x_gt_5);

    let mut engine = BackwardChainer::new();
    // NO axioms - oracle must handle

    let result = engine.prove(goal);

    assert!(result.is_ok(), "Oracle should verify linear arithmetic");

    let proof = result.unwrap();
    assert!(
        matches!(proof.rule, InferenceRule::OracleVerification(_)),
        "Should use OracleVerification rule, got {:?}",
        proof.rule
    );

    println!("✓ Oracle verified: (x + y > 10 ∧ y = 5) → x > 5");
}

#[test]
fn test_oracle_skipped_when_structural_succeeds() {
    // Goal: P → P (trivially provable structurally)
    // Oracle should NOT be called when prover succeeds.

    let mut engine = BackwardChainer::new();

    // Axiom: P
    engine.add_axiom(ProofExpr::Atom("P".into()));

    // Goal: P (should match directly)
    let goal = ProofExpr::Atom("P".into());

    let result = engine.prove(goal);

    assert!(result.is_ok(), "Should prove P directly");

    let proof = result.unwrap();
    // Should NOT use oracle - should use PremiseMatch
    assert!(
        matches!(proof.rule, InferenceRule::PremiseMatch),
        "Should use PremiseMatch, not Oracle. Got {:?}",
        proof.rule
    );

    println!("✓ Structural prover handled P directly");
}

#[test]
fn test_oracle_for_universal_arithmetic() {
    // Goal: ∀x. x > 0 → x > -1
    // Universal arithmetic truth - oracle should verify.

    let x = var("x");

    // x > 0
    let x_gt_0 = gt(x.clone(), num(0));

    // x > -1
    let x_gt_neg1 = gt(x.clone(), num(-1));

    // ∀x. (x > 0 → x > -1)
    let goal = ProofExpr::ForAll {
        variable: "x".into(),
        body: Box::new(implies(x_gt_0, x_gt_neg1)),
    };

    let mut engine = BackwardChainer::new();

    let result = engine.prove(goal);

    assert!(result.is_ok(), "Oracle should verify universal arithmetic");

    let proof = result.unwrap();
    assert!(
        matches!(proof.rule, InferenceRule::OracleVerification(_)),
        "Should use OracleVerification rule, got {:?}",
        proof.rule
    );

    println!("✓ Oracle verified: ∀x. x > 0 → x > -1");
}

#[test]
#[cfg_attr(not(feature = "verification"), ignore)]
fn test_oracle_respects_context_assumptions() {
    use logicaffeine_proof::ProofGoal;

    // If we have x > 10 as an assumption (in context), prove x > 5.
    // The oracle should use assumptions from the goal context.
    let x = var("x");
    let assumption = gt(x.clone(), num(10)); // x > 10
    let target = gt(x, num(5)); // x > 5

    let goal = ProofGoal::with_context(target, vec![assumption]);
    let mut engine = BackwardChainer::new();

    let result = engine.prove_with_goal(goal);
    assert!(
        result.is_ok(),
        "Should prove x > 5 given x > 10: {:?}",
        result
    );
    println!("Proved: x > 5 (given x > 10)");
}

// =============================================================================
// ORACLE FAILURE TESTS - Z3 should return Unknown/Unsat for invalid goals
// =============================================================================

#[test]
fn test_oracle_rejects_false_arithmetic() {
    // Goal: x > 10 → x < 5 (FALSE - contradictory)
    // Oracle should NOT prove this.

    let x = var("x");

    // x > 10
    let gt_10 = gt(x.clone(), num(10));

    // x < 5
    let lt_5 = lt(x, num(5));

    // Goal: (x > 10) → (x < 5) -- FALSE
    let goal = implies(gt_10, lt_5);

    let mut engine = BackwardChainer::new();

    let result = engine.prove(goal);

    // Should fail - this is not valid
    assert!(
        result.is_err(),
        "Oracle should reject invalid arithmetic: x > 10 → x < 5"
    );

    println!("✓ Oracle correctly rejected: x > 10 → x < 5");
}
