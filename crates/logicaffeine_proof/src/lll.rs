//! Satisfiability **from sparsity** — the Lovász Local Lemma certificate and its constructive
//! Moser–Tardos witness. The lower bookend to the first-moment bound.
//!
//! The first-moment bound ([`crate::families::ksat_threshold_first_moment_upper`]) says: *above* a clause
//! density the expected number of solutions vanishes, so the formula is UNSAT with high probability. This
//! module is the opposite side: a formula that is *locally sparse* — every clause shares variables with
//! few others — is **satisfiable**, and the witness can be **constructed**.
//!
//! Symmetric LLL: a uniform random assignment violates a width-`w` clause with probability `2⁻ʷ`. If each
//! clause shares a variable with at most `d` others and `e · p · (d+1) ≤ 1` (where `p = 2^{−w_min}` is the
//! worst-case violation probability), then some assignment satisfies *all* clauses. This is:
//! - **sound** — the LLL theorem (and we fuzz it against brute force: a certificate never lies);
//! - **re-checkable** — recompute `w_min` and `d`;
//! - **constructive** — Moser–Tardos resampling reaches a model in expected `O(m)` steps under the
//!   condition, so the certificate is also a *witness recipe*.

use crate::cdcl::Lit;

/// A re-checkable SAT certificate from the Lovász Local Lemma: the minimum clause width `w_min` and the
/// maximum dependency degree `d` that together satisfy `e · 2^{−w_min} · (d+1) ≤ 1`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LllSatCert {
    pub min_width: usize,
    pub max_degree: usize,
}

/// The LLL **dependency degree**: the maximum, over all clauses, of the number of *other* clauses that
/// share at least one variable with it. Clauses over disjoint variable sets are independent events and do
/// not count toward the degree.
pub fn lll_dependency_degree(clauses: &[Vec<Lit>]) -> usize {
    use std::collections::{HashMap, HashSet};
    let mut occ: HashMap<u32, Vec<usize>> = HashMap::new();
    for (i, c) in clauses.iter().enumerate() {
        for l in c {
            occ.entry(l.var()).or_default().push(i);
        }
    }
    let mut max_d = 0;
    for (i, c) in clauses.iter().enumerate() {
        let mut neigh: HashSet<usize> = HashSet::new();
        for l in c {
            for &j in &occ[&l.var()] {
                if j != i {
                    neigh.insert(j);
                }
            }
        }
        max_d = max_d.max(neigh.len());
    }
    max_d
}

/// Certify satisfiability from sparsity via the symmetric LLL. Returns the witnessing degrees when
/// `e · 2^{−w_min} · (d+1) ≤ 1`; otherwise `None` (the condition is *sufficient, not necessary* — `None`
/// means "this certificate does not apply", never "UNSAT"). An empty clause set is vacuously SAT; a set
/// containing an empty clause can never be certified (it is UNSAT).
pub fn lll_certifies_sat(clauses: &[Vec<Lit>]) -> Option<LllSatCert> {
    if clauses.is_empty() {
        return Some(LllSatCert {
            min_width: usize::MAX,
            max_degree: 0,
        });
    }
    let min_width = clauses.iter().map(|c| c.len()).min().unwrap();
    if min_width == 0 {
        return None; // an empty clause is unsatisfiable
    }
    let d = lll_dependency_degree(clauses);
    let p = 2f64.powi(-(min_width as i32));
    if std::f64::consts::E * p * (d as f64 + 1.0) <= 1.0 {
        Some(LllSatCert {
            min_width,
            max_degree: d,
        })
    } else {
        None
    }
}

/// Constructively find a satisfying assignment by **Moser–Tardos resampling**: start from a random
/// assignment and, while some clause is violated, resample the variables of one violated clause. Under
/// the LLL condition this terminates in expected `O(m)` resamples; the `max_resamples` cap is a safety
/// net (only reachable when the condition does not hold). Returns a model, or `None` if the cap is hit.
pub fn moser_tardos_witness(
    num_vars: usize,
    clauses: &[Vec<Lit>],
    seed: u64,
    max_resamples: usize,
) -> Option<Vec<bool>> {
    let mut rng = seed;
    let mut next = move || {
        rng = rng.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = rng;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    };
    let mut assign: Vec<bool> = (0..num_vars).map(|_| next() & 1 == 0).collect();
    let violated = |a: &[bool], c: &[Lit]| c.iter().all(|l| a[l.var() as usize] != l.is_positive());
    for _ in 0..max_resamples {
        match clauses.iter().find(|c| violated(&assign, c)) {
            None => return Some(assign),
            Some(c) => {
                for l in c {
                    assign[l.var() as usize] = next() & 1 == 0;
                }
            }
        }
    }
    clauses
        .iter()
        .all(|c| !violated(&assign, c))
        .then_some(assign)
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/lll/tests.rs"]
mod tests;
