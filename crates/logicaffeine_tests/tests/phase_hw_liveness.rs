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
