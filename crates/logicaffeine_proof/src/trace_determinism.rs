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
mod tests {
    use super::*;

    #[test]
    fn the_commutation_2cell_is_literally_state_agreement() {
        // THE BRIDGE. A pair's commutation square commutes in the cube complex IFF running them in both
        // orders yields the same state. Bernstein-independent ops agree on every state (the 2-cell is
        // present); a write-write race diverges (the 2-cell is absent). The homotopy 2-cell = real
        // determinism of that step.
        let p = Op { reads: vec![0], write: 1, f: |a| a[0] + 1 };
        let q = Op { reads: vec![2], write: 3, f: |a| a[0] * 2 };
        assert!(bernstein_independent(&p, &q), "disjoint footprints ⇒ independent");
        let ops = vec![p, q];
        for init in [[0, 0, 0, 0], [3, 9, 5, 7], [-1, 2, 4, -8]] {
            assert_eq!(
                run(&ops, &[0, 1], &init),
                run(&ops, &[1, 0], &init),
                "independent ops: both orders agree — the 2-cell commutes"
            );
        }

        // a genuine write-write race: both write cell 1
        let r = Op { reads: vec![0], write: 1, f: |a| a[0] + 1 };
        let s = Op { reads: vec![2], write: 1, f: |a| a[0] + 10 };
        assert!(!bernstein_independent(&r, &s), "write-write race ⇒ dependent");
        let race = vec![r, s];
        let init = [5, 0, 7, 0];
        assert_ne!(
            run(&race, &[0, 1], &init),
            run(&race, &[1, 0], &init),
            "the race diverges — the 2-cell is genuinely absent"
        );
    }

    #[test]
    fn determinism_is_contractibility_run_every_schedule() {
        // GLOBAL IDENTIFICATION. A set of pairwise-independent ops (disjoint footprints) is
        // DETERMINISTIC — every one of the n! schedules yields byte-identical final state (we run them
        // all) — and this is EXACTLY the contractibility of the trace cube complex (χ = 1, cube
        // condition), the same complex the proof tower uses. Computed two ways, reconciled on the nose.
        let inc = |a: &[i64]| a[0] + 1;
        let ops = vec![
            Op { reads: vec![0], write: 1, f: inc },
            Op { reads: vec![2], write: 3, f: inc },
            Op { reads: vec![4], write: 5, f: inc },
            Op { reads: vec![6], write: 7, f: inc },
        ];
        let init = [10, 0, 20, 0, 30, 0, 40, 0];
        let canonical = run(&ops, &[0, 1, 2, 3], &init);
        let mut schedules = 0;
        for perm in permutations(4) {
            assert_eq!(run(&ops, &perm, &init), canonical, "schedule {perm:?} agrees — determinism");
            schedules += 1;
        }
        assert_eq!(schedules, 24);

        let poset = trace_poset(&ops);
        assert_eq!(poset.euler_characteristic(), 1, "deterministic execution ⇒ contractible trace complex");
        assert!(poset.satisfies_cube_condition(), "all interleavings fill the cubes (CAT(0))");
        assert_eq!(poset.linear_extensions().len(), 24, "the 24 schedules ARE the linear extensions");
    }

    #[test]
    fn cooperative_equals_work_stealing_byte_identical() {
        // THE REPO'S SHAPE, abstracted. Two schedulers — cooperative round-robin [0,1,2,3] and
        // work-stealing LIFO [3,2,1,0] — over independent tasks give BYTE-IDENTICAL state, because both
        // are linear extensions of one contractible trace complex, connected by commutation 2-cells.
        // diff_cooperative_eq_workstealing as a homotopy: the certificate of agreement IS the 2-cell path.
        let ops = vec![
            Op { reads: vec![0], write: 1, f: |a| a[0] * 2 },
            Op { reads: vec![2], write: 3, f: |a| a[0] + 7 },
            Op { reads: vec![4], write: 5, f: |a| a[0] - 1 },
            Op { reads: vec![6], write: 7, f: |a| a[0] * a[0] },
        ];
        let init = [3, 0, 4, 0, 5, 0, 6, 0];
        let cooperative = run(&ops, &[0, 1, 2, 3], &init);
        let work_stealing = run(&ops, &[3, 2, 1, 0], &init);
        assert_eq!(cooperative, work_stealing, "cooperative ≡ work-stealing, byte-identical");

        let poset = trace_poset(&ops);
        assert!(poset.extensions_connected_by_commutation(), "the two schedules are joined by 2-cells");
        assert_eq!(poset.euler_characteristic(), 1, "their agreement is the contractibility of the trace");
    }

    #[test]
    fn seeded_replay_is_symmetry_breaking_the_scheduler_and_its_sound() {
        // KEEP SYMMETRY BREAKING — now on the SCHEDULER. The freedom to order independent enabled tasks
        // is a symmetry of the trace complex: its linear extensions form ONE commutation orbit. Picking a
        // single canonical interleaving (the lex-least schedule — or, in the engine, a seed-chosen one)
        // is SYMMETRY BREAKING: one representative per orbit, the very π₀ move the assignment tower makes,
        // now applied to schedules. And it is SOUND precisely because the trace is CONTRACTIBLE: every
        // schedule yields the same final state, so collapsing to the canonical one loses nothing.
        // Determinism = contractibility is the certificate that breaking the scheduler symmetry is safe.
        let ops = vec![
            Op { reads: vec![0], write: 1, f: |a| a[0] + 5 },
            Op { reads: vec![2], write: 3, f: |a| a[0] * 3 },
            Op { reads: vec![4], write: 5, f: |a| a[0] - 2 },
            Op { reads: vec![6], write: 7, f: |a| a[0] + a[0] },
        ];
        let init = [1, 0, 2, 0, 3, 0, 4, 0];
        let poset = trace_poset(&ops);

        // the scheduler symmetry: all schedules are one commutation orbit (π₀ = 1)
        assert!(poset.extensions_connected_by_commutation(), "all schedules = one commutation orbit");

        // SYMMETRY BREAKING: pick the canonical (lex-least) representative — one schedule per trace
        let canonical = poset.canonical_extension();
        assert_eq!(canonical, vec![0, 1, 2, 3], "the canonical (lex-least) schedule is the orbit rep");

        // SOUNDNESS of the break: the canonical schedule's result = EVERY schedule's result
        let canonical_state = run(&ops, &canonical, &init);
        for perm in permutations(4) {
            assert_eq!(
                run(&ops, &perm, &init),
                canonical_state,
                "breaking the scheduler symmetry to {canonical:?} loses nothing — contractibility certifies it"
            );
        }
    }

    #[test]
    fn a_data_race_is_a_missing_2cell_nondeterminism_detected() {
        // RACE DETECTION VIA HOMOTOPY. A write-write conflict (two ops writing cell 1) makes the trace
        // poset ORDER them — the 2-cell is absent — and running the two orders diverges. The missing
        // 2-cell IS the race; nondeterminism is the failure of the square to commute.
        let ops = vec![
            Op { reads: vec![0], write: 1, f: |a| a[0] + 1 },
            Op { reads: vec![2], write: 1, f: |a| a[0] + 100 },
        ];
        assert!(!bernstein_independent(&ops[0], &ops[1]), "write-write conflict on cell 1");
        let poset = trace_poset(&ops);
        assert!(!poset.independent(0, 1), "the racing ops are ordered — the 2-cell is absent");

        let init = [5, 0, 7, 0];
        assert_ne!(
            run(&ops, &[0, 1], &init),
            run(&ops, &[1, 0], &init),
            "the race is genuinely nondeterministic — the missing 2-cell, observed"
        );
    }
}
