//! Phase 9: The Operator - Manual Control Tactics
//!
//! rewrite, destruct, apply - precision tools for when auto fails.

use logos::interface::Repl;

// =============================================================================
// REWRITE TACTIC - THE SNIPER
// =============================================================================

// -----------------------------------------------------------------------------
// Type Checks
// -----------------------------------------------------------------------------

#[test]
fn test_try_rewrite_exists() {
    let mut repl = Repl::new();
    let result = repl.execute("Check try_rewrite.");
    assert!(result.is_ok(), "try_rewrite should exist: {:?}", result);
}

#[test]
fn test_drewrite_exists() {
    let mut repl = Repl::new();
    let result = repl.execute("Check DRewrite.");
    assert!(result.is_ok(), "DRewrite should exist: {:?}", result);
}

// -----------------------------------------------------------------------------
// Basic Rewrite: Replace x with y in goal using Eq A x y
// -----------------------------------------------------------------------------

#[test]
fn test_rewrite_replaces_lhs_with_rhs() {
    let mut repl = Repl::new();

    // Hypothesis: Eq Nat x y (as axiom)
    // Build: Eq Nat (SVar 0) (SVar 1) as the equality
    repl.execute(
        r#"
        Definition eq_type : Syntax :=
            SApp (SApp (SApp (SName "Eq") (SName "Nat")) (SVar 0)) (SVar 1).
    "#,
    )
    .unwrap();

    // Create axiom proof of this equality
    repl.execute("Definition eq_proof : Derivation := DAxiom eq_type.")
        .unwrap();

    // Goal: P(x) represented as (SApp P (SVar 0))
    // After rewrite: P(y) = (SApp P (SVar 1))
    repl.execute("Definition goal : Syntax := SApp (SName \"P\") (SVar 0).")
        .unwrap();

    // Apply rewrite
    repl.execute("Definition rewritten : Derivation := try_rewrite eq_proof goal.")
        .unwrap();

    // The conclusion should be P(y) = SApp (SName "P") (SVar 1)
    let result = repl.execute("Eval (concludes rewritten).").unwrap();
    assert!(
        result.contains("SVar") && result.contains("1"),
        "Should have replaced SVar 0 with SVar 1: {}",
        result
    );
}

#[test]
fn test_rewrite_in_nested_expression() {
    let mut repl = Repl::new();

    // Hypothesis: Eq Nat Zero (Succ Zero) - obviously false but we're testing mechanics
    repl.execute(
        r#"
        Definition eq_type : Syntax :=
            SApp (SApp (SApp (SName "Eq") (SName "Nat")) (SName "Zero")) (SApp (SName "Succ") (SName "Zero")).
    "#,
    )
    .unwrap();

    repl.execute("Definition eq_proof : Derivation := DAxiom eq_type.")
        .unwrap();

    // Goal: add Zero Zero = add (Succ Zero) Zero after rewrite
    repl.execute(
        r#"
        Definition goal : Syntax :=
            SApp (SApp (SName "add") (SName "Zero")) (SName "Zero").
    "#,
    )
    .unwrap();

    repl.execute("Definition rewritten : Derivation := try_rewrite eq_proof goal.")
        .unwrap();

    // Should replace first Zero with (Succ Zero)
    let result = repl.execute("Eval (concludes rewritten).").unwrap();
    assert!(
        result.contains("Succ"),
        "Should have replaced Zero with Succ Zero: {}",
        result
    );
}

#[test]
fn test_rewrite_no_match_errors() {
    let mut repl = Repl::new();

    // Hypothesis: Eq Nat x y
    repl.execute(
        r#"
        Definition eq_type : Syntax :=
            SApp (SApp (SApp (SName "Eq") (SName "Nat")) (SVar 0)) (SVar 1).
    "#,
    )
    .unwrap();

    repl.execute("Definition eq_proof : Derivation := DAxiom eq_type.")
        .unwrap();

    // Goal doesn't contain x (SVar 0) - only contains SVar 5
    repl.execute("Definition goal : Syntax := SApp (SName \"Q\") (SVar 5).")
        .unwrap();

    repl.execute("Definition rewritten : Derivation := try_rewrite eq_proof goal.")
        .unwrap();

    // Should error since no match
    let result = repl.execute("Eval (concludes rewritten).").unwrap();
    assert!(
        result.contains("Error"),
        "Should error when LHS not found in goal: {}",
        result
    );
}

#[test]
fn test_rewrite_non_equality_errors() {
    let mut repl = Repl::new();

    // Hypothesis is NOT an equality - just some random type
    repl.execute("Definition not_eq : Syntax := SApp (SName \"P\") (SName \"x\").")
        .unwrap();

    repl.execute("Definition bad_proof : Derivation := DAxiom not_eq.")
        .unwrap();

    repl.execute("Definition goal : Syntax := SName \"Q\".")
        .unwrap();

    repl.execute("Definition result : Derivation := try_rewrite bad_proof goal.")
        .unwrap();

    // Should error since proof doesn't prove an equality
    let result = repl.execute("Eval (concludes result).").unwrap();
    assert!(
        result.contains("Error"),
        "Should error when proof is not an equality: {}",
        result
    );
}

// -----------------------------------------------------------------------------
// Reverse Rewrite: Replace y with x using Eq A x y
// -----------------------------------------------------------------------------

#[test]
fn test_try_rewrite_rev_exists() {
    let mut repl = Repl::new();
    let result = repl.execute("Check try_rewrite_rev.");
    assert!(
        result.is_ok(),
        "try_rewrite_rev should exist: {:?}",
        result
    );
}

#[test]
fn test_rewrite_rev_replaces_rhs_with_lhs() {
    let mut repl = Repl::new();

    // Hypothesis: Eq Nat x y
    repl.execute(
        r#"
        Definition eq_type : Syntax :=
            SApp (SApp (SApp (SName "Eq") (SName "Nat")) (SVar 0)) (SVar 1).
    "#,
    )
    .unwrap();

    repl.execute("Definition eq_proof : Derivation := DAxiom eq_type.")
        .unwrap();

    // Goal contains y (SVar 1), should become x (SVar 0)
    repl.execute("Definition goal : Syntax := SApp (SName \"P\") (SVar 1).")
        .unwrap();

    repl.execute("Definition rewritten : Derivation := try_rewrite_rev eq_proof goal.")
        .unwrap();

    let result = repl.execute("Eval (concludes rewritten).").unwrap();
    assert!(
        result.contains("SVar") && result.contains("0"),
        "Should have replaced SVar 1 with SVar 0: {}",
        result
    );
}

// =============================================================================
// DESTRUCT TACTIC - THE FORK
// =============================================================================

#[test]
fn test_try_destruct_exists() {
    let mut repl = Repl::new();
    let result = repl.execute("Check try_destruct.");
    assert!(result.is_ok(), "try_destruct should exist: {:?}", result);
}

#[test]
fn test_ddestruct_exists() {
    let mut repl = Repl::new();
    let result = repl.execute("Check DDestruct.");
    assert!(result.is_ok(), "DDestruct should exist: {:?}", result);
}

#[test]
fn test_destruct_bool_generates_two_cases() {
    let mut repl = Repl::new();

    // Destruct Bool: should generate cases for true and false
    // Motive: P : Bool -> Prop
    repl.execute("Definition motive : Syntax := SLam (SName \"Bool\") (SApp (SName \"P\") (SVar 0)).")
        .unwrap();

    // Case proofs: P(true) and P(false)
    repl.execute("Definition case_true : Derivation := DAxiom (SApp (SName \"P\") (SName \"true\")).")
        .unwrap();
    repl.execute(
        "Definition case_false : Derivation := DAxiom (SApp (SName \"P\") (SName \"false\")).",
    )
    .unwrap();

    repl.execute("Definition cases : Derivation := DCase case_true (DCase case_false DCaseEnd).")
        .unwrap();

    repl.execute(
        "Definition proof : Derivation := try_destruct (SName \"Bool\") motive cases.",
    )
    .unwrap();

    let result = repl.execute("Eval (concludes proof).").unwrap();
    // Should conclude: forall b:Bool. P(b)
    assert!(
        result.contains("Forall") || result.contains("Bool"),
        "Should prove forall b:Bool. P(b): {}",
        result
    );
}

#[test]
fn test_destruct_nat_no_induction_hypothesis() {
    let mut repl = Repl::new();

    // Destruct Nat - the key difference from induction:
    // Zero case: P(Zero)
    // Succ case: forall k:Nat. P(Succ k)  <- NO P(k) -> P(Succ k)

    repl.execute("Definition motive : Syntax := SLam (SName \"Nat\") (SApp (SName \"P\") (SVar 0)).")
        .unwrap();

    // Zero case: just P(Zero)
    repl.execute("Definition case_zero : Derivation := DAxiom (SApp (SName \"P\") (SName \"Zero\")).")
        .unwrap();

    // Succ case: forall k. P(Succ k) - NOT forall k. P(k) -> P(Succ k)
    repl.execute(
        r#"
        Definition succ_goal : Syntax :=
            SApp (SName "Forall")
                (SLam (SName "Nat")
                    (SApp (SName "P") (SApp (SName "Succ") (SVar 0)))).
    "#,
    )
    .unwrap();

    repl.execute("Definition case_succ : Derivation := DAxiom succ_goal.")
        .unwrap();

    repl.execute("Definition cases : Derivation := DCase case_zero (DCase case_succ DCaseEnd).")
        .unwrap();

    repl.execute("Definition proof : Derivation := try_destruct (SName \"Nat\") motive cases.")
        .unwrap();

    let result = repl.execute("Eval (concludes proof).").unwrap();
    // Should conclude: forall n:Nat. P(n)
    assert!(
        !result.contains("Error"),
        "Destruct Nat should work without IH: {}",
        result
    );
}

// =============================================================================
// APPLY TACTIC - THE ARROW
// =============================================================================

#[test]
fn test_try_apply_exists() {
    let mut repl = Repl::new();
    let result = repl.execute("Check try_apply.");
    assert!(result.is_ok(), "try_apply should exist: {:?}", result);
}

#[test]
fn test_dapply_exists() {
    let mut repl = Repl::new();
    let result = repl.execute("Check DApply.");
    assert!(result.is_ok(), "DApply should exist: {:?}", result);
}

#[test]
fn test_apply_implication_transforms_goal() {
    let mut repl = Repl::new();

    // Hypothesis H: P -> Q (as implication in Syntax)
    repl.execute(
        r#"
        Definition h_type : Syntax :=
            SPi (SName "P") (SName "Q").
    "#,
    )
    .unwrap();

    // Register as axiom
    repl.execute("Definition h_proof : Derivation := DAxiom h_type.")
        .unwrap();

    // Goal: Q
    repl.execute("Definition goal : Syntax := SName \"Q\".")
        .unwrap();

    // Apply H to goal Q - should return new goal P
    repl.execute("Definition applied : Derivation := try_apply (SName \"H\") h_proof goal.")
        .unwrap();

    let result = repl.execute("Eval (concludes applied).").unwrap();
    // Conclusion should be the new subgoal P
    assert!(
        result.contains("P") && !result.contains("Error"),
        "Applying H:P->Q to goal Q should give subgoal P: {}",
        result
    );
}

#[test]
fn test_apply_forall_instantiation() {
    let mut repl = Repl::new();

    // Hypothesis: forall x:Nat. P(x)
    repl.execute(
        r#"
        Definition h_type : Syntax :=
            SApp (SName "Forall")
                (SLam (SName "Nat") (SApp (SName "P") (SVar 0))).
    "#,
    )
    .unwrap();

    repl.execute("Definition h_proof : Derivation := DAxiom h_type.")
        .unwrap();

    // Goal: P(3) = P(Succ(Succ(Succ Zero)))
    repl.execute(
        r#"
        Definition three : Syntax :=
            SApp (SName "Succ") (SApp (SName "Succ") (SApp (SName "Succ") (SName "Zero"))).
    "#,
    )
    .unwrap();

    repl.execute("Definition goal : Syntax := SApp (SName \"P\") three.")
        .unwrap();

    // Apply should instantiate forall with 3
    repl.execute("Definition applied : Derivation := try_apply (SName \"lemma\") h_proof goal.")
        .unwrap();

    let result = repl.execute("Eval (concludes applied).").unwrap();
    // Should succeed - forall instantiated at 3
    assert!(
        !result.contains("Error"),
        "Applying forall x. P(x) to goal P(3) should work: {}",
        result
    );
}

#[test]
fn test_apply_mismatch_errors() {
    let mut repl = Repl::new();

    // Hypothesis: P -> Q
    repl.execute("Definition h_type : Syntax := SPi (SName \"P\") (SName \"Q\").")
        .unwrap();

    repl.execute("Definition h_proof : Derivation := DAxiom h_type.")
        .unwrap();

    // Goal: R (doesn't match Q)
    repl.execute("Definition goal : Syntax := SName \"R\".")
        .unwrap();

    repl.execute("Definition applied : Derivation := try_apply (SName \"H\") h_proof goal.")
        .unwrap();

    let result = repl.execute("Eval (concludes applied).").unwrap();
    // Should error since R != Q
    assert!(
        result.contains("Error"),
        "Applying P->Q to goal R should error: {}",
        result
    );
}
