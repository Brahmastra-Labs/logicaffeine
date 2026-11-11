//! **Certified resolution-width lower bounds**, with the same zero-trust certificate discipline as the
//! Nullstellensatz dual witnesses of [`crate::polycalc`].
//!
//! Width-`w` resolution keeps every clause in the derivation to `≤ w` literals; Ben-Sasson–Wigderson
//! made width the master resolution complexity measure (short proofs imply narrow proofs). At a fixed
//! budget `w` the derivable set is a **finite least fixpoint**: saturate pairwise resolution, keeping
//! resolvents of width `≤ w`. That fixpoint *is* the lower-bound certificate: a clause set `S` that
//! (i) contains the admissible inputs, (ii) is closed under width-`≤ w` resolution, and (iii) lacks
//! the empty clause proves — by induction over any purported derivation — that **no width-`w`
//! resolution refutation exists**. The certificate is re-checked by [`check_res_width_lower_bound`]
//! with zero trust in the producer, mirroring `polycalc::check_ns_lower_bound`.
//!
//! Two width conventions live in the literature, and they genuinely differ on wide-axiom families
//! like pigeonhole (pigeon clauses have width `m−1`):
//! [`WidthConvention::Strict`] counts axioms against the budget (the
//! `hypercube::min_resolution_width` convention); [`WidthConvention::WideAxioms`] admits every axiom
//! and bounds only *derived* clauses — the convention under which PHP's width question is non-trivial.
//! Tautologies are dropped throughout: a resolvent with a second clashing variable is tautological
//! (mirrored from `Subcube::resolve`, which requires a single clean pivot), and tautological axioms
//! only ever produce subsumed or tautological resolvents.

use crate::cdcl::Lit;
use std::collections::BTreeSet;

/// A clause as a signed bitmask pair `(pos, neg)` over ≤ 63 variables — `pos` the positive literals'
/// variable set, `neg` the negative ones'. The empty clause is `(0, 0)`; a tautology has
/// `pos & neg ≠ 0`. This is the `Subcube` blocker seen from the clause side (`care = pos|neg`,
/// `value = neg`).
pub type MaskClause = (u64, u64);

/// Which clauses the width budget `w` applies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WidthConvention {
    /// Every clause in the derivation — axioms included — must have width `≤ w` (the
    /// Ben-Sasson–Wigderson refutation-width measure, and `hypercube::min_resolution_width`'s convention).
    Strict,
    /// Axioms enter at any width; only *derived* clauses must have width `≤ w`. The convention under
    /// which wide-axiom families (pigeonhole) have a non-trivial width question.
    WideAxioms,
}

/// The bitmask form of a clause (duplicates collapse; ≤ 63 variables).
pub fn mask_clause(clause: &[Lit]) -> MaskClause {
    let (mut pos, mut neg) = (0u64, 0u64);
    for l in clause {
        assert!(l.var() < 63, "the u64 clause mask carries ≤ 63 variables");
        if l.is_positive() {
            pos |= 1u64 << l.var();
        } else {
            neg |= 1u64 << l.var();
        }
    }
    (pos, neg)
}

/// The width of a mask clause — its literal count.
pub fn mask_width(c: MaskClause) -> usize {
    (c.0.count_ones() + c.1.count_ones()) as usize
}

/// Resolve two mask clauses on their single clashing variable. `Some(resolvent)` iff exactly one
/// variable appears positively in one and negatively in the other (a second clash would make every
/// resolvent a tautology — the same single-pivot rule as `Subcube::resolve`). The resolvent is
/// automatically tautology-free.
pub fn resolve_masks(a: MaskClause, b: MaskClause) -> Option<MaskClause> {
    let clash = (a.0 & b.1) | (a.1 & b.0);
    if clash.count_ones() != 1 {
        return None;
    }
    Some(((a.0 | b.0) & !clash, (a.1 | b.1) & !clash))
}

/// The admissible axioms under a convention: tautologies dropped, and under [`WidthConvention::Strict`]
/// only clauses of width `≤ w`.
fn seed(clauses: &[Vec<Lit>], w: usize, convention: WidthConvention) -> BTreeSet<MaskClause> {
    clauses
        .iter()
        .map(|c| mask_clause(c))
        .filter(|&m| m.0 & m.1 == 0)
        .filter(|&m| convention == WidthConvention::WideAxioms || mask_width(m) <= w)
        .collect()
}

/// **The width-`w` resolution closure** — the least fixpoint of pairwise resolution over the admissible
/// axioms, keeping every resolvent of width `≤ w`. This set contains every clause any width-`w`
/// resolution derivation (under the convention) can produce, so it *decides* width-`w` refutability
/// (`(0,0)` present ⟺ refutable) and, when the empty clause is absent, the set itself is the
/// re-checkable lower-bound certificate ([`check_res_width_lower_bound`]).
pub fn resolution_width_closure(
    clauses: &[Vec<Lit>],
    w: usize,
    convention: WidthConvention,
) -> BTreeSet<MaskClause> {
    let mut set = seed(clauses, w, convention);
    let mut worklist: Vec<MaskClause> = set.iter().copied().collect();
    while let Some(a) = worklist.pop() {
        let snapshot: Vec<MaskClause> = set.iter().copied().collect();
        for b in snapshot {
            if let Some(r) = resolve_masks(a, b) {
                if mask_width(r) <= w && set.insert(r) {
                    worklist.push(r);
                }
            }
        }
    }
    set
}

/// Does a width-`w` resolution refutation exist under the convention? Decided by the closure.
pub fn width_refutes(clauses: &[Vec<Lit>], w: usize, convention: WidthConvention) -> bool {
    resolution_width_closure(clauses, w, convention).contains(&(0, 0))
}

/// **The minimum resolution-refutation width** under a convention: the least `w` whose closure derives
/// the empty clause. `None` iff the formula is satisfiable (resolution is complete at `w = num_vars`).
pub fn min_res_width_clauses(
    num_vars: usize,
    clauses: &[Vec<Lit>],
    convention: WidthConvention,
) -> Option<usize> {
    (0..=num_vars).find(|&w| width_refutes(clauses, w, convention))
}

/// **Re-check a resolution-width lower-bound certificate** (zero trust in the producer). `closed`
/// certifies "no width-`w` resolution refutation of `clauses` under `convention`" iff:
/// (i) it contains every admissible axiom, (ii) it is tautology-free, (iii) it is closed under
/// width-`≤ w` resolution, and (iv) it lacks the empty clause. Soundness is an induction over any
/// purported width-`w` derivation: its every clause lies in `closed`, and `⊥ ∉ closed`. A padded
/// (superset) certificate still certifies — extra clauses only weaken the producer, never the bound.
pub fn check_res_width_lower_bound(
    clauses: &[Vec<Lit>],
    w: usize,
    convention: WidthConvention,
    closed: &BTreeSet<MaskClause>,
) -> bool {
    if closed.contains(&(0, 0)) {
        return false; // the empty clause is a refutation, not a lower bound
    }
    if closed.iter().any(|&(p, n)| p & n != 0) {
        return false; // tautologies are outside the derivation calculus
    }
    if !seed(clauses, w, convention).iter().all(|m| closed.contains(m)) {
        return false; // an admissible axiom is missing — derivations could escape the set
    }
    let all: Vec<MaskClause> = closed.iter().copied().collect();
    for (i, &a) in all.iter().enumerate() {
        for &b in &all[i..] {
            if let Some(r) = resolve_masks(a, b) {
                if mask_width(r) <= w && !closed.contains(&r) {
                    return false; // not closed — a width-w step escapes
                }
            }
        }
    }
    true
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/res_width/tests.rs"]
mod tests;
