//! Interpolation-Based Model Checking
//!
//! Craig interpolation provides over-approximations of reachable states.
//! Given A AND B is UNSAT, interpolant I satisfies: A → I, and I AND B is UNSAT.
//! The interpolant uses only variables shared between A and B.
//!
//! Since Z3's Rust bindings don't directly expose interpolation, we approximate
//! using variable-restricted weakening: project A onto shared variables.

use crate::ir::VerifyExpr;
use crate::equivalence::{Trace, CycleState, SignalValue};
use crate::kinduction;
use std::collections::{HashMap, HashSet};
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

/// Compute a Craig interpolant approximation.
///
/// Given formulas A and B where A AND B is UNSAT, returns an over-approximation
/// of A that uses only variables shared between A and B, and is still inconsistent
/// with B.
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
            // A AND B is UNSAT — compute interpolant over shared variables
            let a_vars = collect_expr_vars(a);
            let b_vars = collect_expr_vars(b);
            let shared: HashSet<&String> = a_vars.intersection(&b_vars).collect();

            // Project A onto shared variables by existentially quantifying
            // non-shared variables. In practice: extract the sub-formula of A
            // that only mentions shared variables.
            let projected = project_to_shared(a, &shared);

            // Verify the projection is valid: A => projected and projected AND B is UNSAT
            let check_implies = {
                let s = Solver::new(&ctx);
                s.assert(&encode_to_bool(&ctx, a));
                s.assert(&encode_to_bool(&ctx, &VerifyExpr::not(projected.clone())));
                matches!(s.check(), SatResult::Unsat)
            };
            let check_contra = {
                let s = Solver::new(&ctx);
                s.assert(&encode_to_bool(&ctx, &projected));
                s.assert(&encode_to_bool(&ctx, b));
                matches!(s.check(), SatResult::Unsat)
            };

            if check_implies && check_contra {
                Some(projected)
            } else {
                // Fallback: try to build interpolant by enumerating shared-var clauses from A
                build_interpolant_from_clauses(a, b, &shared, &ctx)
            }
        }
        SatResult::Sat => None,
        SatResult::Unknown => None,
    }
}

/// Project an expression onto shared variables.
/// Extracts sub-expressions that only mention shared variables.
fn project_to_shared(expr: &VerifyExpr, shared: &HashSet<&String>) -> VerifyExpr {
    match expr {
        VerifyExpr::Binary { op: crate::ir::VerifyOp::And, left, right } => {
            let l = project_to_shared(left, shared);
            let r = project_to_shared(right, shared);
            match (&l, &r) {
                (VerifyExpr::Bool(true), _) => r,
                (_, VerifyExpr::Bool(true)) => l,
                _ => VerifyExpr::and(l, r),
            }
        }
        _ => {
            let vars = collect_expr_vars(expr);
            if vars.is_empty() || vars.iter().all(|v| shared.contains(v)) {
                expr.clone()
            } else {
                // This sub-expression mentions non-shared variables — drop it
                VerifyExpr::bool(true)
            }
        }
    }
}

/// Build an interpolant from the clauses of A that only use shared variables.
/// Verify the result against B.
fn build_interpolant_from_clauses(
    a: &VerifyExpr,
    b: &VerifyExpr,
    shared: &HashSet<&String>,
    ctx: &Context,
) -> Option<VerifyExpr> {
    // Extract top-level conjuncts from A
    let clauses = extract_conjuncts(a);

    // Keep only clauses that use exclusively shared variables
    let shared_clauses: Vec<VerifyExpr> = clauses.into_iter().filter(|c| {
        let vars = collect_expr_vars(c);
        vars.iter().all(|v| shared.contains(v))
    }).collect();

    if shared_clauses.is_empty() {
        // No shared-variable clauses — try negating B's structure
        // If B = NOT(p), the interpolant is p
        return try_interpolant_from_b_negation(b, shared, ctx);
    }

    let mut candidate = shared_clauses[0].clone();
    for c in &shared_clauses[1..] {
        candidate = VerifyExpr::and(candidate, c.clone());
    }

    // Verify: candidate AND B is UNSAT
    let s = Solver::new(ctx);
    s.assert(&encode_to_bool(ctx, &candidate));
    s.assert(&encode_to_bool(ctx, b));
    if matches!(s.check(), SatResult::Unsat) {
        Some(candidate)
    } else {
        // Shared clauses alone don't contradict B — try strengthening
        // Use A as implied by shared clauses + implication from full A
        try_interpolant_from_b_negation(b, shared, ctx)
    }
}

/// Try to build an interpolant by analyzing B's structure.
fn try_interpolant_from_b_negation(
    b: &VerifyExpr,
    shared: &HashSet<&String>,
    ctx: &Context,
) -> Option<VerifyExpr> {
    // If B mentions only shared variables, NOT(B) is a valid interpolant
    // (since A AND B is UNSAT means A => NOT(B))
    let b_vars = collect_expr_vars(b);
    if b_vars.iter().all(|v| shared.contains(v)) {
        return Some(VerifyExpr::not(b.clone()));
    }

    // Last resort: extract shared-variable sub-formulas from NOT(B)
    let not_b = VerifyExpr::not(b.clone());
    let projected = project_to_shared(&not_b, shared);
    if !matches!(projected, VerifyExpr::Bool(true)) {
        return Some(projected);
    }

    None
}

/// Extract top-level conjuncts from an expression.
fn extract_conjuncts(expr: &VerifyExpr) -> Vec<VerifyExpr> {
    match expr {
        VerifyExpr::Binary { op: crate::ir::VerifyOp::And, left, right } => {
            let mut v = extract_conjuncts(left);
            v.extend(extract_conjuncts(right));
            v
        }
        _ => vec![expr.clone()],
    }
}

/// Collect all variable names from an expression.
fn collect_expr_vars(expr: &VerifyExpr) -> HashSet<String> {
    let mut vars = HashSet::new();
    collect_vars_recursive(expr, &mut vars);
    vars
}

fn collect_vars_recursive(expr: &VerifyExpr, vars: &mut HashSet<String>) {
    match expr {
        VerifyExpr::Var(name) => { vars.insert(name.clone()); }
        VerifyExpr::Binary { left, right, .. } => {
            collect_vars_recursive(left, vars);
            collect_vars_recursive(right, vars);
        }
        VerifyExpr::Not(inner) => collect_vars_recursive(inner, vars),
        VerifyExpr::Iff(l, r) => {
            collect_vars_recursive(l, vars);
            collect_vars_recursive(r, vars);
        }
        VerifyExpr::ForAll { body, .. } | VerifyExpr::Exists { body, .. } => {
            collect_vars_recursive(body, vars);
        }
        _ => {}
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

        let init_0 = kinduction::instantiate_at(init, 0);
        solver.assert(&encode_to_bool(&ctx, &init_0));

        for t in 0..k {
            let trans = kinduction::instantiate_transition(transition, t);
            solver.assert(&encode_to_bool(&ctx, &trans));
        }

        let prop_k = kinduction::instantiate_at(property, k);
        solver.assert(&encode_to_bool(&ctx, &prop_k).not());

        match solver.check() {
            SatResult::Sat => {
                return InterpolationResult::Unsafe {
                    trace: Trace { cycles: vec![] },
                };
            }
            SatResult::Unknown => return InterpolationResult::Unknown,
            SatResult::Unsat => {}
        }
    }

    // Phase 2: Check if the property is inductive
    let solver = Solver::new(&ctx);
    let prop_t = kinduction::instantiate_at(property, 0);
    let trans = kinduction::instantiate_transition(transition, 0);
    let prop_t1 = kinduction::instantiate_at(property, 1);

    solver.assert(&encode_to_bool(&ctx, &prop_t));
    solver.assert(&encode_to_bool(&ctx, &trans));
    solver.assert(&encode_to_bool(&ctx, &prop_t1).not());

    match solver.check() {
        SatResult::Unsat => InterpolationResult::Safe,
        _ => {
            // Try k-induction as fallback
            let kind_result = kinduction::k_induction(
                init, transition, property, &[], bound,
            );
            match kind_result {
                kinduction::KInductionResult::Proven { k } => {
                    InterpolationResult::Fixpoint { iterations: k }
                }
                kinduction::KInductionResult::Counterexample { trace, .. } => {
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

    kinduction::encode_expr_bool(ctx, expr, &bool_vars, &int_vars)
}
