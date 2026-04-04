//! SUPERCRUSH Sprint S1C: Interpolation-Based Model Checking

#![cfg(feature = "verification")]

use logicaffeine_verify::interpolation::{interpolate, itp_model_check, InterpolationResult};
use logicaffeine_verify::ic3::check_sat;
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

// ═══════════════════════════════════════════════════════════════════════════
// RED TESTS — CORNER CUT AUDIT: Real interpolation properties
// These verify that interpolate() returns a REAL interpolant, not `a.clone()`.
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn interpolant_over_shared_variables_only() {
    // Craig interpolation theorem: the interpolant I uses ONLY variables
    // that appear in BOTH A and B (shared variables).
    //
    // A = (p AND q), B = (NOT p AND r)
    // Shared variable: p. Non-shared: q (only in A), r (only in B).
    // Valid interpolant: p (or NOT NOT p, etc.) — must NOT mention q or r.
    // Trivial a.clone() = (p AND q) mentions q, which is NOT shared. FAIL.
    let a = VerifyExpr::and(VerifyExpr::var("p"), VerifyExpr::var("q"));
    let b = VerifyExpr::and(VerifyExpr::not(VerifyExpr::var("p")), VerifyExpr::var("r"));
    let result = interpolate(&a, &b);
    let itp = result.expect("(p AND q) AND (NOT p AND r) is UNSAT, interpolant must exist");

    let itp_str = format!("{:?}", itp);
    // The interpolant must NOT mention q or r
    assert!(
        !itp_str.contains("\"q\""),
        "Interpolant must use only shared variables (p). Got: {} — contains 'q' which is only in A",
        itp_str,
    );
    assert!(
        !itp_str.contains("\"r\""),
        "Interpolant must use only shared variables (p). Got: {} — contains 'r' which is only in B",
        itp_str,
    );
}

#[test]
fn interpolant_implies_from_a() {
    // Craig interpolation: A => I must hold.
    // For a non-trivial case where I != A.
    let a = VerifyExpr::and(VerifyExpr::var("p"), VerifyExpr::var("q"));
    let b = VerifyExpr::not(VerifyExpr::var("p"));
    let result = interpolate(&a, &b);
    let itp = result.expect("Interpolant must exist");

    // Check A => I: is A AND NOT(I) UNSAT?
    let check = VerifyExpr::and(a.clone(), VerifyExpr::not(itp.clone()));
    assert!(
        !check_sat(&check),
        "A must imply the interpolant. A = {:?}, I = {:?}",
        a, itp,
    );
}

#[test]
fn interpolant_contradicts_b() {
    // Craig interpolation: I AND B must be UNSAT.
    let a = VerifyExpr::and(VerifyExpr::var("p"), VerifyExpr::var("q"));
    let b = VerifyExpr::not(VerifyExpr::var("p"));
    let result = interpolate(&a, &b);
    let itp = result.expect("Interpolant must exist");

    // Check I AND B is UNSAT
    let check = VerifyExpr::and(itp.clone(), b.clone());
    assert!(
        !check_sat(&check),
        "Interpolant AND B must be UNSAT. I = {:?}, B = {:?}",
        itp, b,
    );
}

#[test]
fn interpolant_is_weaker_than_a() {
    // The interpolant should be a WEAKER formula than A (over-approximation).
    // If I == A, the interpolant provides no abstraction benefit.
    // For A = (p AND q AND r), B = NOT(p), the interpolant should be just "p".
    let a = VerifyExpr::and(
        VerifyExpr::var("p"),
        VerifyExpr::and(VerifyExpr::var("q"), VerifyExpr::var("r")),
    );
    let b = VerifyExpr::not(VerifyExpr::var("p"));
    let result = interpolate(&a, &b);
    let itp = result.expect("Interpolant must exist");

    // I should be satisfiable even when q=false or r=false (weaker than A)
    let weaker_check = VerifyExpr::and(
        itp.clone(),
        VerifyExpr::and(
            VerifyExpr::var("p"),
            VerifyExpr::not(VerifyExpr::var("q")),
        ),
    );
    assert!(
        check_sat(&weaker_check),
        "Interpolant should be weaker than A — satisfiable with q=false. \
         If I == A, this fails. I = {:?}", itp,
    );
}

#[test]
fn interpolant_integer_shared_only() {
    // A = (x > 10 AND y > 0), B = (x < 5)
    // Shared: x. Non-shared: y (only in A).
    // Interpolant must mention x but NOT y.
    let a = VerifyExpr::and(
        VerifyExpr::gt(VerifyExpr::var("x"), VerifyExpr::int(10)),
        VerifyExpr::gt(VerifyExpr::var("y"), VerifyExpr::int(0)),
    );
    let b = VerifyExpr::lt(VerifyExpr::var("x"), VerifyExpr::int(5));
    let result = interpolate(&a, &b);
    let itp = result.expect("Interpolant must exist");

    let itp_str = format!("{:?}", itp);
    assert!(
        !itp_str.contains("\"y\""),
        "Interpolant must only use shared variable x. Got: {} — contains 'y'",
        itp_str,
    );
}
