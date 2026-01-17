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
