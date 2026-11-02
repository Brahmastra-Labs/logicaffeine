//! The concurrency tie made **literal**: *determinism = contractibility*, on a real machine.
//!
//! `proof_rewrite` showed that a refutation's independent steps commute, and that the space of
//! reorderings is a contractible `CAT(0)` cube complex. That is not an analogy to concurrency — it is
//! the *same* object. Here we run an actual tiny shared-memory machine and prove the identification on
//! the nose:
//!
//! - Two operations' **commutation 2-cell** exists (the square commutes in the cube complex) **iff**
//!   running them in both orders yields the same state — and the structural test for that is exactly
//!   **Bernstein's conditions** (neither writes what the other reads or writes), the independence
//!   relation of the trace monoid.
//! - A set of pairwise-independent operations is **deterministic** — every one of the `n!` schedules
//!   produces byte-identical state (we run them all) — and that is **exactly the contractibility**
//!   (`χ = 1`, the cube condition) of the same [`ProofPoset`] the proof tower uses.
//! - **Cooperative ≡ work-stealing**: two schedulers over independent tasks agree byte-for-byte because
//!   both are linear extensions of one contractible trace complex, joined by commutation 2-cells. This
//!   is the repo's `diff_cooperative_eq_workstealing` as a homotopy — the certificate of agreement *is*
//!   the 2-cell path.
//! - A **data race** is a *missing* 2-cell: a write-write conflict makes the ops dependent (no square),
//!   and the two orders genuinely diverge. Race detection falls out of the homotopy.
//!
//! So the homotopy theory of proofs and the determinism theory of concurrency are one theory, and this
//! module is the bridge with the machine actually running underneath it.

use crate::proof_rewrite::{permutations, ProofPoset};

/// A shared-memory operation: read some cells, then write one cell with a pure function of the reads
/// (in `reads` order). The minimal unit that can race.
#[derive(Clone)]
pub struct Op {
    pub reads: Vec<usize>,
    pub write: usize,
    pub f: fn(&[i64]) -> i64,
}

impl Op {
    /// Apply the operation to mutable shared state.
    pub fn apply(&self, state: &mut [i64]) {
        let args: Vec<i64> = self.reads.iter().map(|&r| state[r]).collect();
        state[self.write] = (self.f)(&args);
    }
}

/// **Bernstein's conditions** — `a` and `b` commute (are independent) iff neither writes a cell the
/// other reads or writes. This is the independence relation of the trace monoid: precisely the pairs
/// whose commutation square is a 2-cell in [`ProofPoset`]. (Read–read sharing is fine; reads commute.)
pub fn bernstein_independent(a: &Op, b: &Op) -> bool {
    a.write != b.write && !b.reads.contains(&a.write) && !a.reads.contains(&b.write)
}

/// Run a schedule (a sequence of op indices) from an initial state, returning the final state.
pub fn run(ops: &[Op], schedule: &[usize], init: &[i64]) -> Vec<i64> {
    let mut state = init.to_vec();
    for &i in schedule {
        ops[i].apply(&mut state);
    }
    state
}

/// The commutation poset of a set of concurrent operations: independent pairs (Bernstein) are left
/// unordered — their commutation is a 2-cell — and conflicting pairs get a program-order edge (lower
/// index first), which is the only way to keep a deterministic execution. The trace's cube complex.
pub fn trace_poset(ops: &[Op]) -> ProofPoset {
    let mut edges = Vec::new();
    for i in 0..ops.len() {
        for j in (i + 1)..ops.len() {
            if !bernstein_independent(&ops[i], &ops[j]) {
                edges.push((i, j));
            }
        }
    }
    ProofPoset::new(ops.len(), &edges)
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/trace_determinism/tests.rs"]
mod tests;
