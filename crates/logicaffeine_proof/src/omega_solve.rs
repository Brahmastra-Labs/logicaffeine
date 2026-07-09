//! `omega` — linear INTEGER arithmetic, the discreteness-aware layer over the
//! rational Farkas core.
//!
//! Rational Fourier-Motzkin ([`crate::linarith_solve`]) decides satisfiability
//! over ℚ; it is sound for ℤ but INCOMPLETE, because it cannot use the one fact
//! that distinguishes the integers: a strict `a < b` is a non-strict `a+1 ≤ b`,
//! since nothing lives strictly between consecutive integers. The classic
//! witness is `x < y ∧ y < x+1` — rationally satisfiable (`x=0, y=½`), integer
//! UNSAT.
//!
//! This module supplies exactly that missing step. It rewrites every strict
//! hypothesis `a < b` to `a+1 ≤ b` through the kernel axiom `lt_succ_le` (a
//! certified [`InferenceRule::LtSuccLe`] node), then hands the enlarged set of
//! `≤`-facts to the existing Farkas refutation ([`crate::engine::cert_farkas`]).
//! The result is one kernel-checked `⊥` derivation, so `omega` is exactly as
//! trusted as `linarith` — it just sees more contradictions.

use crate::engine::{as_le_pair, cert_farkas, le_eq};
use crate::{DerivationTree, InferenceRule, ProofExpr, ProofTerm};

/// If `e` is a strict inequality `lt(a, b) = true`, return `(a, b)`.
fn as_lt_pair(e: &ProofExpr) -> Option<(ProofTerm, ProofTerm)> {
    if let ProofExpr::Identity(lhs, rhs) = e {
        if let (ProofTerm::Function(name, args), ProofTerm::Constant(t)) = (lhs, rhs) {
            if name == "lt" && args.len() == 2 && t == "true" {
                return Some((args[0].clone(), args[1].clone()));
            }
        }
    }
    None
}

/// `a + 1` as a proof term.
fn succ(a: &ProofTerm) -> ProofTerm {
    ProofTerm::Function("add".to_string(), vec![a.clone(), ProofTerm::Constant("1".to_string())])
}

/// If `t` is `add(b, 1)`, return `b` — the shape `lt_add1_le` cancels.
fn as_succ(t: &ProofTerm) -> Option<ProofTerm> {
    if let ProofTerm::Function(name, args) = t {
        if name == "add" && args.len() == 2 && args[1] == ProofTerm::Constant("1".to_string()) {
            return Some(args[0].clone());
        }
    }
    None
}

/// Rewrite a known fact to an equivalent (or stronger) `≤`-fact for the Farkas
/// core. A `≤`-fact passes through. A strict `a < b` becomes `≤` via integer
/// discreteness: when the bound is already `b′+1` we cancel it to the clean
/// `a ≤ b′` (`lt_add1_le`), keeping the reconstructed terms small; otherwise it
/// tightens to `a+1 ≤ b` (`lt_succ_le`). Anything else is dropped.
fn to_le_fact(prop: &ProofExpr, proof: &DerivationTree) -> Option<(ProofExpr, DerivationTree)> {
    if as_le_pair(prop).is_some() {
        return Some((prop.clone(), proof.clone()));
    }
    if let Some((a, b)) = as_lt_pair(prop) {
        if let Some(b_inner) = as_succ(&b) {
            // a < b′ + 1  ⊢  a ≤ b′
            let concl = le_eq(a, b_inner);
            let tree =
                DerivationTree::new(concl.clone(), InferenceRule::LtAdd1Le, vec![proof.clone()]);
            return Some((concl, tree));
        }
        // a < b  ⊢  a + 1 ≤ b
        let concl = le_eq(succ(&a), b);
        let tree = DerivationTree::new(concl.clone(), InferenceRule::LtSuccLe, vec![proof.clone()]);
        return Some((concl, tree));
    }
    None
}

/// Refute a set of integer hypotheses (`≤` and strict `<`), returning a
/// kernel-checked `⊥` derivation when they are jointly unsatisfiable over ℤ.
///
/// Strictly stronger than [`cert_farkas`]: it first discreteness-tightens every
/// strict hypothesis, so it catches contradictions the rational core misses.
/// Returns `None` when the (tightened) system is rationally satisfiable — i.e.
/// genuinely has an integer model on the fragment covered here.
pub(crate) fn omega_close(known: &[(ProofExpr, DerivationTree)]) -> Option<DerivationTree> {
    let le_facts: Vec<(ProofExpr, DerivationTree)> = known
        .iter()
        .filter_map(|(p, t)| to_le_fact(p, t))
        .collect();
    // Only worth the extra pass if at least one strict fact was tightened;
    // otherwise the caller's own `cert_farkas` already tried this exact set.
    let tightened = known.iter().any(|(p, _)| as_lt_pair(p).is_some());
    if !tightened {
        return None;
    }
    cert_farkas(&le_facts)
}

/// Does `omega_close` see hypotheses at all — any `≤`/`<` fact? Used by the
/// tactic to give a precise "not an arithmetic goal" error.
pub(crate) fn has_arith_facts(known: &[(ProofExpr, DerivationTree)]) -> bool {
    known
        .iter()
        .any(|(p, _)| as_le_pair(p).is_some() || as_lt_pair(p).is_some())
}
