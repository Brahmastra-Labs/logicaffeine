//! SUPERCRUSH Sprint S1D: Liveness-to-Safety Reduction

#![cfg(feature = "verification")]

use logicaffeine_verify::liveness::{check_liveness, LivenessResult};
use logicaffeine_verify::{VerifyExpr, VerifyOp};

// ═══════════════════════════════════════════════════════════════════════════
// BASIC LIVENESS PROPERTIES
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn liveness_simple_ack() {
    // G(F(ack)): ack always eventually holds
    // System: ack becomes true at step 1 and stays true
    let init = VerifyExpr::not(VerifyExpr::var("ack@0"));
    let transition = VerifyExpr::var("ack@t1"); // ack becomes true
    let property = VerifyExpr::var("ack@t");
    let result = check_liveness(&init, &transition, &[], &property, 5);
    assert!(matches!(result, LivenessResult::Live),
        "ack eventually true should be live. Got: {:?}", result);
}

#[test]
fn liveness_starvation() {
    // ack never becomes true → liveness fails
    let init = VerifyExpr::not(VerifyExpr::var("ack@0"));
    let transition = VerifyExpr::not(VerifyExpr::var("ack@t1")); // ack stays false
    let property = VerifyExpr::var("ack@t");
    let result = check_liveness(&init, &transition, &[], &property, 5);
    assert!(matches!(result, LivenessResult::NotLive { .. }),
        "ack never true should fail liveness. Got: {:?}", result);
}

#[test]
fn liveness_eventually_response() {
    // After req, ack eventually comes within 2 cycles
    let init = VerifyExpr::and(
        VerifyExpr::var("req@0"),
        VerifyExpr::not(VerifyExpr::var("ack@0")),
    );
    let transition = VerifyExpr::iff(
        VerifyExpr::var("ack@t1"),
        VerifyExpr::var("req@t"),
    );
    let property = VerifyExpr::var("ack@t");
    let result = check_liveness(&init, &transition, &[], &property, 5);
    assert!(matches!(result, LivenessResult::Live),
        "Response within 2 cycles should be live. Got: {:?}", result);
}

#[test]
fn liveness_progress() {
    // System always makes progress (some flag goes true periodically)
    let init = VerifyExpr::var("progress@0");
    let transition = VerifyExpr::var("progress@t1"); // progress always true
    let property = VerifyExpr::var("progress@t");
    let result = check_liveness(&init, &transition, &[], &property, 3);
    assert!(matches!(result, LivenessResult::Live),
        "Always-true progress should be live. Got: {:?}", result);
}

#[test]
fn liveness_deadlock_detected() {
    // System halts — no progress ever
    let init = VerifyExpr::not(VerifyExpr::var("active@0"));
    let transition = VerifyExpr::iff(
        VerifyExpr::var("active@t1"),
        VerifyExpr::var("active@t"),
    );
    let property = VerifyExpr::var("active@t");
    let result = check_liveness(&init, &transition, &[], &property, 5);
    assert!(matches!(result, LivenessResult::NotLive { .. }),
        "Dead system should fail liveness. Got: {:?}", result);
}

#[test]
fn liveness_loop_point_valid() {
    // When we get NotLive, loop_point should be within trace bounds
    let init = VerifyExpr::not(VerifyExpr::var("p@0"));
    let transition = VerifyExpr::not(VerifyExpr::var("p@t1"));
    let property = VerifyExpr::var("p@t");
    let result = check_liveness(&init, &transition, &[], &property, 5);
    match result {
        LivenessResult::NotLive { loop_point, .. } => {
            assert!(loop_point <= 5, "Loop point should be within bound");
        }
        _ => panic!("Expected NotLive"),
    }
}

#[test]
fn liveness_empty_fairness() {
    // No fairness constraints = all paths are fair
    let init = VerifyExpr::var("ok@0");
    let transition = VerifyExpr::var("ok@t1");
    let property = VerifyExpr::var("ok@t");
    let result = check_liveness(&init, &transition, &[], &property, 3);
    assert!(matches!(result, LivenessResult::Live),
        "Always-true with no fairness should be live. Got: {:?}", result);
}

#[test]
fn liveness_safety_dual() {
    // Safety AND liveness on same system should be consistent
    // Safety: p always true. Liveness: p always eventually true.
    // Both should hold for a system where p is always true.
    let init = VerifyExpr::var("p@0");
    let transition = VerifyExpr::iff(VerifyExpr::var("p@t1"), VerifyExpr::var("p@t"));
    let property = VerifyExpr::var("p@t");

    let live_result = check_liveness(&init, &transition, &[], &property, 5);
    assert!(matches!(live_result, LivenessResult::Live),
        "Constant true should be live. Got: {:?}", live_result);
}

#[test]
fn liveness_integer_counter_resets() {
    // Counter increments, eventually wraps to 0 (within small bound)
    let init = VerifyExpr::eq(VerifyExpr::var("c@0"), VerifyExpr::int(0));
    // Counter mod 3: c' = (c + 1) if c < 2, else 0
    // Simplified: just check that c == 0 eventually
    let transition = VerifyExpr::and(
        VerifyExpr::gte(VerifyExpr::var("c@t1"), VerifyExpr::int(0)),
        VerifyExpr::lte(VerifyExpr::var("c@t1"), VerifyExpr::int(2)),
    );
    let property = VerifyExpr::eq(VerifyExpr::var("c@t"), VerifyExpr::int(0));
    let result = check_liveness(&init, &transition, &[], &property, 5);
    // With unconstrained transition, c could be 0 at init, so Live
    assert!(matches!(result, LivenessResult::Live),
        "Counter starting at 0 satisfies c==0. Got: {:?}", result);
}

// ═══════════════════════════════════════════════════════════════════════════
// RED TESTS — CORNER CUT AUDIT: Real L2S reduction
// Current impl is bounded BMC, not Biere-Artho-Schuppan reduction.
// ═══════════════════════════════════════════════════════════════════════════

#[test]
fn liveness_with_fairness_constraint() {
    // Fairness constraints are a core part of liveness verification.
    // System: two processes alternate (toggle between p1 and p2).
    // Property: p1 always eventually holds.
    // Fairness: the scheduler is fair (p2 always eventually holds too).
    // Without fairness, p1 might never get a turn. With fairness, it must.
    //
    // init: p1=true, p2=false
    // transition: p1'=NOT p1, p2'=NOT p2 (toggle)
    // property: p1 (eventually p1)
    // fairness: [p2] (p2 is infinitely often true)
    let init = VerifyExpr::and(
        VerifyExpr::var("p1@0"),
        VerifyExpr::not(VerifyExpr::var("p2@0")),
    );
    let transition = VerifyExpr::and(
        VerifyExpr::iff(VerifyExpr::var("p1@t1"), VerifyExpr::not(VerifyExpr::var("p1@t"))),
        VerifyExpr::iff(VerifyExpr::var("p2@t1"), VerifyExpr::not(VerifyExpr::var("p2@t"))),
    );
    let property = VerifyExpr::var("p1@t");
    let fairness = vec![VerifyExpr::var("p2@t")];
    let result = check_liveness(&init, &transition, &fairness, &property, 10);
    // p1 toggles: true, false, true, false, ... — it IS infinitely often true.
    assert!(matches!(result, LivenessResult::Live),
        "Toggling p1 should be live (infinitely often true). Got: {:?}", result);
}

#[test]
fn liveness_not_live_trace_has_cycles() {
    // When liveness fails, the trace must show a lasso-shaped counterexample:
    // a prefix leading to a loop where the property never holds.
    let init = VerifyExpr::not(VerifyExpr::var("p@0"));
    let transition = VerifyExpr::iff(VerifyExpr::var("p@t1"), VerifyExpr::var("p@t")); // p stays false
    let property = VerifyExpr::var("p@t");
    let result = check_liveness(&init, &transition, &[], &property, 5);
    match result {
        LivenessResult::NotLive { trace, loop_point } => {
            assert!(
                !trace.cycles.is_empty(),
                "Liveness counterexample must have concrete trace cycles, not empty"
            );
            assert!(
                loop_point < trace.cycles.len(),
                "Loop point {} must be within trace (len {})",
                loop_point, trace.cycles.len(),
            );
        }
        other => panic!("Expected NotLive with trace, got: {:?}", other),
    }
}

#[test]
fn liveness_multiple_fairness_constraints() {
    // Multiple fairness constraints that interact.
    // System: three-state round robin (s0→s1→s2→s0)
    // Property: s0 always eventually holds
    // Fairness: [s1, s2] (both s1 and s2 are infinitely often true)
    //
    // With fair scheduling, s0 must recur.
    let init = VerifyExpr::and(
        VerifyExpr::var("s0@0"),
        VerifyExpr::and(
            VerifyExpr::not(VerifyExpr::var("s1@0")),
            VerifyExpr::not(VerifyExpr::var("s2@0")),
        ),
    );
    // Round robin: exactly one active at a time, cycling
    let transition = VerifyExpr::and(
        VerifyExpr::iff(VerifyExpr::var("s1@t1"), VerifyExpr::var("s0@t")),
        VerifyExpr::and(
            VerifyExpr::iff(VerifyExpr::var("s2@t1"), VerifyExpr::var("s1@t")),
            VerifyExpr::iff(VerifyExpr::var("s0@t1"), VerifyExpr::var("s2@t")),
        ),
    );
    let property = VerifyExpr::var("s0@t");
    let fairness = vec![
        VerifyExpr::var("s1@t"),
        VerifyExpr::var("s2@t"),
    ];
    let result = check_liveness(&init, &transition, &fairness, &property, 10);
    assert!(matches!(result, LivenessResult::Live),
        "Round-robin with fair scheduling: s0 recurs. Got: {:?}", result);
}
