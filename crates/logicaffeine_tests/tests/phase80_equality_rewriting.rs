//! =============================================================================
//! PHASE 80: THE MIRROR - EQUALITY & REWRITING
//! =============================================================================
//!
//! The kernel knows `1 + 1 = 2`. It knows `P(2)`.
//! But it cannot conclude `P(1 + 1)`.
//!
//! Why? Because the Engine lacks **Substitution**.
//! It sees `1 + 1` and `2` as structurally different until normalized.
//! But with abstract variables `a = b`, normalization cannot help.
//!
//! We need **Leibniz's Law** (Indiscernibility of Identicals):
//! If a = b, then P(a) → P(b).
//!
//! This is implemented via `Eq_rec` in type theory:
//! Eq_rec : Π(A:Type) (x:A) (P:A→Type) (p:P x) (y:A) (e:Eq A x y) . P y

use logicaffeine_kernel::{infer_type, Term};
use logicaffeine_compile::verify_theorem;

// =============================================================================
// PART 1: LEIBNIZ REWRITE (The Core Test)
// =============================================================================

#[test]
fn test_leibniz_rewrite() {
    // The fundamental test: substitute equals for equals.
    // If Clark = Superman and Clark is mortal, then Superman is mortal.

    let input = r#"
## Theorem: Leibniz_Law
Given: Clark is equal to Superman.
Given: Clark is mortal.
Prove: Superman is mortal.
Proof: Auto.
"#;

    let result = verify_theorem(input);

    if let Err(e) = &result {
        println!("Leibniz rewrite failed: {:?}", e);
    }

    assert!(result.is_ok(), "Failed to verify Leibniz rewrite!");

    let (proof_term, ctx) = result.unwrap();
    println!("Leibniz Proof: {}", proof_term);

    // The proof term should use Eq_rec (or equivalent match on Eq)
    let proof_type = infer_type(&ctx, &proof_term).expect("Proof should type-check");
    println!("Proof Type: {}", proof_type);
}

// =============================================================================
// PART 2: CONCRETE EQUALITY REWRITE
// =============================================================================

#[test]
fn test_concrete_equality_rewrite() {
    // More concrete: use a named property.
    // Given: Alice = Bob. Given: Alice is happy. Prove: Bob is happy.

    let input = r#"
## Theorem: Happy_Rewrite
Given: Alice is equal to Bob.
Given: Alice is happy.
Prove: Bob is happy.
Proof: Auto.
"#;

    let result = verify_theorem(input);

    if let Err(e) = &result {
        println!("Concrete rewrite failed: {:?}", e);
    }

    assert!(result.is_ok(), "Failed to verify concrete equality rewrite!");
}

// =============================================================================
// PART 3: SYMMETRY OF EQUALITY
// =============================================================================

#[test]
fn test_equality_symmetry() {
    // Equality is symmetric: a = b implies b = a.
    // Given: Alice equals Bob.
    // Prove: Bob equals Alice.

    let input = r#"
## Theorem: Eq_Symmetry
Given: Alice is equal to Bob.
Prove: Bob is equal to Alice.
Proof: Auto.
"#;

    let result = verify_theorem(input);

    if let Err(e) = &result {
        println!("Symmetry failed: {:?}", e);
    }

    assert!(result.is_ok(), "Failed to verify equality symmetry!");
}

// =============================================================================
// PART 4: TRANSITIVITY OF EQUALITY
// =============================================================================

#[test]
fn test_equality_transitivity() {
    // Equality is transitive: a = b and b = c implies a = c.
    // Given: Alice equals Bob.
    // Given: Bob equals Charlie.
    // Prove: Alice equals Charlie.

    let input = r#"
## Theorem: Eq_Transitivity
Given: Alice is equal to Bob.
Given: Bob is equal to Charlie.
Prove: Alice is equal to Charlie.
Proof: Auto.
"#;

    let result = verify_theorem(input);

    if let Err(e) = &result {
        println!("Transitivity failed: {:?}", e);
    }

    assert!(result.is_ok(), "Failed to verify equality transitivity!");
}

// =============================================================================
// PART 5: REWRITE IN COMPLEX PREDICATE
// =============================================================================

#[test]
fn test_rewrite_in_implication() {
    // Rewrite combined with modus ponens.
    // Given: Alice = Bob. Given: If Alice is happy then Alice is dancing.
    // Given: Bob is happy. Prove: Bob is dancing.

    let input = r#"
## Theorem: Rewrite_Implication
Given: Alice is equal to Bob.
Given: If Alice is happy then Alice is dancing.
Given: Bob is happy.
Prove: Bob is dancing.
Proof: Auto.
"#;

    let result = verify_theorem(input);

    if let Err(e) = &result {
        println!("Rewrite in implication failed: {:?}", e);
    }

    assert!(result.is_ok(), "Failed to verify rewrite in implication!");
}

// =============================================================================
// PART 6: FAILURE CASE - WRONG DIRECTION
// =============================================================================

#[test]
fn test_rewrite_wrong_direction_fails() {
    // Should NOT prove Q(a) from P(a) = P(b) and Q(b) without more info.
    // This tests that we don't over-apply rewriting.

    let input = r#"
## Theorem: Wrong_Direction
Given: Alice is happy.
Prove: Bob is happy.
Proof: Auto.
"#;

    let result = verify_theorem(input);

    // This should FAIL - we have no connection between Alice and Bob
    assert!(result.is_err(), "Should fail - no equality between Alice and Bob!");
}
