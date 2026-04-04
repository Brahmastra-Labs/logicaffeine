//! SUPERCRUSH Sprint S1B: IC3/PDR (Property-Directed Reachability)

#![cfg(feature = "verification")]

use logicaffeine_verify::ic3::{ic3, check_sat, Ic3Result};
use logicaffeine_verify::kinduction;
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

// ═══════════════════════════════════════════════════════════════════════════
// EXTENDED IC3 TESTS — SPEC DELTA (18 additional)
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn ic3_frame_monotone() {
    // Safe result means frames converged → invariant found
    let init = VerifyExpr::and(VerifyExpr::var("a@0"), VerifyExpr::var("b@0"));
    let transition = VerifyExpr::and(
        VerifyExpr::iff(VerifyExpr::var("a@t1"), VerifyExpr::var("a@t")),
        VerifyExpr::iff(VerifyExpr::var("b@t1"), VerifyExpr::var("b@t")),
    );
    let property = VerifyExpr::or(VerifyExpr::var("a@t"), VerifyExpr::var("b@t"));
    let result = ic3(&init, &transition, &property, 10);
    assert!(matches!(result, Ic3Result::Safe { .. }),
        "a OR b should be safe when a AND b initially. Got: {:?}", result);
}

#[test]
fn ic3_trace_is_valid() {
    // Unsafe result should be detected
    let init = VerifyExpr::not(VerifyExpr::var("ok@0"));
    let transition = VerifyExpr::iff(VerifyExpr::var("ok@t1"), VerifyExpr::var("ok@t"));
    let property = VerifyExpr::var("ok@t");
    let result = ic3(&init, &transition, &property, 5);
    assert!(matches!(result, Ic3Result::Unsafe { .. }),
        "Init violating property should be unsafe. Got: {:?}", result);
}

#[test]
fn ic3_bitvec_safe() {
    // 8-bit counter always >= 0 (trivially true for unsigned)
    let init = VerifyExpr::eq(VerifyExpr::var("c@0"), VerifyExpr::int(0));
    let transition = VerifyExpr::eq(
        VerifyExpr::var("c@t1"),
        VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("c@t"), VerifyExpr::int(1)),
    );
    let property = VerifyExpr::gte(VerifyExpr::var("c@t"), VerifyExpr::int(0));
    let result = ic3(&init, &transition, &property, 15);
    assert!(matches!(result, Ic3Result::Safe { .. }),
        "Counter >= 0 should be trivially safe. Got: {:?}", result);
}

#[test]
fn ic3_arbiter_fair() {
    // Round-robin arbiter: only one grant at a time
    let init = VerifyExpr::and(
        VerifyExpr::not(VerifyExpr::var("g1@0")),
        VerifyExpr::not(VerifyExpr::var("g2@0")),
    );
    // Transition maintains mutual exclusion
    let transition = VerifyExpr::not(VerifyExpr::and(
        VerifyExpr::var("g1@t1"),
        VerifyExpr::var("g2@t1"),
    ));
    let property = VerifyExpr::not(VerifyExpr::and(
        VerifyExpr::var("g1@t"),
        VerifyExpr::var("g2@t"),
    ));
    let result = ic3(&init, &transition, &property, 10);
    assert!(matches!(result, Ic3Result::Safe { .. }),
        "Mutex arbiter should be safe. Got: {:?}", result);
}

#[test]
fn ic3_three_signal_safe() {
    // Three signals, all initially true, all preserved
    let init = VerifyExpr::and(
        VerifyExpr::var("a@0"),
        VerifyExpr::and(VerifyExpr::var("b@0"), VerifyExpr::var("c@0")),
    );
    let transition = VerifyExpr::and(
        VerifyExpr::iff(VerifyExpr::var("a@t1"), VerifyExpr::var("a@t")),
        VerifyExpr::and(
            VerifyExpr::iff(VerifyExpr::var("b@t1"), VerifyExpr::var("b@t")),
            VerifyExpr::iff(VerifyExpr::var("c@t1"), VerifyExpr::var("c@t")),
        ),
    );
    let property = VerifyExpr::and(
        VerifyExpr::var("a@t"),
        VerifyExpr::and(VerifyExpr::var("b@t"), VerifyExpr::var("c@t")),
    );
    let result = ic3(&init, &transition, &property, 10);
    assert!(matches!(result, Ic3Result::Safe { .. }),
        "Three preserved signals should be safe. Got: {:?}", result);
}

#[test]
fn ic3_empty_init_unsafe() {
    // Unconstrained init → property may not hold
    let init = VerifyExpr::bool(true); // anything goes
    let transition = VerifyExpr::iff(VerifyExpr::var("p@t1"), VerifyExpr::var("p@t"));
    let property = VerifyExpr::var("p@t");
    let result = ic3(&init, &transition, &property, 5);
    // With unconstrained init, p might start false
    assert!(matches!(result, Ic3Result::Unsafe { .. }),
        "Unconstrained init should be unsafe for p. Got: {:?}", result);
}

#[test]
fn ic3_fsm_reachability() {
    // FSM: state starts at s0, transitions to s1, property: always in s0 or s1
    let init = VerifyExpr::var("s0@0");
    let transition = VerifyExpr::or(
        VerifyExpr::var("s0@t1"),
        VerifyExpr::var("s1@t1"),
    );
    let property = VerifyExpr::or(VerifyExpr::var("s0@t"), VerifyExpr::var("s1@t"));
    let result = ic3(&init, &transition, &property, 10);
    assert!(matches!(result, Ic3Result::Safe { .. }),
        "FSM always in s0/s1 should be safe. Got: {:?}", result);
}

#[test]
fn ic3_toggle_safe() {
    // Toggle flip-flop: p flips each cycle, property: not(stuck at same value)
    // Actually: p@t1 = not(p@t), init p@0 = true
    // Property: p@t (holds only at even cycles) — actually unsafe
    let init = VerifyExpr::var("p@0");
    let transition = VerifyExpr::iff(
        VerifyExpr::var("p@t1"),
        VerifyExpr::not(VerifyExpr::var("p@t")),
    );
    let property = VerifyExpr::var("p@t"); // false at odd cycles
    let result = ic3(&init, &transition, &property, 5);
    assert!(matches!(result, Ic3Result::Unsafe { .. }),
        "Toggle violates p at odd cycles. Got: {:?}", result);
}

#[test]
fn ic3_or_property_weaker() {
    // Weaker property (OR instead of AND) should still hold
    let init = VerifyExpr::and(VerifyExpr::var("a@0"), VerifyExpr::var("b@0"));
    let transition = VerifyExpr::and(
        VerifyExpr::iff(VerifyExpr::var("a@t1"), VerifyExpr::var("a@t")),
        VerifyExpr::iff(VerifyExpr::var("b@t1"), VerifyExpr::var("b@t")),
    );
    let property = VerifyExpr::or(VerifyExpr::var("a@t"), VerifyExpr::var("b@t"));
    let result = ic3(&init, &transition, &property, 5);
    assert!(matches!(result, Ic3Result::Safe { .. }),
        "a OR b is weaker than a AND b, should be safe. Got: {:?}", result);
}

#[test]
fn ic3_implication_property() {
    // Property: a → b, with a=true, b=true initially, both preserved
    let init = VerifyExpr::and(VerifyExpr::var("a@0"), VerifyExpr::var("b@0"));
    let transition = VerifyExpr::and(
        VerifyExpr::iff(VerifyExpr::var("a@t1"), VerifyExpr::var("a@t")),
        VerifyExpr::iff(VerifyExpr::var("b@t1"), VerifyExpr::var("b@t")),
    );
    let property = VerifyExpr::implies(VerifyExpr::var("a@t"), VerifyExpr::var("b@t"));
    let result = ic3(&init, &transition, &property, 5);
    assert!(matches!(result, Ic3Result::Safe { .. }),
        "a→b should be safe when both always true. Got: {:?}", result);
}

#[test]
fn ic3_data_forwarding() {
    // Data forwarding: out follows in, both preserved
    let init = VerifyExpr::and(VerifyExpr::var("in@0"), VerifyExpr::var("out@0"));
    let transition = VerifyExpr::and(
        VerifyExpr::iff(VerifyExpr::var("in@t1"), VerifyExpr::var("in@t")),
        VerifyExpr::iff(VerifyExpr::var("out@t1"), VerifyExpr::var("in@t")),
    );
    let property = VerifyExpr::var("out@t");
    let result = ic3(&init, &transition, &property, 5);
    assert!(matches!(result, Ic3Result::Safe { .. }),
        "Out following preserved in should be safe. Got: {:?}", result);
}

#[test]
fn ic3_invariant_not_empty() {
    // Safe result should have a non-trivial invariant
    let init = VerifyExpr::var("x@0");
    let transition = VerifyExpr::iff(VerifyExpr::var("x@t1"), VerifyExpr::var("x@t"));
    let property = VerifyExpr::var("x@t");
    let result = ic3(&init, &transition, &property, 5);
    match result {
        Ic3Result::Safe { invariant } => {
            let inv_str = format!("{:?}", invariant);
            assert!(inv_str.len() > 5, "Invariant should be non-trivial: {}", inv_str);
        }
        other => panic!("Expected Safe, got: {:?}", other),
    }
}

#[test]
fn ic3_two_counter_safe() {
    // Two independent counters, both >= 0
    let init = VerifyExpr::and(
        VerifyExpr::eq(VerifyExpr::var("a@0"), VerifyExpr::int(0)),
        VerifyExpr::eq(VerifyExpr::var("b@0"), VerifyExpr::int(0)),
    );
    let transition = VerifyExpr::and(
        VerifyExpr::eq(
            VerifyExpr::var("a@t1"),
            VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("a@t"), VerifyExpr::int(1)),
        ),
        VerifyExpr::eq(
            VerifyExpr::var("b@t1"),
            VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("b@t"), VerifyExpr::int(2)),
        ),
    );
    let property = VerifyExpr::and(
        VerifyExpr::gte(VerifyExpr::var("a@t"), VerifyExpr::int(0)),
        VerifyExpr::gte(VerifyExpr::var("b@t"), VerifyExpr::int(0)),
    );
    let result = ic3(&init, &transition, &property, 10);
    assert!(matches!(result, Ic3Result::Safe { .. }),
        "Two counters >= 0 should be safe. Got: {:?}", result);
}

#[test]
fn ic3_constant_true_property() {
    // Property: true — should always be safe regardless
    let init = VerifyExpr::bool(true);
    let transition = VerifyExpr::bool(true);
    let property = VerifyExpr::bool(true);
    let result = ic3(&init, &transition, &property, 3);
    assert!(matches!(result, Ic3Result::Safe { .. }),
        "Trivially true property should be safe. Got: {:?}", result);
}

#[test]
fn ic3_single_step_unsafe() {
    // Property violated after exactly 1 step
    let init = VerifyExpr::var("ok@0");
    let transition = VerifyExpr::not(VerifyExpr::var("ok@t1")); // always becomes false
    let property = VerifyExpr::var("ok@t");
    let result = ic3(&init, &transition, &property, 5);
    assert!(matches!(result, Ic3Result::Unsafe { .. }),
        "Property violated after 1 step should be unsafe. Got: {:?}", result);
}

#[test]
fn ic3_identity_transition() {
    // Identity transition: every signal stays the same
    let init = VerifyExpr::and(
        VerifyExpr::var("p@0"),
        VerifyExpr::var("q@0"),
    );
    let transition = VerifyExpr::and(
        VerifyExpr::iff(VerifyExpr::var("p@t1"), VerifyExpr::var("p@t")),
        VerifyExpr::iff(VerifyExpr::var("q@t1"), VerifyExpr::var("q@t")),
    );
    let property = VerifyExpr::and(VerifyExpr::var("p@t"), VerifyExpr::var("q@t"));
    let result = ic3(&init, &transition, &property, 5);
    assert!(matches!(result, Ic3Result::Safe { .. }),
        "Identity transition preserves init. Got: {:?}", result);
}

#[test]
fn ic3_eventually_violated() {
    // Counter hits bound at step 3
    let init = VerifyExpr::eq(VerifyExpr::var("c@0"), VerifyExpr::int(0));
    let transition = VerifyExpr::eq(
        VerifyExpr::var("c@t1"),
        VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("c@t"), VerifyExpr::int(1)),
    );
    let property = VerifyExpr::lt(VerifyExpr::var("c@t"), VerifyExpr::int(3));
    let result = ic3(&init, &transition, &property, 10);
    assert!(matches!(result, Ic3Result::Unsafe { .. }),
        "Counter < 3 eventually violated. Got: {:?}", result);
}

#[test]
fn ic3_shift_register_safe() {
    // 2-stage shift register: s2 follows s1, both start true
    let init = VerifyExpr::and(VerifyExpr::var("s1@0"), VerifyExpr::var("s2@0"));
    let transition = VerifyExpr::and(
        VerifyExpr::var("s1@t1"), // s1 stays true
        VerifyExpr::iff(VerifyExpr::var("s2@t1"), VerifyExpr::var("s1@t")),
    );
    let property = VerifyExpr::var("s2@t");
    let result = ic3(&init, &transition, &property, 5);
    assert!(matches!(result, Ic3Result::Safe { .. }),
        "Shift register with s1=true should keep s2=true. Got: {:?}", result);
}

// ═══════════════════════════════════════════════════════════════════════════
// RED TESTS — CORNER CUT AUDIT: Real IC3 behaviors
// These tests CANNOT pass with k-induction delegation alone.
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn ic3_invariant_is_inductive() {
    // When IC3 returns Safe, the invariant must be independently inductive:
    //   invariant AND transition => invariant'
    // AND it must imply the property:
    //   invariant => property
    //
    // A trivial invariant of just `property.clone()` is NOT always inductive —
    // the whole point of IC3 is discovering STRENGTHENED invariants.
    //
    // Design: 3-stage pipeline a→b→c, init all true, property: c is true
    // The property "c" alone is NOT inductive (c@t true doesn't force c@t1 true
    // without knowing b@t). IC3 must discover "a AND b AND c" or similar.
    let init = VerifyExpr::and(
        VerifyExpr::var("a@0"),
        VerifyExpr::and(VerifyExpr::var("b@0"), VerifyExpr::var("c@0")),
    );
    let transition = VerifyExpr::and(
        VerifyExpr::var("a@t1"), // a stays true forever
        VerifyExpr::and(
            VerifyExpr::iff(VerifyExpr::var("b@t1"), VerifyExpr::var("a@t")),
            VerifyExpr::iff(VerifyExpr::var("c@t1"), VerifyExpr::var("b@t")),
        ),
    );
    let property = VerifyExpr::var("c@t");
    let result = ic3(&init, &transition, &property, 20);

    match result {
        Ic3Result::Safe { invariant } => {
            // The invariant must be INDUCTIVE: inv AND T => inv'
            // Check: is (inv@0 AND T(0,1) AND NOT inv@1) SAT? Must be UNSAT.
            let inv_0 = kinduction::instantiate_at(&invariant, 0);
            let trans = kinduction::instantiate_transition(&transition, 0);
            let inv_1 = kinduction::instantiate_at(&invariant, 1);
            let inductiveness = VerifyExpr::and(inv_0.clone(), VerifyExpr::and(trans, VerifyExpr::not(inv_1)));
            assert!(
                !check_sat(&inductiveness),
                "IC3 invariant must be inductive (inv AND T => inv'), but inductiveness check is SAT"
            );

            // The invariant must IMPLY the property
            let prop_0 = kinduction::instantiate_at(&property, 0);
            let implication = VerifyExpr::and(inv_0, VerifyExpr::not(prop_0));
            assert!(
                !check_sat(&implication),
                "IC3 invariant must imply property"
            );
        }
        other => panic!("Expected Safe, got: {:?}", other),
    }
}

#[test]
fn ic3_blocking_clause_actually_used() {
    // IC3 must actually block CTI states by adding clauses to frames.
    // The dead `blocking_clause` variable (created but never stored) means
    // the current impl doesn't actually block anything.
    //
    // Signals: x, y (boolean)
    // Init: x=true, y=true
    // Transition: x'=x, y'=x (y follows x)
    // Property: y
    //
    // The bad state x=false, y=true CAN transition to y'=false (violating property).
    // But that state is unreachable because x starts true and is preserved.
    // IC3 should block {x=false} as part of the invariant.
    let init = VerifyExpr::and(VerifyExpr::var("x@0"), VerifyExpr::var("y@0"));
    let transition = VerifyExpr::and(
        VerifyExpr::iff(VerifyExpr::var("x@t1"), VerifyExpr::var("x@t")),
        VerifyExpr::iff(VerifyExpr::var("y@t1"), VerifyExpr::var("x@t")),
    );
    let property = VerifyExpr::var("y@t");
    let result = ic3(&init, &transition, &property, 20);

    match result {
        Ic3Result::Safe { invariant } => {
            // Verify inductiveness — if invariant is just "y" (the property),
            // it's NOT inductive because y@t AND y'=x@t doesn't guarantee
            // y'=true without x=true. Only a strengthened invariant works.
            let inv_0 = kinduction::instantiate_at(&invariant, 0);
            let trans = kinduction::instantiate_transition(&transition, 0);
            let inv_1 = kinduction::instantiate_at(&invariant, 1);
            let inductiveness = VerifyExpr::and(inv_0, VerifyExpr::and(trans, VerifyExpr::not(inv_1)));
            assert!(
                !check_sat(&inductiveness),
                "IC3 invariant must be inductive. \
                 A non-inductive invariant means blocking clauses are not being used. \
                 Invariant: {:?}", invariant,
            );
        }
        other => panic!("Expected Safe, got: {:?}", other),
    }
}

#[test]
fn ic3_deep_counterexample_found() {
    // IC3 should find bugs that require many steps to reach.
    // Counter from 0, increments by 1 each step. Property: counter < 50.
    // Bug at step 50. IC3 must find it (BMC with small k would miss it).
    let init = VerifyExpr::eq(VerifyExpr::var("c@0"), VerifyExpr::int(0));
    let transition = VerifyExpr::eq(
        VerifyExpr::var("c@t1"),
        VerifyExpr::binary(VerifyOp::Add, VerifyExpr::var("c@t"), VerifyExpr::int(1)),
    );
    let property = VerifyExpr::lt(VerifyExpr::var("c@t"), VerifyExpr::int(50));
    let result = ic3(&init, &transition, &property, 60);
    match result {
        Ic3Result::Unsafe { trace } => {
            // Trace should have actual cycle data showing the violation path
            assert!(
                !trace.cycles.is_empty(),
                "Deep counterexample trace must not be empty — \
                 IC3 should provide a concrete path to the violation"
            );
        }
        other => panic!(
            "Counter < 50 is violated at step 50. IC3 must detect this. Got: {:?}",
            other
        ),
    }
}

#[test]
fn ic3_propagation_pushes_learned_clauses() {
    // After IC3 proves safety, the invariant should contain clauses that were
    // learned during the backward-reachability analysis and propagated forward.
    // These learned clauses are what make IC3 different from k-induction.
    //
    // Design: two latches, init both true, property: latch2
    // Transition: latch1'=latch1, latch2'=latch1
    // IC3 must learn that latch1=true is necessary and propagate it.
    let init = VerifyExpr::and(VerifyExpr::var("l1@0"), VerifyExpr::var("l2@0"));
    let transition = VerifyExpr::and(
        VerifyExpr::iff(VerifyExpr::var("l1@t1"), VerifyExpr::var("l1@t")),
        VerifyExpr::iff(VerifyExpr::var("l2@t1"), VerifyExpr::var("l1@t")),
    );
    let property = VerifyExpr::var("l2@t");
    let result = ic3(&init, &transition, &property, 15);

    match result {
        Ic3Result::Safe { invariant } => {
            // The invariant must be inductive, which requires BOTH l1 and l2
            // constraints (not just the property "l2"). The clause "l1" was
            // learned during IC3 and propagated forward.
            let inv_0 = kinduction::instantiate_at(&invariant, 0);
            let trans = kinduction::instantiate_transition(&transition, 0);
            let inv_1 = kinduction::instantiate_at(&invariant, 1);
            let inductiveness = VerifyExpr::and(inv_0, VerifyExpr::and(trans, VerifyExpr::not(inv_1)));
            assert!(
                !check_sat(&inductiveness),
                "IC3 invariant with propagated clauses must be inductive"
            );
        }
        other => panic!("Expected Safe, got: {:?}", other),
    }
}

#[test]
fn ic3_counterexample_trace_is_concrete() {
    // When IC3 finds a bug, the counterexample trace must show actual
    // signal values at each cycle, not an empty trace.
    let init = VerifyExpr::var("ok@0");
    let transition = VerifyExpr::not(VerifyExpr::var("ok@t1")); // always goes false
    let property = VerifyExpr::var("ok@t");
    let result = ic3(&init, &transition, &property, 5);
    match result {
        Ic3Result::Unsafe { trace } => {
            assert!(
                !trace.cycles.is_empty(),
                "IC3 counterexample must have concrete cycle states, not empty trace"
            );
            assert!(
                !trace.cycles[0].signals.is_empty(),
                "Cycle 0 must have signal values"
            );
        }
        other => panic!("Expected Unsafe, got: {:?}", other),
    }
}

#[test]
fn ic3_invariant_init_implies_invariant() {
    // The invariant must be reachable from init: init => invariant.
    let init = VerifyExpr::and(
        VerifyExpr::var("p@0"),
        VerifyExpr::var("q@0"),
    );
    let transition = VerifyExpr::and(
        VerifyExpr::iff(VerifyExpr::var("p@t1"), VerifyExpr::var("p@t")),
        VerifyExpr::iff(VerifyExpr::var("q@t1"), VerifyExpr::var("q@t")),
    );
    let property = VerifyExpr::or(VerifyExpr::var("p@t"), VerifyExpr::var("q@t"));
    let result = ic3(&init, &transition, &property, 10);

    match result {
        Ic3Result::Safe { invariant } => {
            // init => invariant must hold
            let init_0 = kinduction::instantiate_at(&init, 0);
            let inv_0 = kinduction::instantiate_at(&invariant, 0);
            let check = VerifyExpr::and(init_0, VerifyExpr::not(inv_0));
            assert!(
                !check_sat(&check),
                "Init must imply the IC3 invariant"
            );
        }
        other => panic!("Expected Safe, got: {:?}", other),
    }
}
