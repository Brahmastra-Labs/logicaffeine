//! **Certified resolution-width lower bounds** — the third proof system of the separations atlas,
//! with the same zero-trust certificate discipline as the Nullstellensatz dual witnesses.
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
//! [`WidthConvention::Strict`] counts axioms against the budget (the census's
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
    /// Ben-Sasson–Wigderson refutation-width measure, and the census's convention).
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
mod tests {
    use super::*;
    use crate::hypercube::Subcube;

    /// **The clause-mask closure agrees with the census's geometric oracle.** The census computes
    /// `min_resolution_width` on subcube covers (`Subcube::resolve` — the same single-pivot rule seen
    /// from the blocker side). On every minimal-UNSAT orbit representative at `n ≤ 3`, the mask-clause
    /// closure under [`WidthConvention::Strict`] must reproduce the recorded width exactly; and on the
    /// SAT side, no width ever refutes.
    #[test]
    fn width_closure_matches_the_subcube_resolution_width_on_the_census() {
        for n in 1..=3usize {
            for rec in crate::census::census(n) {
                let clauses: Vec<Vec<Lit>> = rec
                    .rep
                    .blockers
                    .iter()
                    .map(|b: &Subcube| {
                        b.clause_literals()
                            .into_iter()
                            .map(|(v, positive)| Lit::new(v as u32, positive))
                            .collect()
                    })
                    .collect();
                let ours = min_res_width_clauses(n, &clauses, WidthConvention::Strict);
                assert_eq!(
                    ours,
                    Some(rec.min_res_width),
                    "n={n}: mask-clause closure width = census width on the orbit representative"
                );
            }
        }
        // SAT ⟹ no refutation at any width, under either convention.
        let sat = vec![vec![Lit::pos(0), Lit::pos(1)], vec![Lit::neg(0), Lit::pos(2)]];
        for conv in [WidthConvention::Strict, WidthConvention::WideAxioms] {
            assert_eq!(min_res_width_clauses(3, &sat, conv), None, "a satisfiable formula has no width");
        }
    }

    /// **The closed clause set is a re-checkable lower-bound certificate.** For UNSAT formulas with
    /// measured minimum width `w*`: at every `w < w*` the closure (without `⊥`) passes the independent
    /// checker — certifying `res-width > w` with zero trust in the producer — and the checker fails
    /// closed on every mutilation: a set containing `⊥`, a set missing an axiom, a set with a closure
    /// hole, and a set smuggling a tautology.
    #[test]
    fn resolution_width_lower_bounds_are_certified_by_the_closed_clause_set() {
        let p = |v: u32| Lit::pos(v);
        let q = |v: u32| Lit::neg(v);
        // The transitive-XOR core (min width 3 among Strict derivations) and PHP(3).
        let xor_core = vec![
            vec![q(0), p(1)], vec![p(0), q(1)],
            vec![q(1), p(2)], vec![p(1), q(2)],
            vec![p(0), p(2)], vec![q(0), q(2)],
        ];
        let (php3, _) = crate::families::php(3);
        for (nv, clauses) in [(3usize, xor_core), (php3.num_vars, php3.clauses)] {
            for conv in [WidthConvention::Strict, WidthConvention::WideAxioms] {
                let wstar = min_res_width_clauses(nv, &clauses, conv)
                    .expect("UNSAT ⟹ some width refutes");
                for w in 0..wstar {
                    let closed = resolution_width_closure(&clauses, w, conv);
                    assert!(
                        !closed.contains(&(0, 0)),
                        "below the minimum width the closure must not refute (w={w})"
                    );
                    assert!(
                        check_res_width_lower_bound(&clauses, w, conv, &closed),
                        "the closure certifies res-width > {w}"
                    );
                    // Fail-closed on mutilations.
                    let mut with_empty = closed.clone();
                    with_empty.insert((0, 0));
                    assert!(
                        !check_res_width_lower_bound(&clauses, w, conv, &with_empty),
                        "a set containing ⊥ is no lower bound"
                    );
                    if let Some(&first) = closed.iter().next() {
                        let mut missing = closed.clone();
                        missing.remove(&first);
                        assert!(
                            !check_res_width_lower_bound(&clauses, w, conv, &missing),
                            "a set with a hole (missing axiom or closure gap) must be rejected"
                        );
                    }
                    let mut with_taut = closed.clone();
                    with_taut.insert((1, 1));
                    assert!(
                        !check_res_width_lower_bound(&clauses, w, conv, &with_taut),
                        "a tautology-smuggling set must be rejected"
                    );
                }
                // At w*, the closure genuinely refutes — the two-sidedness of the measure.
                assert!(width_refutes(&clauses, wstar, conv), "the closure refutes at w*");
            }
        }
    }

    /// **Tseitin expanders carry certified resolution-width lower bounds.** Parity on a 3-regular
    /// expander is the classical width-hard family (Ben-Sasson–Wigderson: width `Ω(expansion)`). We pin
    /// the exact minimum width on small expanders under both conventions (they agree — Tseitin axioms
    /// are already narrow, width 3) and certify each `w < w*` with the re-checked closed-set
    /// certificate. This is the resolution half of the Tseitin atlas row (the GF(2)-rank refutation is
    /// its polynomial upper half).
    #[test]
    fn tseitin_expander_has_certified_resolution_width_lower_bounds() {
        for n in [6usize, 8] {
            let (_eqs, cnf, verdict) = crate::families::tseitin_expander(n, 0xC0FFEE + n as u64);
            assert!(matches!(verdict, crate::families::ExpectedVerdict::Unsat));
            let ws = min_res_width_clauses(cnf.num_vars, &cnf.clauses, WidthConvention::Strict)
                .expect("Tseitin expanders are UNSAT");
            let ww = min_res_width_clauses(cnf.num_vars, &cnf.clauses, WidthConvention::WideAxioms)
                .expect("Tseitin expanders are UNSAT");
            assert_eq!(ws, ww, "n={n}: width-3 axioms ⟹ the conventions coincide");
            assert!(ws > 3, "n={n}: the width exceeds the axiom width — a genuine, non-axiom bound");
            for w in [ws - 1] {
                let closed = resolution_width_closure(&cnf.clauses, w, WidthConvention::Strict);
                assert!(
                    check_res_width_lower_bound(&cnf.clauses, w, WidthConvention::Strict, &closed),
                    "n={n}: certified res-width > {w}"
                );
            }
            eprintln!("tseitin({n}): {} vars, certified min res-width = {ws}", cnf.num_vars);
        }
    }

    /// **Pigeonhole's certified width completes its three-system atlas row.** Under
    /// [`WidthConvention::WideAxioms`] (the convention that makes wide-axiom width non-trivial) PHP(m)'s
    /// minimum refutation width is measured exactly and its lower half certified by the closed set —
    /// alongside the certified NS degree `2(m−1)` (dual witness) and the `m(m−1)/2`-step SR upper bound,
    /// this is the third machine-certified coordinate of the same family. The width grows with `m`.
    #[test]
    fn php_resolution_width_certificate_completes_the_three_system_row() {
        let mut widths = Vec::new();
        for m in [3usize, 4] {
            let (php, _) = crate::families::php(m);
            let w = min_res_width_clauses(php.num_vars, &php.clauses, WidthConvention::WideAxioms)
                .expect("PHP is UNSAT");
            let closed = resolution_width_closure(&php.clauses, w - 1, WidthConvention::WideAxioms);
            assert!(
                check_res_width_lower_bound(&php.clauses, w - 1, WidthConvention::WideAxioms, &closed),
                "PHP({m}): certified res-width > {}",
                w - 1
            );
            eprintln!("PHP({m}): certified min res-width (wide axioms) = {w}");
            widths.push(w);
        }
        assert!(widths[1] > widths[0], "the pigeonhole width grows with m: {widths:?}");
    }
}
