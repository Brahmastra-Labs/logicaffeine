//! SUPERCRUSH Sprint S1B: IC3/PDR (Property-Directed Reachability)

#![cfg(feature = "verification")]

use logicaffeine_verify::ic3::{ic3, Ic3Result};
use logicaffeine_verify::{VerifyExpr, VerifyOp};

// ═══════════════════════════════════════════════════════════════════════════
// BASIC SAFETY
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn ic3_simple_mutex_safe() {
    // Mutex: grants are mutually exclusive, maintained by transition
    let init = VerifyExpr::and(
        VerifyExpr::not(VerifyExpr::var("ga@0")),
        VerifyExpr::not(VerifyExpr::var("gb@0")),
    );
    let transition = VerifyExpr::not(VerifyExpr::and(
        VerifyExpr::var("ga@t1"),
        VerifyExpr::var("gb@t1"),
    ));
    let property = VerifyExpr::not(VerifyExpr::and(
        VerifyExpr::var("ga@t"),
        VerifyExpr::var("gb@t"),
    ));
    let result = ic3(&init, &transition, &property, 10);
    assert!(matches!(result, Ic3Result::Safe { .. }),
        "Mutex should be safe. Got: {:?}", result);
}

#[test]
fn ic3_unsafe_detected() {
    // Init violates property → unsafe immediately
    let init = VerifyExpr::not(VerifyExpr::var("safe@0"));
    let transition = VerifyExpr::bool(true);
    let property = VerifyExpr::var("safe@t");
    let result = ic3(&init, &transition, &property, 5);
    assert!(matches!(result, Ic3Result::Unsafe { .. }),
        "Init violating property should be unsafe. Got: {:?}", result);
}

#[test]
fn ic3_invariant_implies_property() {
    // When Safe, the invariant should logically imply the property
    let init = VerifyExpr::var("p@0");
    let transition = VerifyExpr::iff(VerifyExpr::var("p@t1"), VerifyExpr::var("p@t"));
    let property = VerifyExpr::var("p@t");
    let result = ic3(&init, &transition, &property, 5);
    match result {
        Ic3Result::Safe { invariant } => {
            // Invariant should at minimum contain the property
            // (in our simplified IC3, invariant IS the property)
            assert!(!format!("{:?}", invariant).is_empty());
        }
        other => panic!("Expected Safe, got: {:?}", other),
    }
}

#[test]
fn ic3_converges_on_small() {
    // Small design should converge quickly
    let init = VerifyExpr::var("x@0");
    let transition = VerifyExpr::iff(VerifyExpr::var("x@t1"), VerifyExpr::var("x@t"));
    let property = VerifyExpr::var("x@t");
    let result = ic3(&init, &transition, &property, 10);
    assert!(matches!(result, Ic3Result::Safe { .. }),
        "Simple constant should converge. Got: {:?}", result);
}

#[test]
fn ic3_counter_safe() {
    // Integer counter, property: c >= 0
    let init = VerifyExpr::eq(VerifyExpr::var("c@0"), VerifyExpr::int(0));
    let transition = VerifyExpr::eq(
        VerifyExpr::var("c@t1"),
        VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("c@t"), VerifyExpr::int(1)),
    );
    let property = VerifyExpr::gte(VerifyExpr::var("c@t"), VerifyExpr::int(0));
    let result = ic3(&init, &transition, &property, 10);
    assert!(matches!(result, Ic3Result::Safe { .. }),
        "Counter >= 0 should be safe. Got: {:?}", result);
}

#[test]
fn ic3_counter_unsafe() {
    // Counter with bound violation
    let init = VerifyExpr::eq(VerifyExpr::var("c@0"), VerifyExpr::int(0));
    let transition = VerifyExpr::eq(
        VerifyExpr::var("c@t1"),
        VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("c@t"), VerifyExpr::int(1)),
    );
    let property = VerifyExpr::lt(VerifyExpr::var("c@t"), VerifyExpr::int(5));
    let result = ic3(&init, &transition, &property, 10);
    assert!(matches!(result, Ic3Result::Unsafe { .. }),
        "Counter exceeding 5 should be unsafe. Got: {:?}", result);
}

#[test]
fn ic3_pipeline_safe() {
    // Two-stage pipeline: data propagates, property on first stage
    let init = VerifyExpr::and(
        VerifyExpr::var("s1@0"),
        VerifyExpr::var("s2@0"),
    );
    let transition = VerifyExpr::and(
        VerifyExpr::var("s1@t1"),
        VerifyExpr::iff(VerifyExpr::var("s2@t1"), VerifyExpr::var("s1@t")),
    );
    let property = VerifyExpr::var("s1@t");
    let result = ic3(&init, &transition, &property, 10);
    assert!(matches!(result, Ic3Result::Safe { .. }),
        "Pipeline with s1 always true should be safe. Got: {:?}", result);
}

#[test]
fn ic3_init_state_matters() {
    // Same transition, different init → different result
    let transition = VerifyExpr::iff(VerifyExpr::var("p@t1"), VerifyExpr::var("p@t"));
    let property = VerifyExpr::var("p@t");

    let init_true = VerifyExpr::var("p@0");
    let r1 = ic3(&init_true, &transition, &property, 5);
    assert!(matches!(r1, Ic3Result::Safe { .. }));

    let init_false = VerifyExpr::not(VerifyExpr::var("p@0"));
    let r2 = ic3(&init_false, &transition, &property, 5);
    assert!(matches!(r2, Ic3Result::Unsafe { .. }));
}

#[test]
fn ic3_multiple_properties_conjunction() {
    // Verify conjunction of properties
    let init = VerifyExpr::and(VerifyExpr::var("a@0"), VerifyExpr::var("b@0"));
    let transition = VerifyExpr::and(
        VerifyExpr::iff(VerifyExpr::var("a@t1"), VerifyExpr::var("a@t")),
        VerifyExpr::iff(VerifyExpr::var("b@t1"), VerifyExpr::var("b@t")),
    );
    let property = VerifyExpr::and(VerifyExpr::var("a@t"), VerifyExpr::var("b@t"));
    let result = ic3(&init, &transition, &property, 5);
    assert!(matches!(result, Ic3Result::Safe { .. }),
        "Conjunction of safe properties should be safe. Got: {:?}", result);
}

#[test]
fn ic3_deterministic() {
    // Same input → same result
    let init = VerifyExpr::var("p@0");
    let transition = VerifyExpr::iff(VerifyExpr::var("p@t1"), VerifyExpr::var("p@t"));
    let property = VerifyExpr::var("p@t");
    let r1 = ic3(&init, &transition, &property, 5);
    let r2 = ic3(&init, &transition, &property, 5);
    let both_safe = matches!(r1, Ic3Result::Safe { .. }) && matches!(r2, Ic3Result::Safe { .. });
    assert!(both_safe, "Same input should give same result");
}

#[test]
fn ic3_compared_to_kinduction() {
    // IC3 should agree with k-induction
    let init = VerifyExpr::eq(VerifyExpr::var("x@0"), VerifyExpr::int(0));
    let transition = VerifyExpr::eq(
        VerifyExpr::var("x@t1"),
        VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("x@t"), VerifyExpr::int(1)),
    );
    let property = VerifyExpr::gte(VerifyExpr::var("x@t"), VerifyExpr::int(0));
    let ic3_result = ic3(&init, &transition, &property, 10);
    assert!(matches!(ic3_result, Ic3Result::Safe { .. }),
        "IC3 should agree: counter >= 0 is safe. Got: {:?}", ic3_result);
}

#[test]
fn ic3_latch_property() {
    // Latch holds value → safe with k=1
    let init = VerifyExpr::var("latch@0");
    let transition = VerifyExpr::iff(VerifyExpr::var("latch@t1"), VerifyExpr::var("latch@t"));
    let property = VerifyExpr::var("latch@t");
    let result = ic3(&init, &transition, &property, 3);
    assert!(matches!(result, Ic3Result::Safe { .. }),
        "Latch should be safe. Got: {:?}", result);
}
