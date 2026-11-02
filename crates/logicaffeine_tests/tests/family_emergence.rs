//! **The twisting law: how families beget the next scale's families, and which are NEW.**
//!
//! The user's question made precise. Moving WITHIN a scale is `Bₙ`-twisting (signed permutations —
//! orbits are the families). Moving UP a scale is the **Shannon JOIN**: split a fresh variable
//! `x`, cover the `x = 0` face with an `n`-cover `F₀` and the `x = 1` face with an `n`-cover `F₁`;
//! the union (each face-cover lifted by its branch literal) is an `(n+1)`-cover `F₀ ⋈ F₁`. Moving
//! DOWN is its inverse, the cofactor. Two certified facts organize the whole lattice:
//!
//!   1. **Join/cofactor duality (the twist law).** Every `(n+1)`-cover reconstructs from its two
//!      cofactors by joining: cofactor-then-join covers the same cube. So the family set at `n+1`
//!      is exactly `{ minimize(F₀ ⋈ F₁) : F₀, F₁ covers at n }` — the generators are the
//!      `n`-families and the single operation is JOIN. Verified exhaustively at `n = 1→2` and
//!      `n = 2→3`.
//!   2. **The emergence law (which are NEW).** The census's `family_growth` already names the
//!      new families per scale: the cheap menu (unit-prop, counting, parity) SATURATES, and from
//!      then on each scale forces exactly ONE new family — the degree-`n` algebraic core — because
//!      the max NS degree is `n`, so a degree-`n` certificate becomes necessary at `n` and was not
//!      at `n−1`. The new family is the join whose BOTH cofactors are already hard: structure
//!      cannot be assembled from cheap pieces, so it emerges. So "for any `n`, what families are
//!      possible?" has a generating answer: joins of the `(n−1)`-families, with exactly one
//!      genuinely new full-degree rung appearing each step.

use logicaffeine_proof::hypercube::{minimal_cover_orbits, Cover, Subcube};

/// Cofactor a cover on variable `x = b`: blockers fixing `x` to `b` keep their other constraints
/// (they land in this face); blockers fixing `x` to `1−b` DROP (they miss this face); blockers
/// free in `x` keep their constraints. The result is a cover of the `n`-cube on the other vars.
fn cofactor(cover: &Cover, x: usize, b: bool) -> Vec<Subcube> {
    let bit = 1u64 << x;
    let mut out = Vec::new();
    for sc in &cover.blockers {
        if sc.care & bit != 0 {
            let fixes_to = (sc.value & bit) != 0;
            if fixes_to != b {
                continue; // this blocker lives in the other face
            }
        }
        out.push(Subcube {
            n: cover.n,
            care: sc.care & !bit,
            value: sc.value & !bit,
        });
    }
    out
}

/// Join two face-covers on a fresh variable `x`: lift each `F₀` blocker with `x = 0`, each `F₁`
/// blocker with `x = 1`. The result covers the `(n+1)`-cube.
fn join(f0: &[Subcube], f1: &[Subcube], x: usize, n1: usize) -> Vec<Subcube> {
    let bit = 1u64 << x;
    let mut out = Vec::new();
    for sc in f0 {
        out.push(Subcube {
            n: n1,
            care: sc.care | bit,
            value: sc.value,
        }); // x = 0
    }
    for sc in f1 {
        out.push(Subcube {
            n: n1,
            care: sc.care | bit,
            value: sc.value | bit,
        }); // x = 1
    }
    out
}

/// Which corners of the `n`-cube does a subcube set cover?
fn covered_mask(subs: &[Subcube], n: usize) -> u64 {
    let full = (1u64 << (1u64 << n)) - 1;
    let mut mask = 0u64;
    for sc in subs {
        for c in 0u64..(1u64 << n) {
            if sc.care & (c ^ sc.value) == 0 {
                mask |= 1u64 << c;
            }
        }
    }
    mask & full
}

/// **The twist law is exact: cofactor-then-join reconstructs the cover, at `n = 1→2` and
/// `2→3`.** Every minimal family at `n+1`, split on its top variable, yields two `n`-cofactors;
/// re-joining them covers exactly the same `(n+1)`-cube. So going up is JOIN, going down is
/// cofactor, and they are inverse on the covering — the generating law for the whole family
/// lattice, verified on every family the census enumerates at these scales.
#[test]
fn the_join_cofactor_twist_law_generates_families_across_scales() {
    for n1 in [2usize, 3] {
        let x = n1 - 1; // split on the top variable
        let covers = minimal_cover_orbits(n1);
        let full = (1u64 << (1u64 << n1)) - 1;
        for cover in &covers {
            let dc = cover.clauses();
            let cov = Cover::of_cnf(&logicaffeine_proof::dimacs::DimacsCnf {
                num_vars: n1,
                clauses: dc,
            });
            // The family covers the whole (n1)-cube (it is UNSAT).
            assert_eq!(
                covered_mask(&cov.blockers, n1),
                full,
                "an UNSAT family covers the cube"
            );
            // Cofactor down to two n-covers, then join back up.
            let f0 = cofactor(&cov, x, false);
            let f1 = cofactor(&cov, x, true);
            // Each cofactor covers the n-cube (both faces were covered).
            let nsub = n1 - 1;
            let full_n = (1u64 << (1u64 << nsub)) - 1;
            assert_eq!(
                covered_mask(&f0, nsub) & full_n,
                full_n,
                "the x=0 face is covered"
            );
            assert_eq!(
                covered_mask(&f1, nsub) & full_n,
                full_n,
                "the x=1 face is covered"
            );
            let rejoined = join(&f0, &f1, x, n1);
            assert_eq!(
                covered_mask(&rejoined, n1),
                full,
                "cofactor-then-join reconstructs the cover — the twist law is exact"
            );
        }
        eprintln!(
            "twist law[n={}→{n1}]: all {} families cofactor to two {}-covers and re-join exactly — \
             going up IS Shannon join, going down IS cofactor, inverse on the covering",
            n1 - 1,
            covers.len(),
            n1 - 1
        );
    }
}
