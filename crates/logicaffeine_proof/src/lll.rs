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
    max_degree_within(clauses, usize::MAX).unwrap_or(0)
}

/// Certify satisfiability from sparsity via the symmetric LLL. Returns the witnessing degrees when
/// `e · 2^{−w_min} · (d+1) ≤ 1`; otherwise `None` (the condition is *sufficient, not necessary* — `None`
/// means "this certificate does not apply", never "UNSAT"). An empty clause set is vacuously SAT; a set
/// containing an empty clause can never be certified (it is UNSAT).
///
/// The condition is monotone in `d`, so it fixes the largest degree it admits, and the degree is counted only up to
/// that bound: a variable in more clauses than the bound allows declines the formula from its occurrence count, and a
/// clause's neighbour count stops at the bound. The work is the literals times the bound (`⌊2^{w_min}/e⌋`, at most the
/// clause count) — one pass on the formulas with a short clause, which is nearly every formula.
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
    let p = 2f64.powi(-(min_width.min(i32::MAX as usize) as i32));
    let holds = |d: usize| std::f64::consts::E * p * (d as f64 + 1.0) <= 1.0;
    if !holds(0) {
        return None;
    }
    let most = clauses.len() - 1;
    let bound = if holds(most) {
        most
    } else {
        let (mut admitted, mut refused) = (0usize, most);
        while refused - admitted > 1 {
            let mid = admitted + (refused - admitted) / 2;
            if holds(mid) {
                admitted = mid;
            } else {
                refused = mid;
            }
        }
        admitted
    };
    let max_degree = max_degree_within(clauses, bound)?;
    Some(LllSatCert { min_width, max_degree })
}

/// The clauses each variable occurs in, each clause once per variable however often the variable repeats in it, in
/// one flat array: `clauses[start[v]..start[v + 1]]` are variable `v`'s.
struct Occurrences {
    start: Vec<usize>,
    clauses: Vec<u32>,
}

impl Occurrences {
    /// The occurrences of `formula`'s variables, or `None` as soon as counting finds a variable in more than `limit`
    /// clauses — before any list is laid out.
    fn within(formula: &[Vec<Lit>], limit: usize) -> Option<Occurrences> {
        let vars = formula.iter().flatten().map(|l| l.var() as usize + 1).max().unwrap_or(0);
        let mut last = vec![u32::MAX; vars];
        let mut start = vec![0usize; vars + 1];
        for (i, c) in formula.iter().enumerate() {
            for l in c {
                let v = l.var() as usize;
                if last[v] != i as u32 {
                    last[v] = i as u32;
                    start[v + 1] += 1;
                    if start[v + 1] > limit {
                        return None;
                    }
                }
            }
        }
        for v in 0..vars {
            start[v + 1] += start[v];
        }
        let mut fill = start.clone();
        let mut clauses = vec![0u32; start[vars]];
        last.fill(u32::MAX);
        for (i, c) in formula.iter().enumerate() {
            for l in c {
                let v = l.var() as usize;
                if last[v] != i as u32 {
                    last[v] = i as u32;
                    clauses[fill[v]] = i as u32;
                    fill[v] += 1;
                }
            }
        }
        Some(Occurrences { start, clauses })
    }

    fn of_var(&self, v: usize) -> &[u32] {
        &self.clauses[self.start[v]..self.start[v + 1]]
    }
}

/// The largest dependency degree of `formula` when no clause has more than `bound` neighbours, `None` as soon as one
/// does. A variable in more than `bound + 1` clauses gives each of them that many neighbours, so it declines while the
/// occurrences are counted; each clause's neighbours are counted once by a stamp, up to `bound + 1`.
fn max_degree_within(formula: &[Vec<Lit>], bound: usize) -> Option<usize> {
    let occurrences = Occurrences::within(formula, bound.saturating_add(1))?;
    let mut stamp = vec![u32::MAX; formula.len()];
    let mut most = 0usize;
    for (i, c) in formula.iter().enumerate() {
        stamp[i] = i as u32;
        let mut degree = 0usize;
        for l in c {
            for &j in occurrences.of_var(l.var() as usize) {
                if stamp[j as usize] != i as u32 {
                    stamp[j as usize] = i as u32;
                    degree += 1;
                    if degree > bound {
                        return None;
                    }
                }
            }
        }
        most = most.max(degree);
    }
    Some(most)
}

/// Constructively find a satisfying assignment by **Moser–Tardos resampling**: start from a random
/// assignment and, while some clause is violated, resample the variables of one violated clause. Under
/// the LLL condition this terminates in expected `O(m)` resamples; the `max_resamples` cap is a safety
/// net (only reachable when the condition does not hold). Returns a model, or `None` if the cap is hit.
///
/// The clause resampled is always the first violated one. The violated clauses are kept in order and a resample
/// re-examines only the clauses sharing one of its variables, so a step costs that neighbourhood, not a scan.
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
    let occurrences = Occurrences::within(clauses, usize::MAX).expect("no occurrence count exceeds usize::MAX");
    let mut open: std::collections::BTreeSet<usize> =
        (0..clauses.len()).filter(|&i| violated(&assign, &clauses[i])).collect();
    for _ in 0..max_resamples {
        let Some(&i) = open.first() else {
            return Some(assign);
        };
        for l in &clauses[i] {
            assign[l.var() as usize] = next() & 1 == 0;
        }
        for l in &clauses[i] {
            for &j in occurrences.of_var(l.var() as usize) {
                let j = j as usize;
                if violated(&assign, &clauses[j]) {
                    open.insert(j);
                } else {
                    open.remove(&j);
                }
            }
        }
    }
    open.is_empty().then_some(assign)
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/lll/tests.rs"]
mod tests;
