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

use crate::cdcl::{Lit, SolveResult};
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
            let learned: Vec<Vec<Lit>> = solver.learned().iter().map(|c| c.lits.clone()).collect();
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

/// What a clause says under an assignment.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ClauseState {
    /// Every literal is false.
    Conflict,
    /// No literal is true and exactly one distinct literal is unset: it is forced.
    Unit(Lit),
    /// A literal is true, or two distinct literals are unset (a tautology's `x` and `¬x` among them): no literal
    /// after them can make the clause a conflict or a unit.
    Quiet,
}

/// The state of `clause` under `assign`. A repeated literal counts once (`x ∨ x` is the unit `x`); a tautology
/// (`x ∨ ¬x`) has two distinct unset literals and never propagates.
#[inline]
pub(crate) fn clause_state(clause: &[Lit], assign: &[Option<bool>]) -> ClauseState {
    let mut unset: Option<Lit> = None;
    for &l in clause {
        match lit_val(assign, l) {
            Some(true) => return ClauseState::Quiet,
            Some(false) => {}
            None => match unset {
                None => unset = Some(l),
                Some(u) if u == l => {}
                Some(_) => return ClauseState::Quiet,
            },
        }
    }
    unset.map_or(ClauseState::Conflict, ClauseState::Unit)
}

/// Fixpoint unit propagation over `db`. Returns `true` on conflict (some clause all-false).
/// Naive on purpose: a checker you can read in one sitting is a checker you can trust.
pub(crate) fn propagate(db: &[Vec<Lit>], assign: &mut [Option<bool>]) -> bool {
    loop {
        let mut changed = false;
        for clause in db {
            match clause_state(clause, assign) {
                ClauseState::Conflict => return true,
                ClauseState::Unit(l) => {
                    set_true(assign, l);
                    changed = true;
                }
                ClauseState::Quiet => {}
            }
        }
        if !changed {
            return false;
        }
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/rup/tests.rs"]
mod tests;
