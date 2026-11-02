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
    Unsat {
        steps: Vec<ProofStep>,
        conflicts: u64,
        amplified: usize,
    },
    /// Satisfiable, with a model.
    Sat(Vec<bool>),
    /// Gave up within the round/budget bounds without a verdict (the procedure is deliberately
    /// incomplete — it never returns a wrong answer, only an honest "don't know").
    Unknown { conflicts: u64 },
}

/// Canonical clause key (sorted, deduped literal codes) for the seen-set.
fn canon(c: &[Lit]) -> Vec<u32> {
    let mut k: Vec<u32> = c
        .iter()
        .map(|l| l.var() * 2 + u32::from(!l.is_positive()))
        .collect();
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
                let learned: Vec<Vec<Lit>> =
                    solver.learned().iter().map(|l| l.lits.clone()).collect();
                for c in learned {
                    try_add(num_vars, &mut db, &mut steps, &mut seen, c);
                }
                break;
            }
            BudgetedResult::Budget => {
                let learned: Vec<Vec<Lit>> =
                    solver.learned().iter().map(|l| l.lits.clone()).collect();
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
        SelOutcome::Unsat {
            steps,
            conflicts: total_conflicts,
            amplified,
        }
    } else {
        SelOutcome::Unknown {
            conflicts: total_conflicts,
        }
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/sym_dynamic/tests.rs"]
mod tests;
