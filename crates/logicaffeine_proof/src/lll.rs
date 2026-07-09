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
        return Some(LllSatCert { min_width: usize::MAX, max_degree: 0 });
    }
    let min_width = clauses.iter().map(|c| c.len()).min().unwrap();
    if min_width == 0 {
        return None; // an empty clause is unsatisfiable
    }
    let d = lll_dependency_degree(clauses);
    let p = 2f64.powi(-(min_width as i32));
    if std::f64::consts::E * p * (d as f64 + 1.0) <= 1.0 {
        Some(LllSatCert { min_width, max_degree: d })
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
    clauses.iter().all(|c| !violated(&assign, c)).then_some(assign)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn brute_sat(nv: usize, clauses: &[Vec<Lit>]) -> bool {
        (0u64..(1u64 << nv))
            .any(|x| clauses.iter().all(|c| c.iter().any(|l| ((x >> l.var()) & 1 != 0) == l.is_positive())))
    }

    // A decorrelated instance seed — never seed along SplitMix64's own increment γ (see the
    // seed-collapse lesson: `s·γ` makes consecutive trials the same stream shifted by one).
    fn seed_of(tag: u64, i: u64) -> u64 {
        let mut z = tag.wrapping_mul(0xD1B5_4A32_D192_ED03).wrapping_add(i).wrapping_add(0x9E3779B97F4A7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^ (z >> 31)
    }

    /// **The LLL certificate never lies — soundness against brute force.** Over a sweep of sparse-to-dense
    /// random k-SAT, whenever `lll_certifies_sat` fires, brute force must confirm the instance is
    /// satisfiable. The certificate is a *sufficient* condition, so we only check the one direction it
    /// claims (Some ⟹ SAT); and we require it to fire at least once so the test is not vacuous.
    #[test]
    fn lll_certificate_is_sound_against_brute_force() {
        let n = 14usize;
        let mut fired = 0usize;
        for k in 3..=5usize {
            // very low densities — this is where local sparsity holds
            for &num in &[2usize, 3, 4, 6, 8, 10, 14, 20] {
                for t in 0..12u64 {
                    let tag = (k as u64) << 40 ^ (num as u64) << 20;
                    let cnf = crate::families::random_ksat(k, n, num, seed_of(tag, t));
                    if let Some(cert) = lll_certifies_sat(&cnf.clauses) {
                        fired += 1;
                        assert!(
                            brute_sat(n, &cnf.clauses),
                            "LLL certified SAT but brute force says UNSAT: k={k} m={num} cert={cert:?}"
                        );
                        // the certificate re-checks: recomputing degree and width reproduces it
                        assert_eq!(lll_dependency_degree(&cnf.clauses), cert.max_degree);
                    }
                }
            }
        }
        assert!(fired >= 5, "the LLL certificate fired on too few instances to be meaningful: {fired}");
    }

    /// **Moser–Tardos constructs the witness whenever the LLL certifies.** The certificate is not just an
    /// existence proof — when it fires, resampling reaches an actual model, and that model is verified to
    /// satisfy every clause. This makes the LLL the constructive, witness-producing lower bookend.
    #[test]
    fn moser_tardos_constructs_a_model_under_the_lll_condition() {
        let n = 16usize;
        let mut constructed = 0usize;
        for k in 3..=5usize {
            for &num in &[2usize, 3, 4, 6, 8, 12] {
                for t in 0..10u64 {
                    let tag = (k as u64) << 40 ^ (num as u64) << 20 ^ 0xA5;
                    let cnf = crate::families::random_ksat(k, n, num, seed_of(tag, t));
                    if lll_certifies_sat(&cnf.clauses).is_some() {
                        let model = moser_tardos_witness(n, &cnf.clauses, seed_of(tag ^ 0xF00D, t), 100 * num.max(1))
                            .expect("under the LLL condition Moser–Tardos must reach a model");
                        assert!(
                            cnf.clauses.iter().all(|c| c.iter().any(|l| model[l.var() as usize] == l.is_positive())),
                            "the constructed witness must satisfy every clause"
                        );
                        constructed += 1;
                    }
                }
            }
        }
        assert!(constructed >= 5, "constructed too few witnesses to be meaningful: {constructed}");
    }

    /// **The two bookends are consistent and complementary.** A sparse instance the LLL certifies SAT sits
    /// far *below* the first-moment density `α*(k)` (which only forbids satisfiability *above* it) — so the
    /// SAT certificate and the UNSAT bound never disagree, they bracket the threshold from opposite sides.
    #[test]
    fn lll_sat_region_sits_below_the_first_moment_unsat_bound() {
        let n = 16usize;
        let k = 4usize;
        let alpha_star = crate::families::ksat_threshold_first_moment_upper(k as u32);
        let mut checked = 0usize;
        for &num in &[3usize, 4, 5] {
            for t in 0..8u64 {
                let cnf = crate::families::random_ksat(k, n, num, seed_of(0xBEEF, (num as u64) << 8 ^ t));
                if lll_certifies_sat(&cnf.clauses).is_some() {
                    let alpha = num as f64 / n as f64;
                    assert!(alpha < alpha_star, "an LLL-certified instance must lie below α*({k})={alpha_star:.3}, got α={alpha:.3}");
                    assert!(brute_sat(n, &cnf.clauses), "and it is genuinely SAT");
                    checked += 1;
                }
            }
        }
        assert!(checked >= 1, "expected at least one LLL-certified sparse instance");
    }
}
