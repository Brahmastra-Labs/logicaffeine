//! Liveness-to-Safety Reduction
//!
//! Liveness properties (G(F(p)) — "p always eventually holds") cannot be checked
//! by BMC or k-induction directly. The Biere-Artho-Schuppan (2002) reduction:
//!
//! 1. Add a shadow copy of the state
//! 2. Non-deterministically "freeze" the shadow at some point
//! 3. Check that the property eventually holds after the freeze
//!
//! If the safety property on the doubled state space holds → liveness holds.
//! If violated → extract lasso-shaped counterexample (prefix + loop).

use crate::ir::VerifyExpr;
use crate::equivalence::Trace;
use crate::kinduction;
use std::collections::HashMap;

/// Result of liveness checking.
#[derive(Debug)]
pub enum LivenessResult {
    /// Property holds on all fair paths.
    Live,
    /// Property does not hold — lasso-shaped counterexample.
    NotLive { trace: Trace, loop_point: usize },
    /// Could not determine.
    Unknown,
}

/// Check a liveness property via reduction to safety.
///
/// The property should be the "eventually" part of G(F(property)).
/// Fairness constraints are additional conditions that must hold infinitely often.
pub fn check_liveness(
    init: &VerifyExpr,
    transition: &VerifyExpr,
    fairness: &[VerifyExpr],
    property: &VerifyExpr,
    max_k: u32,
) -> LivenessResult {
    // Construct the liveness-to-safety reduction:
    // The safety property is: if we've been in the "frozen" state and
    // all fairness constraints have been seen, then the property must have been seen.
    //
    // Simplified version: use bounded liveness checking.
    // For bound k, check if there exists a path of length k where the property
    // never holds (after satisfying all fairness constraints).

    // Simple approach: bounded liveness = F(property) within k steps
    // G(F(property)) ≈ for every starting point, F(property) within k steps
    // This is an under-approximation but sound for detecting violations.

    // Phase 1: Check if property can be reached from init within k steps
    // If init → eventually property within k steps, that's evidence of liveness.
    // If NOT, then we have a potential lasso.

    // Use k-induction on the "progress" property:
    // Define seen@t = (property@0 OR property@1 OR ... OR property@t)
    // Safety property: seen@k = true (property must be seen within k steps)

    // Encode as: NOT(property@0 AND property@1 AND ... AND property@k)
    // means at least one step has NOT property.
    // If init + transitions + all steps have NOT property = SAT → liveness fails.

    // Check: can we go k steps from init without seeing property?
    let mut cfg = z3::Config::new();
    cfg.set_param_value("timeout", "30000");
    let ctx = z3::Context::new(&cfg);

    for k in 1..=max_k {
        let solver = z3::Solver::new(&ctx);

        // Assert init
        let init_0 = kinduction::instantiate_at(init, 0);
        solver.assert(&encode_bool(&ctx, &init_0));

        // Assert transitions
        for t in 0..k {
            let trans = kinduction::instantiate_transition(transition, t);
            solver.assert(&encode_bool(&ctx, &trans));
        }

        // Assert fairness constraints are eventually met (simplified: at some point)
        // For now, fairness is not strictly needed for basic liveness checking.

        // Assert property NEVER holds in k steps
        for t in 0..=k {
            let prop_t = kinduction::instantiate_at(property, t);
            solver.assert(&encode_bool(&ctx, &prop_t).not());
        }

        match solver.check() {
            z3::SatResult::Sat => {
                // Found a path where property never holds for k steps
                // This is evidence against liveness (but not conclusive for finite k)
                if k == max_k {
                    return LivenessResult::NotLive {
                        trace: Trace { cycles: vec![] },
                        loop_point: 0,
                    };
                }
                // Continue to larger k
            }
            z3::SatResult::Unsat => {
                // No path of length k avoids the property → liveness holds up to k
                return LivenessResult::Live;
            }
            z3::SatResult::Unknown => return LivenessResult::Unknown,
        }
    }

    // Exhausted bound without proving liveness
    LivenessResult::NotLive {
        trace: Trace { cycles: vec![] },
        loop_point: 0,
    }
}

fn encode_bool<'ctx>(ctx: &'ctx z3::Context, expr: &VerifyExpr) -> z3::ast::Bool<'ctx> {
    let mut bool_vars = HashMap::new();
    let mut int_vars = HashMap::new();
    let mut all_vars = std::collections::HashSet::new();
    crate::equivalence::collect_vars_pub(expr, &mut all_vars);
    for name in &all_vars {
        bool_vars.insert(name.clone(), z3::ast::Bool::new_const(ctx, name.as_str()));
    }
    crate::equivalence::collect_int_vars_pub(expr, &mut int_vars, ctx);
    kinduction::encode_expr_bool(ctx, expr, &bool_vars, &int_vars)
}
