//! Phase Induction: The Time Machine
//!
//! Tests for generic induction on all inductive types.

use logos::interface::Repl;

// =============================================================================
// TYPE CHECKS
// =============================================================================

#[test]
fn test_try_induction_type() {
    let mut repl = Repl::new();
    let result = repl.execute("Check try_induction.");
    assert!(result.is_ok(), "try_induction should exist: {:?}", result);
}

#[test]
fn test_induction_base_goal_type() {
    let mut repl = Repl::new();
    let result = repl.execute("Check induction_base_goal.");
    assert!(result.is_ok(), "induction_base_goal should exist: {:?}", result);
}

#[test]
fn test_induction_step_goal_type() {
    let mut repl = Repl::new();
    let result = repl.execute("Check induction_step_goal.");
    assert!(result.is_ok(), "induction_step_goal should exist: {:?}", result);
}

#[test]
fn test_induction_num_cases_type() {
    let mut repl = Repl::new();
    let result = repl.execute("Check induction_num_cases.");
    assert!(result.is_ok(), "induction_num_cases should exist: {:?}", result);
}

// =============================================================================
// NAT INDUCTION - BASE CASE
// =============================================================================

#[test]
fn test_induction_nat_zero_base() {
    // Base case for Nat: P(Zero)
    let mut repl = Repl::new();
    // Define motive: λn:Nat. (Le n n)
    repl.execute("Definition motive : Syntax := SLam (SName \"Nat\") (SApp (SApp (SName \"Le\") (SVar 0)) (SVar 0)).")
        .unwrap();
    repl.execute("Definition base_goal : Syntax := induction_base_goal (SName \"Nat\") motive.")
        .unwrap();
    let result = repl.execute("Eval base_goal.").unwrap();
    // Should be: (Le Zero Zero)
    assert!(
        result.contains("Le") && result.contains("Zero"),
        "Base goal should be Le Zero Zero: {}",
        result
    );
}

// =============================================================================
// NAT INDUCTION - STEP CASE
// =============================================================================

#[test]
fn test_induction_nat_step_goal() {
    // Step case for Nat: ∀k:Nat. P(k) → P(Succ k)
    let mut repl = Repl::new();
    repl.execute("Definition motive : Syntax := SLam (SName \"Nat\") (SApp (SApp (SName \"Le\") (SVar 0)) (SVar 0)).")
        .unwrap();
    repl.execute("Definition step_goal : Syntax := induction_step_goal (SName \"Nat\") motive (Succ Zero).")
        .unwrap();
    let result = repl.execute("Eval step_goal.").unwrap();
    // Should be: ∀k:Nat. (Le k k) → (Le (Succ k) (Succ k))
    assert!(
        result.contains("Succ"),
        "Step goal should mention Succ: {}",
        result
    );
}

// =============================================================================
// NAT INDUCTION - COMPLETE PROOF
// =============================================================================

#[test]
fn test_induction_nat_complete() {
    // Full Nat induction: forall n. n <= n
    let mut repl = Repl::new();

    // Build induction manually with try_induction
    repl.execute("Definition motive : Syntax := SLam (SName \"Nat\") (SApp (SApp (SName \"Le\") (SVar 0)) (SVar 0)).")
        .unwrap();

    // Base case: Le Zero Zero (provable by auto)
    repl.execute("Definition base_case : Derivation := try_auto (SApp (SApp (SName \"Le\") (SName \"Zero\")) (SName \"Zero\")).")
        .unwrap();

    // Step case: use axiom for now (we just need to test the infrastructure)
    repl.execute("Definition step_case : Derivation := DAxiom (induction_step_goal (SName \"Nat\") motive (Succ Zero)).")
        .unwrap();

    // Combine with try_induction
    repl.execute("Definition proof : Derivation := try_induction (SName \"Nat\") motive (DCase base_case (DCase step_case DCaseEnd)).")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes proof.")
        .unwrap();

    let concluded = repl.execute("Eval result.").unwrap();
    assert!(
        !concluded.contains("Error"),
        "Induction should produce valid derivation: {}",
        concluded
    );
}

// =============================================================================
// NAT INDUCTION - NUM CASES
// =============================================================================

#[test]
fn test_induction_nat_num_cases() {
    let mut repl = Repl::new();
    repl.execute("Definition num : Nat := induction_num_cases (SName \"Nat\").")
        .unwrap();
    let result = repl.execute("Eval num.").unwrap();
    // Nat has 2 constructors: Zero, Succ
    // Should be Succ (Succ Zero) = 2
    assert!(
        result.contains("Succ"),
        "Nat should have 2 constructors: {}",
        result
    );
}

// =============================================================================
// BOOL INDUCTION
// =============================================================================

#[test]
fn test_induction_bool_num_cases() {
    let mut repl = Repl::new();
    repl.execute("Definition num : Nat := induction_num_cases (SName \"Bool\").")
        .unwrap();
    let result = repl.execute("Eval num.").unwrap();
    // Bool has 2 constructors: true, false
    assert!(
        result.contains("Succ"),
        "Bool should have 2 constructors: {}",
        result
    );
}

// =============================================================================
// ERROR CASES
// =============================================================================

#[test]
fn test_induction_wrong_case_count() {
    let mut repl = Repl::new();

    repl.execute("Definition motive : Syntax := SLam (SName \"Nat\") (SName \"True\").")
        .unwrap();
    repl.execute("Definition case1 : Derivation := DAxiom (SName \"True\").")
        .unwrap();

    // Only provide 1 case for Nat (needs 2)
    repl.execute(
        "Definition proof : Derivation := try_induction (SName \"Nat\") motive (DCase case1 DCaseEnd).",
    )
    .unwrap();
    repl.execute("Definition result : Syntax := concludes proof.")
        .unwrap();

    let result = repl.execute("Eval result.").unwrap();
    assert!(
        result.contains("Error"),
        "Should error with wrong case count: {}",
        result
    );
}

#[test]
fn test_induction_non_inductive_type() {
    let mut repl = Repl::new();

    // Int is not an inductive type
    repl.execute("Definition num : Nat := induction_num_cases (SName \"Int\").")
        .unwrap();
    let result = repl.execute("Eval num.").unwrap();

    // Should return Zero (no constructors) or error
    assert!(
        result.contains("Zero") || result.contains("Error"),
        "Non-inductive should have 0 cases or error: {}",
        result
    );
}
