//! Security Property Verification — Non-Interference
//!
//! Hardware non-interference: two executions differing only in secret inputs
//! must produce identical public outputs. Encoded as:
//! (init_1 == init_2 on public) AND (secret_1 != secret_2) AND T_1 AND T_2
//! AND (public_out_1 != public_out_2) is UNSAT.

use crate::ir::VerifyExpr;
use crate::kinduction;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub enum SecurityLabel { Public, Secret }

#[derive(Debug, Clone)]
pub struct TaintedSignal {
    pub name: String,
    pub label: SecurityLabel,
}

#[derive(Debug)]
pub enum SecurityResult {
    NonInterference,
    InformationLeak { path: Vec<String> },
    TimingLeak { condition: VerifyExpr },
    Unknown,
}

/// Check non-interference: public outputs independent of secret inputs.
pub fn check_non_interference(
    transition: &VerifyExpr,
    signals: &[TaintedSignal],
) -> SecurityResult {
    let mut cfg = z3::Config::new();
    cfg.set_param_value("timeout", "30000");
    let ctx = z3::Context::new(&cfg);

    let public_inputs: Vec<&TaintedSignal> = signals.iter()
        .filter(|s| s.label == SecurityLabel::Public)
        .collect();
    let secret_inputs: Vec<&TaintedSignal> = signals.iter()
        .filter(|s| s.label == SecurityLabel::Secret)
        .collect();

    if secret_inputs.is_empty() {
        return SecurityResult::NonInterference;
    }

    // Create two copies of the system: copy1 and copy2
    // Public inputs are the same, secret inputs differ
    let solver = z3::Solver::new(&ctx);

    // Assert public inputs are the same in both copies
    for sig in &public_inputs {
        let v1 = format!("{}@0_copy1", sig.name);
        let v2 = format!("{}@0_copy2", sig.name);
        let eq = VerifyExpr::iff(VerifyExpr::var(&v1), VerifyExpr::var(&v2));
        solver.assert(&encode_bool(&ctx, &eq));
    }

    // Assert at least one secret input differs
    if !secret_inputs.is_empty() {
        let first_secret = &secret_inputs[0];
        let v1 = format!("{}@0_copy1", first_secret.name);
        let v2 = format!("{}@0_copy2", first_secret.name);
        let neq = VerifyExpr::not(VerifyExpr::iff(VerifyExpr::var(&v1), VerifyExpr::var(&v2)));
        solver.assert(&encode_bool(&ctx, &neq));
    }

    // Assert transitions for both copies
    let t1 = rename_copy(transition, "copy1");
    let t2 = rename_copy(transition, "copy2");
    solver.assert(&encode_bool(&ctx, &t1));
    solver.assert(&encode_bool(&ctx, &t2));

    // Check if public outputs differ
    // We check all variables that appear in the transition outputs
    let mut output_diffs = Vec::new();
    let mut transition_vars = std::collections::HashSet::new();
    crate::equivalence::collect_vars_pub(&t1, &mut transition_vars);
    for sig in &public_inputs {
        // Check at step 0 (always — constrained by public equality)
        let o1_0 = format!("{}@0_copy1", sig.name);
        let o2_0 = format!("{}@0_copy2", sig.name);
        // Only check step-1 if the transition actually mentions this signal at step 1
        let step1_name = format!("{}@1_copy1", sig.name);
        if transition_vars.contains(&step1_name) {
            let o1_1 = format!("{}@1_copy1", sig.name);
            let o2_1 = format!("{}@1_copy2", sig.name);
            let diff_1 = VerifyExpr::not(VerifyExpr::iff(VerifyExpr::var(&o1_1), VerifyExpr::var(&o2_1)));
            output_diffs.push(diff_1);
        }
    }

    if output_diffs.is_empty() {
        return SecurityResult::NonInterference;
    }

    // Check each output diff individually — ANY SAT means information leak
    for diff in &output_diffs {
        let mut check_solver = z3::Solver::new(&ctx);

        // Re-assert all base constraints
        for sig in &public_inputs {
            let v1 = format!("{}@0_copy1", sig.name);
            let v2 = format!("{}@0_copy2", sig.name);
            let eq = VerifyExpr::iff(VerifyExpr::var(&v1), VerifyExpr::var(&v2));
            check_solver.assert(&encode_bool(&ctx, &eq));
        }
        if !secret_inputs.is_empty() {
            let first_secret = &secret_inputs[0];
            let v1 = format!("{}@0_copy1", first_secret.name);
            let v2 = format!("{}@0_copy2", first_secret.name);
            let neq = VerifyExpr::not(VerifyExpr::iff(VerifyExpr::var(&v1), VerifyExpr::var(&v2)));
            check_solver.assert(&encode_bool(&ctx, &neq));
        }
        check_solver.assert(&encode_bool(&ctx, &t1));
        check_solver.assert(&encode_bool(&ctx, &t2));
        check_solver.assert(&encode_bool(&ctx, diff));

        match check_solver.check() {
            z3::SatResult::Sat => {
                let path = secret_inputs.iter().map(|s| s.name.clone()).collect();
                return SecurityResult::InformationLeak { path };
            }
            z3::SatResult::Unknown => return SecurityResult::Unknown,
            z3::SatResult::Unsat => {} // This output is safe, check next
        }
    }

    SecurityResult::NonInterference
}

fn rename_copy(expr: &VerifyExpr, suffix: &str) -> VerifyExpr {
    match expr {
        VerifyExpr::Var(name) => {
            // Insert suffix before @ if present: "sig@0" → "sig@0_copy1"
            VerifyExpr::Var(format!("{}_{}", name, suffix))
        }
        VerifyExpr::Binary { op, left, right } => VerifyExpr::binary(
            *op, rename_copy(left, suffix), rename_copy(right, suffix),
        ),
        VerifyExpr::Not(inner) => VerifyExpr::not(rename_copy(inner, suffix)),
        VerifyExpr::Bool(b) => VerifyExpr::Bool(*b),
        VerifyExpr::Int(n) => VerifyExpr::Int(*n),
        VerifyExpr::Iff(l, r) => VerifyExpr::iff(rename_copy(l, suffix), rename_copy(r, suffix)),
        _ => expr.clone(),
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
