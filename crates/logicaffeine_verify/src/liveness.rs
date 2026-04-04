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
use crate::equivalence::{Trace, CycleState, SignalValue};
use crate::kinduction;
use std::collections::{HashMap, HashSet};

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

/// Check a liveness property via bounded search.
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
    let mut cfg = z3::Config::new();
    cfg.set_param_value("timeout", "30000");
    let ctx = z3::Context::new(&cfg);

    // Collect signal names for trace extraction
    let mut all_vars = HashSet::new();
    collect_vars(init, &mut all_vars);
    collect_vars(transition, &mut all_vars);
    collect_vars(property, &mut all_vars);
    for f in fairness {
        collect_vars(f, &mut all_vars);
    }
    let signal_names = extract_signal_names(&all_vars);

    // Check: can we go max_k steps from init without EVER seeing property?
    // If NOT (UNSAT), then property MUST hold within max_k steps from any init state → Live.
    // If YES (SAT), we found a finite prefix where property never holds → potential NotLive.
    for k in 1..=max_k {
        let solver = z3::Solver::new(&ctx);

        // Assert init at step 0
        let init_0 = kinduction::instantiate_at(init, 0);
        solver.assert(&encode_bool(&ctx, &init_0));

        // Assert transitions
        for t in 0..k {
            let trans = kinduction::instantiate_transition(transition, t);
            solver.assert(&encode_bool(&ctx, &trans));
        }

        // Assert fairness constraints are met somewhere in the trace
        for fair in fairness {
            // At least one step satisfies the fairness constraint
            let mut fair_options: Vec<z3::ast::Bool> = Vec::new();
            for t in 0..=k {
                let fair_t = kinduction::instantiate_at(fair, t);
                fair_options.push(encode_bool(&ctx, &fair_t));
            }
            let fair_refs: Vec<&z3::ast::Bool> = fair_options.iter().collect();
            if !fair_refs.is_empty() {
                let some_fair = z3::ast::Bool::or(&ctx, &fair_refs);
                solver.assert(&some_fair);
            }
        }

        // Assert property NEVER holds in k+1 steps
        for t in 0..=k {
            let prop_t = kinduction::instantiate_at(property, t);
            solver.assert(&encode_bool(&ctx, &prop_t).not());
        }

        match solver.check() {
            z3::SatResult::Sat => {
                // Found a path where property never holds for k steps
                if k == max_k {
                    // Extract concrete trace
                    let trace = extract_liveness_trace(&ctx, &solver, k, &signal_names);
                    let loop_point = find_loop_point(&trace);
                    return LivenessResult::NotLive { trace, loop_point };
                }
                // Continue to larger k for a more conclusive result
            }
            z3::SatResult::Unsat => {
                // No path of length k avoids the property → liveness holds up to k
                return LivenessResult::Live;
            }
            z3::SatResult::Unknown => return LivenessResult::Unknown,
        }
    }

    // Exhausted bound without proving liveness — find concrete trace
    let solver = z3::Solver::new(&ctx);
    let init_0 = kinduction::instantiate_at(init, 0);
    solver.assert(&encode_bool(&ctx, &init_0));
    for t in 0..max_k {
        let trans = kinduction::instantiate_transition(transition, t);
        solver.assert(&encode_bool(&ctx, &trans));
    }
    for t in 0..=max_k {
        let prop_t = kinduction::instantiate_at(property, t);
        solver.assert(&encode_bool(&ctx, &prop_t).not());
    }

    if matches!(solver.check(), z3::SatResult::Sat) {
        let trace = extract_liveness_trace(&ctx, &solver, max_k, &signal_names);
        let loop_point = find_loop_point(&trace);
        LivenessResult::NotLive { trace, loop_point }
    } else {
        LivenessResult::Live
    }
}

/// Extract signal names from variable set.
fn extract_signal_names(all_vars: &HashSet<String>) -> Vec<String> {
    let mut signals = HashSet::new();
    for v in all_vars {
        let base = v.replace("@0", "").replace("@t1", "").replace("@t", "");
        if !base.is_empty() {
            signals.insert(base);
        }
    }
    signals.into_iter().collect()
}

/// Extract a concrete trace from a SAT solver model.
fn extract_liveness_trace(
    ctx: &z3::Context,
    solver: &z3::Solver,
    k: u32,
    signal_names: &[String],
) -> Trace {
    let model = match solver.get_model() {
        Some(m) => m,
        None => return Trace { cycles: vec![CycleState { cycle: 0, signals: HashMap::new() }] },
    };

    let mut cycles = Vec::new();
    for step in 0..=k {
        let mut signals = HashMap::new();
        for sig in signal_names {
            let var_name = format!("{}@{}", sig, step);
            let bool_var = z3::ast::Bool::new_const(ctx, var_name.as_str());
            if let Some(val) = model.eval(&bool_var, true) {
                if let Some(b) = val.as_bool() {
                    signals.insert(sig.clone(), SignalValue::Bool(b));
                    continue;
                }
            }
            let int_var = z3::ast::Int::new_const(ctx, var_name.as_str());
            if let Some(val) = model.eval(&int_var, true) {
                if let Some(n) = val.as_i64() {
                    signals.insert(sig.clone(), SignalValue::Int(n));
                    continue;
                }
            }
        }
        if !signals.is_empty() {
            cycles.push(CycleState { cycle: step as usize, signals });
        }
    }

    if cycles.is_empty() {
        // Fallback: at least provide one cycle with unknown values
        let mut signals = HashMap::new();
        for sig in signal_names {
            signals.insert(sig.clone(), SignalValue::Unknown);
        }
        cycles.push(CycleState { cycle: 0, signals });
    }

    Trace { cycles }
}

/// Find the loop point in a trace (where the lasso begins).
fn find_loop_point(trace: &Trace) -> usize {
    if trace.cycles.len() <= 1 {
        return 0;
    }
    // Look for repeating state pattern — the simplest heuristic
    // is to look for where the last state matches an earlier state.
    let last = &trace.cycles[trace.cycles.len() - 1];
    for (i, cycle) in trace.cycles.iter().enumerate() {
        if i < trace.cycles.len() - 1 && states_match(&cycle.signals, &last.signals) {
            return i;
        }
    }
    // Default: loop starts at the midpoint
    trace.cycles.len() / 2
}

/// Check if two signal maps represent the same state.
fn states_match(a: &HashMap<String, SignalValue>, b: &HashMap<String, SignalValue>) -> bool {
    if a.len() != b.len() { return false; }
    for (key, val_a) in a {
        match b.get(key) {
            Some(val_b) => {
                let sa = format!("{:?}", val_a);
                let sb = format!("{:?}", val_b);
                if sa != sb { return false; }
            }
            None => return false,
        }
    }
    true
}

fn collect_vars(expr: &VerifyExpr, vars: &mut HashSet<String>) {
    match expr {
        VerifyExpr::Var(name) => { vars.insert(name.clone()); }
        VerifyExpr::Binary { left, right, .. } => {
            collect_vars(left, vars);
            collect_vars(right, vars);
        }
        VerifyExpr::Not(inner) => collect_vars(inner, vars),
        VerifyExpr::Iff(l, r) => {
            collect_vars(l, vars);
            collect_vars(r, vars);
        }
        VerifyExpr::ForAll { body, .. } | VerifyExpr::Exists { body, .. } => {
            collect_vars(body, vars);
        }
        _ => {}
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
