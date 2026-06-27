//! =============================================================================
//! PHASE 78: THE GREAT INTEGRATION - END-TO-END VERIFICATION
//! =============================================================================
//!
//! This is the culmination: natural language → verified proof object.
//! The Socrates syllogism, machine-checked.
//!
//! Pipeline:
//! Input → Parse → Engine → Certify → Type-Check → Verified

use logicaffeine_kernel::{infer_type, Term};
use logicaffeine_compile::verify_theorem;

// =============================================================================
// THE SOCRATES VERIFICATION
// =============================================================================

#[test]
fn test_socrates_full_pipeline_verification() {
    // The classic syllogism, verified end-to-end:
    // Given: Socrates is a man.
    // Given: Every man is mortal.
    // Prove: Socrates is mortal.

    let input = r#"
## Theorem: Socrates_Mortality_Verified
Given: Socrates is a man.
Given: Every man is mortal.
Prove: Socrates is mortal.
Proof: Auto.
"#;

    // 1. Run the full pipeline
    let result = verify_theorem(input);

    if let Err(e) = &result {
        println!("Pipeline Failure: {:?}", e);
    }

    assert!(
        result.is_ok(),
        "The Socrates proof failed kernel verification!"
    );

    let (proof_term, ctx) = result.unwrap();

    println!("Certified Proof Object: {}", proof_term);

    // 2. Independent Verification
    // The returned term should type-check in the returned context.
    let validation = infer_type(&ctx, &proof_term);
    assert!(
        validation.is_ok(),
        "Returned proof object is invalid in its context!"
    );

    // 3. Check the Type
    // The type should represent "mortal(Socrates)"
    // Note: predicates are normalized to lowercase for semantic consistency
    let proof_type = validation.unwrap();
    println!("Proof Type: {}", proof_type);

    // The type should be an application: (mortal Socrates)
    if let Term::App(func, arg) = proof_type {
        match *func {
            Term::Global(s) => assert_eq!(s, "mortal"),
            _ => panic!("Expected mortal predicate"),
        }
        match *arg {
            Term::Global(s) => assert_eq!(s, "Socrates"),
            _ => panic!("Expected Socrates argument"),
        }
    } else {
        panic!("Proof type has wrong structure: {:?}", proof_type);
    }
}

// =============================================================================
// CHAIN REASONING VERIFICATION
// =============================================================================

#[test]
fn test_chain_reasoning_verification() {
    // Multi-step: All men are mortal. All mortals are doomed. Socrates is a man.
    // ∴ Socrates is doomed.

    let input = r#"
## Theorem: Socrates_Doom_Verified
Given: Socrates is a man.
Given: Every man is mortal.
Given: Every mortal is doomed.
Prove: Socrates is doomed.
Proof: Auto.
"#;

    let result = verify_theorem(input);
    assert!(
        result.is_ok(),
        "Chain reasoning verification failed: {:?}",
        result
    );

    let (proof_term, ctx) = result.unwrap();
    let proof_type = infer_type(&ctx, &proof_term).expect("Type check failed");

    println!("Chain proof term: {}", proof_term);
    println!("Chain proof type: {}", proof_type);
}

// =============================================================================
// DIRECT MATCH VERIFICATION
// =============================================================================

#[test]
fn test_trivial_verification() {
    // Trivial case: goal equals premise.

    let input = r#"
## Theorem: Trivial_Verified
Given: Socrates is mortal.
Prove: Socrates is mortal.
Proof: Auto.
"#;

    let result = verify_theorem(input);
    assert!(result.is_ok(), "Trivial verification failed: {:?}", result);
}

// =============================================================================
// VERIFICATION FAILURE TEST
// =============================================================================

#[test]
fn test_verification_fails_for_unprovable() {
    // Should fail: we don't have "Socrates is a man" → cannot derive mortality.

    let input = r#"
## Theorem: Incomplete
Given: Every man is mortal.
Prove: Socrates is mortal.
Proof: Auto.
"#;

    let result = verify_theorem(input);
    assert!(result.is_err(), "Should fail - missing premise!");
}

// =============================================================================
// RUNG 0a — USER-DEFINED PREDICATES (`## Define`)
// =============================================================================

/// A `## Define` block mints a new predicate that a later `## Theorem` can use
/// and the prover can prove — end-to-end: English `gizmo(x) :↔ shiny(x) ∧ round(x)`,
/// then `Pat is a gizmo` proved from `Pat is shiny` and `Pat is round` via `Auto`.
#[test]
fn define_block_lets_theorem_use_a_new_predicate() {
    let input = r#"
## Define
x is a gizmo if and only if x is shiny and x is round.

## Theorem: Pat_Is_A_Gizmo
Given: Pat is shiny.
Given: Pat is round.
Prove: Pat is a gizmo.
Proof: Auto.
"#;

    let result = verify_theorem(input);
    assert!(
        result.is_ok(),
        "a theorem using a user-defined predicate should verify: {:?}",
        result.err()
    );

    // The returned proof object re-checks in its context (defeq to `gizmo Pat`).
    let (proof_term, ctx) = result.unwrap();
    assert!(
        infer_type(&ctx, &proof_term).is_ok(),
        "the certified proof term must type-check"
    );
}

/// A definition is NOT a free pass: without the second premise, the unfolded
/// goal still has an unmet conjunct, so it must fail.
#[test]
fn define_block_does_not_make_unfounded_goals_pass() {
    let input = r#"
## Define
x is a gizmo if and only if x is shiny and x is round.

## Theorem: Pat_Is_A_Gizmo_Unfounded
Given: Pat is shiny.
Prove: Pat is a gizmo.
Proof: Auto.
"#;

    assert!(
        verify_theorem(input).is_err(),
        "gizmo(Pat) must not verify from shiny(Pat) alone"
    );
}
