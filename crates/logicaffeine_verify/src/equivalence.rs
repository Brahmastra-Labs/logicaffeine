//! Semantic Equivalence Checking — The Core Contribution
//!
//! The question nobody else asks: does the LLM-generated SVA express
//! the SAME property as the formally parsed FOL?
//!
//! ## Method
//!
//! Given two VerifyExpr formulas (one from FOL, one from SVA), both
//! unrolled to bounded timesteps:
//!
//! 1. Declare all signal@timestep variables as Z3 booleans
//! 2. Construct ¬(FOL ↔ SVA)
//! 3. If UNSAT → equivalent (no assignment makes them differ)
//! 4. If SAT → extract counterexample trace from Z3 model
//! 5. If UNKNOWN → timeout/undecidable
//!
//! ## Why This Matters
//!
//! Current industry practice: SVAs are checked against RTL (the hardware
//! implementation). Nobody checks SVAs against the specification, because
//! nobody has a formal specification. LOGOS provides one.

use crate::ir::{VerifyExpr, VerifyOp, VerifyType, BitVecOp};
use std::collections::{HashMap, HashSet};
use z3::{ast::Ast, ast::Bool, ast::Dynamic, ast::Int, Config, Context, SatResult, Solver};

/// Result of checking semantic equivalence.
#[derive(Debug)]
pub enum EquivalenceResult {
    /// The two expressions are semantically equivalent at the given bound.
    Equivalent,
    /// The expressions differ. Counterexample shows concrete signal values
    /// where they diverge.
    NotEquivalent { counterexample: Trace },
    /// Z3 returned unknown (timeout or undecidable).
    Unknown,
}

/// A counterexample trace showing signal values at each clock cycle.
#[derive(Debug, Clone)]
pub struct Trace {
    pub cycles: Vec<CycleState>,
}

/// Multi-sorted signal value for counterexample traces.
#[derive(Debug, Clone, PartialEq)]
pub enum SignalValue {
    Bool(bool),
    Int(i64),
    BitVec { width: u32, value: u64 },
    Unknown,
}

impl SignalValue {
    /// Get as bool if this is a boolean value.
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            SignalValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Get as i64 if this is an integer value.
    pub fn as_int(&self) -> Option<i64> {
        match self {
            SignalValue::Int(n) => Some(*n),
            _ => None,
        }
    }
}

impl std::fmt::Display for SignalValue {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SignalValue::Bool(b) => write!(f, "{}", b),
            SignalValue::Int(n) => write!(f, "{}", n),
            SignalValue::BitVec { width, value } => {
                let hex_digits = ((*width + 3) / 4) as usize;
                write!(f, "0x{:0>width$X}", value, width = hex_digits)
            }
            SignalValue::Unknown => write!(f, "?"),
        }
    }
}

/// Signal assignments at a single clock cycle.
#[derive(Debug, Clone)]
pub struct CycleState {
    pub cycle: usize,
    pub signals: HashMap<String, SignalValue>,
}

/// Check whether two VerifyExpr formulas are semantically equivalent.
///
/// Both formulas should be bounded (timestep-unrolled) — i.e., all temporal
/// operators have been expanded to conjunctions/disjunctions over timesteps,
/// and signal references use the "name@timestep" naming convention.
///
/// The `signals` parameter lists signal names (without @timestep) to declare
/// as boolean variables at each timestep.
///
/// # Example
///
/// ```ignore
/// let fol = VerifyExpr::binary(VerifyOp::Implies,
///     VerifyExpr::Var("req@0".into()),
///     VerifyExpr::Var("ack@0".into()),
/// );
/// let sva = VerifyExpr::binary(VerifyOp::Implies,
///     VerifyExpr::Var("req@0".into()),
///     VerifyExpr::Var("ack@0".into()),
/// );
/// let result = check_equivalence(&fol, &sva, &["req".into(), "ack".into()], 1);
/// assert!(matches!(result, EquivalenceResult::Equivalent));
/// ```
pub fn check_equivalence(
    fol_expr: &VerifyExpr,
    sva_expr: &VerifyExpr,
    signals: &[String],
    bound: usize,
) -> EquivalenceResult {
    let mut cfg = Config::new();
    cfg.set_param_value("timeout", "10000"); // 10 second timeout
    let ctx = Context::new(&cfg);
    let solver = Solver::new(&ctx);

    // 1. Collect all variable names referenced in both expressions
    let mut all_vars: HashSet<String> = HashSet::new();
    collect_vars(fol_expr, &mut all_vars);
    collect_vars(sva_expr, &mut all_vars);

    // Also declare signal@timestep variables from the signals list
    for sig in signals {
        for t in 0..=bound {
            all_vars.insert(format!("{}@{}", sig, t));
        }
    }

    // 2. Declare all signal@timestep variables as Z3 booleans,
    //    and collect any integer variable names from the expressions.
    let mut bool_vars: HashMap<String, Bool> = HashMap::new();
    let mut int_vars: HashMap<String, Int> = HashMap::new();
    let mut bv_vars: HashMap<String, z3::ast::BV> = HashMap::new();
    let mut array_vars: HashMap<String, z3::ast::Array> = HashMap::new();
    for var_name in &all_vars {
        bool_vars.insert(
            var_name.clone(),
            Bool::new_const(&ctx, var_name.as_str()),
        );
    }
    // Pre-declare integer variables found in Int-typed positions
    collect_int_vars(fol_expr, &mut int_vars, &ctx);
    collect_int_vars(sva_expr, &mut int_vars, &ctx);
    // Also collect variables used as array indices/values (they need Int sort)
    collect_array_index_vars(fol_expr, &mut int_vars, &ctx);
    collect_array_index_vars(sva_expr, &mut int_vars, &ctx);
    // Collect quantifier-bound variables with their declared types
    collect_quantifier_bound_vars(fol_expr, &mut int_vars, &mut bv_vars, &mut array_vars, &ctx);
    collect_quantifier_bound_vars(sva_expr, &mut int_vars, &mut bv_vars, &mut array_vars, &ctx);
    // Pre-declare bitvector variables found in BV-typed positions
    collect_bv_vars(fol_expr, &mut bv_vars, &ctx);
    collect_bv_vars(sva_expr, &mut bv_vars, &ctx);
    // Pre-declare array variables found in array-typed positions
    collect_array_vars(fol_expr, &mut array_vars, &ctx);
    collect_array_vars(sva_expr, &mut array_vars, &ctx);

    // 3. Encode both expressions into Z3 using the dynamic encoder
    let encoder = EquivEncoder {
        ctx: &ctx,
        bool_vars: &bool_vars,
        int_vars: &int_vars,
        bv_vars: &bv_vars,
        array_vars: &array_vars,
    };
    let fol_z3 = encoder.encode(fol_expr);
    let sva_z3 = encoder.encode(sva_expr);

    // 4. Construct ¬(FOL ↔ SVA) and check satisfiability
    //    For Bool sorts, use iff; for other sorts, use _eq
    //    If sorts differ (e.g. BV vs Bool), return NotEquivalent immediately
    let fol_sort = fol_z3.get_sort();
    let sva_sort = sva_z3.get_sort();
    if fol_sort != sva_sort {
        // Sort mismatch — expressions cannot be equivalent
        return EquivalenceResult::NotEquivalent {
            counterexample: Trace { cycles: vec![] },
        };
    }
    let not_iff = if let (Some(fb), Some(sb)) = (fol_z3.as_bool(), sva_z3.as_bool()) {
        fb.iff(&sb).not()
    } else {
        fol_z3._eq(&sva_z3).not()
    };

    solver.assert(&not_iff);

    match solver.check() {
        SatResult::Unsat => {
            // No assignment makes them differ → equivalent
            EquivalenceResult::Equivalent
        }
        SatResult::Sat => {
            // Found an assignment where they differ → extract counterexample
            let model = solver.get_model().unwrap();
            let trace = extract_trace(&ctx, &model, signals, bound, &bool_vars, &int_vars, &bv_vars);
            EquivalenceResult::NotEquivalent { counterexample: trace }
        }
        SatResult::Unknown => EquivalenceResult::Unknown,
    }
}

/// Extract unique signal names from a VerifyExpr by collecting Var references
/// and stripping the @timestep suffix.
pub fn extract_signals(expr: &VerifyExpr) -> Vec<String> {
    let mut vars = HashSet::new();
    collect_vars(expr, &mut vars);
    let mut signals: HashSet<String> = HashSet::new();
    for var in &vars {
        if let Some(at_pos) = var.find('@') {
            signals.insert(var[..at_pos].to_string());
        }
    }
    signals.into_iter().collect()
}

/// Collect all variable names referenced in a VerifyExpr (public API).
pub fn collect_vars_pub(expr: &VerifyExpr, vars: &mut HashSet<String>) {
    collect_vars(expr, vars);
}

/// Collect integer-typed variable names from a VerifyExpr (public API).
pub fn collect_int_vars_pub<'ctx>(expr: &VerifyExpr, int_vars: &mut HashMap<String, Int<'ctx>>, ctx: &'ctx Context) {
    collect_int_vars(expr, int_vars, ctx);
}

/// Collect quantifier-bound variables from a VerifyExpr (public API).
pub fn collect_quantifier_bound_vars_pub<'ctx>(
    expr: &VerifyExpr,
    int_vars: &mut HashMap<String, Int<'ctx>>,
    bv_vars: &mut HashMap<String, z3::ast::BV<'ctx>>,
    array_vars: &mut HashMap<String, z3::ast::Array<'ctx>>,
    ctx: &'ctx Context,
) {
    collect_quantifier_bound_vars(expr, int_vars, bv_vars, array_vars, ctx);
}

/// Collect all variable names referenced in a VerifyExpr.
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
        VerifyExpr::ForAll { body, .. } => collect_vars(body, vars),
        VerifyExpr::Exists { body, .. } => collect_vars(body, vars),
        VerifyExpr::Apply { args, .. } => {
            for arg in args { collect_vars(arg, vars); }
        }
        // Bitvector and array variants
        VerifyExpr::BitVecBinary { left, right, .. } => {
            collect_vars(left, vars);
            collect_vars(right, vars);
        }
        VerifyExpr::BitVecExtract { operand, .. } => collect_vars(operand, vars),
        VerifyExpr::BitVecConcat(l, r) => {
            collect_vars(l, vars);
            collect_vars(r, vars);
        }
        VerifyExpr::Select { array, index } => {
            collect_vars(array, vars);
            collect_vars(index, vars);
        }
        VerifyExpr::Store { array, index, value } => {
            collect_vars(array, vars);
            collect_vars(index, vars);
            collect_vars(value, vars);
        }
        VerifyExpr::AtState { state, expr } => {
            collect_vars(state, vars);
            collect_vars(expr, vars);
        }
        VerifyExpr::Transition { from, to } => {
            collect_vars(from, vars);
            collect_vars(to, vars);
        }
        // Literals have no variables
        VerifyExpr::Int(_) | VerifyExpr::Bool(_) | VerifyExpr::BitVecConst { .. } => {}
    }
}

/// Collect variable names that appear in integer-typed positions (inside arithmetic ops).
fn collect_int_vars<'ctx>(expr: &VerifyExpr, int_vars: &mut HashMap<String, Int<'ctx>>, ctx: &'ctx Context) {
    match expr {
        VerifyExpr::Int(_) => {}
        VerifyExpr::Bool(_) => {}
        VerifyExpr::Var(_) => {} // Vars are bool by default; promoted to int when used in arith context
        VerifyExpr::Binary { op, left, right } => {
            match op {
                // Arithmetic ops: children are integer-typed
                VerifyOp::Add | VerifyOp::Sub | VerifyOp::Mul | VerifyOp::Div => {
                    collect_int_var_leaves(left, int_vars, ctx);
                    collect_int_var_leaves(right, int_vars, ctx);
                }
                // Comparison ops: children are integer-typed
                VerifyOp::Gt | VerifyOp::Lt | VerifyOp::Gte | VerifyOp::Lte => {
                    collect_int_var_leaves(left, int_vars, ctx);
                    collect_int_var_leaves(right, int_vars, ctx);
                }
                // Eq/Neq: could be bool or int — check if children look integer
                VerifyOp::Eq | VerifyOp::Neq => {
                    if expr_is_integer(left) || expr_is_integer(right) {
                        collect_int_var_leaves(left, int_vars, ctx);
                        collect_int_var_leaves(right, int_vars, ctx);
                    }
                }
                _ => {}
            }
            collect_int_vars(left, int_vars, ctx);
            collect_int_vars(right, int_vars, ctx);
        }
        VerifyExpr::Not(inner) => collect_int_vars(inner, int_vars, ctx),
        VerifyExpr::Iff(l, r) => {
            collect_int_vars(l, int_vars, ctx);
            collect_int_vars(r, int_vars, ctx);
        }
        VerifyExpr::ForAll { body, .. } | VerifyExpr::Exists { body, .. } => {
            collect_int_vars(body, int_vars, ctx);
        }
        VerifyExpr::Apply { args, .. } => {
            for arg in args { collect_int_vars(arg, int_vars, ctx); }
        }
        _ => {}
    }
}

/// Recursively collect Var leaves from integer-typed subexpressions.
fn collect_int_var_leaves<'ctx>(expr: &VerifyExpr, int_vars: &mut HashMap<String, Int<'ctx>>, ctx: &'ctx Context) {
    match expr {
        VerifyExpr::Var(name) => {
            if !int_vars.contains_key(name) {
                int_vars.insert(name.clone(), Int::new_const(ctx, name.as_str()));
            }
        }
        VerifyExpr::Binary { left, right, .. } => {
            collect_int_var_leaves(left, int_vars, ctx);
            collect_int_var_leaves(right, int_vars, ctx);
        }
        _ => {}
    }
}

/// Collect variable names that appear in bitvector-typed positions.
fn collect_bv_vars<'ctx>(expr: &VerifyExpr, bv_vars: &mut HashMap<String, z3::ast::BV<'ctx>>, ctx: &'ctx Context) {
    match expr {
        VerifyExpr::BitVecBinary { op: _, left, right } => {
            collect_bv_var_leaves(left, bv_vars, ctx, None);
            collect_bv_var_leaves(right, bv_vars, ctx, None);
            collect_bv_vars(left, bv_vars, ctx);
            collect_bv_vars(right, bv_vars, ctx);
        }
        VerifyExpr::BitVecExtract { operand, .. } => {
            collect_bv_var_leaves(operand, bv_vars, ctx, None);
            collect_bv_vars(operand, bv_vars, ctx);
        }
        VerifyExpr::BitVecConcat(l, r) => {
            collect_bv_var_leaves(l, bv_vars, ctx, None);
            collect_bv_var_leaves(r, bv_vars, ctx, None);
            collect_bv_vars(l, bv_vars, ctx);
            collect_bv_vars(r, bv_vars, ctx);
        }
        VerifyExpr::Binary { left, right, .. } => {
            collect_bv_vars(left, bv_vars, ctx);
            collect_bv_vars(right, bv_vars, ctx);
        }
        VerifyExpr::Not(inner) => collect_bv_vars(inner, bv_vars, ctx),
        VerifyExpr::Iff(l, r) => {
            collect_bv_vars(l, bv_vars, ctx);
            collect_bv_vars(r, bv_vars, ctx);
        }
        VerifyExpr::ForAll { body, .. } | VerifyExpr::Exists { body, .. } => {
            collect_bv_vars(body, bv_vars, ctx);
        }
        VerifyExpr::Apply { args, .. } => {
            for arg in args { collect_bv_vars(arg, bv_vars, ctx); }
        }
        VerifyExpr::Select { array, index } => {
            collect_bv_vars(array, bv_vars, ctx);
            collect_bv_vars(index, bv_vars, ctx);
        }
        VerifyExpr::Store { array, index, value } => {
            collect_bv_vars(array, bv_vars, ctx);
            collect_bv_vars(index, bv_vars, ctx);
            collect_bv_vars(value, bv_vars, ctx);
        }
        VerifyExpr::AtState { state, expr } => {
            collect_bv_vars(state, bv_vars, ctx);
            collect_bv_vars(expr, bv_vars, ctx);
        }
        VerifyExpr::Transition { from, to } => {
            collect_bv_vars(from, bv_vars, ctx);
            collect_bv_vars(to, bv_vars, ctx);
        }
        _ => {}
    }
}

/// Recursively collect Var leaves from bitvector-typed subexpressions.
fn collect_bv_var_leaves<'ctx>(expr: &VerifyExpr, bv_vars: &mut HashMap<String, z3::ast::BV<'ctx>>, ctx: &'ctx Context, width_hint: Option<u32>) {
    match expr {
        VerifyExpr::Var(name) => {
            if !bv_vars.contains_key(name) {
                // Determine width from context or default to 8
                let width = width_hint.unwrap_or_else(|| infer_bv_width_from_name(name));
                bv_vars.insert(name.clone(), z3::ast::BV::new_const(ctx, name.as_str(), width));
            }
        }
        VerifyExpr::BitVecConst { width, .. } => {
            // A constant tells us the width of sibling vars
            // (handled by the caller)
        }
        VerifyExpr::BitVecBinary { left, right, .. } => {
            // Try to extract width from either side
            let w = bv_expr_width(expr);
            collect_bv_var_leaves(left, bv_vars, ctx, w);
            collect_bv_var_leaves(right, bv_vars, ctx, w);
        }
        VerifyExpr::Binary { left, right, .. } => {
            collect_bv_var_leaves(left, bv_vars, ctx, width_hint);
            collect_bv_var_leaves(right, bv_vars, ctx, width_hint);
        }
        _ => {}
    }
}

/// Infer bitvector width from variable name convention (e.g., "x_bv8" → 8, "data_bv16" → 16).
fn infer_bv_width_from_name(name: &str) -> u32 {
    if let Some(pos) = name.rfind("_bv") {
        if let Ok(w) = name[pos + 3..].parse::<u32>() {
            return w;
        }
    }
    // Also try "bvN" at end without underscore
    if let Some(pos) = name.rfind("bv") {
        if let Ok(w) = name[pos + 2..].parse::<u32>() {
            return w;
        }
    }
    8 // default width
}

/// Try to determine the bitvector width of an expression.
fn bv_expr_width(expr: &VerifyExpr) -> Option<u32> {
    match expr {
        VerifyExpr::BitVecConst { width, .. } => Some(*width),
        VerifyExpr::BitVecBinary { left, right, .. } => {
            bv_expr_width(left).or_else(|| bv_expr_width(right))
        }
        VerifyExpr::BitVecExtract { high, low, .. } => Some(high - low + 1),
        VerifyExpr::BitVecConcat(l, r) => {
            match (bv_expr_width(l), bv_expr_width(r)) {
                (Some(wl), Some(wr)) => Some(wl + wr),
                _ => None,
            }
        }
        VerifyExpr::Var(name) => {
            let w = infer_bv_width_from_name(name);
            if w != 8 { Some(w) } else { None } // Only return if we found an explicit width
        }
        _ => None,
    }
}

/// Collect variable names that appear in array-typed positions.
/// Also collects index/value variables as int vars since arrays use Int → Int by default.
fn collect_array_vars<'ctx>(expr: &VerifyExpr, array_vars: &mut HashMap<String, z3::ast::Array<'ctx>>, ctx: &'ctx Context) {
    collect_array_vars_inner(expr, array_vars, ctx);
}

fn collect_array_vars_inner<'ctx>(expr: &VerifyExpr, array_vars: &mut HashMap<String, z3::ast::Array<'ctx>>, ctx: &'ctx Context) {
    match expr {
        VerifyExpr::Select { array, index } => {
            if let VerifyExpr::Var(name) = array.as_ref() {
                if !array_vars.contains_key(name) {
                    let int_sort = z3::Sort::int(ctx);
                    array_vars.insert(
                        name.clone(),
                        z3::ast::Array::new_const(ctx, name.as_str(), &int_sort, &int_sort),
                    );
                }
            }
            collect_array_vars_inner(array, array_vars, ctx);
            collect_array_vars_inner(index, array_vars, ctx);
        }
        VerifyExpr::Store { array, index, value } => {
            if let VerifyExpr::Var(name) = array.as_ref() {
                if !array_vars.contains_key(name) {
                    let int_sort = z3::Sort::int(ctx);
                    array_vars.insert(
                        name.clone(),
                        z3::ast::Array::new_const(ctx, name.as_str(), &int_sort, &int_sort),
                    );
                }
            }
            collect_array_vars_inner(array, array_vars, ctx);
            collect_array_vars_inner(index, array_vars, ctx);
            collect_array_vars_inner(value, array_vars, ctx);
        }
        VerifyExpr::Binary { left, right, .. } => {
            collect_array_vars_inner(left, array_vars, ctx);
            collect_array_vars_inner(right, array_vars, ctx);
        }
        VerifyExpr::Not(inner) => collect_array_vars_inner(inner, array_vars, ctx),
        VerifyExpr::Iff(l, r) => {
            collect_array_vars_inner(l, array_vars, ctx);
            collect_array_vars_inner(r, array_vars, ctx);
        }
        VerifyExpr::ForAll { body, .. } | VerifyExpr::Exists { body, .. } => {
            collect_array_vars_inner(body, array_vars, ctx);
        }
        _ => {}
    }
}

/// Collect variables used as array indices or values (they need Int sort, not Bool).
fn collect_array_index_vars<'ctx>(expr: &VerifyExpr, int_vars: &mut HashMap<String, Int<'ctx>>, ctx: &'ctx Context) {
    match expr {
        VerifyExpr::Select { array, index } => {
            collect_int_var_leaves(index, int_vars, ctx);
            collect_array_index_vars(array, int_vars, ctx);
            collect_array_index_vars(index, int_vars, ctx);
        }
        VerifyExpr::Store { array, index, value } => {
            collect_int_var_leaves(index, int_vars, ctx);
            collect_int_var_leaves(value, int_vars, ctx);
            collect_array_index_vars(array, int_vars, ctx);
            collect_array_index_vars(index, int_vars, ctx);
            collect_array_index_vars(value, int_vars, ctx);
        }
        VerifyExpr::Binary { left, right, .. } => {
            collect_array_index_vars(left, int_vars, ctx);
            collect_array_index_vars(right, int_vars, ctx);
        }
        VerifyExpr::Not(inner) => collect_array_index_vars(inner, int_vars, ctx),
        VerifyExpr::Iff(l, r) => {
            collect_array_index_vars(l, int_vars, ctx);
            collect_array_index_vars(r, int_vars, ctx);
        }
        VerifyExpr::ForAll { body, .. } | VerifyExpr::Exists { body, .. } => {
            collect_array_index_vars(body, int_vars, ctx);
        }
        _ => {}
    }
}

/// Collect quantifier-bound variables with their declared types.
fn collect_quantifier_bound_vars<'ctx>(
    expr: &VerifyExpr,
    int_vars: &mut HashMap<String, Int<'ctx>>,
    bv_vars: &mut HashMap<String, z3::ast::BV<'ctx>>,
    array_vars: &mut HashMap<String, z3::ast::Array<'ctx>>,
    ctx: &'ctx Context,
) {
    match expr {
        VerifyExpr::ForAll { vars, body } | VerifyExpr::Exists { vars, body } => {
            for (name, ty) in vars {
                match ty {
                    VerifyType::Int | VerifyType::Object | VerifyType::Real => {
                        if !int_vars.contains_key(name) {
                            int_vars.insert(name.clone(), Int::new_const(ctx, name.as_str()));
                        }
                    }
                    VerifyType::BitVector(w) => {
                        if !bv_vars.contains_key(name) {
                            bv_vars.insert(name.clone(), z3::ast::BV::new_const(ctx, name.as_str(), *w));
                        }
                    }
                    VerifyType::Array(idx, elem) => {
                        if !array_vars.contains_key(name) {
                            let idx_sort = type_to_z3_sort(ctx, idx);
                            let elem_sort = type_to_z3_sort(ctx, elem);
                            array_vars.insert(
                                name.clone(),
                                z3::ast::Array::new_const(ctx, name.as_str(), &idx_sort, &elem_sort),
                            );
                        }
                    }
                    VerifyType::Bool => {
                        // Bool vars are already declared in bool_vars by default
                    }
                }
            }
            collect_quantifier_bound_vars(body, int_vars, bv_vars, array_vars, ctx);
        }
        VerifyExpr::Binary { left, right, .. } => {
            collect_quantifier_bound_vars(left, int_vars, bv_vars, array_vars, ctx);
            collect_quantifier_bound_vars(right, int_vars, bv_vars, array_vars, ctx);
        }
        VerifyExpr::Not(inner) => {
            collect_quantifier_bound_vars(inner, int_vars, bv_vars, array_vars, ctx);
        }
        VerifyExpr::Iff(l, r) => {
            collect_quantifier_bound_vars(l, int_vars, bv_vars, array_vars, ctx);
            collect_quantifier_bound_vars(r, int_vars, bv_vars, array_vars, ctx);
        }
        _ => {}
    }
}

/// Convert a VerifyType to a Z3 Sort (standalone function).
fn type_to_z3_sort<'ctx>(ctx: &'ctx Context, ty: &VerifyType) -> z3::Sort<'ctx> {
    match ty {
        VerifyType::Int => z3::Sort::int(ctx),
        VerifyType::Bool => z3::Sort::bool(ctx),
        VerifyType::Object => z3::Sort::int(ctx),
        VerifyType::Real => z3::Sort::real(ctx),
        VerifyType::BitVector(w) => z3::Sort::bitvector(ctx, *w),
        VerifyType::Array(idx, elem) => {
            let idx_sort = type_to_z3_sort(ctx, idx);
            let elem_sort = type_to_z3_sort(ctx, elem);
            z3::Sort::array(ctx, &idx_sort, &elem_sort)
        }
    }
}

/// Heuristic: does this expression look like it produces an integer?
fn expr_is_integer(expr: &VerifyExpr) -> bool {
    matches!(expr,
        VerifyExpr::Int(_)
        | VerifyExpr::Binary { op: VerifyOp::Add | VerifyOp::Sub | VerifyOp::Mul | VerifyOp::Div, .. }
    )
}

/// Encoder that handles Bool, Int, BitVec, and Array Z3 expressions for equivalence checking.
pub struct EquivEncoder<'ctx> {
    ctx: &'ctx Context,
    bool_vars: &'ctx HashMap<String, Bool<'ctx>>,
    int_vars: &'ctx HashMap<String, Int<'ctx>>,
    bv_vars: &'ctx HashMap<String, z3::ast::BV<'ctx>>,
    array_vars: &'ctx HashMap<String, z3::ast::Array<'ctx>>,
}

impl<'ctx> EquivEncoder<'ctx> {
    /// Create a full encoder with all variable maps.
    pub fn new(
        ctx: &'ctx Context,
        bool_vars: &'ctx HashMap<String, Bool<'ctx>>,
        int_vars: &'ctx HashMap<String, Int<'ctx>>,
        bv_vars: &'ctx HashMap<String, z3::ast::BV<'ctx>>,
        array_vars: &'ctx HashMap<String, z3::ast::Array<'ctx>>,
    ) -> Self {
        Self { ctx, bool_vars, int_vars, bv_vars, array_vars }
    }

    /// Encode as a Dynamic Z3 expression (may be Bool or Int).
    fn encode(&self, expr: &VerifyExpr) -> Dynamic<'ctx> {
        match expr {
            VerifyExpr::Bool(b) => Dynamic::from_ast(&Bool::from_bool(self.ctx, *b)),

            VerifyExpr::Int(n) => Dynamic::from_ast(&Int::from_i64(self.ctx, *n)),

            VerifyExpr::Var(name) => {
                // Check in order of specificity: BV > Array > Int > Bool
                if let Some(bv) = self.bv_vars.get(name) {
                    Dynamic::from_ast(bv)
                } else if let Some(arr) = self.array_vars.get(name) {
                    Dynamic::from_ast(arr)
                } else if let Some(iv) = self.int_vars.get(name) {
                    Dynamic::from_ast(iv)
                } else if let Some(bv) = self.bool_vars.get(name) {
                    Dynamic::from_ast(bv)
                } else {
                    Dynamic::from_ast(&Bool::new_const(self.ctx, name.as_str()))
                }
            }

            VerifyExpr::Binary { op, left, right } => {
                let l = self.encode(left);
                let r = self.encode(right);
                match op {
                    // Boolean ops
                    VerifyOp::And => {
                        if let (Some(lb), Some(rb)) = (l.as_bool(), r.as_bool()) {
                            Dynamic::from_ast(&Bool::and(self.ctx, &[&lb, &rb]))
                        } else {
                            l // fallback
                        }
                    }
                    VerifyOp::Or => {
                        if let (Some(lb), Some(rb)) = (l.as_bool(), r.as_bool()) {
                            Dynamic::from_ast(&Bool::or(self.ctx, &[&lb, &rb]))
                        } else {
                            l
                        }
                    }
                    VerifyOp::Implies => {
                        if let (Some(lb), Some(rb)) = (l.as_bool(), r.as_bool()) {
                            Dynamic::from_ast(&lb.implies(&rb))
                        } else {
                            l
                        }
                    }
                    // Equality: works for both Bool and Int
                    VerifyOp::Eq => {
                        if let (Some(li), Some(ri)) = (l.as_int(), r.as_int()) {
                            Dynamic::from_ast(&li._eq(&ri))
                        } else if let (Some(lb), Some(rb)) = (l.as_bool(), r.as_bool()) {
                            Dynamic::from_ast(&lb.iff(&rb))
                        } else {
                            Dynamic::from_ast(&l._eq(&r))
                        }
                    }
                    VerifyOp::Neq => {
                        if let (Some(li), Some(ri)) = (l.as_int(), r.as_int()) {
                            Dynamic::from_ast(&li._eq(&ri).not())
                        } else if let (Some(lb), Some(rb)) = (l.as_bool(), r.as_bool()) {
                            Dynamic::from_ast(&lb.iff(&rb).not())
                        } else {
                            Dynamic::from_ast(&l._eq(&r).not())
                        }
                    }
                    // Arithmetic ops → Int
                    VerifyOp::Add => {
                        if let (Some(li), Some(ri)) = (l.as_int(), r.as_int()) {
                            Dynamic::from_ast(&Int::add(self.ctx, &[&li, &ri]))
                        } else { l }
                    }
                    VerifyOp::Sub => {
                        if let (Some(li), Some(ri)) = (l.as_int(), r.as_int()) {
                            Dynamic::from_ast(&Int::sub(self.ctx, &[&li, &ri]))
                        } else { l }
                    }
                    VerifyOp::Mul => {
                        if let (Some(li), Some(ri)) = (l.as_int(), r.as_int()) {
                            Dynamic::from_ast(&Int::mul(self.ctx, &[&li, &ri]))
                        } else { l }
                    }
                    VerifyOp::Div => {
                        if let (Some(li), Some(ri)) = (l.as_int(), r.as_int()) {
                            Dynamic::from_ast(&li.div(&ri))
                        } else { l }
                    }
                    // Comparison ops → Bool (from Int operands)
                    VerifyOp::Gt => {
                        if let (Some(li), Some(ri)) = (l.as_int(), r.as_int()) {
                            Dynamic::from_ast(&li.gt(&ri))
                        } else { Dynamic::from_ast(&Bool::from_bool(self.ctx, false)) }
                    }
                    VerifyOp::Lt => {
                        if let (Some(li), Some(ri)) = (l.as_int(), r.as_int()) {
                            Dynamic::from_ast(&li.lt(&ri))
                        } else { Dynamic::from_ast(&Bool::from_bool(self.ctx, false)) }
                    }
                    VerifyOp::Gte => {
                        if let (Some(li), Some(ri)) = (l.as_int(), r.as_int()) {
                            Dynamic::from_ast(&li.ge(&ri))
                        } else { Dynamic::from_ast(&Bool::from_bool(self.ctx, false)) }
                    }
                    VerifyOp::Lte => {
                        if let (Some(li), Some(ri)) = (l.as_int(), r.as_int()) {
                            Dynamic::from_ast(&li.le(&ri))
                        } else { Dynamic::from_ast(&Bool::from_bool(self.ctx, false)) }
                    }
                }
            }

            VerifyExpr::Not(inner) => {
                let i = self.encode(inner);
                if let Some(b) = i.as_bool() {
                    Dynamic::from_ast(&b.not())
                } else {
                    i
                }
            }

            VerifyExpr::Iff(l, r) => {
                let lb = self.encode(l);
                let rb = self.encode(r);
                if let (Some(lbool), Some(rbool)) = (lb.as_bool(), rb.as_bool()) {
                    Dynamic::from_ast(&lbool.iff(&rbool))
                } else {
                    Dynamic::from_ast(&lb._eq(&rb))
                }
            }

            // Uninterpreted functions: create unique Z3 function symbol per name
            VerifyExpr::Apply { name, args } => {
                let encoded_args: Vec<Dynamic<'ctx>> = args.iter().map(|a| self.encode(a)).collect();
                // Create a unique uninterpreted boolean function for each name
                let arg_sorts: Vec<z3::Sort<'ctx>> = encoded_args.iter().map(|a| a.get_sort()).collect();
                let arg_sort_refs: Vec<&z3::Sort<'ctx>> = arg_sorts.iter().collect();
                let bool_sort = z3::Sort::bool(self.ctx);
                let func_decl = z3::FuncDecl::new(self.ctx, name.as_str(), &arg_sort_refs, &bool_sort);
                let arg_refs: Vec<&dyn z3::ast::Ast<'ctx>> = encoded_args.iter().map(|a| a as &dyn z3::ast::Ast<'ctx>).collect();
                func_decl.apply(&arg_refs)
            }

            // Quantifiers: proper Z3 quantifier encoding
            VerifyExpr::ForAll { vars, body } => {
                if vars.is_empty() {
                    return self.encode(body);
                }
                let body_encoded = self.encode_as_bool(body);
                let bound_consts: Vec<Dynamic<'ctx>> = vars.iter().map(|(name, ty)| {
                    self.make_quantifier_var(name, ty)
                }).collect();
                let bound_refs: Vec<&dyn Ast<'ctx>> = bound_consts.iter().map(|d| d as &dyn Ast<'ctx>).collect();
                Dynamic::from_ast(&z3::ast::forall_const(self.ctx, &bound_refs, &[], &body_encoded))
            }

            VerifyExpr::Exists { vars, body } => {
                if vars.is_empty() {
                    return self.encode(body);
                }
                let body_encoded = self.encode_as_bool(body);
                let bound_consts: Vec<Dynamic<'ctx>> = vars.iter().map(|(name, ty)| {
                    self.make_quantifier_var(name, ty)
                }).collect();
                let bound_refs: Vec<&dyn Ast<'ctx>> = bound_consts.iter().map(|d| d as &dyn Ast<'ctx>).collect();
                Dynamic::from_ast(&z3::ast::exists_const(self.ctx, &bound_refs, &[], &body_encoded))
            }

            // ---- Bitvector operations ----

            VerifyExpr::BitVecConst { width, value } => {
                Dynamic::from_ast(&z3::ast::BV::from_u64(self.ctx, *value, *width))
            }

            VerifyExpr::BitVecBinary { op, left, right } => {
                let l = self.encode(left);
                let r = self.encode(right);
                self.encode_bv_binary(op, l, r)
            }

            VerifyExpr::BitVecExtract { high, low, operand } => {
                let bv = self.encode(operand);
                if let Some(bv) = bv.as_bv() {
                    Dynamic::from_ast(&bv.extract(*high, *low))
                } else {
                    bv
                }
            }

            VerifyExpr::BitVecConcat(left, right) => {
                let l = self.encode(left);
                let r = self.encode(right);
                if let (Some(lb), Some(rb)) = (l.as_bv(), r.as_bv()) {
                    Dynamic::from_ast(&lb.concat(&rb))
                } else {
                    l
                }
            }

            // ---- Array theory ----

            VerifyExpr::Select { array, index } => {
                let a = self.encode(array);
                let i = self.encode(index);
                if let Some(arr) = a.as_array() {
                    Dynamic::from_ast(&arr.select(&i))
                } else {
                    a
                }
            }

            VerifyExpr::Store { array, index, value } => {
                let a = self.encode(array);
                let i = self.encode(index);
                let v = self.encode(value);
                if let Some(arr) = a.as_array() {
                    Dynamic::from_ast(&arr.store(&i, &v))
                } else {
                    a
                }
            }

            // ---- Temporal (BMC) ----

            VerifyExpr::AtState { expr, .. } => {
                self.encode(expr)
            }

            VerifyExpr::Transition { from, to } => {
                let f = self.encode(from);
                let t = self.encode(to);
                if let (Some(fb), Some(tb)) = (f.as_bool(), t.as_bool()) {
                    Dynamic::from_ast(&Bool::and(self.ctx, &[&fb, &tb]))
                } else {
                    f
                }
            }
        }
    }

    /// Create a Z3 constant for a quantifier-bound variable with the correct sort.
    fn make_quantifier_var(&self, name: &str, ty: &VerifyType) -> Dynamic<'ctx> {
        match ty {
            VerifyType::Int => Dynamic::from_ast(&Int::new_const(self.ctx, name)),
            VerifyType::Bool => Dynamic::from_ast(&Bool::new_const(self.ctx, name)),
            VerifyType::BitVector(w) => Dynamic::from_ast(&z3::ast::BV::new_const(self.ctx, name, *w)),
            VerifyType::Object => Dynamic::from_ast(&Int::new_const(self.ctx, name)),
            VerifyType::Real => Dynamic::from_ast(&z3::ast::Real::new_const(self.ctx, name)),
            VerifyType::Array(idx, elem) => {
                let idx_sort = self.type_to_sort(idx);
                let elem_sort = self.type_to_sort(elem);
                Dynamic::from_ast(&z3::ast::Array::new_const(self.ctx, name, &idx_sort, &elem_sort))
            }
        }
    }

    /// Convert a VerifyType to a Z3 Sort.
    fn type_to_sort(&self, ty: &VerifyType) -> z3::Sort<'ctx> {
        match ty {
            VerifyType::Int => z3::Sort::int(self.ctx),
            VerifyType::Bool => z3::Sort::bool(self.ctx),
            VerifyType::Object => z3::Sort::int(self.ctx),
            VerifyType::Real => z3::Sort::real(self.ctx),
            VerifyType::BitVector(w) => z3::Sort::bitvector(self.ctx, *w),
            VerifyType::Array(idx, elem) => {
                let idx_sort = self.type_to_sort(idx);
                let elem_sort = self.type_to_sort(elem);
                z3::Sort::array(self.ctx, &idx_sort, &elem_sort)
            }
        }
    }

    /// Encode a bitvector binary operation.
    fn encode_bv_binary(&self, op: &BitVecOp, l: Dynamic<'ctx>, r: Dynamic<'ctx>) -> Dynamic<'ctx> {
        if let (Some(lb), Some(rb)) = (l.as_bv(), r.as_bv()) {
            match op {
                BitVecOp::And => Dynamic::from_ast(&lb.bvand(&rb)),
                BitVecOp::Or => Dynamic::from_ast(&lb.bvor(&rb)),
                BitVecOp::Xor => Dynamic::from_ast(&lb.bvxor(&rb)),
                BitVecOp::Not => Dynamic::from_ast(&lb.bvnot()),
                BitVecOp::Shl => Dynamic::from_ast(&lb.bvshl(&rb)),
                BitVecOp::Shr => Dynamic::from_ast(&lb.bvlshr(&rb)),
                BitVecOp::AShr => Dynamic::from_ast(&lb.bvashr(&rb)),
                BitVecOp::Add => Dynamic::from_ast(&lb.bvadd(&rb)),
                BitVecOp::Sub => Dynamic::from_ast(&lb.bvsub(&rb)),
                BitVecOp::Mul => Dynamic::from_ast(&lb.bvmul(&rb)),
                BitVecOp::ULt => Dynamic::from_ast(&lb.bvult(&rb)),
                BitVecOp::SLt => Dynamic::from_ast(&lb.bvslt(&rb)),
                BitVecOp::ULe => Dynamic::from_ast(&lb.bvule(&rb)),
                BitVecOp::SLe => Dynamic::from_ast(&lb.bvsle(&rb)),
                BitVecOp::Eq => Dynamic::from_ast(&lb._eq(&rb)),
            }
        } else {
            l
        }
    }

    /// Encode as a Z3 Bool, coercing if necessary.
    pub fn encode_as_bool(&self, expr: &VerifyExpr) -> Bool<'ctx> {
        let dyn_expr = self.encode(expr);
        dyn_expr.as_bool().unwrap_or_else(|| {
            // If we got an Int or other type, it can't be directly used as Bool.
            // Fail closed: false, not true. Unsupported constructs must NOT
            // silently become equivalent to anything (Sprint 0A consistency).
            Bool::from_bool(self.ctx, false)
        })
    }
}

/// Extract a counterexample trace from a Z3 model.
fn extract_trace<'ctx>(
    ctx: &'ctx Context,
    model: &z3::Model<'ctx>,
    signals: &[String],
    bound: usize,
    bool_vars: &HashMap<String, Bool<'ctx>>,
    int_vars: &HashMap<String, Int<'ctx>>,
    bv_vars: &HashMap<String, z3::ast::BV<'ctx>>,
) -> Trace {
    let mut cycles = Vec::new();
    for t in 0..=bound {
        let mut signal_values = HashMap::new();
        for sig in signals {
            let var_name = format!("{}@{}", sig, t);
            // Try bitvector first, then int, then bool
            if let Some(z3_var) = bv_vars.get(&var_name) {
                if let Some(eval) = model.eval(z3_var, true) {
                    let val = eval.as_u64().unwrap_or(0);
                    let width = z3_var.get_size();
                    signal_values.insert(sig.clone(), SignalValue::BitVec { width, value: val });
                }
                continue;
            }
            if let Some(z3_var) = int_vars.get(&var_name) {
                if let Some(eval) = model.eval(z3_var, true) {
                    if let Some(n) = eval.as_i64() {
                        signal_values.insert(sig.clone(), SignalValue::Int(n));
                    }
                }
                continue;
            }
            if let Some(z3_var) = bool_vars.get(&var_name) {
                let value = model.eval(z3_var, true)
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                signal_values.insert(sig.clone(), SignalValue::Bool(value));
            }
        }
        if !signal_values.is_empty() {
            cycles.push(CycleState {
                cycle: t,
                signals: signal_values,
            });
        }
    }
    Trace { cycles }
}
