//! Fast refuter for the **modular counting principle** `Count_q(n)`: partition an `n`-element set into
//! blocks of size `q`. Encoded as: one Boolean per `q`-subset (block); a coverage clause per element
//! (the incident blocks, at least one chosen); and a disjointness clause for every pair of overlapping
//! blocks. It is unsatisfiable exactly when `q ∤ n`.
//!
//! The refutation is a one-line counting argument: coverage + disjointness force each element into
//! EXACTLY one chosen block, and every block covers exactly `q` elements, so summing over the elements
//! gives `n = q · (#chosen blocks)` — impossible when `q ∤ n`. Recognizing the structure and checking
//! `n mod q ≠ 0` is `O(clauses)`; the certificate is the triple `(n, q, n mod q)`.
//!
//! **Soundness:** the detector fires only when the clauses faithfully form `Count_q(n)` — every coverage
//! clause's incident blocks are pairwise disjoint (a full at-most-one clique, so each element is covered
//! at most once), every block covers exactly `q` elements, and `q ∤ n`. Then the counting argument makes
//! the formula unsatisfiable. Any deviation returns `None`; never a false refutation.

use crate::cdcl::Lit;
use std::collections::{HashMap, HashSet};

/// A modular-counting refutation: an `n`-element set cannot split into blocks of size `q` because
/// `q ∤ n` (the remainder is non-zero). Re-checkable in O(1): `n % q == remainder != 0`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CountingCert {
    pub n: u64,
    pub q: u64,
    pub remainder: u64,
}

impl CountingCert {
    /// Re-check from scratch: the certificate witnesses UNSAT iff `n mod q` is the stated non-zero
    /// remainder.
    pub fn check(&self) -> bool {
        self.q >= 2 && self.remainder != 0 && self.n % self.q == self.remainder
    }

    pub fn byte_len(&self) -> usize {
        24
    }
}

/// Recover a `Count_q(n)` core from `clauses` and refute it by the `q ∤ n` counting argument, or `None`
/// if there is no such core. Conservative / fail-closed — see the module docs.
pub fn counting_certificate(num_vars: usize, clauses: &[Vec<Lit>]) -> Option<CountingCert> {
    if num_vars < 2 {
        return None;
    }
    let key2 = |x: u32, y: u32| if x < y { (x, y) } else { (y, x) };
    // Coverage clauses = all-positive clauses (one per element, its incident blocks). Disjointness =
    // binary all-negative clauses (overlapping blocks are mutually exclusive).
    let mut coverage: Vec<Vec<u32>> = Vec::new();
    let mut disjoint: HashSet<(u32, u32)> = HashSet::new();
    for c in clauses {
        if c.iter().all(|l| l.is_positive()) {
            if c.is_empty() {
                return None;
            }
            coverage.push(c.iter().map(|l| l.var()).collect());
        } else if c.len() == 2 && !c[0].is_positive() && !c[1].is_positive() {
            disjoint.insert(key2(c[0].var(), c[1].var()));
        } else {
            return None; // a clause that is neither coverage nor disjointness — not a clean Count_q
        }
    }
    let n = coverage.len();
    if n < 2 {
        return None;
    }
    // Each element's incident blocks must be pairwise disjoint (a full at-most-one clique), so the
    // element is covered at most once; with coverage that is exactly once.
    let mut degree: HashMap<u32, usize> = HashMap::new();
    for cov in &coverage {
        let set: HashSet<u32> = cov.iter().copied().collect();
        if set.len() != cov.len() {
            return None; // a repeated block in one coverage clause
        }
        for i in 0..cov.len() {
            *degree.entry(cov[i]).or_insert(0) += 1;
            for j in (i + 1)..cov.len() {
                if !disjoint.contains(&key2(cov[i], cov[j])) {
                    return None; // two blocks share this element but are not mutually exclusive
                }
            }
        }
    }
    // Every block must cover the SAME number of elements q ≥ 2 (a uniform block size).
    let q = *degree.values().next()?;
    if q < 2 || degree.values().any(|&d| d != q) {
        return None;
    }
    let (n, q) = (n as u64, q as u64);
    let remainder = n % q;
    if remainder == 0 {
        return None; // q | n — a partition can exist, so this is not a refutation
    }
    Some(CountingCert { n, q, remainder })
}

/// Refute a `Count_q(n)` core (`q ∤ n`). `true` iff a certificate is recovered. Never a false refutation.
pub fn refute_counting(num_vars: usize, clauses: &[Vec<Lit>]) -> bool {
    counting_certificate(num_vars, clauses).is_some_and(|c| c.check())
}
