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
//! `≤`-facts to the existing Farkas refutation (`crate::engine::cert_farkas`).
//! The result is one kernel-checked `⊥` derivation, so `omega` is exactly as
//! trusted as `linarith` — it just sees more contradictions.

use crate::engine::{as_le_pair, cert_farkas, le_eq};
use crate::{DerivationTree, InferenceRule, ProofExpr, ProofTerm};

/// The strict inequality `a < b`, encoded as the Prop `lt(a, b) = true`.
pub(crate) fn lt_eq(a: ProofTerm, b: ProofTerm) -> ProofExpr {
    ProofExpr::Identity(
        ProofTerm::Function("lt".to_string(), vec![a, b]),
        ProofTerm::Constant("true".to_string()),
    )
}

/// If `e` is a strict inequality `lt(a, b) = true`, return `(a, b)`.
pub(crate) fn as_lt_pair(e: &ProofExpr) -> Option<(ProofTerm, ProofTerm)> {
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
pub(crate) fn succ(a: &ProofTerm) -> ProofTerm {
    ProofTerm::Function(
        "add".to_string(),
        vec![a.clone(), ProofTerm::Constant("1".to_string())],
    )
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
pub(crate) fn to_le_fact(
    prop: &ProofExpr,
    proof: &DerivationTree,
) -> Option<(ProofExpr, DerivationTree)> {
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
    let le_facts: Vec<(ProofExpr, DerivationTree)> =
        known.iter().filter_map(|(p, t)| to_le_fact(p, t)).collect();
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

// =============================================================================
// THE GOAL-DIRECTED HALF
//
// A refutation engine proves `⊥`. A DECISION PROCEDURE proves the goal — Lean's
// `omega` negates it, adds it to the hypothesis set and refutes. Everything
// below is that half, and it adds NO axiom: the case split runs on `le_total`,
// which the trusted base already registers and whose own comment in
// `prelude.rs` names this exact use.
//
// The one thing totality does not give is the STRICT negation. Over ℤ,
// `¬(a ≤ b)` is `b + 1 ≤ a`; `le_total a b` yields the weaker `b ≤ a`, so the
// fragment reached here is "every goal whose negation-by-totality is
// refutable" rather than all of Presburger. Closing that last unit needs a new
// `Int` axiom (`¬(a ≤ b) → b + 1 ≤ a`, or antisymmetry), and buying tactic
// reach with trust is the one trade this campaign forbids — so it is named
// here and not taken.
// =============================================================================

/// Is `t` an integer-valued term on its face — an arithmetic operator or a
/// numeral? Mirrors `certifier::term_domain`'s `Int` classification (which is
/// what decides the `Eq` domain of the goal type), so the two agree on which
/// equalities `omega` may claim.
fn is_int_valued(t: &ProofTerm) -> bool {
    match t {
        ProofTerm::Function(n, _) => matches!(
            n.as_str(),
            "add" | "sub" | "mul" | "div" | "mod" | "Add" | "Sub" | "Mul" | "Div"
        ),
        ProofTerm::Constant(s) => crate::numeral::is_numeral(s),
        _ => false,
    }
}

/// If `e` is an `Int` equality goal (`n + 0 = n`), return its two sides.
fn as_int_identity(e: &ProofExpr) -> Option<(ProofTerm, ProofTerm)> {
    if let ProofExpr::Identity(l, r) = e {
        if is_int_valued(l) || is_int_valued(r) {
            return Some((l.clone(), r.clone()));
        }
    }
    None
}

/// Is this a goal `omega` has any business attacking? The old gate asked only
/// about the HYPOTHESES, so `⊢ n + 0 = n` — an `omega` goal in Lean with no
/// hypotheses whatever — was declined for having no hypotheses.
pub(crate) fn is_arith_goal(e: &ProofExpr) -> bool {
    as_le_pair(e).is_some() || as_lt_pair(e).is_some() || as_int_identity(e).is_some()
}

/// An integer literal operand, if this term is one.
fn as_literal(t: &ProofTerm) -> Option<logicaffeine_base::BigInt> {
    match t {
        ProofTerm::Constant(s) => crate::numeral::int_of_constant(s),
        _ => None,
    }
}

/// Prove `a ≤ b` (as the `Identity` `goal`) from the known facts, WITHOUT any
/// case split: by computation when both operands are literals, else by chaining
/// the in-scope `≤` facts through `le_trans`/`le_refl`.
fn prove_le_directly(
    known: &[(ProofExpr, DerivationTree)],
    a: &ProofTerm,
    b: &ProofTerm,
    goal: &ProofExpr,
) -> Option<DerivationTree> {
    if let (Some(av), Some(bv)) = (as_literal(a), as_literal(b)) {
        // `le m n` δ-reduces, so `refl` closes the goal exactly when it holds.
        return (av <= bv).then(|| DerivationTree::leaf(goal.clone(), InferenceRule::Reflexivity));
    }
    crate::engine::cert_le_chain(known, a, b)
}

/// Decide a positive linear goal. `None` means "not proved" — never "false".
///
/// * a ground `≤`/`<` by computation;
/// * a `≤` goal by chaining the in-scope `≤` facts;
/// * an `Int` EQUALITY by the proof-producing ring oracle (the `ArithDecision`
///   channel), which is how `n + 0 = n` is discharged;
/// * otherwise, for a `≤` goal, SPLIT on `le_total a b`: the left disjunct IS
///   the goal, and the right (`b ≤ a`) is refuted together with the hypotheses,
///   discharging the goal by ex falso. Both branches certify, so the whole
///   `DisjunctionCases` node is one kernel-checked object.
pub(crate) fn omega_prove(
    known: &[(ProofExpr, DerivationTree)],
    goal: &ProofExpr,
) -> Option<DerivationTree> {
    if let Some((a, b)) = as_le_pair(goal) {
        if let Some(direct) = prove_le_directly(known, &a, &b, goal) {
            return Some(direct);
        }
        // `(a ≤ b) ∨ (b ≤ a)` — totality, a certified leaf.
        let flipped = le_eq(b.clone(), a.clone());
        let disjunction = ProofExpr::Or(Box::new(goal.clone()), Box::new(flipped.clone()));
        let totality = DerivationTree::leaf(disjunction, InferenceRule::LeTotal);
        // Left: the disjunct assumed IS the goal.
        let left = DerivationTree::leaf(goal.clone(), InferenceRule::PremiseMatch);
        // Right: `b ≤ a` alongside the hypotheses. Refute them (then ex falso), or
        // — when the extra edge completes a chain — prove the goal outright.
        let mut extended = known.to_vec();
        extended.push((
            flipped.clone(),
            DerivationTree::leaf(flipped, InferenceRule::PremiseMatch),
        ));
        let right = match crate::engine::cert_farkas(&extended).or_else(|| omega_close(&extended)) {
            Some(bot) => DerivationTree::new(goal.clone(), InferenceRule::ExFalso, vec![bot]),
            None => prove_le_directly(&extended, &a, &b, goal)?,
        };
        return Some(DerivationTree::new(
            goal.clone(),
            InferenceRule::DisjunctionCases,
            vec![totality, left, right],
        ));
    }
    if let Some((a, b)) = as_lt_pair(goal) {
        // Ground strict comparison: `lt m n` δ-reduces. A SYMBOLIC `<` conclusion
        // has no image in the trusted base at all — of the Int order axioms none
        // CONCLUDES a `lt` (`lt_succ_le`/`lt_add1_le` both take one as a premise) —
        // so the tightened `a + 1 ≤ b` is the strongest thing provable here. See
        // the module comment: the converse `le_succ_lt` is the missing axiom.
        if let (Some(av), Some(bv)) = (as_literal(&a), as_literal(&b)) {
            return (av < bv)
                .then(|| DerivationTree::leaf(goal.clone(), InferenceRule::Reflexivity));
        }
        return crate::engine::cert_lt_chain(known, &a, &b);
    }
    if let Some((l, r)) = as_int_identity(goal) {
        if crate::tactic::int_identity_holds(&l, &r) {
            return Some(DerivationTree::leaf(
                goal.clone(),
                InferenceRule::ArithDecision,
            ));
        }
    }
    None
}
