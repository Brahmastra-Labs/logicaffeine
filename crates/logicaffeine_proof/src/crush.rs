//! `crush` — the grind-style closer: E-match quantified equality lemmas into
//! the ground e-graph, then discharge the goal by certified congruence closure.
//!
//! Lean's `grind` fuses an e-graph, E-matching of annotated lemmas, and theory
//! solvers. This is the congruence+instantiation core of that, built entirely
//! on machinery that already certifies: the ground equality graph and its
//! transitive/congruence explanations (`crate::engine::cert_congruence_path`),
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
use crate::unify::{apply_subst_to_expr, instantiate_expr, match_term_pattern, Substitution};
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
            known.push((
                prem.clone(),
                DerivationTree::leaf(prem.clone(), InferenceRule::PremiseMatch),
            ));
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
            let Some(subst) = match_term_pattern(&lhs, g) else {
                continue;
            };
            if !params.iter().all(|p| subst.contains_key(p)) {
                continue; // trigger did not pin every variable
            }
            let key = format!("{g}");
            if !fired.insert(key) {
                continue;
            }
            let base = DerivationTree::leaf((*lemma).clone(), InferenceRule::PremiseMatch);
            if let Some(inst) = instantiate_eq_lemma(lemma, &base, &params, &subst) {
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
        let ProofExpr::Predicate {
            name: kn,
            args: kargs,
            world: kw,
        } = prop
        else {
            continue;
        };
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
            // Forward Leibniz: the equality proves `from = to`, the source proves `P(cur)`,
            // the conclusion is `P(next)`. The motive's POSITION matters — two arguments can
            // converge on one term (`P(a, cc)` closing `P(b, b)`), and the all-occurrences
            // motive `λz. P(z, z)` then asks for a source `P(cc, cc)` nobody proved, so the
            // tactic would report success and the kernel would refuse the tree.
            let Some(step) =
                crate::engine::leibniz_step(pred(&next), &cur[i], &goal_args[i], eq, proof.clone())
            else {
                ok = false;
                break;
            };
            proof = step;
            cur = next;
        }
        if ok && cur == goal_args {
            return Some(proof);
        }
    }
    None
}

/// Expand the `∀`-equality steps of a named chain into their GROUND instances at the terms
/// the chain actually mentions.
///
/// Lean writes a `calc` step as `h₂ b` — the lemma APPLIED at the waypoint — so the
/// instantiation is part of the source text. A LOGOS `calc` block names its steps, so the
/// application has to be FOUND: each `∀x̄. lhs = rhs` is E-matched (both sides, since a chain
/// may enter the step from either end) against every subterm of the goal and of the chain's
/// ground steps, and each firing becomes a [`InferenceRule::UniversalInstTerm`] chain built
/// on the step's own derivation. A step that is already ground travels through unchanged.
///
/// This is the same E-matching [`crush_prove`] performs over everything in scope, restricted
/// to the steps a `calc` block NAMED — which is the distinction between the two tactics.
pub(crate) fn instantiate_steps(
    steps: &[(ProofExpr, DerivationTree)],
    goal: &ProofExpr,
) -> Vec<(ProofExpr, DerivationTree)> {
    if !steps.iter().any(|(p, _)| is_forall_eq_lemma(p)) {
        return steps.to_vec();
    }
    let mut universe: Vec<ProofTerm> = Vec::new();
    collect_expr_terms(goal, &mut universe);
    for (prop, _) in steps {
        if !is_forall_eq_lemma(prop) {
            collect_expr_terms(prop, &mut universe);
        }
    }
    dedup_terms(&mut universe);

    let mut out: Vec<(ProofExpr, DerivationTree)> = Vec::new();
    for (prop, proof) in steps {
        let Some((params, lhs, rhs)) = peel_eq_lemma(prop) else {
            out.push((prop.clone(), proof.clone()));
            continue;
        };
        let mut fired: HashSet<String> = HashSet::new();
        for pattern in [&lhs, &rhs] {
            for g in &universe {
                let Some(subst) = match_term_pattern(pattern, g) else {
                    continue;
                };
                if !params.iter().all(|p| subst.contains_key(p)) {
                    continue;
                }
                let Some(inst) = instantiate_eq_lemma(prop, proof, &params, &subst) else {
                    continue;
                };
                if fired.insert(format!("{}", inst.0)) {
                    out.push(inst);
                }
            }
        }
    }
    out
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
    while let ProofExpr::ForAll {
        variable,
        body: inner,
    } = body
    {
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
/// `UniversalInstTerm`-chain proof built on top of `base` — the derivation that already
/// proves the quantified lemma (a `PremiseMatch` leaf for a premise, whatever a named `calc`
/// step carries for a hypothesis).
fn instantiate_eq_lemma(
    lemma: &ProofExpr,
    base: &DerivationTree,
    params: &[String],
    subst: &Substitution,
) -> Option<(ProofExpr, DerivationTree)> {
    let mut tree = base.clone();
    let mut expr = lemma.clone();
    for param in params {
        let witness = subst.get(param)?.clone();
        let ProofExpr::ForAll { variable, body } = expr else {
            return None;
        };
        debug_assert_eq!(&variable, param);
        let inst = instantiate_expr(&body, param, &witness);
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
