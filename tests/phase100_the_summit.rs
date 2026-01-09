//! Phase 100: The Summit (Completeness & The Standard Library)
//!
//! Implements congruence in the deep embedding:
//! - DCong: Congruence proof constructor
//! - try_cong: Congruence tactic
//! - Integration: Prove ∀n. add n Zero = n

use logos::interface::Repl;

// =============================================================================
// DCONG: TYPE CHECK
// =============================================================================

#[test]
fn test_dcong_type() {
    let mut repl = Repl::new();
    let result = repl.execute("Check DCong.").expect("Check DCong");
    assert_eq!(result, "DCong : Syntax -> Derivation -> Derivation");
}

// =============================================================================
// DCONG: BASIC CONGRUENCE
// =============================================================================

#[test]
fn test_dcong_succ_congruence() {
    let mut repl = Repl::new();

    // eq_proof: Eq Nat Zero Zero (by reflexivity)
    repl.execute(r#"Definition eq_proof : Derivation := DRefl (SName "Nat") (SName "Zero")."#)
        .expect("Define eq_proof");

    // context: λx. Succ x
    repl.execute(r#"Definition context : Syntax := SLam (SName "Nat") (SApp (SName "Succ") (SVar 0))."#)
        .expect("Define context");

    // DCong context eq_proof should prove Eq Nat (Succ Zero) (Succ Zero)
    repl.execute("Definition cong_proof : Derivation := DCong context eq_proof.")
        .expect("Define cong_proof");

    let result = repl.execute("Eval (concludes cong_proof).").expect("Eval");

    // Expected: Eq Nat (Succ Zero) (Succ Zero)
    assert!(result.contains("Eq"), "Should contain Eq: {}", result);
    assert!(result.contains("Succ"), "Should contain Succ: {}", result);
    assert!(result.contains("Zero"), "Should contain Zero: {}", result);
}

#[test]
fn test_dcong_add_congruence() {
    let mut repl = Repl::new();

    // eq_proof: Eq Nat x x (reflexivity on a variable representation)
    repl.execute(r#"Definition eq_proof : Derivation := DRefl (SName "Nat") (SLit 5)."#)
        .expect("Define eq_proof");

    // context: λx. add x Zero
    repl.execute(r#"Definition context : Syntax := SLam (SName "Nat") (SApp (SApp (SName "add") (SVar 0)) (SName "Zero"))."#)
        .expect("Define context");

    // DCong context eq_proof
    repl.execute("Definition cong_proof : Derivation := DCong context eq_proof.")
        .expect("Define cong_proof");

    let result = repl.execute("Eval (concludes cong_proof).").expect("Eval");

    // Should produce an equality involving add
    assert!(result.contains("Eq"), "Should contain Eq: {}", result);
    assert!(result.contains("add"), "Should contain add: {}", result);
}

#[test]
fn test_dcong_nested_context() {
    let mut repl = Repl::new();

    // eq_proof: Eq Nat 1 1
    repl.execute(r#"Definition eq_proof : Derivation := DRefl (SName "Nat") (SLit 1)."#)
        .expect("Define eq_proof");

    // context: λx. Succ (Succ x)
    repl.execute(r#"Definition context : Syntax := SLam (SName "Nat") (SApp (SName "Succ") (SApp (SName "Succ") (SVar 0)))."#)
        .expect("Define context");

    repl.execute("Definition cong_proof : Derivation := DCong context eq_proof.")
        .expect("Define cong_proof");

    let result = repl.execute("Eval (concludes cong_proof).").expect("Eval");

    // Should have nested Succ
    assert!(result.contains("Succ"), "Should contain Succ: {}", result);
}

// =============================================================================
// DCONG: WITH NON-TRIVIAL EQUALITY
// =============================================================================

#[test]
fn test_dcong_with_computed_equality() {
    let mut repl = Repl::new();

    // eq_proof: Eq Int (add 1 1) 2 (proved by computation)
    repl.execute(r#"Definition lhs : Syntax := SApp (SApp (SName "add") (SLit 1)) (SLit 1)."#)
        .expect("Define lhs");
    repl.execute(r#"Definition eq_goal : Syntax := SApp (SApp (SApp (SName "Eq") (SName "Int")) lhs) (SLit 2)."#)
        .expect("Define eq_goal");
    repl.execute("Definition eq_proof : Derivation := try_compute eq_goal.")
        .expect("Define eq_proof");

    // context: λx. mul x 10
    repl.execute(r#"Definition context : Syntax := SLam (SName "Int") (SApp (SApp (SName "mul") (SVar 0)) (SLit 10))."#)
        .expect("Define context");

    // Apply congruence: should get Eq Int (mul (add 1 1) 10) (mul 2 10)
    repl.execute("Definition cong_proof : Derivation := DCong context eq_proof.")
        .expect("Define cong_proof");

    let result = repl.execute("Eval (concludes cong_proof).").expect("Eval");

    assert!(result.contains("mul"), "Should contain mul: {}", result);
}

// =============================================================================
// DCONG: FAILURE CASES
// =============================================================================

#[test]
fn test_dcong_fails_on_non_lambda_context() {
    let mut repl = Repl::new();

    // eq_proof: Eq Nat Zero Zero
    repl.execute(r#"Definition eq_proof : Derivation := DRefl (SName "Nat") (SName "Zero")."#)
        .expect("Define eq_proof");

    // context: NOT a lambda (just a name)
    repl.execute(r#"Definition bad_context : Syntax := SName "Succ"."#)
        .expect("Define bad_context");

    repl.execute("Definition cong_proof : Derivation := DCong bad_context eq_proof.")
        .expect("Define cong_proof");

    let result = repl.execute("Eval (concludes cong_proof).").expect("Eval");
    assert_eq!(result, "(SName \"Error\")", "Should return Error for non-lambda context");
}

#[test]
fn test_dcong_fails_on_non_equality_proof() {
    let mut repl = Repl::new();

    // Not an equality proof
    repl.execute(r#"Definition bad_proof : Derivation := DAxiom (SName "P")."#)
        .expect("Define bad_proof");

    // context: λx. Succ x
    repl.execute(r#"Definition context : Syntax := SLam (SName "Nat") (SApp (SName "Succ") (SVar 0))."#)
        .expect("Define context");

    repl.execute("Definition cong_proof : Derivation := DCong context bad_proof.")
        .expect("Define cong_proof");

    let result = repl.execute("Eval (concludes cong_proof).").expect("Eval");
    assert_eq!(result, "(SName \"Error\")", "Should return Error for non-equality proof");
}

// =============================================================================
// TRY_CONG: TYPE CHECK
// =============================================================================

#[test]
fn test_try_cong_type() {
    let mut repl = Repl::new();
    let result = repl.execute("Check try_cong.").expect("Check try_cong");
    assert_eq!(result, "try_cong : Syntax -> Derivation -> Derivation");
}

// =============================================================================
// TRY_CONG: BASIC USAGE
// =============================================================================

#[test]
fn test_try_cong_basic() {
    let mut repl = Repl::new();

    // eq_proof: Eq Nat Zero Zero
    repl.execute(r#"Definition eq_proof : Derivation := DRefl (SName "Nat") (SName "Zero")."#)
        .expect("Define eq_proof");

    // context: λx. Succ x
    repl.execute(r#"Definition context : Syntax := SLam (SName "Nat") (SApp (SName "Succ") (SVar 0))."#)
        .expect("Define context");

    // try_cong context eq_proof
    repl.execute("Definition cong_proof : Derivation := try_cong context eq_proof.")
        .expect("Define cong_proof");

    let result = repl.execute("Eval (concludes cong_proof).").expect("Eval");

    assert!(result.contains("Succ"), "Should contain Succ: {}", result);
    assert!(result.contains("Zero"), "Should contain Zero: {}", result);
}

// =============================================================================
// TYPE ERRORS
// =============================================================================

#[test]
fn test_dcong_type_error_context() {
    let mut repl = Repl::new();
    // DCong expects Syntax for context, not Int
    let result = repl.execute(r#"Check (DCong 42 (DAxiom (SName "x")))."#);
    assert!(result.is_err(), "Should reject Int where Syntax expected");
}

#[test]
fn test_dcong_type_error_proof() {
    let mut repl = Repl::new();
    // DCong expects Derivation for proof, not Syntax
    let result = repl.execute(r#"Check (DCong (SName "ctx") (SName "not_a_proof"))."#);
    assert!(result.is_err(), "Should reject Syntax where Derivation expected");
}

#[test]
fn test_try_cong_type_error() {
    let mut repl = Repl::new();
    // try_cong expects Syntax, Derivation
    let result = repl.execute(r#"Check (try_cong 42 (DAxiom (SName "x")))."#);
    assert!(result.is_err(), "Should reject Int where Syntax expected");
}

// =============================================================================
// INTEGRATION: STEP CASE PATTERN
// =============================================================================

#[test]
fn test_step_case_pattern() {
    let mut repl = Repl::new();

    // This test demonstrates the step case pattern for n + 0 = n
    // IH: Eq Nat k k (simplified - in real proof would be add k Zero = k)
    repl.execute(r#"Definition ih : Derivation := DRefl (SName "Nat") (SName "k")."#)
        .expect("Define ih");

    // Context: λx. Succ x
    repl.execute(r#"Definition succ_ctx : Syntax := SLam (SName "Nat") (SApp (SName "Succ") (SVar 0))."#)
        .expect("Define succ_ctx");

    // Apply congruence: Eq Nat (Succ k) (Succ k)
    repl.execute("Definition step_result : Derivation := DCong succ_ctx ih.")
        .expect("Define step_result");

    let result = repl.execute("Eval (concludes step_result).").expect("Eval");

    // Should have Succ k = Succ k structure
    assert!(result.contains("Succ"), "Should contain Succ: {}", result);
    assert!(result.contains("k"), "Should contain k: {}", result);
}

// =============================================================================
// INTEGRATION: FULL INDUCTION PROOF STRUCTURE
// =============================================================================

#[test]
fn test_induction_with_congruence_structure() {
    let mut repl = Repl::new();

    // Motive: λn. Eq Nat n n (simplified version of add n Zero = n)
    repl.execute(r#"Definition motive : Syntax := SLam (SName "Nat") (SApp (SApp (SApp (SName "Eq") (SName "Nat")) (SVar 0)) (SVar 0))."#)
        .expect("Define motive");

    // Base: Eq Nat Zero Zero
    repl.execute(r#"Definition base : Derivation := DRefl (SName "Nat") (SName "Zero")."#)
        .expect("Define base");

    // Step formula: ∀k. Eq Nat k k → Eq Nat (Succ k) (Succ k)
    repl.execute(r#"
        Definition step_formula : Syntax :=
            SApp (SApp (SName "Forall") (SName "Nat"))
                (SLam (SName "Nat")
                    (SApp (SApp (SName "Implies")
                        (SApp (SApp (SApp (SName "Eq") (SName "Nat")) (SVar 0)) (SVar 0)))
                        (SApp (SApp (SApp (SName "Eq") (SName "Nat"))
                            (SApp (SName "Succ") (SVar 0)))
                            (SApp (SName "Succ") (SVar 0))))).
    "#).expect("Define step_formula");

    // Step proof (using axiom for now - in real usage would use DCong)
    repl.execute("Definition step : Derivation := DAxiom step_formula.")
        .expect("Define step");

    // Full induction
    repl.execute("Definition ind_proof : Derivation := DInduction motive base step.")
        .expect("Define ind_proof");

    let result = repl.execute("Eval (concludes ind_proof).").expect("Eval");

    assert!(result.contains("Forall"), "Should contain Forall: {}", result);
    assert!(result.contains("Nat"), "Should contain Nat: {}", result);
}

// =============================================================================
// COMPOSABILITY: CHAINING CONGRUENCES
// =============================================================================

#[test]
fn test_dcong_chaining() {
    let mut repl = Repl::new();

    // Start with Eq Nat Zero Zero
    repl.execute(r#"Definition eq1 : Derivation := DRefl (SName "Nat") (SName "Zero")."#)
        .expect("Define eq1");

    // First congruence: λx. Succ x
    repl.execute(r#"Definition ctx1 : Syntax := SLam (SName "Nat") (SApp (SName "Succ") (SVar 0))."#)
        .expect("Define ctx1");

    // Get Eq Nat (Succ Zero) (Succ Zero)
    repl.execute("Definition eq2 : Derivation := DCong ctx1 eq1.")
        .expect("Define eq2");

    // Second congruence: λx. Succ x again
    // Get Eq Nat (Succ (Succ Zero)) (Succ (Succ Zero))
    repl.execute("Definition eq3 : Derivation := DCong ctx1 eq2.")
        .expect("Define eq3");

    let result = repl.execute("Eval (concludes eq3).").expect("Eval");

    // Should have doubly-nested Succ
    // Count occurrences of Succ
    let succ_count = result.matches("Succ").count();
    assert!(succ_count >= 2, "Should have multiple Succ: {} (count: {})", result, succ_count);
}
