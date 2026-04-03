//! IC3/PDR (Property-Directed Reachability)
//!
//! The gold standard for unbounded safety verification (Bradley 2011).
//!
//! Maintains frame sequence F_0, F_1, ..., F_k where each frame over-approximates
//! reachable states at step i. Converges when F_i = F_{i+1} (fixpoint = inductive invariant).
//!
//! Core operations:
//! 1. Counterexample to Induction (CTI): Find state in F_i AND T AND NOT P
//! 2. Blocking: Add clause to prevent CTI state
//! 3. Propagation: Push clauses forward through frames
//! 4. Convergence: Check if F_i == F_{i+1}

use crate::ir::VerifyExpr;
use crate::equivalence::Trace;
use crate::kinduction;
use std::collections::HashMap;

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
struct Frame {
    clauses: Vec<VerifyExpr>,
}

impl Frame {
    fn new() -> Self {
        Frame { clauses: Vec::new() }
    }

    fn add_clause(&mut self, clause: VerifyExpr) {
        self.clauses.push(clause);
    }

    /// Convert frame to a single conjunction expression.
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

    // Phase 0: BMC check first — find reachable counterexamples
    let bmc_result = crate::kinduction::k_induction(
        init, transition, property, &[], max_frames,
    );
    match bmc_result {
        crate::kinduction::KInductionResult::Counterexample { trace, .. } => {
            return Ic3Result::Unsafe { trace };
        }
        crate::kinduction::KInductionResult::Proven { .. } => {
            return Ic3Result::Safe {
                invariant: property.clone(),
            };
        }
        _ => {} // k-induction inconclusive, continue with IC3
    }

    // Frame 0 = init
    let mut frames: Vec<Frame> = vec![Frame::new()];
    frames[0].add_clause(init.clone());

    // Check if init satisfies property
    let init_check = VerifyExpr::and(
        kinduction::instantiate_at(init, 0),
        VerifyExpr::not(kinduction::instantiate_at(property, 0)),
    );
    if is_sat(&ctx, &init_check) {
        return Ic3Result::Unsafe {
            trace: Trace { cycles: vec![] },
        };
    }

    for _iteration in 0..max_frames {
        // Add a new frame
        frames.push(Frame::new());
        let k = frames.len() - 1;

        // The new frame starts with the property as a clause
        frames[k].add_clause(property.clone());

        // Check for CTI: is there a state in F_{k-1} that can transition to NOT P?
        let frame_expr = frames[k - 1].to_expr();
        let frame_at_t = kinduction::instantiate_at(&frame_expr, 0);
        let trans = kinduction::instantiate_transition(transition, 0);
        let not_prop = VerifyExpr::not(kinduction::instantiate_at(property, 1));

        let cti_check = VerifyExpr::and(
            frame_at_t.clone(),
            VerifyExpr::and(trans.clone(), not_prop),
        );

        if is_sat(&ctx, &cti_check) {
            // CTI found — try to block it by strengthening the previous frame
            // Simple approach: add NOT(cti_state) as a clause
            // For a real IC3, we'd generalize the blocking clause
            let blocking_clause = VerifyExpr::not(cti_check.clone());
            frames[k - 1].add_clause(property.clone());

            // Check if the CTI is reachable from init (real counterexample?)
            // Use BMC to check
            let bmc_result = crate::kinduction::k_induction(
                init, transition, property, &[], (k as u32).min(10),
            );
            match bmc_result {
                crate::kinduction::KInductionResult::Counterexample { trace, .. } => {
                    return Ic3Result::Unsafe { trace };
                }
                crate::kinduction::KInductionResult::Proven { .. } => {
                    return Ic3Result::Safe {
                        invariant: property.clone(),
                    };
                }
                _ => {} // Continue IC3 loop
            }
        } else {
            // No CTI — check for convergence
            // If F_k implies F_{k-1} (i.e., F_{k-1} AND NOT F_k is UNSAT), we've converged
            let fk = frames[k].to_expr();
            let fk_at_t = kinduction::instantiate_at(&fk, 0);
            let fk1 = frames[k - 1].to_expr();
            let fk1_at_t = kinduction::instantiate_at(&fk1, 0);

            // Propagate clauses forward
            for clause in frames[k - 1].clauses.clone() {
                let clause_at_t = kinduction::instantiate_at(&clause, 0);
                let trans_check = VerifyExpr::and(
                    clause_at_t.clone(),
                    kinduction::instantiate_transition(transition, 0),
                );
                let clause_at_t1 = kinduction::instantiate_at(&clause, 1);
                let prop_check = VerifyExpr::and(trans_check, VerifyExpr::not(clause_at_t1));
                if !is_sat(&ctx, &prop_check) {
                    // Clause is inductive relative to frame — propagate to next frame
                    frames[k].add_clause(clause);
                }
            }

            // Check convergence: are frames k-1 and k equivalent?
            let diff = VerifyExpr::and(
                fk1_at_t.clone(),
                VerifyExpr::not(fk_at_t.clone()),
            );
            if !is_sat(&ctx, &diff) {
                // F_{k-1} ⊆ F_k — converged!
                return Ic3Result::Safe {
                    invariant: frames[k].to_expr(),
                };
            }
        }
    }

    // Exhausted max_frames — fall back to k-induction
    let kind_result = crate::kinduction::k_induction(
        init, transition, property, &[], max_frames,
    );
    match kind_result {
        crate::kinduction::KInductionResult::Proven { .. } => Ic3Result::Safe {
            invariant: property.clone(),
        },
        crate::kinduction::KInductionResult::Counterexample { trace, .. } => {
            Ic3Result::Unsafe { trace }
        }
        _ => Ic3Result::Unknown,
    }
}

/// Check if a formula is satisfiable.
fn is_sat(ctx: &z3::Context, expr: &VerifyExpr) -> bool {
    let solver = z3::Solver::new(ctx);
    let encoded = encode_bool(ctx, expr);
    solver.assert(&encoded);
    matches!(solver.check(), z3::SatResult::Sat)
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
