//! Phase: The Scalpel - Inversion Tactic
//!
//! Inversion runs constructors backwards to derive contradictions.
//! If Even(3) is claimed, inversion proves False.

use logos::interface::Repl;

// =============================================================================
// TYPE CHECKS
// =============================================================================

#[test]
fn test_try_inversion_exists() {
    let mut repl = Repl::new();
    let result = repl.execute("Check try_inversion.");
    assert!(result.is_ok(), "try_inversion should exist: {:?}", result);
}

#[test]
fn test_dinversion_exists() {
    let mut repl = Repl::new();
    let result = repl.execute("Check DInversion.");
    assert!(result.is_ok(), "DInversion should exist: {:?}", result);
}

// =============================================================================
// NAT 3 = 0 → FALSE (Test discriminate on Nat)
// =============================================================================

#[test]
fn test_inversion_nat_discriminate() {
    // Test that Eq Nat 3 0 can be inverted to prove False
    // (refl requires same values, 3 ≠ 0)
    let mut repl = Repl::new();

    // Build: Eq Nat (Succ (Succ (Succ Zero))) Zero
    repl.execute(
        r#"
        Definition three : Syntax :=
            SApp (SName "Succ") (SApp (SName "Succ") (SApp (SName "Succ") (SName "Zero"))).
    "#,
    )
    .unwrap();

    repl.execute(
        r#"
        Definition hyp : Syntax :=
            SApp (SApp (SApp (SName "Eq") (SName "Nat")) three) (SName "Zero").
    "#,
    )
    .unwrap();

    repl.execute("Definition proof : Derivation := try_inversion hyp.")
        .unwrap();

    let result = repl.execute("Eval (concludes proof).").unwrap();
    assert!(
        result.contains("False"),
        "Eq Nat 3 0 should prove False: {}",
        result
    );
}

// =============================================================================
// FALSE INVERSION (Empty Inductive)
// =============================================================================

#[test]
fn test_inversion_false_trivial() {
    let mut repl = Repl::new();

    // False has no constructors → trivially False
    repl.execute("Definition hyp : Syntax := SName \"False\".")
        .unwrap();
    repl.execute("Definition proof : Derivation := try_inversion hyp.")
        .unwrap();

    let result = repl.execute("Eval (concludes proof).").unwrap();
    assert!(
        result.contains("False"),
        "Inverting False should give False: {}",
        result
    );
}

// =============================================================================
// EQ ZERO ZERO - NO CONTRADICTION (Constructor Can Match)
// =============================================================================

#[test]
fn test_inversion_eq_reflexive_not_false() {
    // Eq Nat Zero Zero CAN be constructed by refl
    // So inversion should NOT derive False
    let mut repl = Repl::new();

    repl.execute(
        r#"
        Definition hyp : Syntax :=
            SApp (SApp (SApp (SName "Eq") (SName "Nat")) (SName "Zero")) (SName "Zero").
    "#,
    )
    .unwrap();

    repl.execute("Definition proof : Derivation := try_inversion hyp.")
        .unwrap();

    let result = repl.execute("Eval (concludes proof).").unwrap();
    // Should NOT be False (refl can construct this)
    assert!(
        !result.contains("(SName \"False\")"),
        "Eq Zero Zero should not prove False: {}",
        result
    );
}

// =============================================================================
// EQ TRUE FALSE → FALSE (Discriminate)
// =============================================================================

#[test]
fn test_inversion_eq_discriminate() {
    let mut repl = Repl::new();

    // Eq Bool true false - only constructor is refl, requires same args
    repl.execute(
        r#"
        Definition hyp : Syntax :=
            SApp (SApp (SApp (SName "Eq") (SName "Bool")) (SName "true")) (SName "false").
    "#,
    )
    .unwrap();

    repl.execute("Definition proof : Derivation := try_inversion hyp.")
        .unwrap();

    let result = repl.execute("Eval (concludes proof).").unwrap();
    assert!(
        result.contains("False"),
        "Eq true false should prove False: {}",
        result
    );
}

// =============================================================================
// ERROR HANDLING
// =============================================================================

#[test]
fn test_inversion_non_inductive_errors() {
    let mut repl = Repl::new();

    repl.execute("Definition hyp : Syntax := SVar 0.").unwrap();
    repl.execute("Definition proof : Derivation := try_inversion hyp.")
        .unwrap();

    let result = repl.execute("Eval (concludes proof).").unwrap();
    assert!(
        result.contains("Error"),
        "Non-inductive should error: {}",
        result
    );
}
