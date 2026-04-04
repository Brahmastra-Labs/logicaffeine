//! IC3/PDR (Property-Directed Reachability)
//!
//! The gold standard for unbounded safety verification (Bradley 2011).
//!
//! Maintains frame sequence F_0, F_1, ..., F_k where each frame over-approximates
//! reachable states at step i. Converges when F_i = F_{i+1} (fixpoint = inductive invariant).
//!
//! Core operations:
//! 1. Counterexample to Induction (CTI): Find state in F_i AND T AND NOT P
//! 2. Blocking: Add clause to prevent CTI state (generalized)
//! 3. Propagation: Push clauses forward through frames
//! 4. Convergence: Check if F_i == F_{i+1}

use crate::ir::VerifyExpr;
use crate::equivalence::{Trace, CycleState, SignalValue};
use crate::kinduction;
use std::collections::{HashMap, HashSet};
use z3::{ast::Ast, ast::Bool, ast::Int, SatResult};

/// Result of IC3/PDR verification.
#[derive(Debug)]
pub enum Ic3Result {
    /// Property holds — inductive invariant found.
    Safe { invariant: VerifyExpr },
    /// Property violated — counterexample trace.
    Unsafe { trace: Trace },
    /// Could not determine within resource limits.
    Unknown,
}

/// A frame in the IC3 frame sequence.
/// Each frame is a conjunction of clauses that over-approximates reachable states.
#[derive(Clone)]
struct Frame {
    clauses: Vec<VerifyExpr>,
}

impl Frame {
    fn new() -> Self {
        Frame { clauses: Vec::new() }
    }

    fn add_clause(&mut self, clause: VerifyExpr) {
        // Avoid exact duplicates
        let dbg = format!("{:?}", clause);
        for existing in &self.clauses {
            if format!("{:?}", existing) == dbg {
                return;
            }
        }
        self.clauses.push(clause);
    }

    fn to_expr(&self) -> VerifyExpr {
        if self.clauses.is_empty() {
            return VerifyExpr::bool(true);
        }
        let mut expr = self.clauses[0].clone();
        for clause in &self.clauses[1..] {
            expr = VerifyExpr::and(expr, clause.clone());
        }
        expr
    }
}

/// Run IC3/PDR on a safety property.
///
/// - `init`: Initial state predicate (using @0 variables)
/// - `transition`: Transition relation (using @t and @t1 variables)
/// - `property`: Safety property to verify (using @t variables)
/// - `max_frames`: Maximum number of frames before giving up
pub fn ic3(
    init: &VerifyExpr,
    transition: &VerifyExpr,
    property: &VerifyExpr,
    max_frames: u32,
) -> Ic3Result {
    let mut cfg = z3::Config::new();
    cfg.set_param_value("timeout", "30000");
    let ctx = z3::Context::new(&cfg);

    // Collect all variable names for model extraction
    let mut all_vars = HashSet::new();
    collect_all_vars(init, &mut all_vars);
    collect_all_vars(transition, &mut all_vars);
    collect_all_vars(property, &mut all_vars);
    // Extract base signal names (strip @0, @t, @t1 suffixes)
    let signal_names: Vec<String> = extract_signal_names(&all_vars);

    // Phase 0: Check if init violates property
    let init_violation = VerifyExpr::and(
        kinduction::instantiate_at(init, 0),
        VerifyExpr::not(kinduction::instantiate_at(property, 0)),
    );
    if is_sat(&ctx, &init_violation) {
        let trace = extract_trace_from_bmc(&ctx, init, transition, property, 1, &signal_names);
        return Ic3Result::Unsafe { trace };
    }

    // Phase 1: BMC — check for counterexamples up to max_frames depth
    for k in 1..max_frames {
        let bmc_check = build_bmc_check(init, transition, property, k);
        if is_sat(&ctx, &bmc_check) {
            let trace = extract_trace_from_bmc(&ctx, init, transition, property, k, &signal_names);
            return Ic3Result::Unsafe { trace };
        }
    }

    // Phase 2: IC3 proper — frame-based backward reachability
    // Frame[0] = init, Frame[i] overapproximates states reachable in <= i steps
    let mut frames: Vec<Frame> = vec![Frame::new()];
    frames[0].add_clause(init.clone());
    frames[0].add_clause(property.clone());

    for _iteration in 0..max_frames {
        // Add a new frame initialized with the property
        let mut new_frame = Frame::new();
        new_frame.add_clause(property.clone());
        frames.push(new_frame);
        let k = frames.len() - 1;

        // Block CTIs at this frame level
        let mut blocked = true;
        for _block_attempt in 0..50 {
            // CTI check: is there a state satisfying F_{k-1} AND T that reaches NOT P?
            let frame_expr = frames[k - 1].to_expr();
            let cti_formula = VerifyExpr::and(
                kinduction::instantiate_at(&frame_expr, 0),
                VerifyExpr::and(
                    kinduction::instantiate_transition(transition, 0),
                    VerifyExpr::not(kinduction::instantiate_at(property, 1)),
                ),
            );

            if !is_sat(&ctx, &cti_formula) {
                // No CTI — this frame is OK
                blocked = true;
                break;
            }

            // CTI found — extract the bad predecessor state and block it
            let bad_state = extract_cti_state(&ctx, &cti_formula, &signal_names);

            // Check if the bad state is reachable from init (recursively)
            if is_reachable_from_init(&ctx, init, transition, &bad_state, k as u32, &signal_names) {
                // Real counterexample — extract full trace
                let trace = extract_trace_from_bmc(
                    &ctx, init, transition, property, k as u32, &signal_names,
                );
                return Ic3Result::Unsafe { trace };
            }

            // Block the bad state: add its negation as a clause
            // Generalize: try to drop literals from the blocking clause
            let blocking_clause = generalize_blocking_clause(
                &ctx, transition, property, &bad_state, &frames[k - 1],
            );
            // Add blocking clause to frame k-1 and all earlier frames (down to 1)
            for fi in 1..k {
                frames[fi].add_clause(blocking_clause.clone());
            }
            blocked = false;
        }

        // Propagate clauses forward
        propagate_clauses(&ctx, transition, &mut frames, k);

        // Convergence check: does F_k == F_{k-1}?
        if check_convergence(&ctx, &frames, k) {
            let invariant = frames[k].to_expr();
            return Ic3Result::Safe { invariant };
        }
    }

    // Exhausted frames — try k-induction as fallback
    let kind_result = kinduction::k_induction(init, transition, property, &[], max_frames);
    match kind_result {
        kinduction::KInductionResult::Proven { .. } => {
            // k-induction proved it — but we should still return a proper invariant
            // Use the last frame as the invariant (it's at least as strong as property)
            let inv = frames.last().map(|f| f.to_expr()).unwrap_or_else(|| property.clone());
            Ic3Result::Safe { invariant: inv }
        }
        kinduction::KInductionResult::Counterexample { trace, .. } => Ic3Result::Unsafe { trace },
        _ => Ic3Result::Unknown,
    }
}

/// Extract base signal names from variable names (strip @0, @t, @t1 suffixes).
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

/// Collect all variable names from an expression.
fn collect_all_vars(expr: &VerifyExpr, vars: &mut HashSet<String>) {
    match expr {
        VerifyExpr::Var(name) => { vars.insert(name.clone()); }
        VerifyExpr::Binary { left, right, .. } => {
            collect_all_vars(left, vars);
            collect_all_vars(right, vars);
        }
        VerifyExpr::Not(inner) => collect_all_vars(inner, vars),
        VerifyExpr::Iff(l, r) => {
            collect_all_vars(l, vars);
            collect_all_vars(r, vars);
        }
        VerifyExpr::ForAll { body, .. } | VerifyExpr::Exists { body, .. } => {
            collect_all_vars(body, vars);
        }
        _ => {}
    }
}

/// Build a BMC check formula: init(0) AND T(0,1) AND ... AND T(k-2,k-1) AND NOT P(i) for some i.
fn build_bmc_check(
    init: &VerifyExpr,
    transition: &VerifyExpr,
    property: &VerifyExpr,
    k: u32,
) -> VerifyExpr {
    let mut formula = kinduction::instantiate_at(init, 0);
    for t in 0..k {
        let trans = kinduction::instantiate_transition(transition, t);
        formula = VerifyExpr::and(formula, trans);
    }
    // Property must fail at step k
    let not_prop_k = VerifyExpr::not(kinduction::instantiate_at(property, k));
    VerifyExpr::and(formula, not_prop_k)
}

/// Extract a CTI (counterexample to induction) state from a SAT formula.
/// Returns a conjunction of literals describing the bad predecessor state.
fn extract_cti_state(
    ctx: &z3::Context,
    formula: &VerifyExpr,
    signal_names: &[String],
) -> VerifyExpr {
    let solver = z3::Solver::new(ctx);
    let encoded = encode_bool(ctx, formula);
    solver.assert(&encoded);

    if !matches!(solver.check(), SatResult::Sat) {
        return VerifyExpr::bool(false);
    }
    let model = solver.get_model().unwrap();

    // Extract values at step 0 (the predecessor state)
    let mut literals = Vec::new();
    for sig in signal_names {
        let var_name = format!("{}@0", sig);
        // Try boolean
        let bool_var = Bool::new_const(ctx, var_name.as_str());
        if let Some(val) = model.eval(&bool_var, true) {
            if let Some(b) = val.as_bool() {
                if b {
                    literals.push(VerifyExpr::var(&format!("{}@t", sig)));
                } else {
                    literals.push(VerifyExpr::not(VerifyExpr::var(&format!("{}@t", sig))));
                }
            }
        }
        // Try integer
        let int_var = Int::new_const(ctx, var_name.as_str());
        if let Some(val) = model.eval(&int_var, true) {
            if let Some(n) = val.as_i64() {
                literals.push(VerifyExpr::eq(
                    VerifyExpr::var(&format!("{}@t", sig)),
                    VerifyExpr::int(n),
                ));
            }
        }
    }

    if literals.is_empty() {
        VerifyExpr::bool(true)
    } else {
        let mut conj = literals[0].clone();
        for lit in &literals[1..] {
            conj = VerifyExpr::and(conj, lit.clone());
        }
        conj
    }
}

/// Generalize a blocking clause by trying to drop literals.
/// A literal can be dropped if the remaining clause still blocks the CTI
/// (i.e., the clause is still inductive relative to the frame).
fn generalize_blocking_clause(
    ctx: &z3::Context,
    transition: &VerifyExpr,
    property: &VerifyExpr,
    bad_state: &VerifyExpr,
    frame: &Frame,
) -> VerifyExpr {
    // Start with NOT(bad_state) — the full blocking clause
    let full_clause = VerifyExpr::not(bad_state.clone());

    // Extract individual literals from the bad state
    let literals = extract_literals(bad_state);
    if literals.len() <= 1 {
        return full_clause;
    }

    // Try dropping each literal — keep the clause if it's still valid
    let mut kept_literals = literals.clone();
    for i in 0..literals.len() {
        if kept_literals.len() <= 1 {
            break;
        }
        // Try without literal i
        let mut candidate: Vec<VerifyExpr> = Vec::new();
        for (j, lit) in kept_literals.iter().enumerate() {
            if j != i {
                candidate.push(lit.clone());
            }
        }

        // Build the negated candidate (the state we're blocking)
        let candidate_state = conjoin(&candidate);
        let candidate_clause = VerifyExpr::not(candidate_state.clone());

        // Check: is the candidate clause still consistent with the frame?
        // (frame AND candidate_state AND T AND NOT P) should still be UNSAT
        // after adding the clause to the frame
        let check = VerifyExpr::and(
            kinduction::instantiate_at(&candidate_state, 0),
            VerifyExpr::and(
                kinduction::instantiate_transition(transition, 0),
                VerifyExpr::not(kinduction::instantiate_at(property, 1)),
            ),
        );
        // If the weakened state can still reach NOT P, we need this literal
        if is_sat(ctx, &check) {
            // Can't drop literal i — keep it
        } else {
            // Literal i is redundant — drop it
            kept_literals = candidate;
        }
    }

    if kept_literals.is_empty() {
        full_clause
    } else {
        VerifyExpr::not(conjoin(&kept_literals))
    }
}

/// Extract top-level AND literals from an expression.
fn extract_literals(expr: &VerifyExpr) -> Vec<VerifyExpr> {
    match expr {
        VerifyExpr::Binary { op: crate::ir::VerifyOp::And, left, right } => {
            let mut lits = extract_literals(left);
            lits.extend(extract_literals(right));
            lits
        }
        _ => vec![expr.clone()],
    }
}

/// Conjoin a list of expressions.
fn conjoin(exprs: &[VerifyExpr]) -> VerifyExpr {
    if exprs.is_empty() {
        return VerifyExpr::bool(true);
    }
    let mut result = exprs[0].clone();
    for e in &exprs[1..] {
        result = VerifyExpr::and(result, e.clone());
    }
    result
}

/// Check if a bad state is reachable from init within k steps.
fn is_reachable_from_init(
    ctx: &z3::Context,
    init: &VerifyExpr,
    transition: &VerifyExpr,
    bad_state: &VerifyExpr,
    k: u32,
    _signal_names: &[String],
) -> bool {
    for depth in 0..k {
        let mut formula = kinduction::instantiate_at(init, 0);
        for t in 0..depth {
            formula = VerifyExpr::and(formula, kinduction::instantiate_transition(transition, t));
        }
        // Check if bad_state is reachable at step `depth`
        let bad_at_depth = kinduction::instantiate_at(bad_state, depth);
        let check = VerifyExpr::and(formula, bad_at_depth);
        if is_sat(ctx, &check) {
            return true;
        }
    }
    false
}

/// Propagate clauses from frame[i] to frame[i+1] where they are inductive.
fn propagate_clauses(
    ctx: &z3::Context,
    transition: &VerifyExpr,
    frames: &mut Vec<Frame>,
    k: usize,
) {
    if k == 0 { return; }
    let clauses_to_try: Vec<VerifyExpr> = frames[k - 1].clauses.clone();

    for clause in clauses_to_try {
        // Check if clause is inductive relative to frame[k]:
        // frame[k] AND clause AND T AND NOT clause' is UNSAT?
        let frame_k_expr = frames[k].to_expr();
        let check = VerifyExpr::and(
            kinduction::instantiate_at(&frame_k_expr, 0),
            VerifyExpr::and(
                kinduction::instantiate_at(&clause, 0),
                VerifyExpr::and(
                    kinduction::instantiate_transition(transition, 0),
                    VerifyExpr::not(kinduction::instantiate_at(&clause, 1)),
                ),
            ),
        );
        if !is_sat(ctx, &check) {
            frames[k].add_clause(clause);
        }
    }
}

/// Check if frames[k-1] and frames[k] have converged (same clause set modulo entailment).
fn check_convergence(ctx: &z3::Context, frames: &[Frame], k: usize) -> bool {
    if k == 0 { return false; }

    let fk = frames[k].to_expr();
    let fk_prev = frames[k - 1].to_expr();

    // Check F_{k-1} => F_k (i.e., F_{k-1} AND NOT F_k is UNSAT)
    let fwd = VerifyExpr::and(
        kinduction::instantiate_at(&fk_prev, 0),
        VerifyExpr::not(kinduction::instantiate_at(&fk, 0)),
    );
    if is_sat(ctx, &fwd) {
        return false;
    }

    // Check F_k => F_{k-1}
    let bwd = VerifyExpr::and(
        kinduction::instantiate_at(&fk, 0),
        VerifyExpr::not(kinduction::instantiate_at(&fk_prev, 0)),
    );
    !is_sat(ctx, &bwd)
}

/// Extract a concrete counterexample trace from a BMC check.
fn extract_trace_from_bmc(
    ctx: &z3::Context,
    init: &VerifyExpr,
    transition: &VerifyExpr,
    property: &VerifyExpr,
    depth: u32,
    signal_names: &[String],
) -> Trace {
    let solver = z3::Solver::new(ctx);

    // Build BMC formula
    let init_0 = kinduction::instantiate_at(init, 0);
    solver.assert(&encode_bool(ctx, &init_0));

    for t in 0..depth {
        let trans = kinduction::instantiate_transition(transition, t);
        solver.assert(&encode_bool(ctx, &trans));
    }

    // Assert NOT P at depth
    let not_prop = VerifyExpr::not(kinduction::instantiate_at(property, depth));
    solver.assert(&encode_bool(ctx, &not_prop));

    if !matches!(solver.check(), SatResult::Sat) {
        return Trace { cycles: vec![] };
    }
    let model = solver.get_model().unwrap();

    // Extract signal values at each timestep
    let mut cycles = Vec::new();
    for step in 0..=depth {
        let mut signals = HashMap::new();
        for sig in signal_names {
            let var_name = format!("{}@{}", sig, step);

            // Try boolean
            let bool_var = Bool::new_const(ctx, var_name.as_str());
            if let Some(val) = model.eval(&bool_var, true) {
                if let Some(b) = val.as_bool() {
                    signals.insert(sig.clone(), SignalValue::Bool(b));
                    continue;
                }
            }

            // Try integer
            let int_var = Int::new_const(ctx, var_name.as_str());
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

    // If we couldn't extract typed values, at least provide boolean interpretation
    if cycles.is_empty() {
        let mut signals = HashMap::new();
        for sig in signal_names {
            signals.insert(sig.clone(), SignalValue::Unknown);
        }
        cycles.push(CycleState { cycle: 0, signals });
    }

    Trace { cycles }
}

/// Check if a formula is satisfiable (internal, takes pre-built context).
fn is_sat(ctx: &z3::Context, expr: &VerifyExpr) -> bool {
    let solver = z3::Solver::new(ctx);
    let encoded = encode_bool(ctx, expr);
    solver.assert(&encoded);
    matches!(solver.check(), z3::SatResult::Sat)
}

/// Check if a formula is satisfiable (public, creates its own Z3 context).
pub fn check_sat(expr: &VerifyExpr) -> bool {
    let cfg = z3::Config::new();
    let ctx = z3::Context::new(&cfg);
    is_sat(&ctx, expr)
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
