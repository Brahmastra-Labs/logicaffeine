//! `crush` — the grind-style closer: E-match quantified equality lemmas into
//! the ground e-graph, then discharge the goal by certified congruence closure.
//!
//! Lean's `grind` fuses an e-graph, E-matching of annotated lemmas, and theory
//! solvers. This is the congruence+instantiation core of that, built entirely
//! on machinery that already certifies: the ground equality graph and its
//! transitive/congruence explanations ([`crate::engine::cert_congruence_path`]),
//! and the one-sided matcher that fires a `∀`-lemma at a ground subterm
//! ([`crate::unify::match_term_pattern`]). Each instantiation is a
//! [`InferenceRule::UniversalInstTerm`] chain from the lemma; the final goal
//! closes by rewriting a known atom through the congruence-derived equalities.
//! The whole result is one kernel-checked derivation — unlike `grind`, whose
//! core is trusted.
//!
//! Signature demo (which plain `auto` cannot do): `∀x. f(x)=g(x)`, `a=b`,
//! `P(g(b))` ⊢ `P(f(a))`.

use std::collections::HashSet;

use crate::engine::cert_congruence_path;
use crate::unify::{apply_subst_to_expr, match_term_pattern, Substitution};
use crate::{DerivationTree, InferenceRule, ProofExpr, ProofTerm};

/// Prove `goal` from `premises` by E-matching the `∀`-equality lemmas among
/// them and closing by congruence. Returns a kernel-checkable derivation, or
/// `None` when this fragment cannot close the goal.
pub(crate) fn crush_prove(premises: &[ProofExpr], goal: &ProofExpr) -> Option<DerivationTree> {
    // Ground facts (with PremiseMatch proofs) and the `∀`-equality lemmas.
    let mut known: Vec<(ProofExpr, DerivationTree)> = Vec::new();
    let mut eq_lemmas: Vec<&ProofExpr> = Vec::new();
    for prem in premises {
        if is_forall_eq_lemma(prem) {
            eq_lemmas.push(prem);
        } else {
            known.push((prem.clone(), DerivationTree::leaf(prem.clone(), InferenceRule::PremiseMatch)));
        }
    }

    // The ground term universe: every subterm of a ground fact or the goal.
    let mut universe: Vec<ProofTerm> = Vec::new();
    for (prop, _) in &known {
        collect_expr_terms(prop, &mut universe);
    }
    collect_expr_terms(goal, &mut universe);
    dedup_terms(&mut universe);

    // E-match each lemma's LHS against ground subterms; add the instances.
    for lemma in &eq_lemmas {
        let (params, lhs, _rhs) = peel_eq_lemma(lemma).expect("checked by is_forall_eq_lemma");
        let mut fired: HashSet<String> = HashSet::new();
        for g in &universe {
            let Some(subst) = match_term_pattern(&lhs, g) else { continue };
            if !params.iter().all(|p| subst.contains_key(p)) {
                continue; // trigger did not pin every variable
            }
            let key = format!("{g}");
            if !fired.insert(key) {
                continue;
            }
            if let Some(inst) = instantiate_eq_lemma(lemma, &params, &subst) {
                known.push(inst);
            }
        }
    }

    // Close.
    match goal {
        ProofExpr::Identity(a, b) => cert_congruence_path(a, b, &known),
        ProofExpr::Predicate { name, args, world } => {
            close_predicate(name, args, world.as_deref(), &known)
        }
        _ => None,
    }
}

/// Prove `P(s̄)` from a known atom `P(t̄)` and the equalities in `known`: rewrite
/// each argument `tᵢ ⇝ sᵢ` (justified by a congruence path) one at a time,
/// starting from the known atom.
fn close_predicate(
    name: &str,
    goal_args: &[ProofTerm],
    world: Option<&str>,
    known: &[(ProofExpr, DerivationTree)],
) -> Option<DerivationTree> {
    let pred = |args: &[ProofTerm]| ProofExpr::Predicate {
        name: name.to_string(),
        args: args.to_vec(),
        world: world.map(str::to_string),
    };
    for (prop, atom_proof) in known {
        let ProofExpr::Predicate { name: kn, args: kargs, world: kw } = prop else { continue };
        if kn != name || kargs.len() != goal_args.len() || kw.as_deref() != world {
            continue;
        }
        // Rewrite kargs → goal_args argument by argument.
        let mut cur = kargs.clone();
        let mut proof = atom_proof.clone();
        let mut ok = true;
        for i in 0..goal_args.len() {
            if cur[i] == goal_args[i] {
                continue;
            }
            // A proof `cur[i] = goal_args[i]` from the e-graph.
            let Some(eq) = cert_congruence_path(&cur[i], &goal_args[i], known) else {
                ok = false;
                break;
            };
            let mut next = cur.clone();
            next[i] = goal_args[i].clone();
            // Forward Rewrite (as `build_congruence_proof`): the equality proves
            // `from = to`, the source proves `P(cur)`, the conclusion is `P(next)`.
            proof = DerivationTree::new(
                pred(&next),
                InferenceRule::Rewrite { from: cur[i].clone(), to: goal_args[i].clone() },
                vec![eq, proof],
            );
            cur = next;
        }
        if ok && cur == goal_args {
            return Some(proof);
        }
    }
    None
}

/// A premise of shape `∀xs. lhs = rhs`.
fn is_forall_eq_lemma(e: &ProofExpr) -> bool {
    peel_eq_lemma(e).is_some()
}

/// Peel `∀xs. lhs = rhs` into `(xs, lhs, rhs)`; `None` if it is not that shape
/// (must have at least one binder and an equational body).
fn peel_eq_lemma(e: &ProofExpr) -> Option<(Vec<String>, ProofTerm, ProofTerm)> {
    let mut params = Vec::new();
    let mut body = e;
    while let ProofExpr::ForAll { variable, body: inner } = body {
        params.push(variable.clone());
        body = inner;
    }
    if params.is_empty() {
        return None;
    }
    match body {
        ProofExpr::Identity(l, r) => Some((params, l.clone(), r.clone())),
        _ => None,
    }
}

/// Instantiate `∀xs. body` at `subst`, producing the ground fact and its
/// `UniversalInstTerm`-chain proof from the lemma's `PremiseMatch`.
fn instantiate_eq_lemma(
    lemma: &ProofExpr,
    params: &[String],
    subst: &Substitution,
) -> Option<(ProofExpr, DerivationTree)> {
    let mut tree = DerivationTree::leaf(lemma.clone(), InferenceRule::PremiseMatch);
    let mut expr = lemma.clone();
    for param in params {
        let witness = subst.get(param)?.clone();
        let ProofExpr::ForAll { variable, body } = expr else { return None };
        debug_assert_eq!(&variable, param);
        let mut single = Substitution::new();
        single.insert(param.clone(), witness.clone());
        let inst = apply_subst_to_expr(&body, &single);
        tree = DerivationTree::new(
            inst.clone(),
            InferenceRule::UniversalInstTerm(witness),
            vec![tree],
        );
        expr = inst;
    }
    Some((expr, tree))
}

fn collect_expr_terms(e: &ProofExpr, out: &mut Vec<ProofTerm>) {
    match e {
        ProofExpr::Predicate { args, .. } => {
            for a in args {
                collect_term(a, out);
            }
        }
        ProofExpr::Identity(l, r) => {
            collect_term(l, out);
            collect_term(r, out);
        }
        ProofExpr::And(l, r)
        | ProofExpr::Or(l, r)
        | ProofExpr::Implies(l, r)
        | ProofExpr::Iff(l, r) => {
            collect_expr_terms(l, out);
            collect_expr_terms(r, out);
        }
        ProofExpr::Not(p) => collect_expr_terms(p, out),
        ProofExpr::ForAll { body, .. } | ProofExpr::Exists { body, .. } => {
            collect_expr_terms(body, out)
        }
        _ => {}
    }
}

fn collect_term(t: &ProofTerm, out: &mut Vec<ProofTerm>) {
    out.push(t.clone());
    if let ProofTerm::Function(_, args) | ProofTerm::Group(args) = t {
        for a in args {
            collect_term(a, out);
        }
    }
}

fn dedup_terms(v: &mut Vec<ProofTerm>) {
    let mut seen = HashSet::new();
    v.retain(|t| seen.insert(format!("{t}")));
}
