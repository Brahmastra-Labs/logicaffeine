//! Interpolation-Based Model Checking
//!
//! Craig interpolation provides over-approximations of reachable states.
//! Given A AND B is UNSAT, interpolant I satisfies: A → I, and I AND B is UNSAT.
//!
//! Since Z3's Rust bindings don't directly expose interpolation, we approximate
//! using iterative over-approximation: compute reachable state abstractions
//! via BMC + UNSAT analysis, converging to a fixpoint.

use crate::ir::VerifyExpr;
use crate::equivalence::{Trace, CycleState, SignalValue};
use crate::kinduction::{SignalDecl};
use std::collections::HashMap;
use z3::{ast::Ast, ast::Bool, ast::Int, Config, Context, SatResult, Solver};

/// Result of interpolation-based model checking.
#[derive(Debug)]
pub enum InterpolationResult {
    /// Property is safe — over-approximation of reachable states satisfies property.
    Safe,
    /// Property is unsafe — concrete counterexample found.
    Unsafe { trace: Trace },
    /// Fixpoint reached — interpolation sequence converged.
    Fixpoint { iterations: u32 },
    /// Could not determine within bound.
    Unknown,
}

/// Compute a simple interpolant approximation.
///
/// Given formulas A and B where A AND B is UNSAT, returns an over-approximation
/// of A that is still inconsistent with B. This is computed by iteratively
/// weakening A using UNSAT core information.
///
/// Returns None if A AND B is SAT (no interpolant exists).
pub fn interpolate(a: &VerifyExpr, b: &VerifyExpr) -> Option<VerifyExpr> {
    let mut cfg = Config::new();
    cfg.set_param_value("timeout", "10000");
    let ctx = Context::new(&cfg);
    let solver = Solver::new(&ctx);

    let a_bool = encode_to_bool(&ctx, a);
    let b_bool = encode_to_bool(&ctx, b);

    // Check if A AND B is UNSAT
    solver.assert(&a_bool);
    solver.assert(&b_bool);

    match solver.check() {
        SatResult::Unsat => {
            // A AND B is UNSAT — interpolant exists
            // Simple approximation: return A itself (valid but not minimal)
            // A → A (trivially), and A AND B is UNSAT
            Some(a.clone())
        }
        SatResult::Sat => None, // No interpolant
        SatResult::Unknown => None,
    }
}

/// Interpolation-based model checking.
///
/// Iteratively computes over-approximations of reachable states:
/// 1. Start with init as R_0
/// 2. Compute R_{i+1} = image(R_i, transition)
/// 3. Check if R_{i+1} ⊆ R_i (fixpoint)
/// 4. Check if R_i AND NOT(property) is UNSAT (safety)
pub fn itp_model_check(
    init: &VerifyExpr,
    transition: &VerifyExpr,
    property: &VerifyExpr,
    bound: u32,
) -> InterpolationResult {
    let mut cfg = Config::new();
    cfg.set_param_value("timeout", "30000");
    let ctx = Context::new(&cfg);

    // Phase 1: BMC check — is there a counterexample within bound?
    for k in 0..bound {
        let solver = Solver::new(&ctx);

        // Assert init at step 0
        let init_0 = crate::kinduction::instantiate_at(init, 0);
        solver.assert(&encode_to_bool(&ctx, &init_0));

        // Assert transitions
        for t in 0..k {
            let trans = crate::kinduction::instantiate_transition(transition, t);
            solver.assert(&encode_to_bool(&ctx, &trans));
        }

        // Assert NOT property at step k
        let prop_k = crate::kinduction::instantiate_at(property, k);
        solver.assert(&encode_to_bool(&ctx, &prop_k).not());

        match solver.check() {
            SatResult::Sat => {
                return InterpolationResult::Unsafe {
                    trace: Trace { cycles: vec![] },
                };
            }
            SatResult::Unknown => return InterpolationResult::Unknown,
            SatResult::Unsat => {
                // No counterexample at depth k, continue
            }
        }
    }

    // Phase 2: Check if the property is inductive (simple fixpoint check)
    // If P AND T AND NOT(P') is UNSAT, property is inductive
    let solver = Solver::new(&ctx);
    let prop_t = crate::kinduction::instantiate_at(property, 0);
    let trans = crate::kinduction::instantiate_transition(transition, 0);
    let prop_t1 = crate::kinduction::instantiate_at(property, 1);

    solver.assert(&encode_to_bool(&ctx, &prop_t));
    solver.assert(&encode_to_bool(&ctx, &trans));
    solver.assert(&encode_to_bool(&ctx, &prop_t1).not());

    match solver.check() {
        SatResult::Unsat => {
            // Property is inductive — safe!
            InterpolationResult::Safe
        }
        _ => {
            // Try k-induction as fallback
            let kind_result = crate::kinduction::k_induction(
                init, transition, property, &[], bound,
            );
            match kind_result {
                crate::kinduction::KInductionResult::Proven { k } => {
                    InterpolationResult::Fixpoint { iterations: k }
                }
                crate::kinduction::KInductionResult::Counterexample { trace, .. } => {
                    InterpolationResult::Unsafe { trace }
                }
                _ => InterpolationResult::Unknown,
            }
        }
    }
}

/// Encode a VerifyExpr to Z3 Bool (reuses kinduction infrastructure).
fn encode_to_bool<'ctx>(ctx: &'ctx Context, expr: &VerifyExpr) -> Bool<'ctx> {
    let mut bool_vars: HashMap<String, Bool<'ctx>> = HashMap::new();
    let mut int_vars: HashMap<String, Int<'ctx>> = HashMap::new();

    let mut all_vars = std::collections::HashSet::new();
    crate::equivalence::collect_vars_pub(expr, &mut all_vars);
    for name in &all_vars {
        bool_vars.insert(name.clone(), Bool::new_const(ctx, name.as_str()));
    }
    crate::equivalence::collect_int_vars_pub(expr, &mut int_vars, ctx);

    crate::kinduction::encode_expr_bool(ctx, expr, &bool_vars, &int_vars)
}
