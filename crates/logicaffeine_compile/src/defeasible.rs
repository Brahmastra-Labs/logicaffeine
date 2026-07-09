//! P4 defeasible reasoning: circumscription over per-rule abnormality
//! predicates (§6.1 generics, §4.4 habituals, §8.7 implicatures).
//!
//! Premises arrive with each default GUARDED —
//! `GEN x(R(x) → N(x))` ↦ `∀x((R(x) ∧ ¬ab_k(x)) → N(x))` — by
//! [`logic_expr_to_proof_expr_defeasible`](logicaffeine_language::proof_convert::logic_expr_to_proof_expr_defeasible).
//! This module performs the minimization: each abnormality is assumed FALSE
//! per ground individual (`¬ab_k(c)`), greedily, most-specific defaults
//! first, skipping any assumption that is inconsistent with what is already
//! accepted. A default defeated by a more specific rule (penguins among
//! birds) is thereby cancelled WITHOUT contradiction, while unexceptional
//! individuals keep the default.
//!
//! Verdicts are Z3-side three-valued [`SmtVerdict`]s — never kernel-certified
//! and strictly weaker than classical entailment.

use logicaffeine_language::proof_convert::DefaultRule;
use logicaffeine_proof::oracle::{
    oracle_consistent_with_theory, oracle_entails_with_theory, SmtConsistency, SmtTheory,
    SmtVerdict,
};
use logicaffeine_proof::{ProofExpr, ProofTerm};

/// Ground constants mentioned anywhere in the problem — the individuals the
/// per-rule abnormality minimization ranges over.
fn ground_constants(exprs: &[ProofExpr]) -> Vec<String> {
    fn in_term(term: &ProofTerm, out: &mut Vec<String>) {
        match term {
            ProofTerm::Constant(c) => {
                if c.parse::<i64>().is_err() && !out.contains(c) {
                    out.push(c.clone());
                }
            }
            ProofTerm::Function(_, args) | ProofTerm::Group(args) => {
                for arg in args {
                    in_term(arg, out);
                }
            }
            _ => {}
        }
    }
    fn walk(expr: &ProofExpr, out: &mut Vec<String>) {
        match expr {
            ProofExpr::Predicate { args, .. } => {
                for arg in args {
                    in_term(arg, out);
                }
            }
            ProofExpr::Identity(l, r) => {
                in_term(l, out);
                in_term(r, out);
            }
            ProofExpr::NeoEvent { roles, .. } => {
                for (_, term) in roles {
                    in_term(term, out);
                }
            }
            ProofExpr::And(l, r)
            | ProofExpr::Or(l, r)
            | ProofExpr::Implies(l, r)
            | ProofExpr::Iff(l, r) => {
                walk(l, out);
                walk(r, out);
            }
            ProofExpr::Not(i) => walk(i, out),
            ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => walk(body, out),
            ProofExpr::Modal { body, .. } | ProofExpr::Temporal { body, .. } => walk(body, out),
            ProofExpr::Counterfactual {
                antecedent,
                consequent,
            } => {
                walk(antecedent, out);
                walk(consequent, out);
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    for expr in exprs {
        walk(expr, &mut out);
    }
    out
}

/// Order defaults MOST SPECIFIC FIRST: rule i precedes rule j when i's
/// restrictor is (transitively) a subclass of j's, read off the premise
/// implication graph (`∀x((Penguin(x) ∧ …) → Bird(x))` gives Penguin → Bird).
/// More specific defaults claim their individuals first, so the general rule
/// is the one that gets defeated on the overlap — Reiter-style specificity
/// for stratified taxonomies.
fn specificity_order(defaults: &[DefaultRule], premises: &[ProofExpr]) -> Vec<usize> {
    fn implication_edge(expr: &ProofExpr) -> Option<(String, String)> {
        if let ProofExpr::ForAll { body, .. } = expr {
            if let ProofExpr::Implies(lhs, rhs) = body.as_ref() {
                let from = match lhs.as_ref() {
                    ProofExpr::Predicate { name, .. } => Some(name.clone()),
                    // The guarded form: (R(x) ∧ ¬ab(x)) → Q(x)
                    ProofExpr::And(l, _) => match l.as_ref() {
                        ProofExpr::Predicate { name, .. } => Some(name.clone()),
                        _ => None,
                    },
                    _ => None,
                };
                let to = match rhs.as_ref() {
                    ProofExpr::Predicate { name, .. } => Some(name.clone()),
                    _ => None,
                };
                if let (Some(f), Some(t)) = (from, to) {
                    return Some((f, t));
                }
            }
        }
        None
    }
    let edges: Vec<(String, String)> = premises.iter().filter_map(implication_edge).collect();
    let reaches = |from: &str, to: &str| -> bool {
        let mut frontier = vec![from.to_string()];
        let mut seen = vec![from.to_string()];
        while let Some(cur) = frontier.pop() {
            for (f, t) in &edges {
                if *f == cur && !seen.contains(t) {
                    if t == to {
                        return true;
                    }
                    seen.push(t.clone());
                    frontier.push(t.clone());
                }
            }
        }
        false
    };

    let mut order: Vec<usize> = (0..defaults.len()).collect();
    order.sort_by(|&i, &j| {
        match (&defaults[i].restriction_pred, &defaults[j].restriction_pred) {
            (Some(ri), Some(rj)) => {
                let i_specific = reaches(ri, rj);
                let j_specific = reaches(rj, ri);
                match (i_specific, j_specific) {
                    (true, false) => std::cmp::Ordering::Less,
                    (false, true) => std::cmp::Ordering::Greater,
                    _ => defaults[i].ab_name.cmp(&defaults[j].ab_name),
                }
            }
            _ => defaults[i].ab_name.cmp(&defaults[j].ab_name),
        }
    });
    order
}

/// Greedily minimize the abnormality predicates: per default (most specific
/// first) and per ground individual, assume `¬ab_k(c)` unless that is
/// inconsistent with the premises plus everything already assumed. Returns
/// the premise set extended with the surviving assumptions.
fn circumscribe(
    premises: &[ProofExpr],
    goal: Option<&ProofExpr>,
    defaults: &[DefaultRule],
    theory: &SmtTheory,
) -> Vec<ProofExpr> {
    let mut scope: Vec<ProofExpr> = premises.to_vec();
    if defaults.is_empty() {
        return scope;
    }
    let mut universe_src: Vec<ProofExpr> = premises.to_vec();
    if let Some(g) = goal {
        universe_src.push(g.clone());
    }
    let constants = ground_constants(&universe_src);

    for &idx in &specificity_order(defaults, premises) {
        let rule = &defaults[idx];
        let candidates: Vec<ProofExpr> = if rule.unary {
            constants
                .iter()
                .map(|c| {
                    ProofExpr::Not(Box::new(ProofExpr::Predicate {
                        name: rule.ab_name.clone(),
                        args: vec![ProofTerm::Constant(c.clone())],
                        world: None,
                    }))
                })
                .collect()
        } else {
            vec![ProofExpr::Not(Box::new(ProofExpr::Atom(
                rule.ab_name.clone(),
            )))]
        };
        for candidate in candidates {
            let mut attempt = scope.clone();
            attempt.push(candidate);
            if matches!(
                oracle_consistent_with_theory(&attempt, theory),
                SmtConsistency::Consistent
            ) {
                scope = attempt;
            }
        }
    }
    scope
}

/// Does the (circumscribed) premise set defeasibly entail the goal?
pub fn defeasible_entails(
    premises: &[ProofExpr],
    goal: &ProofExpr,
    defaults: &[DefaultRule],
    theory: &SmtTheory,
) -> SmtVerdict {
    let scope = circumscribe(premises, Some(goal), defaults, theory);
    oracle_entails_with_theory(&scope, goal, theory)
}

/// Is the (circumscribed) premise set consistent? A default defeated by an
/// exception must read as cancelled, never as a contradiction.
pub fn defeasible_consistent(
    premises: &[ProofExpr],
    defaults: &[DefaultRule],
    theory: &SmtTheory,
) -> SmtConsistency {
    let scope = circumscribe(premises, None, defaults, theory);
    oracle_consistent_with_theory(&scope, theory)
}
