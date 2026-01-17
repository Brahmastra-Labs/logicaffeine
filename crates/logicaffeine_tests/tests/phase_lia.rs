//! Phase LIA: Linear Integer Arithmetic Tactic
//!
//! Tests for the lia tactic which proves linear inequalities by
//! Fourier-Motzkin elimination.
//!
//! The `lia` tactic proves goals of the form:
//! - Lt a b (a < b)
//! - Le a b (a ≤ b)
//! - Gt a b (a > b)
//! - Ge a b (a ≥ b)
//!
//! Where a and b are linear expressions (constants, variables, and c*x).

use logicaffeine_kernel::interface::Repl;

// =============================================================================
// TYPE CHECKS
// =============================================================================

#[test]
fn test_dliasolve_type() {
    let mut repl = Repl::new();
    let result = repl.execute("Check DLiaSolve.");
    assert!(result.is_ok(), "DLiaSolve should exist: {:?}", result);
}

#[test]
fn test_try_lia_type() {
    let mut repl = Repl::new();
    let result = repl.execute("Check try_lia.");
    assert!(result.is_ok(), "try_lia should exist: {:?}", result);
}

// =============================================================================
// REFLEXIVITY (x ≤ x)
// =============================================================================

#[test]
fn test_try_lia_le_reflexive() {
    let mut repl = Repl::new();

    // x ≤ x is always true
    repl.execute("Definition x : Syntax := SVar 0.").unwrap();
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"Le\") x) x.")
        .unwrap();
    repl.execute("Definition d : Derivation := try_lia goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let concluded = repl.execute("Eval result.").unwrap();
    let original = repl.execute("Eval goal.").unwrap();
    assert_eq!(concluded, original, "LIA should prove x ≤ x");
}

// =============================================================================
// CONSTANT INEQUALITIES
// =============================================================================

#[test]
fn test_try_lia_constant_le_true() {
    let mut repl = Repl::new();

    // 2 ≤ 5 is true
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"Le\") (SLit 2)) (SLit 5).")
        .unwrap();
    repl.execute("Definition d : Derivation := try_lia goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let concluded = repl.execute("Eval result.").unwrap();
    let original = repl.execute("Eval goal.").unwrap();
    assert_eq!(concluded, original, "LIA should prove 2 ≤ 5");
}

#[test]
fn test_try_lia_constant_lt_true() {
    let mut repl = Repl::new();

    // 2 < 5 is true
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"Lt\") (SLit 2)) (SLit 5).")
        .unwrap();
    repl.execute("Definition d : Derivation := try_lia goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let concluded = repl.execute("Eval result.").unwrap();
    let original = repl.execute("Eval goal.").unwrap();
    assert_eq!(concluded, original, "LIA should prove 2 < 5");
}

#[test]
fn test_try_lia_constant_le_equal() {
    let mut repl = Repl::new();

    // 5 ≤ 5 is true
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"Le\") (SLit 5)) (SLit 5).")
        .unwrap();
    repl.execute("Definition d : Derivation := try_lia goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let concluded = repl.execute("Eval result.").unwrap();
    let original = repl.execute("Eval goal.").unwrap();
    assert_eq!(concluded, original, "LIA should prove 5 ≤ 5");
}

// =============================================================================
// LINEAR EXPRESSIONS
// =============================================================================

#[test]
fn test_try_lia_x_lt_x_plus_1() {
    let mut repl = Repl::new();

    // x < x + 1 is always true
    repl.execute("Definition x : Syntax := SVar 0.").unwrap();
    repl.execute("Definition xp1 : Syntax := SApp (SApp (SName \"add\") x) (SLit 1).")
        .unwrap();
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"Lt\") x) xp1.")
        .unwrap();
    repl.execute("Definition d : Derivation := try_lia goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let concluded = repl.execute("Eval result.").unwrap();
    let original = repl.execute("Eval goal.").unwrap();
    assert_eq!(concluded, original, "LIA should prove x < x+1");
}

#[test]
fn test_try_lia_x_le_x_plus_1() {
    let mut repl = Repl::new();

    // x ≤ x + 1 is always true
    repl.execute("Definition x : Syntax := SVar 0.").unwrap();
    repl.execute("Definition xp1 : Syntax := SApp (SApp (SName \"add\") x) (SLit 1).")
        .unwrap();
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"Le\") x) xp1.")
        .unwrap();
    repl.execute("Definition d : Derivation := try_lia goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let concluded = repl.execute("Eval result.").unwrap();
    let original = repl.execute("Eval goal.").unwrap();
    assert_eq!(concluded, original, "LIA should prove x ≤ x+1");
}

#[test]
fn test_try_lia_2x_le_2x() {
    let mut repl = Repl::new();

    // 2*x ≤ 2*x (linear, coefficient multiplication allowed)
    repl.execute("Definition x : Syntax := SVar 0.").unwrap();
    repl.execute("Definition two_x : Syntax := SApp (SApp (SName \"mul\") (SLit 2)) x.")
        .unwrap();
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"Le\") two_x) two_x.")
        .unwrap();
    repl.execute("Definition d : Derivation := try_lia goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let concluded = repl.execute("Eval result.").unwrap();
    let original = repl.execute("Eval goal.").unwrap();
    assert_eq!(concluded, original, "LIA should prove 2*x ≤ 2*x");
}

#[test]
fn test_try_lia_x_minus_1_lt_x() {
    let mut repl = Repl::new();

    // x - 1 < x is always true
    repl.execute("Definition x : Syntax := SVar 0.").unwrap();
    repl.execute("Definition xm1 : Syntax := SApp (SApp (SName \"sub\") x) (SLit 1).")
        .unwrap();
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"Lt\") xm1) x.")
        .unwrap();
    repl.execute("Definition d : Derivation := try_lia goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let concluded = repl.execute("Eval result.").unwrap();
    let original = repl.execute("Eval goal.").unwrap();
    assert_eq!(concluded, original, "LIA should prove x-1 < x");
}

// =============================================================================
// FAILURE CASES
// =============================================================================

#[test]
fn test_try_lia_fails_false_constant() {
    let mut repl = Repl::new();

    // 5 < 2 is false
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"Lt\") (SLit 5)) (SLit 2).")
        .unwrap();
    repl.execute("Definition d : Derivation := try_lia goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let result = repl.execute("Eval result.").unwrap();
    assert!(
        result.contains("Error"),
        "LIA should fail on 5 < 2, got: {}",
        result
    );
}

#[test]
fn test_try_lia_fails_nonlinear() {
    let mut repl = Repl::new();

    // x*y ≤ x*y involves nonlinear term
    repl.execute("Definition x : Syntax := SVar 0.").unwrap();
    repl.execute("Definition y : Syntax := SVar 1.").unwrap();
    repl.execute("Definition xy : Syntax := SApp (SApp (SName \"mul\") x) y.")
        .unwrap();
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"Le\") xy) xy.")
        .unwrap();
    repl.execute("Definition d : Derivation := try_lia goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let result = repl.execute("Eval result.").unwrap();
    assert!(
        result.contains("Error"),
        "LIA should fail on nonlinear x*y, got: {}",
        result
    );
}

#[test]
fn test_try_lia_fails_x_lt_x() {
    let mut repl = Repl::new();

    // x < x is false (strict inequality)
    repl.execute("Definition x : Syntax := SVar 0.").unwrap();
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"Lt\") x) x.")
        .unwrap();
    repl.execute("Definition d : Derivation := try_lia goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let result = repl.execute("Eval result.").unwrap();
    assert!(
        result.contains("Error"),
        "LIA should fail on x < x, got: {}",
        result
    );
}
