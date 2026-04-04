//! SUPERCRUSH Sprint S1A: k-Induction for Unbounded Safety Verification
//!
//! BMC proves for k cycles. k-Induction proves forever.

#![cfg(feature = "verification")]

use logicaffeine_verify::kinduction::{k_induction, KInductionResult, SignalDecl};
use logicaffeine_verify::{VerifyExpr, VerifyOp, VerifyType};

// Helper to make a signal decl
fn sig(name: &str) -> SignalDecl {
    SignalDecl { name: name.into(), width: None }
}

// ═══════════════════════════════════════════════════════════════════════════
// BASIC SAFETY PROPERTIES
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kind_simple_safety_proven() {
    // x starts at 0, increments by 1 each step
    // Property: x >= 0 (always true since x = step number)
    let init = VerifyExpr::eq(VerifyExpr::var("x@0"), VerifyExpr::int(0));
    let transition = VerifyExpr::eq(
        VerifyExpr::var("x@t1"),
        VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("x@t"), VerifyExpr::int(1)),
    );
    let property = VerifyExpr::gte(VerifyExpr::var("x@t"), VerifyExpr::int(0));
    let result = k_induction(&init, &transition, &property, &[sig("x")], 5);
    assert!(matches!(result, KInductionResult::Proven { .. }),
        "x starting at 0, incrementing by 1, should always be >= 0. Got: {:?}", result);
}

#[test]
fn kind_violation_detected() {
    // x starts at 0, increments by 1
    // Property: x < 3 (fails after 3 steps)
    let init = VerifyExpr::eq(VerifyExpr::var("x@0"), VerifyExpr::int(0));
    let transition = VerifyExpr::eq(
        VerifyExpr::var("x@t1"),
        VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("x@t"), VerifyExpr::int(1)),
    );
    let property = VerifyExpr::lt(VerifyExpr::var("x@t"), VerifyExpr::int(3));
    let result = k_induction(&init, &transition, &property, &[sig("x")], 10);
    assert!(matches!(result, KInductionResult::Counterexample { .. }),
        "x < 3 should fail after 3 steps. Got: {:?}", result);
}

#[test]
fn kind_mutex_proven() {
    // Two boolean signals: grant_a@t and grant_b@t
    // Property: NOT(grant_a AND grant_b) — mutual exclusion
    // Transition: just constrain that at most one is true
    let init = VerifyExpr::and(
        VerifyExpr::not(VerifyExpr::var("grant_a@0")),
        VerifyExpr::not(VerifyExpr::var("grant_b@0")),
    );
    let transition = VerifyExpr::not(VerifyExpr::and(
        VerifyExpr::var("grant_a@t1"),
        VerifyExpr::var("grant_b@t1"),
    ));
    let property = VerifyExpr::not(VerifyExpr::and(
        VerifyExpr::var("grant_a@t"),
        VerifyExpr::var("grant_b@t"),
    ));
    let result = k_induction(
        &init, &transition, &property,
        &[sig("grant_a"), sig("grant_b")],
        5,
    );
    assert!(matches!(result, KInductionResult::Proven { .. }),
        "Mutex should be proven. Got: {:?}", result);
}

#[test]
fn kind_k1_is_bmc1() {
    // With k=1 and no inductive step succeeding, it's essentially BMC(1)
    // Property that holds at step 0 but we can't prove inductively
    let init = VerifyExpr::bool(true);
    let transition = VerifyExpr::bool(true);
    let property = VerifyExpr::var("x@t"); // Free variable, may or may not hold
    let result = k_induction(&init, &transition, &property, &[], 1);
    // Should either prove at k=1 or fail induction
    assert!(!matches!(result, KInductionResult::Unknown),
        "Should not return Unknown for simple formula");
}

#[test]
fn kind_empty_transition() {
    // Identity transition (state doesn't change)
    let init = VerifyExpr::var("safe@0");
    let transition = VerifyExpr::iff(
        VerifyExpr::var("safe@t1"),
        VerifyExpr::var("safe@t"),
    );
    let property = VerifyExpr::var("safe@t");
    let result = k_induction(&init, &transition, &property, &[sig("safe")], 3);
    assert!(matches!(result, KInductionResult::Proven { .. }),
        "Constant true signal should be proven safe. Got: {:?}", result);
}

#[test]
fn kind_incremental_k() {
    // Property that needs k=2 to prove inductively (not provable at k=1)
    // Two-phase pipeline: valid propagates through stages
    let init = VerifyExpr::and(
        VerifyExpr::var("stage1@0"),
        VerifyExpr::not(VerifyExpr::var("stage2@0")),
    );
    // stage2' = stage1, stage1' stays true
    let transition = VerifyExpr::and(
        VerifyExpr::var("stage1@t1"),
        VerifyExpr::iff(
            VerifyExpr::var("stage2@t1"),
            VerifyExpr::var("stage1@t"),
        ),
    );
    // Property: stage1 is always true
    let property = VerifyExpr::var("stage1@t");
    let result = k_induction(
        &init, &transition, &property,
        &[sig("stage1"), sig("stage2")],
        5,
    );
    assert!(matches!(result, KInductionResult::Proven { .. }),
        "Stage1 always true should be proven. Got: {:?}", result);
}

#[test]
fn kind_init_matters() {
    // Same transition but different init → different result
    let transition = VerifyExpr::iff(
        VerifyExpr::var("x@t1"),
        VerifyExpr::var("x@t"),
    );
    let property = VerifyExpr::var("x@t");

    // Init: x = true → should prove
    let init_true = VerifyExpr::var("x@0");
    let result1 = k_induction(&init_true, &transition, &property, &[sig("x")], 3);
    assert!(matches!(result1, KInductionResult::Proven { .. }),
        "x starting true, staying true, property x should prove. Got: {:?}", result1);

    // Init: x = false → should fail (counterexample at step 0)
    let init_false = VerifyExpr::not(VerifyExpr::var("x@0"));
    let result2 = k_induction(&init_false, &transition, &property, &[sig("x")], 3);
    assert!(matches!(result2, KInductionResult::Counterexample { .. }),
        "x starting false, property x should fail. Got: {:?}", result2);
}

#[test]
fn kind_multiple_signals() {
    // 3-signal design: a, b, c where c = a AND b always
    let init = VerifyExpr::and(
        VerifyExpr::var("a@0"),
        VerifyExpr::and(
            VerifyExpr::var("b@0"),
            VerifyExpr::iff(
                VerifyExpr::var("c@0"),
                VerifyExpr::and(VerifyExpr::var("a@0"), VerifyExpr::var("b@0")),
            ),
        ),
    );
    let transition = VerifyExpr::and(
        VerifyExpr::var("a@t1"),
        VerifyExpr::and(
            VerifyExpr::var("b@t1"),
            VerifyExpr::iff(
                VerifyExpr::var("c@t1"),
                VerifyExpr::and(VerifyExpr::var("a@t1"), VerifyExpr::var("b@t1")),
            ),
        ),
    );
    // Property: c is always true (since a and b are always true)
    let property = VerifyExpr::var("c@t");
    let result = k_induction(
        &init, &transition, &property,
        &[sig("a"), sig("b"), sig("c")],
        3,
    );
    assert!(matches!(result, KInductionResult::Proven { .. }),
        "c = a AND b with a,b always true should prove. Got: {:?}", result);
}

#[test]
fn kind_integer_property() {
    // Integer counter, property: counter >= 0
    let init = VerifyExpr::eq(VerifyExpr::var("c@0"), VerifyExpr::int(0));
    let transition = VerifyExpr::eq(
        VerifyExpr::var("c@t1"),
        VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("c@t"), VerifyExpr::int(1)),
    );
    let property = VerifyExpr::gte(VerifyExpr::var("c@t"), VerifyExpr::int(0));
    let result = k_induction(&init, &transition, &property, &[sig("c")], 5);
    assert!(matches!(result, KInductionResult::Proven { .. }),
        "Counter starting at 0, incrementing should always be >= 0. Got: {:?}", result);
}

#[test]
fn kind_proven_k_value() {
    // Verify that the returned k value makes sense
    let init = VerifyExpr::var("p@0");
    let transition = VerifyExpr::iff(VerifyExpr::var("p@t1"), VerifyExpr::var("p@t"));
    let property = VerifyExpr::var("p@t");
    let result = k_induction(&init, &transition, &property, &[sig("p")], 10);
    match result {
        KInductionResult::Proven { k } => {
            assert!(k >= 1 && k <= 10, "k should be between 1 and 10, got: {}", k);
        }
        other => panic!("Expected Proven, got: {:?}", other),
    }
}

#[test]
fn kind_counterexample_k_value() {
    // Property fails: x starts true, transitions to false, property: x always true
    let init = VerifyExpr::var("x@0");
    let transition = VerifyExpr::not(VerifyExpr::var("x@t1")); // x becomes false
    let property = VerifyExpr::var("x@t");
    let result = k_induction(&init, &transition, &property, &[sig("x")], 5);
    match result {
        KInductionResult::Counterexample { k, .. } => {
            assert!(k >= 1, "Counterexample should be at k >= 1, got: {}", k);
        }
        other => panic!("Expected Counterexample, got: {:?}", other),
    }
}

// ═══════════════════════════════════════════════════════════════════════════
// EXTENDED K-INDUCTION TESTS — SPEC DELTA
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn kind_latch_holds_value() {
    // Latch: value once set stays set
    let init = VerifyExpr::var("latch@0");
    let transition = VerifyExpr::iff(VerifyExpr::var("latch@t1"), VerifyExpr::var("latch@t"));
    let property = VerifyExpr::var("latch@t");
    let result = k_induction(&init, &transition, &property, &[sig("latch")], 3);
    match result {
        KInductionResult::Proven { k } => {
            assert!(k <= 3, "Latch should prove with small k. Got k={}", k);
        }
        other => panic!("Latch should be proven. Got: {:?}", other),
    }
}

#[test]
fn kind_two_phase_pipeline() {
    // Two-phase pipeline: data at stage1 appears at stage2 next cycle
    // Property: stage2 is always true after initial transient
    let init = VerifyExpr::and(
        VerifyExpr::var("s1@0"),
        VerifyExpr::var("s2@0"),
    );
    let transition = VerifyExpr::and(
        VerifyExpr::var("s1@t1"),
        VerifyExpr::iff(VerifyExpr::var("s2@t1"), VerifyExpr::var("s1@t")),
    );
    let property = VerifyExpr::var("s2@t");
    let result = k_induction(
        &init, &transition, &property,
        &[sig("s1"), sig("s2")],
        5,
    );
    assert!(matches!(result, KInductionResult::Proven { .. }),
        "Pipeline stage2 should hold when s1 always true. Got: {:?}", result);
}

#[test]
fn kind_fairness_not_needed() {
    // Pure safety: no fairness constraint needed
    let init = VerifyExpr::var("safe@0");
    let transition = VerifyExpr::iff(VerifyExpr::var("safe@t1"), VerifyExpr::var("safe@t"));
    let property = VerifyExpr::var("safe@t");
    let result = k_induction(&init, &transition, &property, &[sig("safe")], 3);
    assert!(matches!(result, KInductionResult::Proven { .. }),
        "Pure safety needs no fairness. Got: {:?}", result);
}

#[test]
fn kind_conjunction_property() {
    // Property is a conjunction: both a AND b always hold
    let init = VerifyExpr::and(VerifyExpr::var("a@0"), VerifyExpr::var("b@0"));
    let transition = VerifyExpr::and(
        VerifyExpr::iff(VerifyExpr::var("a@t1"), VerifyExpr::var("a@t")),
        VerifyExpr::iff(VerifyExpr::var("b@t1"), VerifyExpr::var("b@t")),
    );
    let property = VerifyExpr::and(VerifyExpr::var("a@t"), VerifyExpr::var("b@t"));
    let result = k_induction(
        &init, &transition, &property,
        &[sig("a"), sig("b")],
        3,
    );
    assert!(matches!(result, KInductionResult::Proven { .. }),
        "Conjunction of preserved signals should prove. Got: {:?}", result);
}

#[test]
fn kind_or_property() {
    // Weaker property: a OR b — with both initialized true and preserved
    let init = VerifyExpr::and(VerifyExpr::var("a@0"), VerifyExpr::var("b@0"));
    let transition = VerifyExpr::and(
        VerifyExpr::iff(VerifyExpr::var("a@t1"), VerifyExpr::var("a@t")),
        VerifyExpr::iff(VerifyExpr::var("b@t1"), VerifyExpr::var("b@t")),
    );
    let property = VerifyExpr::or(VerifyExpr::var("a@t"), VerifyExpr::var("b@t"));
    let result = k_induction(
        &init, &transition, &property,
        &[sig("a"), sig("b")],
        5,
    );
    assert!(matches!(result, KInductionResult::Proven { .. }),
        "a OR b with both preserved should prove. Got: {:?}", result);
}

#[test]
fn kind_implication_property() {
    // a → b, with both always true
    let init = VerifyExpr::and(VerifyExpr::var("a@0"), VerifyExpr::var("b@0"));
    let transition = VerifyExpr::and(
        VerifyExpr::iff(VerifyExpr::var("a@t1"), VerifyExpr::var("a@t")),
        VerifyExpr::iff(VerifyExpr::var("b@t1"), VerifyExpr::var("b@t")),
    );
    let property = VerifyExpr::implies(VerifyExpr::var("a@t"), VerifyExpr::var("b@t"));
    let result = k_induction(
        &init, &transition, &property,
        &[sig("a"), sig("b")],
        3,
    );
    assert!(matches!(result, KInductionResult::Proven { .. }),
        "a→b with both true should prove. Got: {:?}", result);
}

#[test]
fn kind_counter_bound_violation() {
    // Counter starting at 0, incrementing by 2, property: c < 10
    let init = VerifyExpr::eq(VerifyExpr::var("c@0"), VerifyExpr::int(0));
    let transition = VerifyExpr::eq(
        VerifyExpr::var("c@t1"),
        VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("c@t"), VerifyExpr::int(2)),
    );
    let property = VerifyExpr::lt(VerifyExpr::var("c@t"), VerifyExpr::int(10));
    let result = k_induction(&init, &transition, &property, &[sig("c")], 10);
    assert!(matches!(result, KInductionResult::Counterexample { .. }),
        "Counter exceeding 10 by 2s should fail. Got: {:?}", result);
}

#[test]
fn kind_constant_value() {
    // Constant: x = 42 always
    let init = VerifyExpr::eq(VerifyExpr::var("x@0"), VerifyExpr::int(42));
    let transition = VerifyExpr::eq(VerifyExpr::var("x@t1"), VerifyExpr::int(42));
    let property = VerifyExpr::eq(VerifyExpr::var("x@t"), VerifyExpr::int(42));
    let result = k_induction(&init, &transition, &property, &[sig("x")], 3);
    assert!(matches!(result, KInductionResult::Proven { .. }),
        "Constant 42 should be proven. Got: {:?}", result);
}

#[test]
fn kind_negation_property() {
    // Property: NOT p — with p initially false
    let init = VerifyExpr::not(VerifyExpr::var("p@0"));
    let transition = VerifyExpr::iff(VerifyExpr::var("p@t1"), VerifyExpr::var("p@t"));
    let property = VerifyExpr::not(VerifyExpr::var("p@t"));
    let result = k_induction(&init, &transition, &property, &[sig("p")], 3);
    assert!(matches!(result, KInductionResult::Proven { .. }),
        "NOT p with p always false should prove. Got: {:?}", result);
}

#[test]
fn kind_four_signals() {
    // 4 signals, all preserved from init
    let init = VerifyExpr::and(
        VerifyExpr::and(VerifyExpr::var("a@0"), VerifyExpr::var("b@0")),
        VerifyExpr::and(VerifyExpr::var("c@0"), VerifyExpr::var("d@0")),
    );
    let transition = VerifyExpr::and(
        VerifyExpr::and(
            VerifyExpr::iff(VerifyExpr::var("a@t1"), VerifyExpr::var("a@t")),
            VerifyExpr::iff(VerifyExpr::var("b@t1"), VerifyExpr::var("b@t")),
        ),
        VerifyExpr::and(
            VerifyExpr::iff(VerifyExpr::var("c@t1"), VerifyExpr::var("c@t")),
            VerifyExpr::iff(VerifyExpr::var("d@t1"), VerifyExpr::var("d@t")),
        ),
    );
    let property = VerifyExpr::and(
        VerifyExpr::and(VerifyExpr::var("a@t"), VerifyExpr::var("b@t")),
        VerifyExpr::and(VerifyExpr::var("c@t"), VerifyExpr::var("d@t")),
    );
    let result = k_induction(
        &init, &transition, &property,
        &[sig("a"), sig("b"), sig("c"), sig("d")],
        3,
    );
    assert!(matches!(result, KInductionResult::Proven { .. }),
        "4 preserved signals should prove. Got: {:?}", result);
}

#[test]
fn kind_toggle_counterexample() {
    // Toggle: p flips each cycle
    // Property: p is always true (false at odd cycles)
    let init = VerifyExpr::var("p@0");
    let transition = VerifyExpr::iff(
        VerifyExpr::var("p@t1"),
        VerifyExpr::not(VerifyExpr::var("p@t")),
    );
    let property = VerifyExpr::var("p@t");
    let result = k_induction(&init, &transition, &property, &[sig("p")], 5);
    assert!(matches!(result, KInductionResult::Counterexample { .. }),
        "Toggle violates p at odd cycles. Got: {:?}", result);
}

#[test]
fn kind_max_k_respected() {
    // With max_k = 2, should not try beyond k=2
    let init = VerifyExpr::var("x@0");
    let transition = VerifyExpr::iff(VerifyExpr::var("x@t1"), VerifyExpr::var("x@t"));
    let property = VerifyExpr::var("x@t");
    let result = k_induction(&init, &transition, &property, &[sig("x")], 2);
    match result {
        KInductionResult::Proven { k } => {
            assert!(k <= 2, "k should not exceed max_k=2. Got k={}", k);
        }
        _ => {} // Other results are fine
    }
}

#[test]
fn kind_deterministic_result() {
    // Same input should give same output
    let init = VerifyExpr::var("p@0");
    let transition = VerifyExpr::iff(VerifyExpr::var("p@t1"), VerifyExpr::var("p@t"));
    let property = VerifyExpr::var("p@t");
    let r1 = k_induction(&init, &transition, &property, &[sig("p")], 3);
    let r2 = k_induction(&init, &transition, &property, &[sig("p")], 3);
    let both_proven = matches!(r1, KInductionResult::Proven { .. })
        && matches!(r2, KInductionResult::Proven { .. });
    assert!(both_proven, "Same input should give same result");
}
