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

use crate::ir::{VerifyExpr, VerifyOp, VerifyType};
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
#[derive(Debug)]
pub struct Trace {
    pub cycles: Vec<CycleState>,
}

/// Signal assignments at a single clock cycle.
#[derive(Debug)]
pub struct CycleState {
    pub cycle: usize,
    pub signals: HashMap<String, bool>,
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
    for var_name in &all_vars {
        bool_vars.insert(
            var_name.clone(),
            Bool::new_const(&ctx, var_name.as_str()),
        );
    }
    // Pre-declare integer variables found in Int-typed positions
    collect_int_vars(fol_expr, &mut int_vars, &ctx);
    collect_int_vars(sva_expr, &mut int_vars, &ctx);

    // 3. Encode both expressions into Z3 using the dynamic encoder
    let encoder = EquivEncoder { ctx: &ctx, bool_vars: &bool_vars, int_vars: &int_vars };
    let fol_z3 = encoder.encode_as_bool(fol_expr);
    let sva_z3 = encoder.encode_as_bool(sva_expr);

    // 4. Construct ¬(FOL ↔ SVA) and check satisfiability
    let iff = fol_z3.iff(&sva_z3);
    let not_iff = iff.not();

    solver.assert(&not_iff);

    match solver.check() {
        SatResult::Unsat => {
            // No assignment makes them differ → equivalent
            EquivalenceResult::Equivalent
        }
        SatResult::Sat => {
            // Found an assignment where they differ → extract counterexample
            let model = solver.get_model().unwrap();
            let trace = extract_trace(&ctx, &model, signals, bound, &bool_vars);
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

/// Heuristic: does this expression look like it produces an integer?
fn expr_is_integer(expr: &VerifyExpr) -> bool {
    matches!(expr,
        VerifyExpr::Int(_)
        | VerifyExpr::Binary { op: VerifyOp::Add | VerifyOp::Sub | VerifyOp::Mul | VerifyOp::Div, .. }
    )
}

/// Encoder that handles both Bool and Int Z3 expressions for equivalence checking.
struct EquivEncoder<'ctx> {
    ctx: &'ctx Context,
    bool_vars: &'ctx HashMap<String, Bool<'ctx>>,
    int_vars: &'ctx HashMap<String, Int<'ctx>>,
}

impl<'ctx> EquivEncoder<'ctx> {
    /// Encode as a Dynamic Z3 expression (may be Bool or Int).
    fn encode(&self, expr: &VerifyExpr) -> Dynamic<'ctx> {
        match expr {
            VerifyExpr::Bool(b) => Dynamic::from_ast(&Bool::from_bool(self.ctx, *b)),

            VerifyExpr::Int(n) => Dynamic::from_ast(&Int::from_i64(self.ctx, *n)),

            VerifyExpr::Var(name) => {
                // If this var was seen in an integer context, use Int; otherwise Bool
                if let Some(iv) = self.int_vars.get(name) {
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

            // Quantifiers: encode body (free vars are implicitly universally quantified in Z3)
            VerifyExpr::ForAll { body, .. } => self.encode(body),
            VerifyExpr::Exists { body, .. } => self.encode(body),

            // Bitvector, Array, AtState, Transition: not handled in equivalence checking
            _ => Dynamic::from_ast(&Bool::from_bool(self.ctx, true)),
        }
    }

    /// Encode as a Z3 Bool, coercing if necessary.
    fn encode_as_bool(&self, expr: &VerifyExpr) -> Bool<'ctx> {
        let dyn_expr = self.encode(expr);
        dyn_expr.as_bool().unwrap_or_else(|| {
            // If we got an Int or other type, it can't be directly used as Bool.
            // This shouldn't happen for well-formed equivalence queries.
            Bool::from_bool(self.ctx, true)
        })
    }
}

/// Extract a counterexample trace from a Z3 model.
fn extract_trace<'ctx>(
    ctx: &'ctx Context,
    model: &z3::Model<'ctx>,
    signals: &[String],
    bound: usize,
    var_map: &HashMap<String, Bool<'ctx>>,
) -> Trace {
    let mut cycles = Vec::new();
    for t in 0..=bound {
        let mut signal_values = HashMap::new();
        for sig in signals {
            let var_name = format!("{}@{}", sig, t);
            if let Some(z3_var) = var_map.get(&var_name) {
                let value = model.eval(z3_var, true)
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                signal_values.insert(sig.clone(), value);
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
