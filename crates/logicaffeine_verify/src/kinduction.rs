//! k-Induction for Unbounded Safety Verification
//!
//! BMC proves a property holds for k cycles. k-Induction proves it holds forever.
//!
//! Algorithm:
//! 1. For k = 1, 2, ..., max_k:
//!    a. **Base case:** init AND T^k AND NOT(P) at each step. If UNSAT → base passes.
//!    b. **Inductive step:** P holds for k consecutive steps AND T AND NOT(P) at step k+1.
//!       If UNSAT → proven.
//! 2. If base fails → Counterexample. If induction fails for all k → InductionFailed.

use crate::ir::{VerifyExpr, VerifyOp};
use crate::equivalence::{Trace, CycleState, SignalValue};
use std::collections::HashMap;
use z3::{ast, ast::Ast, ast::Bool, ast::Dynamic, ast::Int, Config, Context, SatResult, Solver};

/// Result of k-induction verification.
#[derive(Debug)]
pub enum KInductionResult {
    /// Property holds for all time, proven at depth k.
    Proven { k: u32 },
    /// Base case violation at step k — property fails.
    Counterexample { k: u32, trace: Trace },
    /// Inductive step failed for all k up to max — may need larger k or strengthening.
    InductionFailed { k: u32, trace: Trace },
    /// Solver returned unknown (timeout or undecidable).
    Unknown,
}

/// Signal declaration for k-induction.
#[derive(Debug, Clone)]
pub struct SignalDecl {
    pub name: String,
    pub width: Option<u32>,
}

/// Run k-induction on a safety property.
///
/// - `init`: Initial state predicate (using variables with @0 suffix)
/// - `transition`: Transition relation (using @t and @(t+1) variables)
/// - `property`: Safety property to verify (using @t variables)
/// - `signals`: Signal declarations
/// - `max_k`: Maximum induction depth
pub fn k_induction(
    init: &VerifyExpr,
    transition: &VerifyExpr,
    property: &VerifyExpr,
    signals: &[SignalDecl],
    max_k: u32,
) -> KInductionResult {
    let mut cfg = Config::new();
    cfg.set_param_value("timeout", "30000");
    let ctx = Context::new(&cfg);

    for k in 1..=max_k {
        // ---- Base case ----
        // Check: init(0) AND T(0,1) AND T(1,2) AND ... AND T(k-2,k-1) AND NOT P(i) for some i
        let base_result = check_base_case(&ctx, init, transition, property, signals, k);
        match base_result {
            SatResult::Sat => {
                // Base case failed — property violated within k steps
                return KInductionResult::Counterexample {
                    k,
                    trace: Trace { cycles: vec![] },
                };
            }
            SatResult::Unknown => return KInductionResult::Unknown,
            SatResult::Unsat => {} // base passes, continue to inductive step
        }

        // ---- Inductive step ----
        // Check: P(0) AND P(1) AND ... AND P(k-1) AND T(0,1) AND ... AND T(k-1,k) AND NOT P(k)
        let step_result = check_inductive_step(&ctx, transition, property, signals, k);
        match step_result {
            SatResult::Unsat => {
                // Inductive step holds — property proven for all time!
                return KInductionResult::Proven { k };
            }
            SatResult::Unknown => return KInductionResult::Unknown,
            SatResult::Sat => {} // induction failed at this k, try larger k
        }
    }

    // Exhausted max_k without proving or disproving
    KInductionResult::InductionFailed {
        k: max_k,
        trace: Trace { cycles: vec![] },
    }
}

/// Check the base case: init AND transitions AND NOT(property) at some step.
fn check_base_case(
    ctx: &Context,
    init: &VerifyExpr,
    transition: &VerifyExpr,
    property: &VerifyExpr,
    signals: &[SignalDecl],
    k: u32,
) -> SatResult {
    let solver = Solver::new(ctx);

    // Assert init at step 0
    let init_0 = instantiate_at(init, 0);
    let init_z3 = encode_to_bool(ctx, &init_0);
    solver.assert(&init_z3);

    // Assert transitions T(0,1), T(1,2), ..., T(k-2, k-1)
    for t in 0..k.saturating_sub(1) {
        let trans = instantiate_transition(transition, t);
        let trans_z3 = encode_to_bool(ctx, &trans);
        solver.assert(&trans_z3);
    }

    // Assert NOT P(i) for at least one step (disjunction)
    // NOT(P(0)) OR NOT(P(1)) OR ... OR NOT(P(k-1))
    let mut not_props: Vec<Bool> = Vec::new();
    for t in 0..k {
        let prop_t = instantiate_at(property, t);
        let prop_z3 = encode_to_bool(ctx, &prop_t);
        not_props.push(prop_z3.not());
    }
    let not_prop_refs: Vec<&Bool> = not_props.iter().collect();
    let some_violation = Bool::or(ctx, &not_prop_refs);
    solver.assert(&some_violation);

    solver.check()
}

/// Check the inductive step: P holds for k steps, transition, NOT P at step k.
fn check_inductive_step(
    ctx: &Context,
    transition: &VerifyExpr,
    property: &VerifyExpr,
    signals: &[SignalDecl],
    k: u32,
) -> SatResult {
    let solver = Solver::new(ctx);

    // Assert P(0), P(1), ..., P(k-1)
    for t in 0..k {
        let prop_t = instantiate_at(property, t);
        let prop_z3 = encode_to_bool(ctx, &prop_t);
        solver.assert(&prop_z3);
    }

    // Assert transitions T(0,1), T(1,2), ..., T(k-1, k)
    for t in 0..k {
        let trans = instantiate_transition(transition, t);
        let trans_z3 = encode_to_bool(ctx, &trans);
        solver.assert(&trans_z3);
    }

    // Assert NOT P(k)
    let prop_k = instantiate_at(property, k);
    let prop_k_z3 = encode_to_bool(ctx, &prop_k);
    solver.assert(&prop_k_z3.not());

    solver.check()
}

/// Instantiate an expression at a specific timestep by replacing @t with @{step}.
pub fn instantiate_at(expr: &VerifyExpr, step: u32) -> VerifyExpr {
    rename_timestep(expr, "t", step)
}

/// Instantiate a transition relation: replace @t with @{step} and @t' with @{step+1}.
pub fn instantiate_transition(expr: &VerifyExpr, step: u32) -> VerifyExpr {
    let e1 = rename_timestep(expr, "t", step);
    rename_timestep(&e1, "t1", step + 1)
}

/// Replace @{old_suffix} with @{new_step} in all variable names.
fn rename_timestep(expr: &VerifyExpr, suffix: &str, step: u32) -> VerifyExpr {
    match expr {
        VerifyExpr::Var(name) => {
            let target = format!("@{}", suffix);
            if name.ends_with(&target) {
                let base = &name[..name.len() - target.len()];
                VerifyExpr::Var(format!("{}@{}", base, step))
            } else {
                VerifyExpr::Var(name.clone())
            }
        }
        VerifyExpr::Binary { op, left, right } => VerifyExpr::binary(
            *op,
            rename_timestep(left, suffix, step),
            rename_timestep(right, suffix, step),
        ),
        VerifyExpr::Not(inner) => VerifyExpr::not(rename_timestep(inner, suffix, step)),
        VerifyExpr::Bool(b) => VerifyExpr::Bool(*b),
        VerifyExpr::Int(n) => VerifyExpr::Int(*n),
        VerifyExpr::Iff(l, r) => VerifyExpr::iff(
            rename_timestep(l, suffix, step),
            rename_timestep(r, suffix, step),
        ),
        VerifyExpr::ForAll { vars, body } => VerifyExpr::forall(
            vars.clone(),
            rename_timestep(body, suffix, step),
        ),
        VerifyExpr::Exists { vars, body } => VerifyExpr::exists(
            vars.clone(),
            rename_timestep(body, suffix, step),
        ),
        VerifyExpr::Apply { name, args } => VerifyExpr::apply(
            name.clone(),
            args.iter().map(|a| rename_timestep(a, suffix, step)).collect(),
        ),
        VerifyExpr::BitVecConst { width, value } => VerifyExpr::bv_const(*width, *value),
        VerifyExpr::BitVecBinary { op, left, right } => VerifyExpr::bv_binary(
            *op,
            rename_timestep(left, suffix, step),
            rename_timestep(right, suffix, step),
        ),
        VerifyExpr::BitVecExtract { high, low, operand } => VerifyExpr::BitVecExtract {
            high: *high,
            low: *low,
            operand: Box::new(rename_timestep(operand, suffix, step)),
        },
        VerifyExpr::BitVecConcat(l, r) => VerifyExpr::BitVecConcat(
            Box::new(rename_timestep(l, suffix, step)),
            Box::new(rename_timestep(r, suffix, step)),
        ),
        VerifyExpr::Select { array, index } => VerifyExpr::Select {
            array: Box::new(rename_timestep(array, suffix, step)),
            index: Box::new(rename_timestep(index, suffix, step)),
        },
        VerifyExpr::Store { array, index, value } => VerifyExpr::Store {
            array: Box::new(rename_timestep(array, suffix, step)),
            index: Box::new(rename_timestep(index, suffix, step)),
            value: Box::new(rename_timestep(value, suffix, step)),
        },
        VerifyExpr::AtState { state, expr } => VerifyExpr::AtState {
            state: Box::new(rename_timestep(state, suffix, step)),
            expr: Box::new(rename_timestep(expr, suffix, step)),
        },
        VerifyExpr::Transition { from, to } => VerifyExpr::Transition {
            from: Box::new(rename_timestep(from, suffix, step)),
            to: Box::new(rename_timestep(to, suffix, step)),
        },
    }
}

/// Encode a VerifyExpr to a Z3 Bool using check_equivalence infrastructure.
/// We encode by creating: expr <-> true, then if equiv, the expr is always true.
/// For direct encoding, we use the internal equivalence encoder.
fn encode_to_bool<'ctx>(ctx: &'ctx Context, expr: &VerifyExpr) -> Bool<'ctx> {
    // Build variable maps
    let mut all_vars = std::collections::HashSet::new();
    crate::equivalence::collect_vars_pub(expr, &mut all_vars);

    let mut bool_vars: HashMap<String, Bool<'ctx>> = HashMap::new();
    let mut int_vars: HashMap<String, Int<'ctx>> = HashMap::new();

    for name in &all_vars {
        bool_vars.insert(name.clone(), Bool::new_const(ctx, name.as_str()));
    }
    crate::equivalence::collect_int_vars_pub(expr, &mut int_vars, ctx);

    // Use a simple recursive encoder
    encode_expr_bool(ctx, expr, &bool_vars, &int_vars)
}

/// Simple recursive Bool encoder for k-induction formulas.
pub fn encode_expr_bool<'ctx>(
    ctx: &'ctx Context,
    expr: &VerifyExpr,
    bool_vars: &HashMap<String, Bool<'ctx>>,
    int_vars: &HashMap<String, Int<'ctx>>,
) -> Bool<'ctx> {
    match expr {
        VerifyExpr::Bool(b) => Bool::from_bool(ctx, *b),
        VerifyExpr::Var(name) => {
            if let Some(bv) = bool_vars.get(name) {
                bv.clone()
            } else {
                Bool::new_const(ctx, name.as_str())
            }
        }
        VerifyExpr::Not(inner) => encode_expr_bool(ctx, inner, bool_vars, int_vars).not(),
        VerifyExpr::Binary { op, left, right } => {
            match op {
                VerifyOp::And => {
                    let l = encode_expr_bool(ctx, left, bool_vars, int_vars);
                    let r = encode_expr_bool(ctx, right, bool_vars, int_vars);
                    Bool::and(ctx, &[&l, &r])
                }
                VerifyOp::Or => {
                    let l = encode_expr_bool(ctx, left, bool_vars, int_vars);
                    let r = encode_expr_bool(ctx, right, bool_vars, int_vars);
                    Bool::or(ctx, &[&l, &r])
                }
                VerifyOp::Implies => {
                    let l = encode_expr_bool(ctx, left, bool_vars, int_vars);
                    let r = encode_expr_bool(ctx, right, bool_vars, int_vars);
                    l.implies(&r)
                }
                VerifyOp::Eq => {
                    // Could be Bool or Int equality
                    let li = encode_expr_int(ctx, left, int_vars);
                    let ri = encode_expr_int(ctx, right, int_vars);
                    if let (Some(l), Some(r)) = (li, ri) {
                        l._eq(&r)
                    } else {
                        let l = encode_expr_bool(ctx, left, bool_vars, int_vars);
                        let r = encode_expr_bool(ctx, right, bool_vars, int_vars);
                        l.iff(&r)
                    }
                }
                VerifyOp::Neq => {
                    let li = encode_expr_int(ctx, left, int_vars);
                    let ri = encode_expr_int(ctx, right, int_vars);
                    if let (Some(l), Some(r)) = (li, ri) {
                        l._eq(&r).not()
                    } else {
                        let l = encode_expr_bool(ctx, left, bool_vars, int_vars);
                        let r = encode_expr_bool(ctx, right, bool_vars, int_vars);
                        l.iff(&r).not()
                    }
                }
                VerifyOp::Gt => {
                    let l = encode_expr_int(ctx, left, int_vars).unwrap_or_else(|| Int::from_i64(ctx, 0));
                    let r = encode_expr_int(ctx, right, int_vars).unwrap_or_else(|| Int::from_i64(ctx, 0));
                    l.gt(&r)
                }
                VerifyOp::Lt => {
                    let l = encode_expr_int(ctx, left, int_vars).unwrap_or_else(|| Int::from_i64(ctx, 0));
                    let r = encode_expr_int(ctx, right, int_vars).unwrap_or_else(|| Int::from_i64(ctx, 0));
                    l.lt(&r)
                }
                VerifyOp::Gte => {
                    let l = encode_expr_int(ctx, left, int_vars).unwrap_or_else(|| Int::from_i64(ctx, 0));
                    let r = encode_expr_int(ctx, right, int_vars).unwrap_or_else(|| Int::from_i64(ctx, 0));
                    l.ge(&r)
                }
                VerifyOp::Lte => {
                    let l = encode_expr_int(ctx, left, int_vars).unwrap_or_else(|| Int::from_i64(ctx, 0));
                    let r = encode_expr_int(ctx, right, int_vars).unwrap_or_else(|| Int::from_i64(ctx, 0));
                    l.le(&r)
                }
                VerifyOp::Add | VerifyOp::Sub | VerifyOp::Mul | VerifyOp::Div => {
                    Bool::from_bool(ctx, false) // arithmetic ops aren't Bool
                }
            }
        }
        VerifyExpr::Iff(l, r) => {
            let lb = encode_expr_bool(ctx, l, bool_vars, int_vars);
            let rb = encode_expr_bool(ctx, r, bool_vars, int_vars);
            lb.iff(&rb)
        }
        VerifyExpr::ForAll { vars, body } => {
            if vars.is_empty() {
                return encode_expr_bool(ctx, body, bool_vars, int_vars);
            }
            let body_bool = encode_expr_bool(ctx, body, bool_vars, int_vars);
            let bound_consts: Vec<Dynamic<'ctx>> = vars.iter().map(|(name, ty)| {
                match ty {
                    crate::ir::VerifyType::Int | crate::ir::VerifyType::Object => {
                        Dynamic::from_ast(&Int::new_const(ctx, name.as_str()))
                    }
                    crate::ir::VerifyType::Bool => {
                        Dynamic::from_ast(&Bool::new_const(ctx, name.as_str()))
                    }
                    crate::ir::VerifyType::BitVector(w) => {
                        Dynamic::from_ast(&z3::ast::BV::new_const(ctx, name.as_str(), *w))
                    }
                    _ => Dynamic::from_ast(&Int::new_const(ctx, name.as_str())),
                }
            }).collect();
            let refs: Vec<&dyn Ast<'ctx>> = bound_consts.iter().map(|d| d as &dyn Ast<'ctx>).collect();
            z3::ast::forall_const(ctx, &refs, &[], &body_bool)
        }
        VerifyExpr::Exists { vars, body } => {
            if vars.is_empty() {
                return encode_expr_bool(ctx, body, bool_vars, int_vars);
            }
            let body_bool = encode_expr_bool(ctx, body, bool_vars, int_vars);
            let bound_consts: Vec<Dynamic<'ctx>> = vars.iter().map(|(name, ty)| {
                match ty {
                    crate::ir::VerifyType::Int | crate::ir::VerifyType::Object => {
                        Dynamic::from_ast(&Int::new_const(ctx, name.as_str()))
                    }
                    crate::ir::VerifyType::Bool => {
                        Dynamic::from_ast(&Bool::new_const(ctx, name.as_str()))
                    }
                    crate::ir::VerifyType::BitVector(w) => {
                        Dynamic::from_ast(&z3::ast::BV::new_const(ctx, name.as_str(), *w))
                    }
                    _ => Dynamic::from_ast(&Int::new_const(ctx, name.as_str())),
                }
            }).collect();
            let refs: Vec<&dyn Ast<'ctx>> = bound_consts.iter().map(|d| d as &dyn Ast<'ctx>).collect();
            z3::ast::exists_const(ctx, &refs, &[], &body_bool)
        }
        _ => Bool::from_bool(ctx, true),
    }
}

/// Try to encode an expression as a Z3 Int. Returns None if not integer-typed.
pub fn encode_expr_int<'ctx>(
    ctx: &'ctx Context,
    expr: &VerifyExpr,
    int_vars: &HashMap<String, Int<'ctx>>,
) -> Option<Int<'ctx>> {
    match expr {
        VerifyExpr::Int(n) => Some(Int::from_i64(ctx, *n)),
        VerifyExpr::Var(name) => {
            if let Some(iv) = int_vars.get(name) {
                Some(iv.clone())
            } else {
                // If it looks like it should be an int, create one
                Some(Int::new_const(ctx, name.as_str()))
            }
        }
        VerifyExpr::Binary { op, left, right } => {
            match op {
                VerifyOp::Add => {
                    let l = encode_expr_int(ctx, left, int_vars)?;
                    let r = encode_expr_int(ctx, right, int_vars)?;
                    Some(Int::add(ctx, &[&l, &r]))
                }
                VerifyOp::Sub => {
                    let l = encode_expr_int(ctx, left, int_vars)?;
                    let r = encode_expr_int(ctx, right, int_vars)?;
                    Some(Int::sub(ctx, &[&l, &r]))
                }
                VerifyOp::Mul => {
                    let l = encode_expr_int(ctx, left, int_vars)?;
                    let r = encode_expr_int(ctx, right, int_vars)?;
                    Some(Int::mul(ctx, &[&l, &r]))
                }
                _ => None,
            }
        }
        _ => None,
    }
}
