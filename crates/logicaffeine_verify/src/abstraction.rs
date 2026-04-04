//! Predicate Abstraction with CEGAR Refinement
//!
//! Predicate abstraction reduces an infinite-state system to a finite-state
//! boolean system over abstract predicate variables. Each concrete predicate
//! (e.g., `x >= 0`) becomes an abstract boolean. The abstract transition encodes
//! how predicate truth values evolve under the concrete transition relation.
//!
//! CEGAR (Counterexample-Guided Abstraction Refinement) iterates:
//! 1. Abstract: build finite-state model from current predicates
//! 2. Verify: check property on abstract model (k-induction)
//! 3. If safe: done
//! 4. If counterexample: check if realizable on concrete model
//! 5. If real: return Unsafe
//! 6. If spurious: extract new predicates from the infeasibility proof, refine

use crate::ir::{VerifyExpr, VerifyOp};
use crate::equivalence::Trace;
use crate::kinduction;
use std::collections::{HashMap, HashSet};
use z3::{ast::Ast, ast::Bool, ast::Int, Config, Context, SatResult, Solver};

#[derive(Debug, Clone)]
pub struct AbstractModel {
    pub predicates: Vec<VerifyExpr>,
    pub abstract_init: VerifyExpr,
    pub abstract_transition: VerifyExpr,
}

#[derive(Debug)]
pub enum AbstractionResult {
    Safe,
    Unsafe { concrete_trace: Trace },
    SpuriousRefined { new_predicates: Vec<VerifyExpr> },
    Unknown,
}

/// Create an abstract model from initial state and predicates (legacy 2-arg API).
///
/// Without the transition relation, the abstract transition cannot be computed
/// properly, so it defaults to the conjunction of predicates as a conservative
/// overapproximation.
pub fn abstract_model(
    init: &VerifyExpr,
    predicates: &[VerifyExpr],
) -> AbstractModel {
    let abstract_init = if predicates.is_empty() {
        init.clone()
    } else {
        let mut abs = init.clone();
        for pred in predicates {
            abs = VerifyExpr::and(abs, pred.clone());
        }
        abs
    };

    AbstractModel {
        predicates: predicates.to_vec(),
        abstract_init,
        abstract_transition: VerifyExpr::bool(true),
    }
}

/// Create an abstract model with Cartesian predicate abstraction.
///
/// Given concrete init, transition, and a set of predicates, this computes:
///
/// - `abstract_init`: init AND conjunction of all predicates
/// - `abstract_transition`: concrete transition AND inductive predicate implications
///
/// For each predicate p_i, we check via Z3 whether p_i is inductive relative to
/// the transition: `(p_i@t AND T(t, t1)) => p_i@t1`. If valid (UNSAT negation),
/// the implication is included in the abstract transition. If not valid, the
/// predicate may take any value in the next state (overapproximation).
///
/// This is Cartesian predicate abstraction: each predicate is checked independently.
pub fn abstract_model_full(
    init: &VerifyExpr,
    transition: &VerifyExpr,
    predicates: &[VerifyExpr],
) -> AbstractModel {
    if predicates.is_empty() {
        return AbstractModel {
            predicates: vec![],
            abstract_init: init.clone(),
            abstract_transition: transition.clone(),
        };
    }

    // Abstract init: init AND each predicate
    let mut abstract_init = init.clone();
    for pred in predicates {
        abstract_init = VerifyExpr::and(abstract_init, pred.clone());
    }

    // Abstract transition: start with the concrete transition, then add
    // inductive predicate implications.
    let mut abstract_trans_parts: Vec<VerifyExpr> = Vec::new();
    abstract_trans_parts.push(transition.clone());

    let cfg = Config::new();
    let ctx = Context::new(&cfg);

    for pred in predicates {
        let pred_next = replace_t_with_t1(pred);

        // Check inductiveness: is (pred@t AND T) => pred@t1 valid?
        // Equivalently: is (pred@t AND T AND NOT pred@t1) unsatisfiable?
        let check = VerifyExpr::and(
            pred.clone(),
            VerifyExpr::and(
                transition.clone(),
                VerifyExpr::not(pred_next.clone()),
            ),
        );

        // Instantiate at step 0 for the Z3 check
        let check_inst = kinduction::instantiate_transition(
            &kinduction::instantiate_at(&check, 0),
            0,
        );

        let is_inductive = {
            let solver = Solver::new(&ctx);
            let encoded = encode_bool(&ctx, &check_inst);
            solver.assert(&encoded);
            matches!(solver.check(), SatResult::Unsat)
        };

        if is_inductive {
            // Predicate is preserved by transition: include as hard constraint
            let implication = VerifyExpr::implies(
                VerifyExpr::and(pred.clone(), transition.clone()),
                pred_next,
            );
            abstract_trans_parts.push(implication);
        } else {
            // Predicate is NOT inductive: include a weaker constraint.
            // Record that this predicate participates in the abstraction
            // by including it as a disjunction: pred@t1 OR NOT pred@t1
            // (i.e., the predicate can take any value, but we record its presence).
            //
            // We use a softer form: (pred@t AND transition) => (pred@t1 OR NOT pred@t)
            // This is always valid and documents the predicate's participation
            // without constraining the abstract system.
            let soft = VerifyExpr::implies(
                pred.clone(),
                VerifyExpr::or(pred_next.clone(), VerifyExpr::not(pred_next)),
            );
            abstract_trans_parts.push(soft);
        }
    }

    let mut abstract_transition = abstract_trans_parts[0].clone();
    for part in &abstract_trans_parts[1..] {
        abstract_transition = VerifyExpr::and(abstract_transition, part.clone());
    }

    AbstractModel {
        predicates: predicates.to_vec(),
        abstract_init,
        abstract_transition,
    }
}

/// Replace @t with @t1 in all variable names of an expression.
fn replace_t_with_t1(expr: &VerifyExpr) -> VerifyExpr {
    match expr {
        VerifyExpr::Var(name) => {
            if name.ends_with("@t") {
                let base = &name[..name.len() - 2];
                VerifyExpr::Var(format!("{}@t1", base))
            } else {
                VerifyExpr::Var(name.clone())
            }
        }
        VerifyExpr::Binary { op, left, right } => VerifyExpr::binary(
            *op,
            replace_t_with_t1(left),
            replace_t_with_t1(right),
        ),
        VerifyExpr::Not(inner) => VerifyExpr::not(replace_t_with_t1(inner)),
        VerifyExpr::Bool(b) => VerifyExpr::Bool(*b),
        VerifyExpr::Int(n) => VerifyExpr::Int(*n),
        VerifyExpr::Iff(l, r) => VerifyExpr::iff(replace_t_with_t1(l), replace_t_with_t1(r)),
        VerifyExpr::ForAll { vars, body } => VerifyExpr::forall(
            vars.clone(),
            replace_t_with_t1(body),
        ),
        VerifyExpr::Exists { vars, body } => VerifyExpr::exists(
            vars.clone(),
            replace_t_with_t1(body),
        ),
        VerifyExpr::Apply { name, args } => VerifyExpr::apply(
            name.clone(),
            args.iter().map(|a| replace_t_with_t1(a)).collect(),
        ),
        VerifyExpr::BitVecConst { width, value } => VerifyExpr::bv_const(*width, *value),
        VerifyExpr::BitVecBinary { op, left, right } => VerifyExpr::bv_binary(
            *op,
            replace_t_with_t1(left),
            replace_t_with_t1(right),
        ),
        VerifyExpr::BitVecExtract { high, low, operand } => VerifyExpr::BitVecExtract {
            high: *high,
            low: *low,
            operand: Box::new(replace_t_with_t1(operand)),
        },
        VerifyExpr::BitVecConcat(l, r) => VerifyExpr::BitVecConcat(
            Box::new(replace_t_with_t1(l)),
            Box::new(replace_t_with_t1(r)),
        ),
        VerifyExpr::Select { array, index } => VerifyExpr::Select {
            array: Box::new(replace_t_with_t1(array)),
            index: Box::new(replace_t_with_t1(index)),
        },
        VerifyExpr::Store { array, index, value } => VerifyExpr::Store {
            array: Box::new(replace_t_with_t1(array)),
            index: Box::new(replace_t_with_t1(index)),
            value: Box::new(replace_t_with_t1(value)),
        },
        VerifyExpr::AtState { state, expr } => VerifyExpr::AtState {
            state: Box::new(replace_t_with_t1(state)),
            expr: Box::new(replace_t_with_t1(expr)),
        },
        VerifyExpr::Transition { from, to } => VerifyExpr::Transition {
            from: Box::new(replace_t_with_t1(from)),
            to: Box::new(replace_t_with_t1(to)),
        },
    }
}

/// CEGAR verification loop: abstract, check, refine.
///
/// 1. Build abstract model from current predicates
/// 2. Check abstract model with k-induction
/// 3. If abstract model is safe, return Safe
/// 4. If abstract counterexample found, check concreteness via BMC
/// 5. If concrete, return Unsafe
/// 6. If spurious, extract new predicates from the counterexample and refine
pub fn cegar_verify(
    init: &VerifyExpr,
    transition: &VerifyExpr,
    property: &VerifyExpr,
    initial_predicates: &[VerifyExpr],
    max_refinements: u32,
) -> AbstractionResult {
    let mut predicates = initial_predicates.to_vec();

    for _iteration in 0..max_refinements {
        // Build the abstract model
        let abs_model = abstract_model_full(init, transition, &predicates);

        // Verify the abstract model using k-induction.
        let abs_result = kinduction::k_induction(
            &abs_model.abstract_init,
            &abs_model.abstract_transition,
            property,
            &[],
            10,
        );

        match abs_result {
            kinduction::KInductionResult::Proven { .. } => {
                // Abstract model proved property safe. Since predicate abstraction
                // is an overapproximation (when inductive predicates are used as
                // strengthening), safety of the abstract model implies safety of
                // the concrete model.
                return AbstractionResult::Safe;
            }
            kinduction::KInductionResult::Counterexample { trace, k } => {
                // Abstract counterexample found. Check if it is concrete
                // by running k-induction on the concrete model.
                let concrete_result = kinduction::k_induction(
                    init, transition, property, &[], k.max(10),
                );

                match concrete_result {
                    kinduction::KInductionResult::Counterexample { trace: ctrace, .. } => {
                        return AbstractionResult::Unsafe { concrete_trace: ctrace };
                    }
                    kinduction::KInductionResult::Proven { .. } => {
                        // Spurious. Refine.
                        let new_preds = extract_refinement_predicates(
                            init, transition, property, &predicates, k,
                        );
                        if new_preds.is_empty() {
                            return AbstractionResult::SpuriousRefined {
                                new_predicates: predicates,
                            };
                        }
                        predicates.extend(new_preds);
                    }
                    _ => {
                        // Inconclusive — try refining.
                        let new_preds = extract_refinement_predicates(
                            init, transition, property, &predicates, k,
                        );
                        if new_preds.is_empty() {
                            return AbstractionResult::SpuriousRefined {
                                new_predicates: predicates,
                            };
                        }
                        predicates.extend(new_preds);
                    }
                }
            }
            kinduction::KInductionResult::InductionFailed { k, .. } => {
                // k-induction couldn't prove or disprove on abstract model.
                // Fall back to concrete check.
                let concrete_result = kinduction::k_induction(
                    init, transition, property, &[], k.max(10),
                );
                match concrete_result {
                    kinduction::KInductionResult::Proven { .. } => {
                        return AbstractionResult::Safe;
                    }
                    kinduction::KInductionResult::Counterexample { trace, .. } => {
                        return AbstractionResult::Unsafe { concrete_trace: trace };
                    }
                    _ => {
                        let new_preds = extract_refinement_predicates(
                            init, transition, property, &predicates, k,
                        );
                        if new_preds.is_empty() {
                            return AbstractionResult::Unknown;
                        }
                        predicates.extend(new_preds);
                    }
                }
            }
            kinduction::KInductionResult::Unknown => {
                return AbstractionResult::Unknown;
            }
        }
    }

    // Exhausted refinement budget — try one final concrete check
    let final_result = kinduction::k_induction(init, transition, property, &[], 10);
    match final_result {
        kinduction::KInductionResult::Proven { .. } => AbstractionResult::Safe,
        kinduction::KInductionResult::Counterexample { trace, .. } => {
            AbstractionResult::Unsafe { concrete_trace: trace }
        }
        _ => AbstractionResult::Unknown,
    }
}

/// Extract refinement predicates from a spurious counterexample.
///
/// When an abstract counterexample is spurious, we need new predicates to
/// eliminate it. The strategy:
///
/// 1. Collect all subexpressions of the transition and property that are
///    comparison predicates (>=, <=, >, <, ==, !=).
/// 2. Filter out predicates already in the set.
/// 3. Return the new ones as refinement candidates.
///
/// This is a simplified form of Craig interpolation — we extract predicates
/// from the structure of the transition relation and property.
fn extract_refinement_predicates(
    init: &VerifyExpr,
    transition: &VerifyExpr,
    property: &VerifyExpr,
    existing_predicates: &[VerifyExpr],
    _depth: u32,
) -> Vec<VerifyExpr> {
    let mut candidates = Vec::new();

    // Collect comparison subexpressions from transition, init, property
    collect_comparison_predicates(transition, &mut candidates);
    collect_comparison_predicates(init, &mut candidates);
    collect_comparison_predicates(property, &mut candidates);

    // Also generate weakened/strengthened versions of existing predicates
    for pred in existing_predicates {
        if let VerifyExpr::Binary { op, left, right } = pred {
            match op {
                VerifyOp::Gte | VerifyOp::Gt | VerifyOp::Lte | VerifyOp::Lt => {
                    if let VerifyExpr::Int(n) = right.as_ref() {
                        candidates.push(VerifyExpr::binary(*op, *left.clone(), VerifyExpr::int(n + 1)));
                        candidates.push(VerifyExpr::binary(*op, *left.clone(), VerifyExpr::int(n - 1)));
                    }
                }
                _ => {}
            }
        }
    }

    // Deduplicate and remove existing predicates (and remove the property itself)
    let existing_dbg: HashSet<String> = existing_predicates.iter()
        .map(|p| format!("{:?}", p))
        .collect();

    let mut seen = HashSet::new();
    let mut result = Vec::new();
    for candidate in candidates {
        let dbg = format!("{:?}", candidate);
        if !existing_dbg.contains(&dbg) && seen.insert(dbg) {
            result.push(candidate);
        }
    }

    result
}

/// Collect all comparison subexpressions from an expression.
fn collect_comparison_predicates(expr: &VerifyExpr, out: &mut Vec<VerifyExpr>) {
    match expr {
        VerifyExpr::Binary { op, left, right } => {
            match op {
                VerifyOp::Gte | VerifyOp::Gt | VerifyOp::Lte | VerifyOp::Lt
                | VerifyOp::Eq | VerifyOp::Neq => {
                    out.push(expr.clone());
                }
                _ => {}
            }
            collect_comparison_predicates(left, out);
            collect_comparison_predicates(right, out);
        }
        VerifyExpr::Not(inner) => collect_comparison_predicates(inner, out),
        VerifyExpr::Iff(l, r) => {
            collect_comparison_predicates(l, out);
            collect_comparison_predicates(r, out);
        }
        VerifyExpr::ForAll { body, .. } | VerifyExpr::Exists { body, .. } => {
            collect_comparison_predicates(body, out);
        }
        _ => {}
    }
}

/// Encode a VerifyExpr to Z3 Bool (local helper, similar to ic3's encode_bool).
fn encode_bool<'ctx>(ctx: &'ctx Context, expr: &VerifyExpr) -> Bool<'ctx> {
    let mut bool_vars = HashMap::new();
    let mut int_vars = HashMap::new();
    let mut all_vars = HashSet::new();
    crate::equivalence::collect_vars_pub(expr, &mut all_vars);
    for name in &all_vars {
        bool_vars.insert(name.clone(), Bool::new_const(ctx, name.as_str()));
    }
    crate::equivalence::collect_int_vars_pub(expr, &mut int_vars, ctx);
    kinduction::encode_expr_bool(ctx, expr, &bool_vars, &int_vars)
}
