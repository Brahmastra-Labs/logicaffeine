//! Phase 97: The Inductor (Deep Induction)
//!
//! Implements structural induction in the deep embedding:
//! - DInduction: Induction proof constructor
//! - concludes verifies base and step proofs

use logicaffeine_kernel::interface::Repl;

// =============================================================================
// DINDUCTION: TYPE CHECK
// =============================================================================

#[test]
fn test_dinduction_type() {
    let mut repl = Repl::new();
    let result = repl.execute("Check DInduction.").expect("Check DInduction");
    assert_eq!(result, "DInduction : Syntax -> Derivation -> Derivation -> Derivation");
}

#[test]
fn test_dinduction_construction() {
    let mut repl = Repl::new();

    // Motive: λn:Nat. Eq Nat n n (trivial property)
    repl.execute(r#"Definition motive : Syntax := SLam (SName "Nat") (SApp (SApp (SApp (SName "Eq") (SName "Nat")) (SVar 0)) (SVar 0))."#)
        .expect("Define motive");

    // Base case: Eq Nat Zero Zero (prove by reflexivity)
    repl.execute(r#"Definition base : Derivation := DRefl (SName "Nat") (SName "Zero")."#)
        .expect("Define base");

    // Step case placeholder (axiom for now)
    repl.execute(r#"Definition step : Derivation := DAxiom (SName "step_placeholder")."#)
        .expect("Define step");

    // Can construct DInduction
    repl.execute("Definition d : Derivation := DInduction motive base step.")
        .expect("Define d");

    let result = repl.execute("Check d.").expect("Check d");
    assert_eq!(result, "d : Derivation");
}

// =============================================================================
// CONCLUDES DINDUCTION: SUCCESS CASE
// =============================================================================

#[test]
fn test_concludes_dinduction_simple() {
    let mut repl = Repl::new();

    // Motive: λn. Eq Nat n n
    repl.execute(r#"Definition motive : Syntax := SLam (SName "Nat") (SApp (SApp (SApp (SName "Eq") (SName "Nat")) (SVar 0)) (SVar 0))."#)
        .expect("Define motive");

    // Expected base: Eq Nat Zero Zero
    repl.execute(r#"Definition expected_base : Syntax := SApp (SApp (SApp (SName "Eq") (SName "Nat")) (SName "Zero")) (SName "Zero")."#)
        .expect("Define expected_base");

    // Base proof: DRefl proves Eq Nat Zero Zero
    repl.execute(r#"Definition base : Derivation := DRefl (SName "Nat") (SName "Zero")."#)
        .expect("Define base");

    // Verify base concludes correctly
    let base_conc = repl.execute("Eval (concludes base).").expect("Eval base");
    let exp_base = repl.execute("Eval expected_base.").expect("Eval expected");
    assert_eq!(base_conc, exp_base, "Base should conclude Eq Nat Zero Zero");
}

#[test]
fn test_concludes_dinduction_returns_forall() {
    let mut repl = Repl::new();

    // Motive: λn. Eq Nat n n
    repl.execute(r#"Definition motive : Syntax := SLam (SName "Nat") (SApp (SApp (SApp (SName "Eq") (SName "Nat")) (SVar 0)) (SVar 0))."#)
        .expect("Define motive");

    // Base proof: Eq Nat Zero Zero
    repl.execute(r#"Definition base : Derivation := DRefl (SName "Nat") (SName "Zero")."#)
        .expect("Define base");

    // Step proof: ∀k. Eq Nat k k → Eq Nat (Succ k) (Succ k)
    // For this simple reflexivity case, we need a proper step proof
    // Step formula: Forall Nat (λk. Implies (Eq Nat k k) (Eq Nat (Succ k) (Succ k)))
    repl.execute(r#"
        Definition step_body : Syntax :=
            SApp (SApp (SName "Implies")
                (SApp (SApp (SApp (SName "Eq") (SName "Nat")) (SVar 0)) (SVar 0)))
                (SApp (SApp (SApp (SName "Eq") (SName "Nat"))
                    (SApp (SName "Succ") (SVar 0)))
                    (SApp (SName "Succ") (SVar 0))).
    "#).expect("Define step_body");

    repl.execute(r#"
        Definition step_formula : Syntax :=
            SApp (SApp (SName "Forall") (SName "Nat")) (SLam (SName "Nat") step_body).
    "#).expect("Define step_formula");

    // Use axiom for step (in real use, would construct proper proof)
    repl.execute("Definition step : Derivation := DAxiom step_formula.")
        .expect("Define step");

    // Construct induction proof
    repl.execute("Definition ind_proof : Derivation := DInduction motive base step.")
        .expect("Define ind_proof");

    // concludes should return: Forall Nat motive
    repl.execute("Definition result : Syntax := concludes ind_proof.")
        .expect("Define result");

    let result = repl.execute("Eval result.").expect("Eval result");

    // Should contain Forall and Nat
    assert!(result.contains("Forall"), "Should contain Forall: {}", result);
    assert!(result.contains("Nat"), "Should contain Nat: {}", result);
}

// =============================================================================
// CONCLUDES DINDUCTION: VERIFICATION FAILURES
// =============================================================================

#[test]
fn test_dinduction_wrong_base_returns_error() {
    let mut repl = Repl::new();

    // Motive: λn. Eq Nat n n
    repl.execute(r#"Definition motive : Syntax := SLam (SName "Nat") (SApp (SApp (SApp (SName "Eq") (SName "Nat")) (SVar 0)) (SVar 0))."#)
        .expect("Define motive");

    // WRONG base: proves Eq Nat (Succ Zero) (Succ Zero) instead of Eq Nat Zero Zero
    repl.execute(r#"Definition wrong_base : Derivation := DRefl (SName "Nat") (SApp (SName "Succ") (SName "Zero"))."#)
        .expect("Define wrong_base");

    // Placeholder step
    repl.execute(r#"Definition step : Derivation := DAxiom (SName "placeholder")."#)
        .expect("Define step");

    repl.execute("Definition ind_proof : Derivation := DInduction motive wrong_base step.")
        .expect("Define ind_proof");

    let result = repl.execute("Eval (concludes ind_proof).").expect("Eval");
    assert_eq!(result, "(SName \"Error\")", "Should return error for wrong base case");
}

#[test]
fn test_dinduction_wrong_step_returns_error() {
    let mut repl = Repl::new();

    // Motive: λn. Eq Nat n n
    repl.execute(r#"Definition motive : Syntax := SLam (SName "Nat") (SApp (SApp (SApp (SName "Eq") (SName "Nat")) (SVar 0)) (SVar 0))."#)
        .expect("Define motive");

    // Correct base
    repl.execute(r#"Definition base : Derivation := DRefl (SName "Nat") (SName "Zero")."#)
        .expect("Define base");

    // WRONG step: not the correct step formula
    repl.execute(r#"Definition wrong_step : Derivation := DAxiom (SName "wrong")."#)
        .expect("Define wrong_step");

    repl.execute("Definition ind_proof : Derivation := DInduction motive base wrong_step.")
        .expect("Define ind_proof");

    let result = repl.execute("Eval (concludes ind_proof).").expect("Eval");
    assert_eq!(result, "(SName \"Error\")", "Should return error for wrong step case");
}

// =============================================================================
// TYPE ERRORS
// =============================================================================

#[test]
fn test_dinduction_type_error_motive() {
    let mut repl = Repl::new();
    // DInduction expects Syntax for motive, not Int
    let result = repl.execute("Check (DInduction 42 (DAxiom (SName \"x\")) (DAxiom (SName \"y\"))).");
    assert!(result.is_err(), "Should reject Int where Syntax expected");
}

#[test]
fn test_dinduction_type_error_base() {
    let mut repl = Repl::new();
    // DInduction expects Derivation for base, not Syntax
    let result = repl.execute("Check (DInduction (SName \"motive\") (SName \"not_derivation\") (DAxiom (SName \"y\"))).");
    assert!(result.is_err(), "Should reject Syntax where Derivation expected");
}

#[test]
fn test_dinduction_type_error_step() {
    let mut repl = Repl::new();
    // DInduction expects Derivation for step, not Int
    let result = repl.execute("Check (DInduction (SName \"motive\") (DAxiom (SName \"x\")) 42).");
    assert!(result.is_err(), "Should reject Int where Derivation expected");
}

// =============================================================================
// HELPER: BUILD STEP FORMULA
// =============================================================================

#[test]
fn test_build_step_formula_structure() {
    let mut repl = Repl::new();

    // For motive λn. P(n), step formula is: ∀k. P(k) → P(Succ k)
    // Let's manually build and check the structure

    // P = Eq Nat (SVar 0) (SVar 0)  (just as example)
    repl.execute(r#"Definition P : Syntax := SApp (SApp (SApp (SName "Eq") (SName "Nat")) (SVar 0)) (SVar 0)."#)
        .expect("Define P");

    // P(Succ k) = Eq Nat (Succ (SVar 0)) (Succ (SVar 0))
    repl.execute(r#"Definition P_succ : Syntax := SApp (SApp (SApp (SName "Eq") (SName "Nat")) (SApp (SName "Succ") (SVar 0))) (SApp (SName "Succ") (SVar 0))."#)
        .expect("Define P_succ");

    // Implies P P_succ
    repl.execute(r#"Definition impl : Syntax := SApp (SApp (SName "Implies") P) P_succ."#)
        .expect("Define impl");

    // Forall Nat (λk. impl)
    repl.execute(r#"Definition step_formula : Syntax := SApp (SApp (SName "Forall") (SName "Nat")) (SLam (SName "Nat") impl)."#)
        .expect("Define step_formula");

    let result = repl.execute("Eval step_formula.").expect("Eval");
    assert!(result.contains("Forall"), "Should have Forall");
    assert!(result.contains("Implies"), "Should have Implies");
    assert!(result.contains("Succ"), "Should have Succ");
}

// =============================================================================
// INTEGRATION: MANUAL INDUCTION PROOF
// =============================================================================

#[test]
fn test_manual_induction_proof_n_equals_n() {
    let mut repl = Repl::new();

    // Goal: ∀n:Nat. n = n
    // Motive: λn. Eq Nat n n

    repl.execute(r#"Definition motive : Syntax := SLam (SName "Nat") (SApp (SApp (SApp (SName "Eq") (SName "Nat")) (SVar 0)) (SVar 0))."#)
        .expect("Define motive");

    // Base case: Eq Nat Zero Zero
    // Proved by DRefl Nat Zero
    repl.execute(r#"Definition base_proof : Derivation := DRefl (SName "Nat") (SName "Zero")."#)
        .expect("Define base_proof");

    // Step case: ∀k:Nat. Eq Nat k k → Eq Nat (Succ k) (Succ k)
    // Build the step formula explicitly
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

    // Step proof (use DAxiom with correct formula for now)
    repl.execute("Definition step_proof : Derivation := DAxiom step_formula.")
        .expect("Define step_proof");

    // Combine into induction proof
    repl.execute("Definition ind_proof : Derivation := DInduction motive base_proof step_proof.")
        .expect("Define ind_proof");

    // Verify conclusion is ∀n:Nat. Eq Nat n n
    repl.execute("Definition conclusion : Syntax := concludes ind_proof.")
        .expect("Define conclusion");

    let result = repl.execute("Eval conclusion.").expect("Eval");

    // The conclusion should be: Forall Nat motive
    assert!(result.contains("Forall"), "Conclusion should be a Forall: {}", result);
    assert!(result.contains("Nat"), "Conclusion should mention Nat: {}", result);
    assert!(result.contains("Eq"), "Conclusion should contain Eq: {}", result);
}
