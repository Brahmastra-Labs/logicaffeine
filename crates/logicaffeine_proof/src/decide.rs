//! `decide` — proof by evaluation for closed decidable goals.
//!
//! The router below is untrusted: it evaluates the goal with machine
//! arithmetic and, only when the goal is TRUE, builds the derivation whose
//! leaves re-derive that truth through certified channels — `ArithDecision`
//! (the proof-producing arithmetic oracle) for ground `Int` identities,
//! `NativeDecide` (the kernel's trusted-evaluator route via the `reduceBool`
//! hook and a `Decidable` instance) for ground comparisons and Bool
//! equalities. Propositional structure recurses through the ordinary intro
//! rules. A false, open, or unsupported goal yields `None` — decide declines,
//! it never guesses.

use crate::{DerivationTree, InferenceRule, ProofExpr, ProofTerm};

/// Evaluate a ground integer term: numeric constants and `add`/`sub`/`mul`/
/// `div`/`mod` trees over them. Checked arithmetic — overflow declines.
fn eval_int(t: &ProofTerm) -> Option<i64> {
    match t {
        ProofTerm::Constant(s) => s.parse::<i64>().ok(),
        ProofTerm::Function(op, args) if args.len() == 2 => {
            let a = eval_int(&args[0])?;
            let b = eval_int(&args[1])?;
            match op.as_str() {
                "add" => a.checked_add(b),
                "sub" => a.checked_sub(b),
                "mul" => a.checked_mul(b),
                "div" => a.checked_div(b),
                "mod" => a.checked_rem(b),
                _ => None,
            }
        }
        _ => None,
    }
}

fn ground_bool(t: &ProofTerm) -> Option<bool> {
    match t {
        ProofTerm::Constant(s) if s == "true" => Some(true),
        ProofTerm::Constant(s) if s == "false" => Some(false),
        _ => None,
    }
}

/// Evaluate a ground comparison term `le(a, b)`/`lt`/`ge`/`gt` to its truth.
fn eval_comparison(t: &ProofTerm) -> Option<bool> {
    let ProofTerm::Function(name, args) = t else { return None };
    if args.len() != 2 {
        return None;
    }
    let a = eval_int(&args[0])?;
    let b = eval_int(&args[1])?;
    match name.as_str() {
        "le" => Some(a <= b),
        "lt" => Some(a < b),
        "ge" => Some(a >= b),
        "gt" => Some(a > b),
        _ => None,
    }
}

/// Decide a closed goal: `Some(tree)` iff the goal evaluates TRUE, where the
/// tree's leaves certify through `ArithDecision`/`NativeDecide` and its
/// structure through the intro rules.
pub(crate) fn decide_expr(goal: &ProofExpr) -> Option<DerivationTree> {
    match goal {
        ProofExpr::Identity(l, r) => {
            // Ground comparison in the canonical encoding `le(a, b) = true`
            // (or `= false`): the kernel evaluator re-derives it.
            if let (Some(truth), Some(claimed)) = (eval_comparison(l), ground_bool(r)) {
                return (truth == claimed)
                    .then(|| DerivationTree::leaf(goal.clone(), InferenceRule::NativeDecide));
            }
            // Ground Int identity: the arithmetic oracle re-derives it.
            if let (Some(a), Some(b)) = (eval_int(l), eval_int(r)) {
                return (a == b)
                    .then(|| DerivationTree::leaf(goal.clone(), InferenceRule::ArithDecision));
            }
            // Ground Bool identity: the kernel evaluator re-derives it.
            if let (Some(a), Some(b)) = (ground_bool(l), ground_bool(r)) {
                return (a == b)
                    .then(|| DerivationTree::leaf(goal.clone(), InferenceRule::NativeDecide));
            }
            None
        }
        ProofExpr::And(l, r) => {
            let lt = decide_expr(l)?;
            let rt = decide_expr(r)?;
            Some(DerivationTree::new(
                goal.clone(),
                InferenceRule::ConjunctionIntro,
                vec![lt, rt],
            ))
        }
        ProofExpr::Or(l, r) => {
            let side = decide_expr(l).or_else(|| decide_expr(r))?;
            Some(DerivationTree::new(
                goal.clone(),
                InferenceRule::DisjunctionIntro,
                vec![side],
            ))
        }
        // An implication holds by weakening whenever its consequent decides
        // true. (A false antecedent is NOT decided here: proving ¬A needs a
        // refutation channel — `of_decide_eq_false` — a documented follow-up.)
        ProofExpr::Implies(_, r) => {
            let rt = decide_expr(r)?;
            Some(DerivationTree::new(
                goal.clone(),
                InferenceRule::ImpliesIntro,
                vec![rt],
            ))
        }
        _ => None,
    }
}
