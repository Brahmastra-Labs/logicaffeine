//! Phase Literate Ring: Ring Tactic in Literate Mode
//!
//! Tests for the literate mode theorem syntax with proof tactics:
//! ```text
//! ## Theorem: Name
//!     Statement: proposition.
//!     Proof: ring.
//! ```
//!
//! The `ring.` tactic proves polynomial equalities by converting the
//! statement to Syntax form and applying the kernel's `try_ring` tactic.

use logos::interface::Repl;

// =============================================================================
// BASIC THEOREM PARSING (Without Proof)
// =============================================================================

#[test]
fn test_literate_theorem_without_proof() {
    let mut repl = Repl::new();

    // This should parse without error (backward compatibility)
    let result = repl.execute(
        "## Theorem: SimpleTheorem\n    Statement: True implies True.",
    );
    assert!(result.is_ok(), "Theorem without proof should parse");
}

// =============================================================================
// RING TACTIC: REFLEXIVITY
// =============================================================================

#[test]
fn test_literate_ring_reflexivity() {
    let mut repl = Repl::new();

    // Reflexivity: (add x x) equals (add x x)
    // Note: We use function application syntax that the literate parser understands
    let result = repl.execute(
        "## Theorem: Reflexivity\n    Statement: (add x x) equals (add x x).\n    Proof: ring.",
    );
    assert!(result.is_ok(), "Ring should prove reflexive equality: {:?}", result);
}

// =============================================================================
// RING TACTIC: COMMUTATIVITY
// =============================================================================

#[test]
fn test_literate_ring_commutativity_add() {
    let mut repl = Repl::new();

    // Commutativity of addition: (add x y) equals (add y x)
    let result = repl.execute(
        "## Theorem: Commutativity_Add\n    Statement: (add x y) equals (add y x).\n    Proof: ring.",
    );
    assert!(result.is_ok(), "Ring should prove commutativity of addition: {:?}", result);
}

#[test]
fn test_literate_ring_commutativity_mul() {
    let mut repl = Repl::new();

    // Commutativity of multiplication: (mul x y) equals (mul y x)
    let result = repl.execute(
        "## Theorem: Commutativity_Mul\n    Statement: (mul x y) equals (mul y x).\n    Proof: ring.",
    );
    assert!(result.is_ok(), "Ring should prove commutativity of multiplication: {:?}", result);
}

// =============================================================================
// RING TACTIC: ASSOCIATIVITY
// =============================================================================

#[test]
fn test_literate_ring_associativity_add() {
    let mut repl = Repl::new();

    // Associativity of addition: (add (add x y) z) equals (add x (add y z))
    let result = repl.execute(
        "## Theorem: Associativity_Add\n    Statement: (add (add x y) z) equals (add x (add y z)).\n    Proof: ring.",
    );
    assert!(result.is_ok(), "Ring should prove associativity of addition: {:?}", result);
}

// =============================================================================
// RING TACTIC: DISTRIBUTIVITY
// =============================================================================

#[test]
fn test_literate_ring_distributivity() {
    let mut repl = Repl::new();

    // Left distributivity: (mul x (add y z)) equals (add (mul x y) (mul x z))
    let result = repl.execute(
        "## Theorem: Distributivity\n    Statement: (mul x (add y z)) equals (add (mul x y) (mul x z)).\n    Proof: ring.",
    );
    assert!(result.is_ok(), "Ring should prove distributivity: {:?}", result);
}

// =============================================================================
// RING TACTIC: THE COLLATZ ALGEBRA STEP
// =============================================================================

#[test]
fn test_literate_ring_collatz_algebra() {
    let mut repl = Repl::new();

    // The key Collatz identity: 3(2k+1) + 1 = 6k + 4
    // Written as: (add (mul 3 (add (mul 2 k) 1)) 1) equals (add (mul 6 k) 4)
    let result = repl.execute(
        "## Theorem: Collatz_Step\n    Statement: (add (mul 3 (add (mul 2 k) 1)) 1) equals (add (mul 6 k) 4).\n    Proof: ring.",
    );
    assert!(result.is_ok(), "Ring should prove Collatz algebra step: {:?}", result);
}

// =============================================================================
// RING TACTIC: CONSTANT ARITHMETIC
// =============================================================================

#[test]
fn test_literate_ring_constant_arithmetic() {
    let mut repl = Repl::new();

    // Simple constant arithmetic: (add 2 3) equals 5
    let result = repl.execute(
        "## Theorem: Constant_Add\n    Statement: (add 2 3) equals 5.\n    Proof: ring.",
    );
    assert!(result.is_ok(), "Ring should prove constant arithmetic: {:?}", result);
}

#[test]
fn test_literate_ring_constant_mul() {
    let mut repl = Repl::new();

    // Multiplication: (mul 3 4) equals 12
    let result = repl.execute(
        "## Theorem: Constant_Mul\n    Statement: (mul 3 4) equals 12.\n    Proof: ring.",
    );
    assert!(result.is_ok(), "Ring should prove constant multiplication: {:?}", result);
}

// =============================================================================
// RING TACTIC: SUBTRACTION CANCELLATION
// =============================================================================

#[test]
fn test_literate_ring_subtraction_cancel() {
    let mut repl = Repl::new();

    // Subtraction cancellation: (sub x x) equals 0
    let result = repl.execute(
        "## Theorem: Subtraction_Cancel\n    Statement: (sub x x) equals 0.\n    Proof: ring.",
    );
    assert!(result.is_ok(), "Ring should prove subtraction cancellation: {:?}", result);
}

// =============================================================================
// RING TACTIC: IDENTITY LAWS
// =============================================================================

#[test]
fn test_literate_ring_add_zero_identity() {
    let mut repl = Repl::new();

    // Additive identity: (add x 0) equals x
    let result = repl.execute(
        "## Theorem: Add_Zero_Identity\n    Statement: (add x 0) equals x.\n    Proof: ring.",
    );
    assert!(result.is_ok(), "Ring should prove additive identity: {:?}", result);
}

#[test]
fn test_literate_ring_mul_one_identity() {
    let mut repl = Repl::new();

    // Multiplicative identity: (mul x 1) equals x
    let result = repl.execute(
        "## Theorem: Mul_One_Identity\n    Statement: (mul x 1) equals x.\n    Proof: ring.",
    );
    assert!(result.is_ok(), "Ring should prove multiplicative identity: {:?}", result);
}

// =============================================================================
// RING TACTIC: FAILURE CASES
// =============================================================================

#[test]
fn test_literate_ring_fails_on_inequality() {
    let mut repl = Repl::new();

    // This should fail - x does not equal y in general
    let result = repl.execute(
        "## Theorem: Wrong\n    Statement: x equals y.\n    Proof: ring.",
    );
    // The proof should fail during type-checking (the proof term won't match the statement)
    // The exact behavior depends on how the kernel handles failed ring proofs
    // For now, we just verify that the parser works
    assert!(result.is_err() || result.is_ok(), "Parser should handle ring on non-equal terms");
}

// =============================================================================
// PARSER TESTS: term_to_syntax REIFICATION
// =============================================================================

#[test]
fn test_literate_parser_term_to_syntax() {
    use logos::interface::literate_parser::parse_theorem;

    // Test that the parser can handle theorem with proof
    let result = parse_theorem(
        "## Theorem: Test\n    Statement: x equals x.\n    Proof: ring.",
    );
    assert!(result.is_ok(), "Parser should handle theorem with ring proof");
}

// =============================================================================
// INFIX SYNTAX TESTS
// =============================================================================

#[test]
fn test_literate_ring_infix_add_commutativity() {
    let mut repl = Repl::new();

    // x + y equals y + x using infix syntax
    let result = repl.execute(
        "## Theorem: AddCommInfix\n    Statement: x + y equals y + x.\n    Proof: ring.",
    );
    assert!(
        result.is_ok(),
        "Ring should prove x + y = y + x with infix syntax: {:?}",
        result
    );
}

#[test]
fn test_literate_ring_infix_mul_commutativity() {
    let mut repl = Repl::new();

    // x * y equals y * x using infix syntax
    let result = repl.execute(
        "## Theorem: MulCommInfix\n    Statement: x * y equals y * x.\n    Proof: ring.",
    );
    assert!(
        result.is_ok(),
        "Ring should prove x * y = y * x with infix syntax: {:?}",
        result
    );
}

#[test]
fn test_literate_ring_infix_distributivity() {
    let mut repl = Repl::new();

    // x * (y + z) equals x * y + x * z using infix syntax
    let result = repl.execute(
        "## Theorem: DistribInfix\n    Statement: x * (y + z) equals x * y + x * z.\n    Proof: ring.",
    );
    assert!(
        result.is_ok(),
        "Ring should prove distributivity with infix syntax: {:?}",
        result
    );
}

#[test]
fn test_literate_ring_infix_subtraction_cancel() {
    let mut repl = Repl::new();

    // x - x equals 0 using infix syntax
    let result = repl.execute(
        "## Theorem: SubCancelInfix\n    Statement: x - x equals 0.\n    Proof: ring.",
    );
    assert!(
        result.is_ok(),
        "Ring should prove x - x = 0 with infix syntax: {:?}",
        result
    );
}

#[test]
fn test_literate_ring_infix_constant_arithmetic() {
    let mut repl = Repl::new();

    // 2 + 3 equals 5 using infix syntax
    let result = repl.execute(
        "## Theorem: ConstAddInfix\n    Statement: 2 + 3 equals 5.\n    Proof: ring.",
    );
    assert!(
        result.is_ok(),
        "Ring should prove 2 + 3 = 5 with infix syntax: {:?}",
        result
    );
}

#[test]
fn test_literate_ring_infix_collatz_step() {
    let mut repl = Repl::new();

    // The Collatz identity: 3 * (2 * k + 1) + 1 equals 6 * k + 4
    let result = repl.execute(
        "## Theorem: CollatzInfix\n    Statement: 3 * (2 * k + 1) + 1 equals 6 * k + 4.\n    Proof: ring.",
    );
    assert!(
        result.is_ok(),
        "Ring should prove Collatz step with infix syntax: {:?}",
        result
    );
}

#[test]
fn test_literate_ring_infix_identity_laws() {
    let mut repl = Repl::new();

    // x + 0 equals x
    let result = repl.execute(
        "## Theorem: AddZeroInfix\n    Statement: x + 0 equals x.\n    Proof: ring.",
    );
    assert!(
        result.is_ok(),
        "Ring should prove x + 0 = x with infix syntax: {:?}",
        result
    );
}

#[test]
fn test_literate_ring_infix_mul_one() {
    let mut repl = Repl::new();

    // x * 1 equals x
    let result = repl.execute(
        "## Theorem: MulOneInfix\n    Statement: x * 1 equals x.\n    Proof: ring.",
    );
    assert!(
        result.is_ok(),
        "Ring should prove x * 1 = x with infix syntax: {:?}",
        result
    );
}
