//! Satisfaction-Driven Clause Learning (Heule, Kiesl & Seidl) — the engine that reasons at the
//! Extended-Frege level by *discovering* propagation-redundant clauses, not just learning implied
//! ones. It is symmetry breaking taken to its limit: where certified symmetry breaking needs an
//! automorphism to supply a witness, SDCL **finds the witness by solving the positive reduct** — a
//! smaller SAT problem — so it works on *any* structure, symmetric or not.
//!
//! When the search is stuck at a partial assignment `α` it cannot extend, the clause `C = ¬α` would
//! block that subtree. `C` is propagation-redundant iff the positive reduct `{C} ∪ {D ∈ F : α ⊨ D}`
//! is satisfiable; a satisfying `ω` is the witness. Every clause this finds is gated through the
//! independent PR checker ([`crate::pr::is_pr`]), so it is sound by construction and emits a
//! checkable PR step — the same machine-checkable currency as the rest of the campaign.

use crate::cdcl::{Lit, SolveResult, Solver, Var};
use crate::pr::{check_pr_refutation, is_pr};
use crate::proof::{Perm, ProofStep, Witness};
use crate::sym_certify::CertifiedRefutation;
use crate::symmetry_detect::find_generators;
use std::collections::HashSet;

/// From a "stuck" partial assignment `alpha` (its true-literals), try to discover a PR clause
/// `C = ¬alpha` that blocks it, via the positive reduct. Returns the clause and its witness, both
/// verified by the PR checker (fail-closed). This is the SDCL primitive — a universal,
/// automorphism-free generalization of certified symmetry breaking.
pub fn find_pr_clause(
    num_vars: usize,
    clauses: &[Vec<Lit>],
    alpha: &[Lit],
) -> Option<(Vec<Lit>, Witness)> {
    // C = ¬alpha — every literal of alpha, negated.
    let c: Vec<Lit> = alpha.iter().map(|l| l.negated()).collect();
    if c.is_empty() {
        return None;
    }
    // Positive reduct: a witness ω must satisfy C and every clause alpha already satisfies. The
    // witness is read off over just the *relevant* variables (C's plus those satisfied clauses'),
    // so untouched clauses impose no obligation on the PR check.
    let mut relevant: HashSet<Var> = c.iter().map(|l| l.var()).collect();
    let mut reduct: Vec<Vec<Lit>> = vec![c.clone()];
    for d in clauses {
        if d.iter().any(|l| alpha.contains(l)) {
            for l in d {
                relevant.insert(l.var());
            }
            reduct.push(d.clone());
        }
    }
    let mut solver = Solver::new(num_vars);
    for cl in &reduct {
        solver.add_clause(cl.clone());
    }
    if let SolveResult::Sat(model) = solver.solve() {
        let omega: Vec<Lit> = relevant
            .iter()
            .map(|&v| Lit::new(v, model[v as usize]))
            .collect();
        let witness = Witness::Assignment(omega);
        // Independent sound gate: only return a clause the PR checker actually certifies.
        if is_pr(num_vars, clauses, &c, &witness) {
            return Some((c, witness));
        }
    }
    None
}

/// A fully automatic SDCL refutation: repeatedly discover propagation-redundant **unit** clauses
/// (by probing each literal as a stuck state and solving the positive reduct), add them as PR
/// steps, until the formula collapses to a RUP refutation. No symmetry, no problem-specific
/// knowledge, no hint — the solver finds the Extended-Frege proof itself, and emits it for the
/// independent checker. This is the universal engine the whole campaign was climbing toward.
/// The SDCL discovery core: repeatedly probe each literal and add any propagation-redundant clause
/// the unified finder certifies (positive-reduct assignment witness, or a substitution witness from
/// the round's freshly-detected symmetry group), until no more are found. Returns the augmented
/// database (a superset of the input — every added clause is satisfiability-preserving) and the PR
/// proof steps that justify the additions.
fn sdcl_discover(num_vars: usize, clauses: &[Vec<Lit>]) -> (Vec<Vec<Lit>>, Vec<ProofStep>) {
    let mut db = clauses.to_vec();
    let mut steps: Vec<ProofStep> = Vec::new();
    let cap = num_vars * num_vars + 8;
    while steps.len() < cap {
        let gens = find_generators(num_vars, &db);
        let mut progressed = false;
        'probe: for v in 0..num_vars as Var {
            for &val in &[true, false] {
                let alpha = [Lit::new(v, val)];
                if let Some((c, w)) = find_pr_clause_unified(num_vars, &db, &alpha, &gens) {
                    if !db.iter().any(|d| d == &c) {
                        db.push(c.clone());
                        steps.push(ProofStep::Pr {
                            clause: c,
                            witness: w,
                        });
                        progressed = true;
                        break 'probe;
                    }
                }
            }
        }
        if !progressed {
            break;
        }
    }
    (db, steps)
}

pub fn sdcl_refute(num_vars: usize, clauses: &[Vec<Lit>]) -> CertifiedRefutation {
    let (db, mut steps) = sdcl_discover(num_vars, clauses);
    let sbp_clauses = steps.len();
    let mut solver = Solver::new(num_vars);
    for c in &db {
        solver.add_clause(c.clone());
    }
    match solver.solve() {
        SolveResult::Sat(_) => CertifiedRefutation {
            refuted: false,
            sbp_clauses,
            steps,
        },
        SolveResult::Unsat => {
            for lc in solver.learned() {
                steps.push(ProofStep::Rup(lc.lits.clone()));
            }
            // Fail-closed: the composed proof, or a plain CDCL refutation if it doesn't certify.
            if check_pr_refutation(num_vars, clauses, &steps) {
                CertifiedRefutation {
                    refuted: true,
                    sbp_clauses,
                    steps,
                }
            } else {
                CertifiedRefutation {
                    refuted: true,
                    sbp_clauses: 0,
                    steps: plain_cdcl_refutation(num_vars, clauses),
                }
            }
        }
    }
}

/// The verdict of the unified certified solver.
pub enum CertifiedOutcome {
    /// Unsatisfiable, with a machine-checkable refutation (PR discovery steps + RUP learned steps)
    /// that re-checks against the original formula, and how many clauses SDCL discovered.
    Unsat {
        steps: Vec<ProofStep>,
        discovered: usize,
    },
    /// Satisfiable, with a model over `0..num_vars`.
    Sat(Vec<bool>),
}

/// Propagate only forced root literals, then validate a complete witness
/// against the original input. This never declares UNSAT and never branches:
/// inconclusive or conflicting inputs keep the existing certified SDCL path.
fn checked_root_model(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<Vec<bool>> {
    if clauses
        .iter()
        .flatten()
        .any(|lit| lit.var() as usize >= num_vars)
    {
        return None;
    }
    let mut assignment = vec![None; num_vars];
    loop {
        let mut changed = false;
        for clause in clauses {
            let mut satisfied = false;
            let mut unassigned = None;
            let mut multiple = false;
            for &literal in clause {
                match assignment[literal.var() as usize] {
                    Some(value) if value == literal.is_positive() => {
                        satisfied = true;
                        break;
                    }
                    Some(_) => {}
                    None => match unassigned {
                        None => unassigned = Some(literal),
                        Some(previous) if previous != literal => multiple = true,
                        Some(_) => {}
                    },
                }
            }
            if satisfied {
                continue;
            }
            let literal = unassigned?;
            if !multiple {
                assignment[literal.var() as usize] = Some(literal.is_positive());
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    let model: Vec<bool> = assignment
        .into_iter()
        .map(|value| value.unwrap_or(false))
        .collect();
    // The cheap root path earns SAT only by a witness that checks EVERY input
    // clause, including clauses that propagation never used.
    clauses
        .iter()
        .all(|clause| {
            clause
                .iter()
                .any(|lit| model[lit.var() as usize] == lit.is_positive())
        })
        .then_some(model)
}

/// The unified certified solver. Root unit propagation first tries to establish
/// a model, checked against every original clause. Otherwise SDCL discovers
/// propagation-redundant clauses through positive reducts and live symmetry,
/// then CDCL solves the augmented formula. UNSAT returns an independently
/// checked refutation; SAT returns a model of the original input.
pub fn solve_certified(num_vars: usize, clauses: &[Vec<Lit>]) -> CertifiedOutcome {
    solve_with_discovery(num_vars, clauses, || sdcl_discover(num_vars, clauses))
}

fn solve_with_discovery(
    num_vars: usize,
    clauses: &[Vec<Lit>],
    discover: impl FnOnce() -> (Vec<Vec<Lit>>, Vec<ProofStep>),
) -> CertifiedOutcome {
    if let Some(model) = checked_root_model(num_vars, clauses) {
        return CertifiedOutcome::Sat(model);
    }
    let (db, mut steps) = discover();
    let discovered = steps.len();
    let mut solver = Solver::new(num_vars);
    for c in &db {
        solver.add_clause(c.clone());
    }
    match solver.solve() {
        SolveResult::Sat(model) => CertifiedOutcome::Sat(model),
        SolveResult::Unsat => {
            for lc in solver.learned() {
                steps.push(ProofStep::Rup(lc.lits.clone()));
            }
            // FAIL-CLOSED: a certified solver must never return an unchecked proof. Verify the
            // composed refutation; if the SDCL composition does not certify, fall back to a plain
            // CDCL refutation of the original (always RUP-checkable). The original is UNSAT iff the
            // augmented database is, since every discovered clause is satisfiability-preserving.
            if check_pr_refutation(num_vars, clauses, &steps) {
                CertifiedOutcome::Unsat { steps, discovered }
            } else {
                CertifiedOutcome::Unsat {
                    steps: plain_cdcl_refutation(num_vars, clauses),
                    discovered: 0,
                }
            }
        }
    }
}

/// A plain CDCL refutation of `clauses` as RUP proof steps — always re-checkable. The certified
/// fallback that guarantees [`solve_certified`] and [`sdcl_refute`] never emit an unchecked proof,
/// and the universal source of a `cake_lpr`-checkable LRAT certificate for any UNSAT instance.
pub fn plain_cdcl_refutation(num_vars: usize, clauses: &[Vec<Lit>]) -> Vec<ProofStep> {
    let mut solver = Solver::new(num_vars);
    for c in clauses {
        solver.add_clause(c.clone());
    }
    let _ = solver.solve();
    solver
        .learned()
        .iter()
        .map(|lc| ProofStep::Rup(lc.lits.clone()))
        .collect()
}

/// The unified PR-clause finder: the universal SDCL discovery (positive-reduct assignment witness)
/// PLUS the scalable symmetry path (substitution witnesses from the supplied generators). The
/// assignment witness is general but can fail to certify at scale (it needs `F|α` UP-refutable); a
/// substitution witness from a live automorphism certifies the lead clauses at *any* size via an
/// immediate conflict. Trying both gives SDCL that discovers AND scales.
pub fn find_pr_clause_unified(
    num_vars: usize,
    clauses: &[Vec<Lit>],
    alpha: &[Lit],
    generators: &[Perm],
) -> Option<(Vec<Lit>, Witness)> {
    if let Some(found) = find_pr_clause(num_vars, clauses, alpha) {
        return Some(found);
    }
    let c: Vec<Lit> = alpha.iter().map(|l| l.negated()).collect();
    for sigma in generators {
        let witness = Witness::Substitution(sigma.extended(num_vars));
        if is_pr(num_vars, clauses, &c, &witness) {
            return Some((c, witness));
        }
    }
    None
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/sdcl/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/sdcl/root_propagation.rs"]
mod root_propagation;
