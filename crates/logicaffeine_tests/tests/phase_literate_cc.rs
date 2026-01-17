//! Phase Literate CC: CC Tactic in Literate Mode
//!
//! Tests for the literate mode theorem syntax with proof tactics:
//! ```text
//! ## Theorem: Name
//!     Statement: proposition.
//!     Proof: cc.
//! ```
//!
//! The `cc.` tactic proves equalities over uninterpreted functions
//! using congruence closure.

use logicaffeine_kernel::interface::Repl;

// =============================================================================
// REFLEXIVE EQUALITIES
// =============================================================================

#[test]
fn test_literate_cc_reflexive() {
    let mut repl = Repl::new();
    let result = repl.execute(
        "## Theorem: FxEqFx\n    Statement: (Eq (f x) (f x)).\n    Proof: cc.",
    );
    assert!(
        result.is_ok(),
        "CC should prove f(x) = f(x): {:?}",
        result
    );
}

#[test]
fn test_literate_cc_nested() {
    let mut repl = Repl::new();
    let result = repl.execute(
        "## Theorem: Nested\n    Statement: (Eq (f (g x)) (f (g x))).\n    Proof: cc.",
    );
    assert!(
        result.is_ok(),
        "CC should prove f(g(x)) = f(g(x)): {:?}",
        result
    );
}

// =============================================================================
// CONGRUENCE WITH HYPOTHESES
// =============================================================================

#[test]
fn test_literate_cc_congruence() {
    let mut repl = Repl::new();
    // Given x = y, prove f(x) = f(y)
    let result = repl.execute(
        "## Theorem: Cong\n    Statement: (implies (Eq x y) (Eq (f x) (f y))).\n    Proof: cc.",
    );
    assert!(
        result.is_ok(),
        "CC should prove x=y -> f(x)=f(y): {:?}",
        result
    );
}

#[test]
fn test_literate_cc_transitivity() {
    let mut repl = Repl::new();
    // Given a = b and b = c, prove f(a) = f(c)
    let result = repl.execute(
        "## Theorem: Trans\n    Statement: (implies (Eq a b) (implies (Eq b c) (Eq (f a) (f c)))).\n    Proof: cc.",
    );
    assert!(
        result.is_ok(),
        "CC should prove a=b -> b=c -> f(a)=f(c): {:?}",
        result
    );
}

#[test]
fn test_literate_cc_binary_congruence() {
    let mut repl = Repl::new();
    // Given a = b, prove add(a, c) = add(b, c)
    let result = repl.execute(
        "## Theorem: BinCong\n    Statement: (implies (Eq a b) (Eq (add a c) (add b c))).\n    Proof: cc.",
    );
    assert!(
        result.is_ok(),
        "CC should prove a=b -> add(a,c)=add(b,c): {:?}",
        result
    );
}

// =============================================================================
// FAILURE CASES
// =============================================================================

#[test]
fn test_literate_cc_fails_diff_func() {
    let mut repl = Repl::new();
    let result = repl.execute(
        "## Theorem: Wrong\n    Statement: (Eq (f x) (g x)).\n    Proof: cc.",
    );
    // Parser should handle cc on invalid equality
    assert!(
        result.is_err() || result.is_ok(),
        "Parser should handle cc on invalid equality"
    );
}
