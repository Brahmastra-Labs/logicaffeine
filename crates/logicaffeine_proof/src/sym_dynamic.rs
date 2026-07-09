//! Dynamic symmetry breaking — **Symmetric Explanation Learning** (SEL), the in-search tier.
//!
//! Static symmetry breaking adds predicates up front (Phase 1). SEL is reactive: it watches what
//! the solver *learns* and multiplies each learned clause by the formula's symmetry group, so the
//! search never has to re-derive the symmetric twin of a lemma it already paid for. On a
//! symmetry-rich UNSAT instance that is the whole game — the exponential blow-up resolution suffers
//! is exactly the repeated rediscovery of symmetric variants.
//!
//! **The certification is free.** If a learned clause `C` is RUP w.r.t. the database and `σ` is an
//! automorphism of that database, then `σ(C)` is RUP too: apply `σ` to `C`'s unit-propagation
//! refutation and every clause it touches maps back into `σ(F) = F`. So a symmetric clause enters
//! the proof as a plain [`ProofStep::Rup`] — DRAT/LRAT-checkable, no PR witness needed. We add a
//! `σ(C)` only after re-confirming it is RUP against the *current* database (`rup::is_rup`), so
//! the procedure is **fail-closed**: an amplification that does not check is silently dropped, never
//! trusted.
//!
//! The loop alternates a conflict-budgeted solve ([`Solver::solve_budgeted`]) with an amplification
//! pass over the round's learned clauses, accumulating a single RUP refutation that
//! [`crate::pr::check_pr_refutation_fast`] verifies against the original formula alone.

use std::collections::HashSet;

use crate::cdcl::{BudgetedResult, Lit, Solver};
use crate::proof::ProofStep;
use crate::symmetry_detect::find_generators;

/// The result of an SEL refutation attempt.
#[derive(Clone, Debug)]
pub enum SelOutcome {
    /// Refuted, with a checkable RUP proof, the total conflicts spent, and how many clauses were
    /// added by symmetric amplification (the lever's footprint).
    Unsat { steps: Vec<ProofStep>, conflicts: u64, amplified: usize },
    /// Satisfiable, with a model.
    Sat(Vec<bool>),
    /// Gave up within the round/budget bounds without a verdict (the procedure is deliberately
    /// incomplete — it never returns a wrong answer, only an honest "don't know").
    Unknown { conflicts: u64 },
}

/// Canonical clause key (sorted, deduped literal codes) for the seen-set.
fn canon(c: &[Lit]) -> Vec<u32> {
    let mut k: Vec<u32> = c.iter().map(|l| l.var() * 2 + u32::from(!l.is_positive())).collect();
    k.sort_unstable();
    k.dedup();
    k
}

/// Add `x` as a RUP step iff it is new and genuinely RUP against the current database (fail-closed).
fn try_add(
    num_vars: usize,
    db: &mut Vec<Vec<Lit>>,
    steps: &mut Vec<ProofStep>,
    seen: &mut HashSet<Vec<u32>>,
    x: Vec<Lit>,
) -> bool {
    let key = canon(&x);
    if seen.contains(&key) {
        return false;
    }
    if crate::rup::is_rup(num_vars, db, &x) {
        seen.insert(key);
        db.push(x.clone());
        steps.push(ProofStep::Rup(x));
        true
    } else {
        false
    }
}

/// Refute `clauses` with Symmetric Explanation Learning, or report SAT / Unknown. The conflict
/// budget per round and the round cap bound the work; symmetric amplification is what makes the
/// total conflict count collapse on symmetry-rich instances.
pub fn sel_refute(num_vars: usize, clauses: &[Vec<Lit>]) -> SelOutcome {
    let gens = find_generators(num_vars, clauses);
    let mut db: Vec<Vec<Lit>> = clauses.to_vec();
    let mut steps: Vec<ProofStep> = Vec::new();
    let mut seen: HashSet<Vec<u32>> = db.iter().map(|c| canon(c)).collect();
    let mut total_conflicts = 0u64;
    let mut amplified = 0usize;
    let mut budget = 64u64;
    const MAX_ROUNDS: usize = 4000;

    for _round in 0..MAX_ROUNDS {
        if crate::rup::is_rup(num_vars, &db, &[]) {
            break;
        }
        let mut solver = Solver::new(num_vars);
        // Keep every learned clause for the round so the RUP trace we lift is complete; reduction
        // would drop clauses the closing chain may depend on (the budget keeps the set small).
        solver.set_reduce(false);
        for c in &db {
            solver.add_clause(c.clone());
        }
        let res = solver.solve_budgeted(budget);
        total_conflicts += solver.conflicts();

        match res {
            BudgetedResult::Sat(model) => return SelOutcome::Sat(model),
            BudgetedResult::Unsat => {
                // Refuted within budget — append the closing learned clauses and finish.
                let learned: Vec<Vec<Lit>> = solver.learned().iter().map(|l| l.lits.clone()).collect();
                for c in learned {
                    try_add(num_vars, &mut db, &mut steps, &mut seen, c);
                }
                break;
            }
            BudgetedResult::Budget => {
                let learned: Vec<Vec<Lit>> = solver.learned().iter().map(|l| l.lits.clone()).collect();
                let mut progress = false;
                for c in learned {
                    if try_add(num_vars, &mut db, &mut steps, &mut seen, c.clone()) {
                        progress = true;
                    }
                    // The orbit of `c` under the generators — each image is RUP when σ is still a
                    // symmetry of the current database, and dropped otherwise (fail-closed).
                    for g in &gens {
                        let image = g.apply_clause(&c);
                        if try_add(num_vars, &mut db, &mut steps, &mut seen, image) {
                            amplified += 1;
                            progress = true;
                        }
                    }
                }
                if !progress {
                    // No new lemmas at this budget — give the solver more rope before conceding.
                    budget = budget.saturating_mul(2);
                    if budget > 1_000_000 {
                        break;
                    }
                }
            }
        }
    }

    if crate::pr::check_pr_refutation_fast(num_vars, clauses, &steps) {
        SelOutcome::Unsat { steps, conflicts: total_conflicts, amplified }
    } else {
        SelOutcome::Unknown { conflicts: total_conflicts }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cdcl::{SolveResult, Solver};

    /// Brute-force satisfiability over `num_vars` variables — the independent oracle.
    fn sat_brute(num_vars: usize, clauses: &[Vec<Lit>]) -> bool {
        for mask in 0u32..(1u32 << num_vars) {
            let model: Vec<bool> = (0..num_vars).map(|v| (mask >> v) & 1 == 1).collect();
            if clauses.iter().all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive())) {
                return true;
            }
        }
        false
    }

    #[test]
    fn sel_never_reports_pigeonhole_satisfiable() {
        // Regression guard: PHP is UNSAT at every size. A larger instance runs long enough to
        // trigger clause-DB reduction inside the budgeted solve — which once deleted the original
        // clauses and produced a bogus SAT. SEL must return UNSAT (or honest Unknown), NEVER SAT.
        for n in 7..=7 {
            let (cnf, _) = crate::families::php(n);
            match sel_refute(cnf.num_vars, &cnf.clauses) {
                SelOutcome::Sat(_) => panic!("PHP({n}) reported SATISFIABLE — soundness violation"),
                SelOutcome::Unsat { steps, .. } => {
                    assert!(crate::pr::check_pr_refutation_fast(cnf.num_vars, &cnf.clauses, &steps));
                }
                SelOutcome::Unknown { .. } => {}
            }
        }
    }

    #[test]
    fn sel_certifies_pigeonhole() {
        // SEL must refute PHP and the accumulated RUP proof must independently check.
        for n in 3..=6 {
            let (cnf, _) = crate::families::php(n);
            match sel_refute(cnf.num_vars, &cnf.clauses) {
                SelOutcome::Unsat { steps, .. } => {
                    assert!(
                        crate::pr::check_pr_refutation_fast(cnf.num_vars, &cnf.clauses, &steps),
                        "PHP({n}) SEL proof must check"
                    );
                }
                other => panic!("PHP({n}) must be refuted, got {other:?}"),
            }
        }
    }

    #[test]
    fn sel_amplification_cuts_conflicts_on_pigeonhole() {
        // The power metric: total conflicts under SEL must be strictly below plain CDCL on a
        // symmetry-rich instance — the symmetric twins of each lemma come for free.
        let (cnf, _) = crate::families::php(6);
        let mut plain = Solver::new(cnf.num_vars);
        for c in &cnf.clauses {
            plain.add_clause(c.clone());
        }
        assert_eq!(plain.solve(), SolveResult::Unsat);
        let plain_conflicts = plain.conflicts();

        match sel_refute(cnf.num_vars, &cnf.clauses) {
            SelOutcome::Unsat { conflicts, amplified, .. } => {
                assert!(amplified > 0, "symmetry amplification must actually fire on PHP");
                eprintln!(
                    "PHP(6): plain CDCL = {plain_conflicts} conflicts, SEL = {conflicts} conflicts ({amplified} symmetric clauses), {:.1}x fewer",
                    plain_conflicts as f64 / conflicts.max(1) as f64
                );
                assert!(
                    conflicts < plain_conflicts,
                    "SEL conflicts ({conflicts}) must beat plain CDCL ({plain_conflicts})"
                );
            }
            other => panic!("expected refutation, got {other:?}"),
        }
    }

    #[test]
    fn sel_never_returns_a_wrong_verdict_random() {
        // Soundness to the point of absurdity: over many seeded random small formulas, SEL must
        // never contradict brute force — a `Unsat` only on truly UNSAT instances (with a checking
        // proof), a `Sat(m)` only with a real model. `Unknown` is always permitted.
        let mut state = 0xC0FFEE123456789Au64;
        let mut next = || {
            state = state.wrapping_add(0x9E3779B97F4A7C15);
            let mut z = state;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        };
        let num_vars = 5usize;
        for _ in 0..1500 {
            let nclauses = next() as usize % 12;
            let clauses: Vec<Vec<Lit>> = (0..nclauses)
                .map(|_| {
                    let len = 1 + (next() as usize % 3);
                    let mut c = Vec::new();
                    for _ in 0..len {
                        let v = (next() as u32) % num_vars as u32;
                        let lit = Lit::new(v, next() & 1 == 0);
                        if !c.contains(&lit) && !c.contains(&lit.negated()) {
                            c.push(lit);
                        }
                    }
                    c
                })
                .filter(|c| !c.is_empty())
                .collect();
            let truth = sat_brute(num_vars, &clauses);
            match sel_refute(num_vars, &clauses) {
                SelOutcome::Unsat { steps, .. } => {
                    assert!(!truth, "SEL refuted a satisfiable formula: {clauses:?}");
                    assert!(
                        crate::pr::check_pr_refutation_fast(num_vars, &clauses, &steps),
                        "SEL Unsat proof must check: {clauses:?}"
                    );
                }
                SelOutcome::Sat(model) => {
                    assert!(truth, "SEL claimed SAT on an unsatisfiable formula: {clauses:?}");
                    assert!(
                        clauses.iter().all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive())),
                        "SEL returned an invalid model"
                    );
                }
                SelOutcome::Unknown { .. } => {}
            }
        }
    }
}
