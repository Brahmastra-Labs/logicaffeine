//! Certified SAT-based optimization. We have no native MaxSAT/ILP solver, so we minimise an
//! integer cost the honest way: binary-search the smallest feasible cost bound, where each query
//! is a certified decision (`prove_unsat` → `Sat` witness or RUP-`Refuted`). The result is a
//! **certified optimum** — a witness at the optimum and a refutation at optimum−1 — not merely a
//! value we happened to find.
//!
//! `feasible_at(bound)` must be MONOTONE: feasible at `b` ⇒ feasible at `b+1` (e.g. "cost ≤ b" via
//! [`crate::cardinality::at_most`]).

use crate::sat::{prove_unsat, UnsatOutcome};
use crate::ProofExpr;

/// The outcome of a certified minimization.
#[derive(Clone, Debug, PartialEq)]
pub struct MinResult {
    /// The smallest feasible cost bound.
    pub optimum: i64,
    /// A satisfying assignment at the optimum (atom → value).
    pub witness: Vec<(String, bool)>,
    /// `true` when `optimum-1` was RUP-`Refuted` (so the optimum is provably minimal), or when the
    /// optimum equals the search floor.
    pub minimal_certified: bool,
}

/// Binary-search `[lo, hi]` for the least `bound` with `feasible_at(bound)` satisfiable.
/// Returns `None` if even `hi` is infeasible or a query leaves the supported fragment.
pub fn minimize_certified(
    feasible_at: impl Fn(i64) -> ProofExpr,
    lo: i64,
    hi: i64,
) -> Option<MinResult> {
    if lo > hi {
        return None;
    }
    // The top of the range must be feasible, else there is no solution at all.
    if !matches!(prove_unsat(&feasible_at(hi)), UnsatOutcome::Sat(_)) {
        return None;
    }
    let mut l = lo;
    let mut h = hi;
    while l < h {
        let mid = l + (h - l) / 2;
        match prove_unsat(&feasible_at(mid)) {
            UnsatOutcome::Sat(_) => h = mid,
            UnsatOutcome::Refuted => l = mid + 1,
            UnsatOutcome::Unsupported => return None,
        }
    }
    let optimum = l;
    let witness = match prove_unsat(&feasible_at(optimum)) {
        UnsatOutcome::Sat(m) => m,
        _ => return None,
    };
    let minimal_certified = optimum <= lo
        || matches!(
            prove_unsat(&feasible_at(optimum - 1)),
            UnsatOutcome::Refuted
        );
    Some(MinResult {
        optimum,
        witness,
        minimal_certified,
    })
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/optimize/tests.rs"]
mod tests;
