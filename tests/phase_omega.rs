//! Phase Omega: True Integer Arithmetic
//!
//! Tests for the omega tactic which handles linear integer arithmetic
//! with proper floor/ceil rounding (unlike lia which uses rationals).

use logos::interface::Repl;

// =============================================================================
// TYPE CHECKS
// =============================================================================

#[test]
fn test_domegasolve_type() {
    let mut repl = Repl::new();
    let result = repl.execute("Check DOmegaSolve.");
    assert!(result.is_ok(), "DOmegaSolve should exist: {:?}", result);
}

#[test]
fn test_try_omega_type() {
    let mut repl = Repl::new();
    let result = repl.execute("Check try_omega.");
    assert!(result.is_ok(), "try_omega should exist: {:?}", result);
}

// =============================================================================
// SIMPLE INEQUALITIES (same as lia)
// =============================================================================

#[test]
fn test_omega_constant_lt() {
    let mut repl = Repl::new();
    // 2 < 5 (trivial)
    repl.execute("Definition two : Syntax := SLit 2.").unwrap();
    repl.execute("Definition five : Syntax := SLit 5.").unwrap();
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"Lt\") two) five.")
        .unwrap();
    repl.execute("Definition d : Derivation := try_omega goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let concluded = repl.execute("Eval result.").unwrap();
    let original = repl.execute("Eval goal.").unwrap();
    assert_eq!(concluded, original, "omega should prove 2 < 5");
}

#[test]
fn test_omega_constant_le() {
    let mut repl = Repl::new();
    // 2 <= 5 (trivial)
    repl.execute("Definition two : Syntax := SLit 2.").unwrap();
    repl.execute("Definition five : Syntax := SLit 5.").unwrap();
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"Le\") two) five.")
        .unwrap();
    repl.execute("Definition d : Derivation := try_omega goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let concluded = repl.execute("Eval result.").unwrap();
    let original = repl.execute("Eval goal.").unwrap();
    assert_eq!(concluded, original, "omega should prove 2 <= 5");
}

#[test]
fn test_omega_variable_lt() {
    let mut repl = Repl::new();
    // x < x + 1
    repl.execute("Definition x : Syntax := SVar 0.").unwrap();
    repl.execute("Definition one : Syntax := SLit 1.").unwrap();
    repl.execute("Definition x_plus_1 : Syntax := SApp (SApp (SName \"add\") x) one.")
        .unwrap();
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"Lt\") x) x_plus_1.")
        .unwrap();
    repl.execute("Definition d : Derivation := try_omega goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let concluded = repl.execute("Eval result.").unwrap();
    let original = repl.execute("Eval goal.").unwrap();
    assert_eq!(concluded, original, "omega should prove x < x+1");
}

#[test]
fn test_omega_variable_le() {
    let mut repl = Repl::new();
    // x <= x (reflexive)
    repl.execute("Definition x : Syntax := SVar 0.").unwrap();
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"Le\") x) x.")
        .unwrap();
    repl.execute("Definition d : Derivation := try_omega goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let concluded = repl.execute("Eval result.").unwrap();
    let original = repl.execute("Eval goal.").unwrap();
    assert_eq!(concluded, original, "omega should prove x <= x");
}

// =============================================================================
// INTEGER-SPECIFIC TESTS (omega can do, lia cannot)
// =============================================================================

#[test]
fn test_omega_strict_to_nonstrict() {
    let mut repl = Repl::new();
    // x > 1 implies x >= 2 (integer-specific!)
    repl.execute("Definition x : Syntax := SVar 0.").unwrap();
    repl.execute("Definition one : Syntax := SLit 1.").unwrap();
    repl.execute("Definition two : Syntax := SLit 2.").unwrap();
    repl.execute("Definition hyp : Syntax := SApp (SApp (SName \"Gt\") x) one.")
        .unwrap();
    repl.execute("Definition concl : Syntax := SApp (SApp (SName \"Ge\") x) two.")
        .unwrap();
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"implies\") hyp) concl.")
        .unwrap();
    repl.execute("Definition d : Derivation := try_omega goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let concluded = repl.execute("Eval result.").unwrap();
    let original = repl.execute("Eval goal.").unwrap();
    assert_eq!(
        concluded, original,
        "omega should prove x>1 -> x>=2 for integers"
    );
}

#[test]
fn test_omega_lt_to_le() {
    let mut repl = Repl::new();
    // x < 5 implies x <= 4 (integer-specific!)
    repl.execute("Definition x : Syntax := SVar 0.").unwrap();
    repl.execute("Definition four : Syntax := SLit 4.").unwrap();
    repl.execute("Definition five : Syntax := SLit 5.").unwrap();
    repl.execute("Definition hyp : Syntax := SApp (SApp (SName \"Lt\") x) five.")
        .unwrap();
    repl.execute("Definition concl : Syntax := SApp (SApp (SName \"Le\") x) four.")
        .unwrap();
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"implies\") hyp) concl.")
        .unwrap();
    repl.execute("Definition d : Derivation := try_omega goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let concluded = repl.execute("Eval result.").unwrap();
    let original = repl.execute("Eval goal.").unwrap();
    assert_eq!(
        concluded, original,
        "omega should prove x<5 -> x<=4 for integers"
    );
}

#[test]
fn test_omega_coefficient_bound() {
    let mut repl = Repl::new();
    // 3x <= 10 implies x <= 3 (floor(10/3) = 3)
    repl.execute("Definition x : Syntax := SVar 0.").unwrap();
    repl.execute("Definition three : Syntax := SLit 3.").unwrap();
    repl.execute("Definition ten : Syntax := SLit 10.").unwrap();
    repl.execute("Definition three_x : Syntax := SApp (SApp (SName \"mul\") three) x.")
        .unwrap();
    repl.execute("Definition hyp : Syntax := SApp (SApp (SName \"Le\") three_x) ten.")
        .unwrap();
    repl.execute("Definition concl : Syntax := SApp (SApp (SName \"Le\") x) three.")
        .unwrap();
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"implies\") hyp) concl.")
        .unwrap();
    repl.execute("Definition d : Derivation := try_omega goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let concluded = repl.execute("Eval result.").unwrap();
    let original = repl.execute("Eval goal.").unwrap();
    assert_eq!(concluded, original, "omega should prove 3x<=10 -> x<=3");
}

// =============================================================================
// FAILURE CASES
// =============================================================================

#[test]
fn test_omega_fails_false_inequality() {
    let mut repl = Repl::new();
    // 5 < 2 (false)
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"Lt\") (SLit 5)) (SLit 2).")
        .unwrap();
    repl.execute("Definition d : Derivation := try_omega goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let result = repl.execute("Eval result.").unwrap();
    assert!(result.contains("Error"), "omega should fail on 5 < 2");
}

#[test]
fn test_omega_fails_equal_strict() {
    let mut repl = Repl::new();
    // x < x (false for any x)
    repl.execute("Definition x : Syntax := SVar 0.").unwrap();
    repl.execute("Definition goal : Syntax := SApp (SApp (SName \"Lt\") x) x.")
        .unwrap();
    repl.execute("Definition d : Derivation := try_omega goal.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes d.")
        .unwrap();

    let result = repl.execute("Eval result.").unwrap();
    assert!(result.contains("Error"), "omega should fail on x < x");
}
