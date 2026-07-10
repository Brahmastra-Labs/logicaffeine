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
use z3::{ast::Ast, ast::Bool, ast::Dynamic, ast::Int, SatResult};

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
/// ```
/// use logicaffeine_verify::ir::{VerifyExpr, VerifyOp};
/// use logicaffeine_verify::equivalence::{check_equivalence, EquivalenceResult};
///
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
    let solver = crate::solver::new_solver();

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
            Bool::new_const(var_name.as_str()),
        );
    }
    // Pre-declare integer variables found in Int-typed positions
    collect_int_vars(fol_expr, &mut int_vars);
    collect_int_vars(sva_expr, &mut int_vars);
    // Also collect variables used as array indices/values (they need Int sort)
    collect_array_index_vars(fol_expr, &mut int_vars);
    collect_array_index_vars(sva_expr, &mut int_vars);
    // Collect quantifier-bound variables with their declared types
    collect_quantifier_bound_vars(fol_expr, &mut int_vars, &mut bv_vars, &mut array_vars);
    collect_quantifier_bound_vars(sva_expr, &mut int_vars, &mut bv_vars, &mut array_vars);
    // Pre-declare bitvector variables found in BV-typed positions
    collect_bv_vars(fol_expr, &mut bv_vars);
    collect_bv_vars(sva_expr, &mut bv_vars);
    // Pre-declare array variables found in array-typed positions
    collect_array_vars(fol_expr, &mut array_vars);
    collect_array_vars(sva_expr, &mut array_vars);

    // 3. Encode both expressions into Z3 using the dynamic encoder
    let encoder = EquivEncoder {
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
        fol_z3.eq(&sva_z3).not()
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
            let trace = extract_trace(&model, signals, bound, &bool_vars, &int_vars, &bv_vars);
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
pub fn collect_int_vars_pub(expr: &VerifyExpr, int_vars: &mut HashMap<String, Int>) {
    collect_int_vars(expr, int_vars);
}

/// Collect quantifier-bound variables from a VerifyExpr (public API).
pub fn collect_quantifier_bound_vars_pub(
    expr: &VerifyExpr,
    int_vars: &mut HashMap<String, Int>,
    bv_vars: &mut HashMap<String, z3::ast::BV>,
    array_vars: &mut HashMap<String, z3::ast::Array>,
) {
    collect_quantifier_bound_vars(expr, int_vars, bv_vars, array_vars);
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
        VerifyExpr::Apply { args, .. } | VerifyExpr::ApplyInt { args, .. } => {
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
fn collect_int_vars(expr: &VerifyExpr, int_vars: &mut HashMap<String, Int>) {
    match expr {
        VerifyExpr::Int(_) => {}
        VerifyExpr::Bool(_) => {}
        VerifyExpr::Var(_) => {} // Vars are bool by default; promoted to int when used in arith context
        VerifyExpr::Binary { op, left, right } => {
            match op {
                // Arithmetic ops: children are integer-typed
                VerifyOp::Add | VerifyOp::Sub | VerifyOp::Mul | VerifyOp::Div | VerifyOp::FloorDiv => {
                    collect_int_var_leaves(left, int_vars);
                    collect_int_var_leaves(right, int_vars);
                }
                // Comparison ops: children are integer-typed
                VerifyOp::Gt | VerifyOp::Lt | VerifyOp::Gte | VerifyOp::Lte => {
                    collect_int_var_leaves(left, int_vars);
                    collect_int_var_leaves(right, int_vars);
                }
                // Eq/Neq: could be bool or int — check if children look integer
                VerifyOp::Eq | VerifyOp::Neq => {
                    if expr_is_integer(left) || expr_is_integer(right) {
                        collect_int_var_leaves(left, int_vars);
                        collect_int_var_leaves(right, int_vars);
                    }
                }
                _ => {}
            }
            collect_int_vars(left, int_vars);
            collect_int_vars(right, int_vars);
        }
        VerifyExpr::Not(inner) => collect_int_vars(inner, int_vars),
        VerifyExpr::Iff(l, r) => {
            collect_int_vars(l, int_vars);
            collect_int_vars(r, int_vars);
        }
        VerifyExpr::ForAll { body, .. } | VerifyExpr::Exists { body, .. } => {
            collect_int_vars(body, int_vars);
        }
        VerifyExpr::Apply { args, .. } | VerifyExpr::ApplyInt { args, .. } => {
            for arg in args { collect_int_vars(arg, int_vars); }
        }
        _ => {}
    }
}

/// Recursively collect Var leaves from integer-typed subexpressions.
fn collect_int_var_leaves(expr: &VerifyExpr, int_vars: &mut HashMap<String, Int>) {
    match expr {
        VerifyExpr::Var(name) => {
            if !int_vars.contains_key(name) {
                int_vars.insert(name.clone(), Int::new_const(name.as_str()));
            }
        }
        VerifyExpr::Binary { left, right, .. } => {
            collect_int_var_leaves(left, int_vars);
            collect_int_var_leaves(right, int_vars);
        }
        _ => {}
    }
}

/// Collect variable names that appear in bitvector-typed positions.
fn collect_bv_vars(expr: &VerifyExpr, bv_vars: &mut HashMap<String, z3::ast::BV>) {
    match expr {
        VerifyExpr::BitVecBinary { op: _, left, right } => {
            collect_bv_var_leaves(left, bv_vars, None);
            collect_bv_var_leaves(right, bv_vars, None);
            collect_bv_vars(left, bv_vars);
            collect_bv_vars(right, bv_vars);
        }
        VerifyExpr::BitVecExtract { operand, .. } => {
            collect_bv_var_leaves(operand, bv_vars, None);
            collect_bv_vars(operand, bv_vars);
        }
        VerifyExpr::BitVecConcat(l, r) => {
            collect_bv_var_leaves(l, bv_vars, None);
            collect_bv_var_leaves(r, bv_vars, None);
            collect_bv_vars(l, bv_vars);
            collect_bv_vars(r, bv_vars);
        }
        VerifyExpr::Binary { left, right, .. } => {
            collect_bv_vars(left, bv_vars);
            collect_bv_vars(right, bv_vars);
        }
        VerifyExpr::Not(inner) => collect_bv_vars(inner, bv_vars),
        VerifyExpr::Iff(l, r) => {
            collect_bv_vars(l, bv_vars);
            collect_bv_vars(r, bv_vars);
        }
        VerifyExpr::ForAll { body, .. } | VerifyExpr::Exists { body, .. } => {
            collect_bv_vars(body, bv_vars);
        }
        VerifyExpr::Apply { args, .. } | VerifyExpr::ApplyInt { args, .. } => {
            for arg in args { collect_bv_vars(arg, bv_vars); }
        }
        VerifyExpr::Select { array, index } => {
            collect_bv_vars(array, bv_vars);
            collect_bv_vars(index, bv_vars);
        }
        VerifyExpr::Store { array, index, value } => {
            collect_bv_vars(array, bv_vars);
            collect_bv_vars(index, bv_vars);
            collect_bv_vars(value, bv_vars);
        }
        VerifyExpr::AtState { state, expr } => {
            collect_bv_vars(state, bv_vars);
            collect_bv_vars(expr, bv_vars);
        }
        VerifyExpr::Transition { from, to } => {
            collect_bv_vars(from, bv_vars);
            collect_bv_vars(to, bv_vars);
        }
        _ => {}
    }
}

/// Recursively collect Var leaves from bitvector-typed subexpressions.
fn collect_bv_var_leaves(expr: &VerifyExpr, bv_vars: &mut HashMap<String, z3::ast::BV>, width_hint: Option<u32>) {
    match expr {
        VerifyExpr::Var(name) => {
            if !bv_vars.contains_key(name) {
                // Determine width from context or default to 8
                let width = width_hint.unwrap_or_else(|| infer_bv_width_from_name(name));
                bv_vars.insert(name.clone(), z3::ast::BV::new_const(name.as_str(), width));
            }
        }
        VerifyExpr::BitVecConst { width: _, .. } => {
            // A constant tells us the width of sibling vars
            // (handled by the caller)
        }
        VerifyExpr::BitVecBinary { left, right, .. } => {
            // Try to extract width from either side
            let w = bv_expr_width(expr);
            collect_bv_var_leaves(left, bv_vars, w);
            collect_bv_var_leaves(right, bv_vars, w);
        }
        VerifyExpr::Binary { left, right, .. } => {
            collect_bv_var_leaves(left, bv_vars, width_hint);
            collect_bv_var_leaves(right, bv_vars, width_hint);
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
fn collect_array_vars(expr: &VerifyExpr, array_vars: &mut HashMap<String, z3::ast::Array>) {
    collect_array_vars_inner(expr, array_vars);
}

fn collect_array_vars_inner(expr: &VerifyExpr, array_vars: &mut HashMap<String, z3::ast::Array>) {
    match expr {
        VerifyExpr::Select { array, index } => {
            if let VerifyExpr::Var(name) = array.as_ref() {
                if !array_vars.contains_key(name) {
                    let int_sort = z3::Sort::int();
                    array_vars.insert(
                        name.clone(),
                        z3::ast::Array::new_const(name.as_str(), &int_sort, &int_sort),
                    );
                }
            }
            collect_array_vars_inner(array, array_vars);
            collect_array_vars_inner(index, array_vars);
        }
        VerifyExpr::Store { array, index, value } => {
            if let VerifyExpr::Var(name) = array.as_ref() {
                if !array_vars.contains_key(name) {
                    let int_sort = z3::Sort::int();
                    array_vars.insert(
                        name.clone(),
                        z3::ast::Array::new_const(name.as_str(), &int_sort, &int_sort),
                    );
                }
            }
            collect_array_vars_inner(array, array_vars);
            collect_array_vars_inner(index, array_vars);
            collect_array_vars_inner(value, array_vars);
        }
        VerifyExpr::Binary { left, right, .. } => {
            collect_array_vars_inner(left, array_vars);
            collect_array_vars_inner(right, array_vars);
        }
        VerifyExpr::Not(inner) => collect_array_vars_inner(inner, array_vars),
        VerifyExpr::Iff(l, r) => {
            collect_array_vars_inner(l, array_vars);
            collect_array_vars_inner(r, array_vars);
        }
        VerifyExpr::ForAll { body, .. } | VerifyExpr::Exists { body, .. } => {
            collect_array_vars_inner(body, array_vars);
        }
        _ => {}
    }
}

/// Collect variables used as array indices or values (they need Int sort, not Bool).
fn collect_array_index_vars(expr: &VerifyExpr, int_vars: &mut HashMap<String, Int>) {
    match expr {
        VerifyExpr::Select { array, index } => {
            collect_int_var_leaves(index, int_vars);
            collect_array_index_vars(array, int_vars);
            collect_array_index_vars(index, int_vars);
        }
        VerifyExpr::Store { array, index, value } => {
            collect_int_var_leaves(index, int_vars);
            collect_int_var_leaves(value, int_vars);
            collect_array_index_vars(array, int_vars);
            collect_array_index_vars(index, int_vars);
            collect_array_index_vars(value, int_vars);
        }
        VerifyExpr::Binary { left, right, .. } => {
            collect_array_index_vars(left, int_vars);
            collect_array_index_vars(right, int_vars);
        }
        VerifyExpr::Not(inner) => collect_array_index_vars(inner, int_vars),
        VerifyExpr::Iff(l, r) => {
            collect_array_index_vars(l, int_vars);
            collect_array_index_vars(r, int_vars);
        }
        VerifyExpr::ForAll { body, .. } | VerifyExpr::Exists { body, .. } => {
            collect_array_index_vars(body, int_vars);
        }
        _ => {}
    }
}

/// Collect quantifier-bound variables with their declared types.
fn collect_quantifier_bound_vars(
    expr: &VerifyExpr,
    int_vars: &mut HashMap<String, Int>,
    bv_vars: &mut HashMap<String, z3::ast::BV>,
    array_vars: &mut HashMap<String, z3::ast::Array>,
) {
    match expr {
        VerifyExpr::ForAll { vars, body } | VerifyExpr::Exists { vars, body } => {
            for (name, ty) in vars {
                match ty {
                    VerifyType::Int | VerifyType::Object | VerifyType::Real => {
                        if !int_vars.contains_key(name) {
                            int_vars.insert(name.clone(), Int::new_const(name.as_str()));
                        }
                    }
                    VerifyType::BitVector(w) => {
                        if !bv_vars.contains_key(name) {
                            bv_vars.insert(name.clone(), z3::ast::BV::new_const(name.as_str(), *w));
                        }
                    }
                    VerifyType::Array(idx, elem) => {
                        if !array_vars.contains_key(name) {
                            let idx_sort = type_to_z3_sort(idx);
                            let elem_sort = type_to_z3_sort(elem);
                            array_vars.insert(
                                name.clone(),
                                z3::ast::Array::new_const(name.as_str(), &idx_sort, &elem_sort),
                            );
                        }
                    }
                    VerifyType::Bool => {
                        // Bool vars are already declared in bool_vars by default
                    }
                }
            }
            collect_quantifier_bound_vars(body, int_vars, bv_vars, array_vars);
        }
        VerifyExpr::Binary { left, right, .. } => {
            collect_quantifier_bound_vars(left, int_vars, bv_vars, array_vars);
            collect_quantifier_bound_vars(right, int_vars, bv_vars, array_vars);
        }
        VerifyExpr::Not(inner) => {
            collect_quantifier_bound_vars(inner, int_vars, bv_vars, array_vars);
        }
        VerifyExpr::Iff(l, r) => {
            collect_quantifier_bound_vars(l, int_vars, bv_vars, array_vars);
            collect_quantifier_bound_vars(r, int_vars, bv_vars, array_vars);
        }
        _ => {}
    }
}

/// Convert a VerifyType to a Z3 Sort (standalone function).
fn type_to_z3_sort(ty: &VerifyType) -> z3::Sort {
    match ty {
        VerifyType::Int => z3::Sort::int(),
        VerifyType::Bool => z3::Sort::bool(),
        VerifyType::Object => z3::Sort::int(),
        VerifyType::Real => z3::Sort::real(),
        VerifyType::BitVector(w) => z3::Sort::bitvector(*w),
        VerifyType::Array(idx, elem) => {
            let idx_sort = type_to_z3_sort(idx);
            let elem_sort = type_to_z3_sort(elem);
            z3::Sort::array(&idx_sort, &elem_sort)
        }
    }
}

/// Heuristic: does this expression look like it produces an integer?
fn expr_is_integer(expr: &VerifyExpr) -> bool {
    matches!(expr,
        VerifyExpr::Int(_)
        | VerifyExpr::Binary { op: VerifyOp::Add | VerifyOp::Sub | VerifyOp::Mul | VerifyOp::Div | VerifyOp::FloorDiv, .. }
    )
}

/// Encoder that handles Bool, Int, BitVec, and Array Z3 expressions for equivalence checking.
pub struct EquivEncoder<'a> {
    bool_vars: &'a HashMap<String, Bool>,
    int_vars: &'a HashMap<String, Int>,
    bv_vars: &'a HashMap<String, z3::ast::BV>,
    array_vars: &'a HashMap<String, z3::ast::Array>,
}

impl<'a> EquivEncoder<'a> {
    /// Create a full encoder with all variable maps.
    pub fn new(
        bool_vars: &'a HashMap<String, Bool>,
        int_vars: &'a HashMap<String, Int>,
        bv_vars: &'a HashMap<String, z3::ast::BV>,
        array_vars: &'a HashMap<String, z3::ast::Array>,
    ) -> Self {
        Self { bool_vars, int_vars, bv_vars, array_vars }
    }

    /// Encode as a Dynamic Z3 expression (may be Bool or Int).
    fn encode(&self, expr: &VerifyExpr) -> Dynamic {
        match expr {
            VerifyExpr::Bool(b) => Dynamic::from_ast(&Bool::from_bool(*b)),

            VerifyExpr::Int(n) => Dynamic::from_ast(&Int::from_i64(*n)),

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
                    Dynamic::from_ast(&Bool::new_const(name.as_str()))
                }
            }

            VerifyExpr::Binary { op, left, right } => {
                let l = self.encode(left);
                let r = self.encode(right);
                match op {
                    // Boolean ops
                    VerifyOp::And => {
                        if let (Some(lb), Some(rb)) = (l.as_bool(), r.as_bool()) {
                            Dynamic::from_ast(&Bool::and(&[&lb, &rb]))
                        } else {
                            l // fallback
                        }
                    }
                    VerifyOp::Or => {
                        if let (Some(lb), Some(rb)) = (l.as_bool(), r.as_bool()) {
                            Dynamic::from_ast(&Bool::or(&[&lb, &rb]))
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
                            Dynamic::from_ast(&li.eq(&ri))
                        } else if let (Some(lb), Some(rb)) = (l.as_bool(), r.as_bool()) {
                            Dynamic::from_ast(&lb.iff(&rb))
                        } else {
                            Dynamic::from_ast(&l.eq(&r))
                        }
                    }
                    VerifyOp::Neq => {
                        if let (Some(li), Some(ri)) = (l.as_int(), r.as_int()) {
                            Dynamic::from_ast(&li.eq(&ri).not())
                        } else if let (Some(lb), Some(rb)) = (l.as_bool(), r.as_bool()) {
                            Dynamic::from_ast(&lb.iff(&rb).not())
                        } else {
                            Dynamic::from_ast(&l.eq(&r).not())
                        }
                    }
                    // Arithmetic ops → Int
                    VerifyOp::Add => {
                        if let (Some(li), Some(ri)) = (l.as_int(), r.as_int()) {
                            Dynamic::from_ast(&Int::add(&[&li, &ri]))
                        } else { l }
                    }
                    VerifyOp::Sub => {
                        if let (Some(li), Some(ri)) = (l.as_int(), r.as_int()) {
                            Dynamic::from_ast(&Int::sub(&[&li, &ri]))
                        } else { l }
                    }
                    VerifyOp::Mul => {
                        if let (Some(li), Some(ri)) = (l.as_int(), r.as_int()) {
                            Dynamic::from_ast(&Int::mul(&[&li, &ri]))
                        } else { l }
                    }
                    VerifyOp::Div => {
                        if let (Some(li), Some(ri)) = (l.as_int(), r.as_int()) {
                            Dynamic::from_ast(&li.div(&ri))
                        } else { l }
                    }
                    // Floor division: real division then floor (`Real::to_int`), exact toward -inf.
                    VerifyOp::FloorDiv => {
                        if let (Some(li), Some(ri)) = (l.as_int(), r.as_int()) {
                            Dynamic::from_ast(&(li.to_real() / ri.to_real()).to_int())
                        } else { l }
                    }
                    // Comparison ops → Bool (from Int operands)
                    VerifyOp::Gt => {
                        if let (Some(li), Some(ri)) = (l.as_int(), r.as_int()) {
                            Dynamic::from_ast(&li.gt(&ri))
                        } else { Dynamic::from_ast(&Bool::from_bool(false)) }
                    }
                    VerifyOp::Lt => {
                        if let (Some(li), Some(ri)) = (l.as_int(), r.as_int()) {
                            Dynamic::from_ast(&li.lt(&ri))
                        } else { Dynamic::from_ast(&Bool::from_bool(false)) }
                    }
                    VerifyOp::Gte => {
                        if let (Some(li), Some(ri)) = (l.as_int(), r.as_int()) {
                            Dynamic::from_ast(&li.ge(&ri))
                        } else { Dynamic::from_ast(&Bool::from_bool(false)) }
                    }
                    VerifyOp::Lte => {
                        if let (Some(li), Some(ri)) = (l.as_int(), r.as_int()) {
                            Dynamic::from_ast(&li.le(&ri))
                        } else { Dynamic::from_ast(&Bool::from_bool(false)) }
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
                    Dynamic::from_ast(&lb.eq(&rb))
                }
            }

            VerifyExpr::ApplyInt { name, args } => {
                let encoded_args: Vec<Dynamic> = args.iter().map(|a| self.encode(a)).collect();
                let arg_sorts: Vec<z3::Sort> = encoded_args.iter().map(|a| a.get_sort()).collect();
                let arg_sort_refs: Vec<&z3::Sort> = arg_sorts.iter().collect();
                let int_sort = z3::Sort::int();
                let func_decl = z3::FuncDecl::new(name.as_str(), &arg_sort_refs, &int_sort);
                let arg_refs: Vec<&dyn z3::ast::Ast> = encoded_args.iter().map(|a| a as &dyn z3::ast::Ast).collect();
                func_decl.apply(&arg_refs)
            }

            // Uninterpreted functions: create unique Z3 function symbol per name
            VerifyExpr::Apply { name, args } => {
                let encoded_args: Vec<Dynamic> = args.iter().map(|a| self.encode(a)).collect();
                // Create a unique uninterpreted boolean function for each name
                let arg_sorts: Vec<z3::Sort> = encoded_args.iter().map(|a| a.get_sort()).collect();
                let arg_sort_refs: Vec<&z3::Sort> = arg_sorts.iter().collect();
                let bool_sort = z3::Sort::bool();
                let func_decl = z3::FuncDecl::new(name.as_str(), &arg_sort_refs, &bool_sort);
                let arg_refs: Vec<&dyn z3::ast::Ast> = encoded_args.iter().map(|a| a as &dyn z3::ast::Ast).collect();
                func_decl.apply(&arg_refs)
            }

            // Quantifiers: proper Z3 quantifier encoding
            VerifyExpr::ForAll { vars, body } => {
                if vars.is_empty() {
                    return self.encode(body);
                }
                let body_encoded = self.encode_as_bool(body);
                let bound_consts: Vec<Dynamic> = vars.iter().map(|(name, ty)| {
                    self.make_quantifier_var(name, ty)
                }).collect();
                let bound_refs: Vec<&dyn Ast> = bound_consts.iter().map(|d| d as &dyn Ast).collect();
                Dynamic::from_ast(&z3::ast::forall_const(&bound_refs, &[], &body_encoded))
            }

            VerifyExpr::Exists { vars, body } => {
                if vars.is_empty() {
                    return self.encode(body);
                }
                let body_encoded = self.encode_as_bool(body);
                let bound_consts: Vec<Dynamic> = vars.iter().map(|(name, ty)| {
                    self.make_quantifier_var(name, ty)
                }).collect();
                let bound_refs: Vec<&dyn Ast> = bound_consts.iter().map(|d| d as &dyn Ast).collect();
                Dynamic::from_ast(&z3::ast::exists_const(&bound_refs, &[], &body_encoded))
            }

            // ---- Bitvector operations ----

            VerifyExpr::BitVecConst { width, value } => {
                Dynamic::from_ast(&z3::ast::BV::from_u64(*value, *width))
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
                    Dynamic::from_ast(&Bool::and(&[&fb, &tb]))
                } else {
                    f
                }
            }
        }
    }

    /// Create a Z3 constant for a quantifier-bound variable with the correct sort.
    fn make_quantifier_var(&self, name: &str, ty: &VerifyType) -> Dynamic {
        match ty {
            VerifyType::Int => Dynamic::from_ast(&Int::new_const(name)),
            VerifyType::Bool => Dynamic::from_ast(&Bool::new_const(name)),
            VerifyType::BitVector(w) => Dynamic::from_ast(&z3::ast::BV::new_const(name, *w)),
            VerifyType::Object => Dynamic::from_ast(&Int::new_const(name)),
            VerifyType::Real => Dynamic::from_ast(&z3::ast::Real::new_const(name)),
            VerifyType::Array(idx, elem) => {
                let idx_sort = self.type_to_sort(idx);
                let elem_sort = self.type_to_sort(elem);
                Dynamic::from_ast(&z3::ast::Array::new_const(name, &idx_sort, &elem_sort))
            }
        }
    }

    /// Convert a VerifyType to a Z3 Sort.
    fn type_to_sort(&self, ty: &VerifyType) -> z3::Sort {
        match ty {
            VerifyType::Int => z3::Sort::int(),
            VerifyType::Bool => z3::Sort::bool(),
            VerifyType::Object => z3::Sort::int(),
            VerifyType::Real => z3::Sort::real(),
            VerifyType::BitVector(w) => z3::Sort::bitvector(*w),
            VerifyType::Array(idx, elem) => {
                let idx_sort = self.type_to_sort(idx);
                let elem_sort = self.type_to_sort(elem);
                z3::Sort::array(&idx_sort, &elem_sort)
            }
        }
    }

    /// Encode a bitvector binary operation.
    fn encode_bv_binary(&self, op: &BitVecOp, l: Dynamic, r: Dynamic) -> Dynamic {
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
                BitVecOp::SDiv => Dynamic::from_ast(&lb.bvsdiv(&rb)),
                BitVecOp::SRem => Dynamic::from_ast(&lb.bvsrem(&rb)),
                BitVecOp::ULt => Dynamic::from_ast(&lb.bvult(&rb)),
                BitVecOp::SLt => Dynamic::from_ast(&lb.bvslt(&rb)),
                BitVecOp::ULe => Dynamic::from_ast(&lb.bvule(&rb)),
                BitVecOp::SLe => Dynamic::from_ast(&lb.bvsle(&rb)),
                BitVecOp::Eq => Dynamic::from_ast(&lb.eq(&rb)),
            }
        } else {
            l
        }
    }

    /// Encode as a Z3 Bool, coercing if necessary.
    pub fn encode_as_bool(&self, expr: &VerifyExpr) -> Bool {
        let dyn_expr = self.encode(expr);
        dyn_expr.as_bool().unwrap_or_else(|| {
            // If we got an Int or other type, it can't be directly used as Bool.
            // Fail closed: false, not true. Unsupported constructs must NOT
            // silently become equivalent to anything (Sprint 0A consistency).
            Bool::from_bool(false)
        })
    }
}

/// Extract a counterexample trace from a Z3 model.
fn extract_trace(
    model: &z3::Model,
    signals: &[String],
    bound: usize,
    bool_vars: &HashMap<String, Bool>,
    int_vars: &HashMap<String, Int>,
    bv_vars: &HashMap<String, z3::ast::BV>,
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
