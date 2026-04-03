//! SUPERCRUSH Sprint S1C: Interpolation-Based Model Checking

#![cfg(feature = "verification")]

use logicaffeine_verify::interpolation::{interpolate, itp_model_check, InterpolationResult};
use logicaffeine_verify::{VerifyExpr, VerifyOp};

// ═══════════════════════════════════════════════════════════════════════════
// INTERPOLANT COMPUTATION
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn interpolant_exists_for_unsat() {
    // A AND B is UNSAT → interpolant exists
    let a = VerifyExpr::var("p");
    let b = VerifyExpr::not(VerifyExpr::var("p"));
    let result = interpolate(&a, &b);
    assert!(result.is_some(), "p AND NOT p is UNSAT, interpolant should exist");
}

#[test]
fn interpolant_none_for_sat() {
    // A AND B is SAT → no interpolant
    let a = VerifyExpr::var("p");
    let b = VerifyExpr::var("q");
    let result = interpolate(&a, &b);
    assert!(result.is_none(), "p AND q is SAT, no interpolant should exist");
}

#[test]
fn interpolant_trivial_false() {
    // false AND B → interpolant exists (false is a valid interpolant)
    let a = VerifyExpr::bool(false);
    let b = VerifyExpr::var("p");
    let result = interpolate(&a, &b);
    assert!(result.is_some(), "false AND p is UNSAT");
}

#[test]
fn interpolant_integer() {
    // (x > 10) AND (x < 5) is UNSAT
    let a = VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(10));
    let b = VerifyExpr::lt(VerifyExpr::var("x"), VerifyExpr::int(5));
    let result = interpolate(&a, &b);
    assert!(result.is_some(), "(x>10) AND (x<5) is UNSAT, interpolant should exist");
}

#[test]
fn interpolant_complex_unsat() {
    // (p AND q) AND (NOT p) is UNSAT
    let a = VerifyExpr::and(VerifyExpr::var("p"), VerifyExpr::var("q"));
    let b = VerifyExpr::not(VerifyExpr::var("p"));
    let result = interpolate(&a, &b);
    assert!(result.is_some());
}

// ═══════════════════════════════════════════════════════════════════════════
// INTERPOLATION MODEL CHECKING
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn itp_mc_safe_constant() {
    // Constant true property with trivial system
    let init = VerifyExpr::bool(true);
    let transition = VerifyExpr::bool(true);
    let property = VerifyExpr::bool(true);
    let result = itp_model_check(&init, &transition, &property, 5);
    assert!(matches!(result, InterpolationResult::Safe | InterpolationResult::Fixpoint { .. }),
        "Trivially true property should be safe. Got: {:?}", result);
}

#[test]
fn itp_mc_unsafe_immediate() {
    // Property violated at step 0
    let init = VerifyExpr::not(VerifyExpr::var("p@0"));
    let transition = VerifyExpr::bool(true);
    let property = VerifyExpr::var("p@t");
    let result = itp_model_check(&init, &transition, &property, 5);
    assert!(matches!(result, InterpolationResult::Unsafe { .. }),
        "Property false at init should be unsafe. Got: {:?}", result);
}

#[test]
fn itp_mc_safe_inductive() {
    // p starts true, stays true
    let init = VerifyExpr::var("p@0");
    let transition = VerifyExpr::iff(VerifyExpr::var("p@t1"), VerifyExpr::var("p@t"));
    let property = VerifyExpr::var("p@t");
    let result = itp_model_check(&init, &transition, &property, 5);
    assert!(matches!(result, InterpolationResult::Safe | InterpolationResult::Fixpoint { .. }),
        "Inductive property should be safe. Got: {:?}", result);
}

#[test]
fn itp_mc_unsafe_reachable() {
    // x starts at 0, increments, property x < 3 fails
    let init = VerifyExpr::eq(VerifyExpr::var("x@0"), VerifyExpr::int(0));
    let transition = VerifyExpr::eq(
        VerifyExpr::var("x@t1"),
        VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("x@t"), VerifyExpr::int(1)),
    );
    let property = VerifyExpr::lt(VerifyExpr::var("x@t"), VerifyExpr::int(3));
    let result = itp_model_check(&init, &transition, &property, 10);
    assert!(matches!(result, InterpolationResult::Unsafe { .. }),
        "Counter exceeding 3 should be unsafe. Got: {:?}", result);
}

#[test]
fn itp_mc_safe_counter() {
    // Counter always >= 0
    let init = VerifyExpr::eq(VerifyExpr::var("c@0"), VerifyExpr::int(0));
    let transition = VerifyExpr::eq(
        VerifyExpr::var("c@t1"),
        VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("c@t"), VerifyExpr::int(1)),
    );
    let property = VerifyExpr::gte(VerifyExpr::var("c@t"), VerifyExpr::int(0));
    let result = itp_model_check(&init, &transition, &property, 10);
    assert!(matches!(result, InterpolationResult::Safe | InterpolationResult::Fixpoint { .. }),
        "Counter >= 0 should be safe. Got: {:?}", result);
}

#[test]
fn itp_mc_pipeline() {
    // Two-stage pipeline: data propagates, stays valid
    let init = VerifyExpr::and(
        VerifyExpr::var("s1@0"),
        VerifyExpr::var("s2@0"),
    );
    let transition = VerifyExpr::and(
        VerifyExpr::var("s1@t1"),
        VerifyExpr::iff(VerifyExpr::var("s2@t1"), VerifyExpr::var("s1@t")),
    );
    let property = VerifyExpr::var("s1@t");
    let result = itp_model_check(&init, &transition, &property, 5);
    assert!(matches!(result, InterpolationResult::Safe | InterpolationResult::Fixpoint { .. }),
        "Pipeline should be safe. Got: {:?}", result);
}

#[test]
fn itp_mc_fixpoint_iterations() {
    // Check that fixpoint returns a reasonable iteration count
    let init = VerifyExpr::var("p@0");
    let transition = VerifyExpr::iff(VerifyExpr::var("p@t1"), VerifyExpr::var("p@t"));
    let property = VerifyExpr::var("p@t");
    let result = itp_model_check(&init, &transition, &property, 10);
    match result {
        InterpolationResult::Safe => {} // acceptable
        InterpolationResult::Fixpoint { iterations } => {
            assert!(iterations <= 10, "Should converge within bound");
        }
        other => panic!("Expected Safe or Fixpoint, got: {:?}", other),
    }
}

#[test]
fn itp_mc_compared_to_kinduction() {
    // Same problem should give same safety result as k-induction
    let init = VerifyExpr::eq(VerifyExpr::var("x@0"), VerifyExpr::int(0));
    let transition = VerifyExpr::eq(
        VerifyExpr::var("x@t1"),
        VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("x@t"), VerifyExpr::int(1)),
    );
    let property = VerifyExpr::gte(VerifyExpr::var("x@t"), VerifyExpr::int(0));

    let itp_result = itp_model_check(&init, &transition, &property, 5);
    let is_safe = matches!(itp_result, InterpolationResult::Safe | InterpolationResult::Fixpoint { .. });
    assert!(is_safe, "ITP should agree with k-induction on safety. Got: {:?}", itp_result);
}
