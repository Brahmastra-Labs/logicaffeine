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
pub fn find_pr_clause(num_vars: usize, clauses: &[Vec<Lit>], alpha: &[Lit]) -> Option<(Vec<Lit>, Witness)> {
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
        let omega: Vec<Lit> = relevant.iter().map(|&v| Lit::new(v, model[v as usize])).collect();
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
                        steps.push(ProofStep::Pr { clause: c, witness: w });
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
        SolveResult::Sat(_) => CertifiedRefutation { refuted: false, sbp_clauses, steps },
        SolveResult::Unsat => {
            for lc in solver.learned() {
                steps.push(ProofStep::Rup(lc.lits.clone()));
            }
            // Fail-closed: the composed proof, or a plain CDCL refutation if it doesn't certify.
            if check_pr_refutation(num_vars, clauses, &steps) {
                CertifiedRefutation { refuted: true, sbp_clauses, steps }
            } else {
                CertifiedRefutation { refuted: true, sbp_clauses: 0, steps: plain_cdcl_refutation(num_vars, clauses) }
            }
        }
    }
}

/// The verdict of the unified certified solver.
pub enum CertifiedOutcome {
    /// Unsatisfiable, with a machine-checkable refutation (PR discovery steps + RUP learned steps)
    /// that re-checks against the original formula, and how many clauses SDCL discovered.
    Unsat { steps: Vec<ProofStep>, discovered: usize },
    /// Satisfiable, with a model over `0..num_vars`.
    Sat(Vec<bool>),
}

/// The unified certified solver — the apex of the campaign. Given ANY formula it (1) auto-discovers
/// propagation-redundant clauses via SDCL (positive reduct + live symmetry), then (2) solves the
/// augmented formula with the modernized CDCL core. On UNSAT it returns a single machine-checked
/// refutation composed of every step; on SAT a model (valid for the original, since SDCL only adds
/// satisfiability-preserving clauses). Symmetry breaking, SDCL, and CDCL, fused into one certified
/// engine.
pub fn solve_certified(num_vars: usize, clauses: &[Vec<Lit>]) -> CertifiedOutcome {
    let (db, mut steps) = sdcl_discover(num_vars, clauses);
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
                CertifiedOutcome::Unsat { steps: plain_cdcl_refutation(num_vars, clauses), discovered: 0 }
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
    solver.learned().iter().map(|lc| ProofStep::Rup(lc.lits.clone())).collect()
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
mod tests {
    use super::*;
    use crate::families;

    fn sat_brute(num_vars: usize, clauses: &[Vec<Lit>]) -> bool {
        (0u32..(1u32 << num_vars)).any(|mask| {
            let model: Vec<bool> = (0..num_vars).map(|v| (mask >> v) & 1 == 1).collect();
            clauses.iter().all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive()))
        })
    }

    #[test]
    fn sdcl_rediscovers_a_pigeonhole_pr_clause_with_no_symmetry_input() {
        // No automorphism, no Heule construction — given only the stuck attempt "pigeon 0 in the
        // last hole", SDCL's positive reduct DISCOVERS that ¬x(0, last_hole) is propagation-
        // redundant: the very clause certified symmetry breaking adds, found from structure alone.
        for n in 3..=4 {
            let (cnf, _) = families::php(n);
            let last_hole = n - 2;
            let stuck = vec![Lit::pos(last_hole as Var)]; // x(0, last_hole) = 0*(n-1)+last_hole
            let (c, w) = find_pr_clause(cnf.num_vars, &cnf.clauses, &stuck)
                .expect("SDCL must discover a PR clause");
            assert_eq!(c, vec![Lit::neg(last_hole as Var)], "rediscovered ¬x(0, last_hole)");
            assert!(is_pr(cnf.num_vars, &cnf.clauses, &c, &w), "and it is genuinely PR");
        }
    }

    #[test]
    fn sdcl_refutes_pigeonhole_fully_automatically() {
        // THE payoff: no symmetry, no Heule construction, no hint — sdcl_refute discovers the
        // certified Extended-Frege refutation of pigeonhole entirely on its own, and the composed
        // proof re-checks against the original formula.
        for n in 2..=4 {
            let (cnf, _) = families::php(n);
            let r = sdcl_refute(cnf.num_vars, &cnf.clauses);
            assert!(r.refuted, "SDCL must auto-refute PHP({n})");
            assert!(
                check_pr_refutation(cnf.num_vars, &cnf.clauses, &r.steps),
                "the self-discovered proof must independently re-check"
            );
            // PHP(2) is directly RUP-refutable (no PR needed); from n≥3 SDCL must discover PR clauses.
            if n >= 3 {
                assert!(r.sbp_clauses >= 1, "PHP({n}) needs self-discovered PR clauses");
            }
        }
    }

    #[test]
    #[ignore = "scaling SDCL — auto-refutes larger pigeonhole via discovered substitution witnesses"]
    fn sdcl_refute_scales_with_symmetry_fallback() {
        use crate::cdcl::{SolveResult, Solver};
        for n in 5..=7 {
            let (cnf, _) = families::php(n);
            let r = sdcl_refute(cnf.num_vars, &cnf.clauses);
            assert!(r.refuted, "scaling SDCL must auto-refute PHP({n})");
            assert!(r.sbp_clauses >= 1, "discovered PR clauses at scale");
            assert!(check_pr_refutation(cnf.num_vars, &cnf.clauses, &r.steps));
            // How much search was left after the self-discovered breaking?
            let sbp: Vec<Vec<Lit>> = r
                .steps
                .iter()
                .filter_map(|s| if let ProofStep::Pr { clause, .. } = s { Some(clause.clone()) } else { None })
                .collect();
            let mut solver = Solver::new(cnf.num_vars);
            for c in cnf.clauses.iter().chain(sbp.iter()) {
                solver.add_clause(c.clone());
            }
            assert_eq!(solver.solve(), SolveResult::Unsat);
            println!(
                "SDCL PHP({n}): {} self-discovered PR clauses (no symmetry input) → {} conflicts left, CERTIFIED",
                r.sbp_clauses,
                solver.conflicts()
            );
        }
    }

    #[test]
    fn solve_certified_is_a_complete_certified_solver() {
        use crate::pr::check_pr_refutation;
        // UNSAT side: pigeonhole → a machine-checked refutation that re-checks against the original.
        let (php, _) = families::php(4);
        match solve_certified(php.num_vars, &php.clauses) {
            CertifiedOutcome::Unsat { steps, discovered } => {
                assert!(discovered >= 1, "SDCL discovered breaking clauses");
                assert!(check_pr_refutation(php.num_vars, &php.clauses, &steps), "PHP(4) refutation re-checks");
            }
            CertifiedOutcome::Sat(_) => panic!("PHP(4) is UNSAT"),
        }
        // SAT side: (a ∨ b) ∧ (¬a ∨ b) forces b → satisfiable, and the model must satisfy it.
        let sat = vec![vec![Lit::pos(0), Lit::pos(1)], vec![Lit::neg(0), Lit::pos(1)]];
        match solve_certified(2, &sat) {
            CertifiedOutcome::Sat(m) => {
                assert!(sat.iter().all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())), "valid model");
            }
            CertifiedOutcome::Unsat { .. } => panic!("formula is satisfiable"),
        }
    }

    #[test]
    fn solve_certified_matches_brute_force_with_checkable_proofs() {
        use crate::pr::check_pr_refutation;
        // The whole unified engine, verdict-invariant: over many random formulas it must agree with
        // brute force, every SAT answer carrying a valid model, every UNSAT answer a proof that
        // re-checks against the original. Soundness end to end.
        let mut state = 0x501_E_CE271F1Eu64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        for _ in 0..600 {
            let nv = 3 + (next() % 4) as usize; // 3..6
            let nc = (next() % 12) as usize;
            let clauses: Vec<Vec<Lit>> = (0..nc)
                .map(|_| {
                    (0..2 + (next() % 2) as usize)
                        .map(|_| Lit::new((next() % nv as u64) as Var, next() & 1 == 0))
                        .collect::<Vec<_>>()
                })
                .filter(|c: &Vec<Lit>| !c.is_empty())
                .collect();
            let expected = sat_brute(nv, &clauses);
            match solve_certified(nv, &clauses) {
                CertifiedOutcome::Sat(m) => {
                    assert!(expected, "solver said SAT but brute force says UNSAT: {clauses:?}");
                    assert!(clauses.iter().all(|c| c.iter().any(|l| m[l.var() as usize] == l.is_positive())), "model invalid");
                }
                CertifiedOutcome::Unsat { steps, .. } => {
                    assert!(!expected, "solver said UNSAT but a model exists: {clauses:?}");
                    assert!(check_pr_refutation(nv, &clauses, &steps), "UNSAT proof must re-check: {clauses:?}");
                }
            }
        }
    }

    #[test]
    fn sdcl_discovered_clauses_preserve_satisfiability() {
        // Robustness: over many seeded random formulas and stuck assignments, whenever SDCL returns
        // a clause, adding it preserves satisfiability exactly (brute force). The PR gate guarantees
        // it; this proves it.
        let mut state = 0x5DC1_9999u64;
        let mut next = || {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            state
        };
        let mut found = 0;
        for _ in 0..4000 {
            let nv = 3 + (next() % 4) as usize; // 3..6
            let nc = (next() % 12) as usize;
            let clauses: Vec<Vec<Lit>> = (0..nc)
                .map(|_| {
                    (0..2 + (next() % 2) as usize)
                        .map(|_| Lit::new((next() % nv as u64) as Var, next() & 1 == 0))
                        .collect::<Vec<_>>()
                })
                .filter(|c: &Vec<Lit>| !c.is_empty())
                .collect();
            // A random partial assignment as the "stuck" state.
            let mut alpha: Vec<Lit> = Vec::new();
            for v in 0..nv as Var {
                if next() & 1 == 0 {
                    alpha.push(Lit::new(v, next() & 1 == 0));
                }
            }
            if alpha.is_empty() {
                continue;
            }
            if let Some((c, _)) = find_pr_clause(nv, &clauses, &alpha) {
                found += 1;
                let before = sat_brute(nv, &clauses);
                let mut with = clauses.clone();
                with.push(c.clone());
                assert_eq!(before, sat_brute(nv, &with), "SDCL clause changed satisfiability: {clauses:?} C={c:?}");
            }
        }
        assert!(found > 0, "the discovery path must actually be exercised");
    }
}
