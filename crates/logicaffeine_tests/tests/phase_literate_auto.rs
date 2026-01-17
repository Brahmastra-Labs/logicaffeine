//! Phase Literate Auto: Auto Tactic in Literate Mode
//!
//! Tests for the auto tactic in literate theorem syntax.

use logicaffeine_kernel::interface::Repl;

// =============================================================================
// SIMP VIA AUTO
// =============================================================================

#[test]
fn test_literate_auto_true() {
    let mut repl = Repl::new();
    let result = repl.execute("## Theorem: TrueIsTrue\n    Statement: True.\n    Proof: auto.");
    assert!(result.is_ok(), "auto should prove True: {:?}", result);
}

// =============================================================================
// RING VIA AUTO
// =============================================================================

#[test]
fn test_literate_auto_ring() {
    let mut repl = Repl::new();
    let result = repl
        .execute("## Theorem: AddComm\n    Statement: (Eq (add a b) (add b a)).\n    Proof: auto.");
    assert!(
        result.is_ok(),
        "auto should prove a+b=b+a via ring: {:?}",
        result
    );
}

#[test]
fn test_literate_auto_ring_assoc() {
    let mut repl = Repl::new();
    let result = repl.execute(
        "## Theorem: AddAssoc\n    Statement: (Eq (add (add a b) c) (add a (add b c))).\n    Proof: auto.",
    );
    assert!(
        result.is_ok(),
        "auto should prove (a+b)+c=a+(b+c): {:?}",
        result
    );
}

// =============================================================================
// CC VIA AUTO
// =============================================================================

#[test]
fn test_literate_auto_cc_refl() {
    let mut repl = Repl::new();
    let result =
        repl.execute("## Theorem: FxEqFx\n    Statement: (Eq (f x) (f x)).\n    Proof: auto.");
    assert!(
        result.is_ok(),
        "auto should prove f(x)=f(x) via cc: {:?}",
        result
    );
}

// =============================================================================
// OMEGA VIA AUTO
// =============================================================================

#[test]
fn test_literate_auto_omega_lt() {
    let mut repl = Repl::new();
    let result =
        repl.execute("## Theorem: TwoLtFive\n    Statement: (Lt 2 5).\n    Proof: auto.");
    assert!(result.is_ok(), "auto should prove 2 < 5: {:?}", result);
}

#[test]
fn test_literate_auto_omega_integer() {
    let mut repl = Repl::new();
    let result = repl.execute(
        "## Theorem: GtToGe\n    Statement: (implies (Gt x 0) (Ge x 1)).\n    Proof: auto.",
    );
    assert!(
        result.is_ok(),
        "auto should prove x>0 -> x>=1 via omega: {:?}",
        result
    );
}

#[test]
fn test_literate_auto_omega_variable() {
    let mut repl = Repl::new();
    let result = repl
        .execute("## Theorem: XLtXPlusOne\n    Statement: (Lt x (add x 1)).\n    Proof: auto.");
    assert!(
        result.is_ok(),
        "auto should prove x < x+1: {:?}",
        result
    );
}

// =============================================================================
// LIA VIA AUTO
// =============================================================================

#[test]
fn test_literate_auto_lia_le() {
    let mut repl = Repl::new();
    let result = repl.execute("## Theorem: XLeX\n    Statement: (Le x x).\n    Proof: auto.");
    assert!(result.is_ok(), "auto should prove x <= x: {:?}", result);
}

#[test]
fn test_literate_auto_lia_transitivity() {
    let mut repl = Repl::new();
    let result = repl.execute(
        "## Theorem: LeTrans\n    Statement: (implies (Le x y) (implies (Le y z) (Le x z))).\n    Proof: auto.",
    );
    assert!(
        result.is_ok(),
        "auto should prove le transitivity: {:?}",
        result
    );
}

// =============================================================================
// FAILURE CASES
// =============================================================================

#[test]
fn test_literate_auto_fails_false() {
    let mut repl = Repl::new();
    repl.execute("## Theorem: Wrong\n    Statement: (Lt 5 2).\n    Proof: auto.")
        .unwrap();
    repl.execute("Definition result : Syntax := concludes Wrong.")
        .unwrap();
    let result = repl.execute("Eval result.").unwrap();
    assert!(
        result.contains("Error"),
        "auto should fail on 5 < 2: {:?}",
        result
    );
}
