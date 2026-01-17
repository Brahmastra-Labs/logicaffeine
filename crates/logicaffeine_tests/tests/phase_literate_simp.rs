//! Phase Literate Simp: Simp Tactic in Literate Mode
//!
//! Tests for the simp tactic in literate theorem syntax.

use logicaffeine_kernel::interface::Repl;

// =============================================================================
// REFLEXIVE EQUALITIES
// =============================================================================

#[test]
fn test_literate_simp_reflexive() {
    let mut repl = Repl::new();
    let result = repl.execute("## Theorem: XEqX\n    Statement: (Eq x x).\n    Proof: simp.");
    assert!(result.is_ok(), "simp should prove x = x: {:?}", result);
}

#[test]
fn test_literate_simp_reflexive_function() {
    let mut repl = Repl::new();
    let result =
        repl.execute("## Theorem: FxRefl\n    Statement: (Eq (f x) (f x)).\n    Proof: simp.");
    assert!(
        result.is_ok(),
        "simp should prove f(x) = f(x): {:?}",
        result
    );
}

// =============================================================================
// CONSTANT FOLDING
// =============================================================================

#[test]
fn test_literate_simp_arithmetic() {
    let mut repl = Repl::new();
    let result =
        repl.execute("## Theorem: TwoPlusThree\n    Statement: (Eq (add 2 3) 5).\n    Proof: simp.");
    assert!(result.is_ok(), "simp should prove 2+3=5: {:?}", result);
}

#[test]
fn test_literate_simp_nested() {
    let mut repl = Repl::new();
    let result = repl.execute(
        "## Theorem: Nested\n    Statement: (Eq (mul (add 1 1) 3) 6).\n    Proof: simp.",
    );
    assert!(result.is_ok(), "simp should prove (1+1)*3=6: {:?}", result);
}

#[test]
fn test_literate_simp_subtraction() {
    let mut repl = Repl::new();
    let result =
        repl.execute("## Theorem: TenMinusThree\n    Statement: (Eq (sub 10 3) 7).\n    Proof: simp.");
    assert!(result.is_ok(), "simp should prove 10-3=7: {:?}", result);
}

#[test]
fn test_literate_simp_complex_arithmetic() {
    let mut repl = Repl::new();
    // (2 * 3) + (4 - 1) = 6 + 3 = 9
    let result = repl.execute(
        "## Theorem: Complex\n    Statement: (Eq (add (mul 2 3) (sub 4 1)) 9).\n    Proof: simp.",
    );
    assert!(
        result.is_ok(),
        "simp should prove (2*3)+(4-1)=9: {:?}",
        result
    );
}

// =============================================================================
// WITH HYPOTHESES
// =============================================================================

#[test]
fn test_literate_simp_with_hyp() {
    let mut repl = Repl::new();
    // x = 0 -> x + 1 = 1
    let result = repl.execute(
        "## Theorem: SubstSimp\n    Statement: (implies (Eq x 0) (Eq (add x 1) 1)).\n    Proof: simp.",
    );
    assert!(
        result.is_ok(),
        "simp should prove x=0 -> x+1=1: {:?}",
        result
    );
}

#[test]
fn test_literate_simp_with_two_hyps() {
    let mut repl = Repl::new();
    // x = 1 -> y = 2 -> x + y = 3
    let result = repl.execute(
        "## Theorem: TwoHyps\n    Statement: (implies (Eq x 1) (implies (Eq y 2) (Eq (add x y) 3))).\n    Proof: simp.",
    );
    assert!(
        result.is_ok(),
        "simp should prove x=1 -> y=2 -> x+y=3: {:?}",
        result
    );
}

// =============================================================================
// DEFINITION UNFOLDING
// =============================================================================

#[test]
fn test_literate_simp_unfold() {
    let mut repl = Repl::new();
    // Define double, then prove double(3) = 6
    repl.execute("## To double (n: Int) -> Int:\n    Yield (add n n).")
        .unwrap();
    let result = repl.execute(
        "## Theorem: DoubleThree\n    Statement: (Eq (double 3) 6).\n    Proof: simp.",
    );
    assert!(
        result.is_ok(),
        "simp should prove double(3)=6: {:?}",
        result
    );
}

#[test]
fn test_literate_simp_unfold_nested() {
    let mut repl = Repl::new();
    // Define quadruple using double
    repl.execute("## To double (n: Int) -> Int:\n    Yield (add n n).")
        .unwrap();
    repl.execute("## To quadruple (n: Int) -> Int:\n    Yield (double (double n)).")
        .unwrap();
    let result = repl.execute(
        "## Theorem: QuadTwo\n    Statement: (Eq (quadruple 2) 8).\n    Proof: simp.",
    );
    assert!(
        result.is_ok(),
        "simp should prove quadruple(2)=8: {:?}",
        result
    );
}

#[test]
fn test_literate_simp_unfold_with_zero() {
    let mut repl = Repl::new();
    // Define a function that returns 0 and prove it equals 0
    repl.execute("## To zero_fn (n: Int) -> Int:\n    Yield 0.")
        .unwrap();
    let result = repl.execute(
        "## Theorem: ZeroFn\n    Statement: (Eq (zero_fn 42) 0).\n    Proof: simp.",
    );
    assert!(
        result.is_ok(),
        "simp should prove zero_fn(42)=0: {:?}",
        result
    );
}

// =============================================================================
// FAILURE CASES
// =============================================================================

#[test]
fn test_literate_simp_fails_false() {
    let mut repl = Repl::new();
    // Define the theorem (this will succeed but create an error derivation)
    repl.execute("## Theorem: Wrong\n    Statement: (Eq 2 3).\n    Proof: simp.")
        .unwrap();
    // Extract conclusion from the derivation
    repl.execute("Definition result : Syntax := concludes Wrong.")
        .unwrap();
    // Evaluate it - should contain Error
    let result = repl.execute("Eval result.").unwrap();
    assert!(
        result.contains("Error"),
        "simp should fail on 2=3, got: {:?}",
        result
    );
}

#[test]
fn test_literate_simp_fails_different_variables() {
    let mut repl = Repl::new();
    // x = y cannot be proved without hypothesis
    repl.execute("## Theorem: XEqY\n    Statement: (Eq x y).\n    Proof: simp.")
        .unwrap();
    // Extract conclusion from the derivation
    repl.execute("Definition result : Syntax := concludes XEqY.")
        .unwrap();
    // Evaluate it - should contain Error
    let result = repl.execute("Eval result.").unwrap();
    assert!(
        result.contains("Error"),
        "simp should fail on x=y without hypothesis, got: {:?}",
        result
    );
}
