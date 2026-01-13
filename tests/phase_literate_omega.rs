//! Phase Literate Omega: Omega Tactic in Literate Mode
//!
//! Tests for the omega tactic in literate theorem syntax.
//! omega handles true integer arithmetic with floor/ceil rounding.

use logos::interface::Repl;

// =============================================================================
// SIMPLE INEQUALITIES
// =============================================================================

#[test]
fn test_literate_omega_constant_lt() {
    let mut repl = Repl::new();
    let result = repl.execute("## Theorem: TwoLtFive\n    Statement: (Lt 2 5).\n    Proof: omega.");
    assert!(result.is_ok(), "omega should prove 2 < 5: {:?}", result);
}

#[test]
fn test_literate_omega_constant_le() {
    let mut repl = Repl::new();
    let result = repl.execute("## Theorem: TwoLeFive\n    Statement: (Le 2 5).\n    Proof: omega.");
    assert!(result.is_ok(), "omega should prove 2 <= 5: {:?}", result);
}

#[test]
fn test_literate_omega_variable_lt() {
    let mut repl = Repl::new();
    let result =
        repl.execute("## Theorem: XLtXPlusOne\n    Statement: (Lt x (add x 1)).\n    Proof: omega.");
    assert!(
        result.is_ok(),
        "omega should prove x < x+1: {:?}",
        result
    );
}

#[test]
fn test_literate_omega_variable_le() {
    let mut repl = Repl::new();
    let result = repl.execute("## Theorem: XLeX\n    Statement: (Le x x).\n    Proof: omega.");
    assert!(result.is_ok(), "omega should prove x <= x: {:?}", result);
}

#[test]
fn test_literate_omega_gt() {
    let mut repl = Repl::new();
    let result =
        repl.execute("## Theorem: XPlusOneGtX\n    Statement: (Gt (add x 1) x).\n    Proof: omega.");
    assert!(
        result.is_ok(),
        "omega should prove x+1 > x: {:?}",
        result
    );
}

#[test]
fn test_literate_omega_ge() {
    let mut repl = Repl::new();
    let result = repl.execute("## Theorem: XGeX\n    Statement: (Ge x x).\n    Proof: omega.");
    assert!(result.is_ok(), "omega should prove x >= x: {:?}", result);
}

// =============================================================================
// INTEGER-SPECIFIC (omega's power!)
// =============================================================================

#[test]
fn test_literate_omega_strict_to_nonstrict_gt() {
    let mut repl = Repl::new();
    // x > 0 implies x >= 1 (integers only!)
    let result = repl.execute(
        "## Theorem: GtToGe\n    Statement: (implies (Gt x 0) (Ge x 1)).\n    Proof: omega.",
    );
    assert!(
        result.is_ok(),
        "omega should prove x>0 -> x>=1: {:?}",
        result
    );
}

#[test]
fn test_literate_omega_strict_to_nonstrict_lt() {
    let mut repl = Repl::new();
    // x < 5 implies x <= 4 (integers only!)
    let result = repl.execute(
        "## Theorem: LtToLe\n    Statement: (implies (Lt x 5) (Le x 4)).\n    Proof: omega.",
    );
    assert!(
        result.is_ok(),
        "omega should prove x<5 -> x<=4: {:?}",
        result
    );
}

#[test]
fn test_literate_omega_coefficient_bound() {
    let mut repl = Repl::new();
    // 3x <= 10 implies x <= 3 (floor(10/3) = 3)
    let result = repl.execute(
        "## Theorem: CoefficientBound\n    Statement: (implies (Le (mul 3 x) 10) (Le x 3)).\n    Proof: omega.",
    );
    assert!(
        result.is_ok(),
        "omega should prove 3x<=10 -> x<=3: {:?}",
        result
    );
}

#[test]
fn test_literate_omega_two_coefficient_bound() {
    let mut repl = Repl::new();
    // 2x <= 5 implies x <= 2 (floor(5/2) = 2)
    let result = repl.execute(
        "## Theorem: TwoCoefficientBound\n    Statement: (implies (Le (mul 2 x) 5) (Le x 2)).\n    Proof: omega.",
    );
    assert!(
        result.is_ok(),
        "omega should prove 2x<=5 -> x<=2: {:?}",
        result
    );
}

// =============================================================================
// TRANSITIVITY AND CHAINS
// =============================================================================

#[test]
fn test_literate_omega_transitivity_lt() {
    let mut repl = Repl::new();
    // x < y -> y < z -> x < z
    let result = repl.execute(
        "## Theorem: LtTrans\n    Statement: (implies (Lt x y) (implies (Lt y z) (Lt x z))).\n    Proof: omega.",
    );
    assert!(
        result.is_ok(),
        "omega should prove transitivity: {:?}",
        result
    );
}

#[test]
fn test_literate_omega_transitivity_le() {
    let mut repl = Repl::new();
    // x <= y -> y <= z -> x <= z
    let result = repl.execute(
        "## Theorem: LeTrans\n    Statement: (implies (Le x y) (implies (Le y z) (Le x z))).\n    Proof: omega.",
    );
    assert!(
        result.is_ok(),
        "omega should prove le transitivity: {:?}",
        result
    );
}

// =============================================================================
// FAILURE CASES
// =============================================================================

#[test]
fn test_literate_omega_fails_false() {
    let mut repl = Repl::new();
    repl.execute("## Theorem: Wrong\n    Statement: (Lt 5 2).\n    Proof: omega.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes Wrong.")
        .unwrap();
    let result = repl.execute("Eval result.").unwrap();
    assert!(
        result.contains("Error"),
        "omega should fail on 5 < 2: {:?}",
        result
    );
}

#[test]
fn test_literate_omega_fails_strict_reflexive() {
    let mut repl = Repl::new();
    repl.execute("## Theorem: XLtX\n    Statement: (Lt x x).\n    Proof: omega.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes XLtX.")
        .unwrap();
    let result = repl.execute("Eval result.").unwrap();
    assert!(
        result.contains("Error"),
        "omega should fail on x < x: {:?}",
        result
    );
}
