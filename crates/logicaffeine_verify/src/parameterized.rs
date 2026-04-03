//! Parameterized Verification
//!
//! Verify properties for any bus width N, any FIFO depth D, etc.
//! Uses Z3 quantifiers: forall N:Int. N > 0 -> property(N).

use crate::ir::{VerifyExpr, VerifyType};
use crate::equivalence::Trace;
use std::collections::HashMap;

#[derive(Debug)]
pub enum ParameterizedResult {
    UniversallyValid,
    ValidUpTo(u64),
    Counterexample { param_value: u64, trace: Trace },
    Unknown,
}

/// Verify a parameterized property.
///
/// Uses Z3 quantifiers to check: forall param. constraint(param) -> property(param).
/// Falls back to bounded enumeration if quantifier reasoning times out.
pub fn verify_parameterized(
    property: &VerifyExpr,
    parameter: &str,
    param_type: VerifyType,
    constraint: Option<&VerifyExpr>,
) -> ParameterizedResult {
    let mut cfg = z3::Config::new();
    cfg.set_param_value("timeout", "10000");
    let ctx = z3::Context::new(&cfg);

    // Build: forall param. constraint(param) -> property(param)
    let body = if let Some(c) = constraint {
        VerifyExpr::implies(c.clone(), property.clone())
    } else {
        property.clone()
    };

    let quantified = VerifyExpr::forall(
        vec![(parameter.to_string(), param_type.clone())],
        body,
    );

    // Check: NOT(quantified) is UNSAT means universally valid
    let negated = VerifyExpr::not(quantified);
    let solver = z3::Solver::new(&ctx);
    let encoded = encode_bool(&ctx, &negated);
    solver.assert(&encoded);

    match solver.check() {
        z3::SatResult::Unsat => ParameterizedResult::UniversallyValid,
        z3::SatResult::Sat => {
            // Try bounded enumeration to find specific counterexample
            for val in 0..=100u64 {
                let specialized = substitute_param(property, parameter, val as i64);
                let neg_spec = VerifyExpr::not(specialized);
                let solver2 = z3::Solver::new(&ctx);
                solver2.assert(&encode_bool(&ctx, &neg_spec));
                if matches!(solver2.check(), z3::SatResult::Sat) {
                    return ParameterizedResult::Counterexample {
                        param_value: val,
                        trace: Trace { cycles: vec![] },
                    };
                }
            }
            ParameterizedResult::ValidUpTo(100)
        }
        z3::SatResult::Unknown => {
            // Fall back to bounded enumeration
            for val in 0..=20u64 {
                let specialized = substitute_param(property, parameter, val as i64);
                let neg_spec = VerifyExpr::not(specialized);
                let solver2 = z3::Solver::new(&ctx);
                solver2.assert(&encode_bool(&ctx, &neg_spec));
                if matches!(solver2.check(), z3::SatResult::Sat) {
                    return ParameterizedResult::Counterexample {
                        param_value: val,
                        trace: Trace { cycles: vec![] },
                    };
                }
            }
            ParameterizedResult::ValidUpTo(20)
        }
    }
}

fn substitute_param(expr: &VerifyExpr, param: &str, value: i64) -> VerifyExpr {
    match expr {
        VerifyExpr::Var(name) if name == param => VerifyExpr::Int(value),
        VerifyExpr::Var(_) => expr.clone(),
        VerifyExpr::Binary { op, left, right } => VerifyExpr::binary(
            *op,
            substitute_param(left, param, value),
            substitute_param(right, param, value),
        ),
        VerifyExpr::Not(inner) => VerifyExpr::not(substitute_param(inner, param, value)),
        VerifyExpr::Bool(_) | VerifyExpr::Int(_) => expr.clone(),
        VerifyExpr::Iff(l, r) => VerifyExpr::iff(
            substitute_param(l, param, value),
            substitute_param(r, param, value),
        ),
        VerifyExpr::ForAll { vars, body } => VerifyExpr::forall(
            vars.clone(),
            substitute_param(body, param, value),
        ),
        VerifyExpr::Exists { vars, body } => VerifyExpr::exists(
            vars.clone(),
            substitute_param(body, param, value),
        ),
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
    let mut bv_vars = HashMap::new();
    let mut array_vars = HashMap::new();
    crate::equivalence::collect_quantifier_bound_vars_pub(expr, &mut int_vars, &mut bv_vars, &mut array_vars, ctx);
    crate::kinduction::encode_expr_bool(ctx, expr, &bool_vars, &int_vars)
}
