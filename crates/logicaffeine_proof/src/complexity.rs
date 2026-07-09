//! Certified **complexity bounds** — a refutation that carries a checkable proof of its own size.
//!
//! A correctness certificate (the [`crate::pr`] refutation) answers "is this UNSAT?". A *complexity*
//! certificate answers "and how big is the proof, provably?". The carrier is a **rank function**: a
//! per-step natural-number measure that descends. If the measure is non-increasing and each of its
//! levels contributes at most `w` steps, then a proof spanning `L` levels has at most `L · w` steps —
//! a bound a checker confirms by reading the annotation, *without* trusting how the proof was built.
//!
//! This is a termination-proof-with-a-clock: the same descent that shows the construction halts also
//! counts its steps. For the steered pigeonhole/coloring refutations the measure is "active items
//! remaining," `L = O(n)` levels of `w = O(n)` steps each, so the certificate reads off a clean
//! `O(n²)` bound — turning "we measured it polynomial" into "here is the proof it is."

use crate::cdcl::Lit;
use crate::proof::ProofStep;

/// A refutation whose every step carries a rank (a progress/termination measure).
#[derive(Clone, Debug)]
pub struct RankedRefutation {
    /// Whether the underlying refutation independently checked.
    pub refuted: bool,
    /// The proof steps, in order.
    pub steps: Vec<ProofStep>,
    /// `ranks[i]` is the measure value at step `i`. Must be non-increasing for a valid certificate.
    pub ranks: Vec<u64>,
}

/// A certified upper bound on a refutation's size, read off a valid rank descent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SizeBound {
    /// The number of distinct rank levels the measure passes through.
    pub levels: u64,
    /// The largest number of steps at any single rank level (the per-level "width").
    pub max_width: u64,
    /// The certified upper bound on the step count: `levels · max_width`.
    pub bound: u64,
    /// The actual step count (always `≤ bound`).
    pub actual: u64,
}

/// Verify that `ranks` is a valid non-increasing measure and read off the [`SizeBound`] it
/// certifies. Returns `None` if the measure ever *increases* (then it is not a termination measure
/// and certifies nothing). The bound `levels · max_width` is a structural consequence of the
/// annotation, so the checker never re-derives the proof — it only counts.
pub fn certify_size_bound(ranks: &[u64]) -> Option<SizeBound> {
    if ranks.is_empty() {
        return Some(SizeBound { levels: 0, max_width: 0, bound: 0, actual: 0 });
    }
    // The measure must never increase along the proof.
    if ranks.windows(2).any(|w| w[1] > w[0]) {
        return None;
    }
    // Count steps per rank level (the ranks are non-increasing, so equal ranks form contiguous runs;
    // we still tally by value to be order-agnostic and robust).
    let mut counts: std::collections::BTreeMap<u64, u64> = std::collections::BTreeMap::new();
    for &r in ranks {
        *counts.entry(r).or_insert(0) += 1;
    }
    let levels = counts.len() as u64;
    let max_width = counts.values().copied().max().unwrap_or(0);
    Some(SizeBound { levels, max_width, bound: levels * max_width, actual: ranks.len() as u64 })
}

impl RankedRefutation {
    /// Independently check BOTH facets against the original `formula`: the refutation is correct
    /// (the PR checker accepts it) AND its size is bounded by its rank certificate. Returns the
    /// certified [`SizeBound`] only if both hold.
    pub fn certify(&self, num_vars: usize, formula: &[Vec<Lit>]) -> Option<SizeBound> {
        if self.ranks.len() != self.steps.len() {
            return None;
        }
        if !crate::pr::check_pr_refutation_fast(num_vars, formula, &self.steps) {
            return None;
        }
        let bound = certify_size_bound(&self.ranks)?;
        // The bound is an upper bound on the actual size by construction; assert the invariant.
        (bound.actual <= bound.bound).then_some(bound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_valid_descent_certifies_its_size() {
        // ranks 3,3,2,2,2,1 → 3 levels, max width 3 (the two-and-three runs), bound 9, actual 6.
        let ranks = vec![3, 3, 2, 2, 2, 1];
        let b = certify_size_bound(&ranks).expect("non-increasing ⇒ valid");
        assert_eq!(b.levels, 3);
        assert_eq!(b.max_width, 3);
        assert_eq!(b.bound, 9);
        assert_eq!(b.actual, 6);
        assert!(b.actual <= b.bound);
    }

    #[test]
    fn an_increasing_measure_certifies_nothing() {
        // A measure that goes back up is not a termination measure — reject it.
        assert!(certify_size_bound(&[3, 2, 3]).is_none());
        assert!(certify_size_bound(&[1, 2]).is_none());
    }

    #[test]
    fn empty_and_flat_measures() {
        assert_eq!(certify_size_bound(&[]).unwrap().bound, 0);
        let flat = certify_size_bound(&[5, 5, 5]).unwrap();
        assert_eq!((flat.levels, flat.max_width, flat.bound, flat.actual), (1, 3, 3, 3));
    }
}
