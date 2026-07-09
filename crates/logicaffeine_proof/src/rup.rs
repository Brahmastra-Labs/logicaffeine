//! The **RUP** (Reverse Unit Propagation) linear checker — the fast default trust tier.
//!
//! Modern certified SAT separates an untrusted, fast solver from a tiny, auditable checker
//! that replays the solver's resolution proof by unit propagation alone (Goldberg & Novikov,
//! 2003 — RUP; Wetzler/Heule/Hunt, 2014 — DRAT; Cruz-Filipe et al., 2017 — LRAT; Tan/Heule/
//! Myreen, 2021 — the formally-verified `cake_lpr`). A learned clause `C` is valid iff
//! assuming `¬C` and unit-propagating the current clause database hits a conflict; the
//! refutation is verified iff, after adding every learned clause in order, unit propagation
//! over the whole database derives the empty clause.
//!
//! This module re-derives NOTHING symbolic and builds no proof tree — so it sidesteps the
//! emit-cost wall that sank eager probing. The solve produces the learned-clause trace for
//! free ([`crate::cdcl::Solver::learned`]); here we *check* it, independently of the solver.
//! The checker is deliberately small and naive (linear, fixpoint unit propagation) — its
//! simplicity IS the trust. If it cannot confirm the solver's UNSAT, we fail closed.

use crate::cdcl::{Lit, SolveResult, Solver};
use crate::cnf::Cnf;
use crate::ProofExpr;

/// A certified propositional entailment verdict.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Verdict {
    /// `premises ⊨ goal`, and an independent RUP replay confirmed the refutation.
    Entailed,
    /// `premises ⊭ goal` (the solver found a model of `premises ∧ ¬goal`).
    NotEntailed,
}

/// Decide `premises ⊨ goal` with the CDCL solver and **independently certify** an
/// `Entailed` verdict by RUP replay. `None` if the problem is not purely propositional over
/// recognisable atoms, OR if the solver claims UNSAT but the checker cannot confirm it (a
/// solver bug — fail closed, never report a false `Entailed`).
pub fn entails_certified(premises: &[ProofExpr], goal: &ProofExpr) -> Option<Verdict> {
    let mut cnf = Cnf::new();
    for p in premises {
        cnf.assert(p)?;
    }
    certified_from_cnf(cnf, goal)
}

/// Certify `prepared ⊨ goal` against a premise CNF clausified ONCE by
/// [`Cnf::from_premises`]. Solving a whole puzzle this way pays the of-pair Tseitin cost a
/// single time, then each cell only adds its `¬goal` unit — the incremental win.
pub fn entails_certified_prepared(prepared: &Cnf, goal: &ProofExpr) -> Option<Verdict> {
    certified_from_cnf(prepared.clone(), goal)
}

/// Shared core: add `¬goal` to `cnf`, solve, and RUP-certify an UNSAT (entailed) verdict.
fn certified_from_cnf(mut cnf: Cnf, goal: &ProofExpr) -> Option<Verdict> {
    cnf.assert_neg(goal)?;
    let num_vars = cnf.num_vars();
    // Hand the clauses straight to the solver (one copy, inside the solver) — the RUP check
    // reads them back via `original_clauses()`, so the clause set is never copied again.
    let mut solver = cnf.into_solver();
    match solver.solve() {
        SolveResult::Sat(_) => Some(Verdict::NotEntailed),
        SolveResult::Unsat => {
            let learned: Vec<Vec<Lit>> =
                solver.learned().iter().map(|c| c.lits.clone()).collect();
            if check_refutation(num_vars, solver.original_clauses(), &learned) {
                Some(Verdict::Entailed)
            } else {
                None // solver said UNSAT, the trusted checker can't confirm → fail closed
            }
        }
    }
}

/// Verify a DRAT/LRAT-style refutation: every learned clause is RUP w.r.t. the database
/// built so far, and the full database is then refuted by unit propagation (the empty
/// clause is RUP). Returns `false` if any step does not check — so a corrupted or bogus
/// trace is rejected.
pub fn check_refutation(num_vars: usize, original: &[Vec<Lit>], learned: &[Vec<Lit>]) -> bool {
    let mut db: Vec<Vec<Lit>> = original.to_vec();
    for c in learned {
        if !is_rup(num_vars, &db, c) {
            return false;
        }
        db.push(c.clone());
    }
    // The empty clause is RUP w.r.t. the final DB iff unit propagation alone conflicts.
    is_rup(num_vars, &db, &[])
}

/// Is clause `c` derivable by reverse unit propagation from `db`? Assume `¬c` (set each of
/// `c`'s literals false) and unit-propagate `db`; `c` is RUP iff that reaches a conflict.
pub(crate) fn is_rup(num_vars: usize, db: &[Vec<Lit>], c: &[Lit]) -> bool {
    let mut assign: Vec<Option<bool>> = vec![None; num_vars];
    for &l in c {
        // Assume ¬l. If that already clashes, ¬c is unsatisfiable ⇒ c is (trivially) RUP.
        if !set_true(&mut assign, l.negated()) {
            return true;
        }
    }
    propagate(db, &mut assign)
}

/// The value of literal `l` under `assign`.
#[inline]
pub(crate) fn lit_val(assign: &[Option<bool>], l: Lit) -> Option<bool> {
    assign[l.var() as usize].map(|b| if l.is_positive() { b } else { !b })
}

/// Set `l` true. Returns `false` if `l` is already false (a conflict).
#[inline]
pub(crate) fn set_true(assign: &mut [Option<bool>], l: Lit) -> bool {
    match lit_val(assign, l) {
        Some(true) => true,
        Some(false) => false,
        None => {
            assign[l.var() as usize] = Some(l.is_positive());
            true
        }
    }
}

/// Fixpoint unit propagation over `db`. Returns `true` on conflict (some clause all-false).
/// Naive on purpose: a checker you can read in one sitting is a checker you can trust.
pub(crate) fn propagate(db: &[Vec<Lit>], assign: &mut [Option<bool>]) -> bool {
    loop {
        let mut changed = false;
        for clause in db {
            // Evaluate the clause robustly to DUPLICATE literals (`x ∨ x` is the unit `x`) and
            // TAUTOLOGIES (`x ∨ ¬x` is always satisfied) — both are legal CNF, and a trusted checker
            // must count *distinct* unset literals or it will miss real unit propagations.
            let mut satisfied = false;
            let mut unset: Vec<Lit> = Vec::new();
            for &l in clause {
                match lit_val(assign, l) {
                    Some(true) => {
                        satisfied = true;
                        break;
                    }
                    Some(false) => {}
                    None => {
                        if unset.contains(&l.negated()) {
                            satisfied = true; // tautology — never propagates, never conflicts
                            break;
                        }
                        if !unset.contains(&l) {
                            unset.push(l);
                        }
                    }
                }
            }
            if satisfied {
                continue;
            }
            if unset.is_empty() {
                return true; // every literal false ⇒ conflict
            }
            if unset.len() == 1 {
                set_true(assign, unset[0]);
                changed = true;
            }
        }
        if !changed {
            return false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdcl::Lit;
    use crate::{ProofExpr, ProofTerm};

    fn c(s: &str) -> ProofTerm {
        ProofTerm::Constant(s.to_string())
    }
    fn pred(name: &str, args: Vec<ProofTerm>) -> ProofExpr {
        ProofExpr::Predicate { name: name.to_string(), args, world: None }
    }
    fn not(e: ProofExpr) -> ProofExpr {
        ProofExpr::Not(Box::new(e))
    }
    fn or(a: ProofExpr, b: ProofExpr) -> ProofExpr {
        ProofExpr::Or(Box::new(a), Box::new(b))
    }
    fn imp(a: ProofExpr, b: ProofExpr) -> ProofExpr {
        ProofExpr::Implies(Box::new(a), Box::new(b))
    }

    #[test]
    fn certified_disjunctive_syllogism() {
        let p = pred("P", vec![c("a")]);
        let q = pred("Q", vec![c("a")]);
        let prem = vec![or(p.clone(), q.clone()), not(p.clone())];
        assert_eq!(entails_certified(&prem, &q), Some(Verdict::Entailed));
        assert_eq!(entails_certified(&[or(p, q.clone())], &q), Some(Verdict::NotEntailed));
    }

    #[test]
    fn certified_two_value_grid() {
        let in_ = |t: &str, s: &str| pred("In", vec![c(t), c(s)]);
        let (fl, me) = ("Florida", "Maine");
        let prem = vec![
            or(in_("Alpha", fl), in_("Alpha", me)),
            or(in_("Beta", fl), in_("Beta", me)),
            imp(in_("Alpha", fl), not(in_("Beta", fl))),
            imp(in_("Alpha", me), not(in_("Beta", me))),
            or(in_("Alpha", fl), in_("Beta", fl)),
            or(in_("Alpha", me), in_("Beta", me)),
            in_("Alpha", fl),
        ];
        assert_eq!(entails_certified(&prem, &in_("Beta", me)), Some(Verdict::Entailed));
        assert_eq!(entails_certified(&prem, &in_("Beta", fl)), Some(Verdict::NotEntailed));
    }

    #[test]
    fn rup_rejects_a_bogus_proof() {
        // Original `(p ∨ q)` is SATISFIABLE. A "proof" that bolts on the un-entailed units
        // `¬p`, `¬q` must NOT verify — the checker rejects clauses that are not RUP.
        let p = Lit::pos(0);
        let q = Lit::pos(1);
        let original = vec![vec![p, q]];
        let bogus_learned = vec![vec![p.negated()], vec![q.negated()]];
        assert!(
            !check_refutation(2, &original, &bogus_learned),
            "a non-RUP learned clause must be rejected"
        );
    }

    #[test]
    fn rup_needs_the_proof_not_just_the_clauses() {
        // A genuinely UNSAT formula whose unsatisfiability is NOT visible to unit
        // propagation alone (all four 2-literal clauses over p,q) must fail to verify
        // WITHOUT the solver's learned clauses — proving the checker really replays the
        // proof rather than re-searching.
        let p = Lit::pos(0);
        let q = Lit::pos(1);
        let original = vec![
            vec![p, q],
            vec![p, q.negated()],
            vec![p.negated(), q],
            vec![p.negated(), q.negated()],
        ];
        assert!(!check_refutation(2, &original, &[]), "UP alone cannot refute this; needs the proof");
        // With the resolvents the solver would learn (`p`, then `¬p` ⇒ empty), it checks.
        let learned = vec![vec![p], vec![p.negated()]];
        assert!(check_refutation(2, &original, &learned), "the learned resolvents make it RUP");
    }
}
