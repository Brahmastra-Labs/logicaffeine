//! Cardinality constraints over boolean [`ProofExpr`] atoms — the missing primitive that lets the
//! certified solver answer "at most / at least / exactly `k` of these are true". Encoded with
//! Sinz's sequential counter (linear in `n·k`, fresh auxiliary atoms under a caller-chosen
//! prefix), so the result is an ordinary boolean obligation the existing CNF/CDCL/RUP pipeline
//! discharges. This is the building block for SAT-based optimization (`crate::optimize`).
//!
//! Correctness is pinned **exhaustively** against a brute-force oracle (every assignment, small n).

use crate::ProofExpr;

fn atom(s: String) -> ProofExpr {
    ProofExpr::Atom(s)
}
fn and(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::And(Box::new(a), Box::new(b))
}
fn not(a: ProofExpr) -> ProofExpr {
    ProofExpr::Not(Box::new(a))
}
fn implies(a: ProofExpr, b: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(a), Box::new(b))
}
fn tautology() -> ProofExpr {
    let t = atom("__card_true".to_string());
    ProofExpr::Or(Box::new(t.clone()), Box::new(not(t)))
}
fn contradiction() -> ProofExpr {
    let f = atom("__card_false".to_string());
    and(f.clone(), not(f))
}
fn conj(parts: Vec<ProofExpr>) -> ProofExpr {
    let mut it = parts.into_iter();
    match it.next() {
        None => tautology(),
        Some(first) => it.fold(first, and),
    }
}

/// "At most `k` of `vars` are true." Auxiliary atoms are named `{aux}_{i}_{j}`, so distinct
/// constraints in one formula must use distinct `aux` prefixes.
pub fn at_most(vars: &[ProofExpr], k: usize, aux: &str) -> ProofExpr {
    let n = vars.len();
    if k >= n {
        return tautology();
    }
    if k == 0 {
        return conj(vars.iter().map(|v| not(v.clone())).collect());
    }
    // s(i, j) ≙ "at least j of x_1..x_i are true" (1-based).
    let s = |i: usize, j: usize| atom(format!("{aux}_{i}_{j}"));
    let mut clauses: Vec<ProofExpr> = Vec::new();

    clauses.push(implies(vars[0].clone(), s(1, 1)));
    for j in 2..=k {
        clauses.push(not(s(1, j)));
    }
    for i in 2..=n {
        let xi = vars[i - 1].clone();
        clauses.push(implies(xi.clone(), s(i, 1)));
        clauses.push(implies(s(i - 1, 1), s(i, 1)));
        for j in 2..=k {
            clauses.push(implies(and(xi.clone(), s(i - 1, j - 1)), s(i, j)));
            clauses.push(implies(s(i - 1, j), s(i, j)));
        }
        // Already k counted and another true ⇒ would exceed k: forbidden.
        clauses.push(not(and(xi, s(i - 1, k))));
    }
    conj(clauses)
}

/// "At least `k` of `vars` are true" — i.e. at most `n−k` are false.
pub fn at_least(vars: &[ProofExpr], k: usize, aux: &str) -> ProofExpr {
    let n = vars.len();
    if k == 0 {
        return tautology();
    }
    if k > n {
        return contradiction();
    }
    let negated: Vec<ProofExpr> = vars.iter().map(|v| not(v.clone())).collect();
    at_most(&negated, n - k, aux)
}

/// "Exactly `k` of `vars` are true."
pub fn exactly(vars: &[ProofExpr], k: usize, aux: &str) -> ProofExpr {
    and(
        at_least(vars, k, &format!("{aux}_ge")),
        at_most(vars, k, &format!("{aux}_le")),
    )
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/cardinality/tests.rs"]
mod tests;
