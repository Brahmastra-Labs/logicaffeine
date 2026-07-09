//! A general **`d`-dimensional cubical-homology engine** — one engine, every rung of the ladder.
//!
//! `progress_complex` produced `π₁` (2 processes) and `π₂` (3 processes) with hand-built 2D and 3D
//! complexes. Rather than write a 4D one, then a 5D one, this is the lift: a single cubical complex in
//! *any* dimension, computing the full Betti vector `(β₀, β₁, …, β_d)` over `GF(2)`. Then the ladder is
//! literal — **`d` processes → a `(d−1)`-sphere void → `π_{d−1}` ≠ 0** — and walking it toward the
//! `∞`-groupoid is just "one more process, one more dimension," handled by the same code.
//!
//! An elementary cube is a [`Cube`]: a minimal corner plus the set of free axes it extends along (its
//! dimension). Its boundary is the `2k` faces obtained by pinning each free axis low or high. From a set
//! of filled top cells we take the downward closure, build the boundary matrices `∂_k`, and read
//! `β_k = #C_k − rank ∂_k − rank ∂_{k+1}` — homology with no shortcuts, in arbitrary dimension.

use crate::progress_complex::gf2_rank_wide;
use std::collections::{BTreeSet, HashMap};

/// An elementary cube: the minimal corner, and the sorted set of axes it extends along (each by one
/// unit). The dimension is the number of free axes; fixed axes hold the corner's value.
#[derive(Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug)]
pub struct Cube {
    pub corner: Vec<usize>,
    pub dirs: Vec<usize>,
}

impl Cube {
    pub fn dim(&self) -> usize {
        self.dirs.len()
    }

    /// The `2k` boundary faces — for each free axis, the lower face (pin it at the corner) and the upper
    /// face (pin it at corner + 1). Over `GF(2)` the boundary is their unsigned sum.
    pub fn boundary(&self) -> Vec<Cube> {
        let mut faces = Vec::with_capacity(2 * self.dirs.len());
        for (idx, &a) in self.dirs.iter().enumerate() {
            let mut sub = self.dirs.clone();
            sub.remove(idx);
            faces.push(Cube { corner: self.corner.clone(), dirs: sub.clone() });
            let mut up = self.corner.clone();
            up[a] += 1;
            faces.push(Cube { corner: up, dirs: sub });
        }
        faces
    }
}

/// A cubical complex, stored as the cells of each dimension `0..=dim`.
pub struct CubicalComplex {
    cells: Vec<BTreeSet<Cube>>,
    dim: usize,
}

impl CubicalComplex {
    /// Build from filled top-dimensional cells, taking the downward closure (all faces of all faces).
    pub fn from_top_cells(top: Vec<Cube>) -> Self {
        let dim = top.iter().map(Cube::dim).max().unwrap_or(0);
        let mut cells: Vec<BTreeSet<Cube>> = vec![BTreeSet::new(); dim + 1];
        for c in top {
            let d = c.dim();
            cells[d].insert(c);
        }
        for k in (1..=dim).rev() {
            let current: Vec<Cube> = cells[k].iter().cloned().collect();
            for c in current {
                for f in c.boundary() {
                    cells[f.dim()].insert(f);
                }
            }
        }
        CubicalComplex { cells, dim }
    }

    /// The progress complex of `d` processes with the given step `lengths`: every grid `d`-cell is
    /// filled, except those in `forbidden` (the mutual-exclusion / synchronization region).
    pub fn progress(lengths: &[usize], forbidden: &[Vec<usize>]) -> Self {
        let d = lengths.len();
        let forbidden: BTreeSet<Vec<usize>> = forbidden.iter().cloned().collect();
        let mut top = Vec::new();
        let mut idx = vec![0usize; d];
        loop {
            if !forbidden.contains(&idx) {
                top.push(Cube { corner: idx.clone(), dirs: (0..d).collect() });
            }
            let mut a = 0;
            while a < d {
                idx[a] += 1;
                if idx[a] < lengths[a] {
                    break;
                }
                idx[a] = 0;
                a += 1;
            }
            if a == d {
                break;
            }
        }
        CubicalComplex::from_top_cells(top)
    }

    pub fn dim(&self) -> usize {
        self.dim
    }

    pub fn num_cells(&self, k: usize) -> usize {
        self.cells.get(k).map_or(0, BTreeSet::len)
    }

    /// `rank ∂_k` over `GF(2)` — the boundary map from `k`-cells to `(k−1)`-cells.
    fn boundary_rank(&self, k: usize) -> usize {
        if k == 0 || k > self.dim {
            return 0;
        }
        let lower: Vec<Cube> = self.cells[k - 1].iter().cloned().collect();
        let ncols = lower.len();
        if ncols == 0 {
            return 0;
        }
        let lidx: HashMap<Cube, usize> = lower.into_iter().enumerate().map(|(i, c)| (c, i)).collect();
        let rows: Vec<Vec<u64>> = self.cells[k]
            .iter()
            .map(|c| {
                let mut row = vec![0u64; ncols.div_ceil(64)];
                for f in c.boundary() {
                    let idx = lidx[&f];
                    row[idx / 64] ^= 1u64 << (idx % 64);
                }
                row
            })
            .collect();
        gf2_rank_wide(rows, ncols)
    }

    /// The Betti vector `(β₀, …, β_d)`: `β_k = #C_k − rank ∂_k − rank ∂_{k+1}` over `GF(2)`.
    pub fn betti(&self) -> Vec<usize> {
        let mut ranks = vec![0usize; self.dim + 2];
        for k in 1..=self.dim {
            ranks[k] = self.boundary_rank(k);
        }
        (0..=self.dim).map(|k| self.cells[k].len() - ranks[k] - ranks[k + 1]).collect()
    }

    /// `χ = Σ (−1)^k #C_k` — the alternating cell count, equal to `Σ (−1)^k β_k` (Euler–Poincaré).
    pub fn euler(&self) -> i64 {
        (0..=self.dim)
            .map(|k| {
                let c = self.cells[k].len() as i64;
                if k % 2 == 0 {
                    c
                } else {
                    -c
                }
            })
            .sum()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn center(d: usize) -> Vec<Vec<usize>> {
        vec![vec![1usize; d]]
    }

    #[test]
    fn the_general_engine_reproduces_the_hand_built_2d_and_3d_complexes() {
        // Cross-validation: the general engine must agree with progress_complex's bespoke 2D/3D homology.
        // 2 processes, one mutex hole ⇒ β = [1,1,0]; 3 processes, forbidden core ⇒ β = [1,0,1,0].
        let two = CubicalComplex::progress(&[3, 3], &center(2));
        assert_eq!(two.betti(), vec![1, 1, 0], "2D mutex hole: β₁ = 1 (π₁ = Z)");
        let three = CubicalComplex::progress(&[3, 3, 3], &center(3));
        assert_eq!(three.betti(), vec![1, 0, 1, 0], "3D forbidden core: β₂ = 1 (π₂ = Z)");
    }

    #[test]
    fn solid_d_cubes_are_contractible_in_every_dimension() {
        // No forbidden cell ⇒ a solid d-dimensional block, contractible: β = [1,0,…,0], χ = 1.
        for d in 1..=4 {
            let lengths = vec![3usize; d];
            let pc = CubicalComplex::progress(&lengths, &[]);
            let beta = pc.betti();
            assert_eq!(beta[0], 1, "connected in dimension {d}");
            assert!(beta[1..].iter().all(|&b| b == 0), "solid d-cube is contractible (d={d})");
            assert_eq!(pc.euler(), 1, "χ = 1 for a contractible complex (d={d})");
        }
    }

    #[test]
    fn four_processes_produce_pi_three_a_3_sphere_void() {
        // THE CRUSH: π₃. Four processes, forbid the center 4-cell — the state where all four are jointly
        // in the forbidden core. Its eight boundary 3-faces survive, forming a 3-CYCLE that no longer
        // bounds: a hollow 3-SPHERE (S³). So β₃ = 1, π₃ = Z. The general engine produced the next rung
        // with no new code — just one more process, one more dimension.
        let pc = CubicalComplex::progress(&[3, 3, 3, 3], &center(4));
        let beta = pc.betti();
        assert_eq!(beta, vec![1, 0, 0, 1, 0], "4 processes, forbidden core ⇒ β₃ = 1: a 3-sphere void, π₃ = Z");
        // Euler–Poincaré: Σ(−1)^k β_k = 1 − 0 + 0 − 1 + 0 = 0, matching the alternating cell count.
        let alt: i64 = beta.iter().enumerate().map(|(k, &b)| if k % 2 == 0 { b as i64 } else { -(b as i64) }).sum();
        assert_eq!(alt, pc.euler(), "Euler–Poincaré holds: Σ(−1)^k β_k = χ");
        assert_eq!(pc.euler(), 0, "χ = 0, the signature of a 3-sphere shell");
    }

    #[test]
    fn infinity_is_a_finite_law_checkable_at_every_rung_not_an_object_we_hold() {
        // "Can we finally understand infinity?" — yes, but honestly. NOT as a finished object we hold:
        // no single finite complex carries every πₙ. We understand it as a GENERATIVE LAW one finite
        // engine obeys at every rung — d processes ↦ a (d−1)-sphere void ↦ π_{d−1} = Z — with no largest
        // rung. Here we push the SAME engine to π₄ (five processes, forbidden core ⇒ a 4-sphere), with no
        // new code, and check the law holds across d = 2..=5 in one breath. The ∞-groupoid IS this rule;
        // we grasp infinity by the rule that generates every level, machine-checked at each finite stage.
        let pc = CubicalComplex::progress(&[3, 3, 3, 3, 3], &center(5));
        let mut expected = vec![0usize; 6];
        expected[0] = 1;
        expected[4] = 1;
        assert_eq!(pc.betti(), expected, "5 processes ⇒ β₄ = 1: a 4-sphere void, π₄ = Z — the next rung");

        for d in 2..=5 {
            let beta = CubicalComplex::progress(&vec![3usize; d], &center(d)).betti();
            assert_eq!(beta[d - 1], 1, "rung d={d}: π_{{d-1}} is realized");
            assert_eq!(beta.iter().sum::<usize>(), 2, "exactly β₀ and β_{{d-1}} fire — a clean (d−1)-sphere");
        }
    }

    #[test]
    fn homology_is_not_all_the_invariants_pi3_of_the_2_sphere_is_invisible() {
        // THE HONEST CEILING, machine-checked. Our engine computes HOMOLOGY (Betti numbers), which is
        // NOT the full set of invariants. Build the 2-sphere as the hollow surface of a cube: β = [1,0,1]
        // — β₀ = 1, β₁ = 0, β₂ = 1, and nothing in degree 3. Yet π₃(S²) = Z, the Hopf map — a nontrivial
        // higher homotopy group homology is utterly blind to. So we did NOT lock infinity down to all its
        // invariants: the homotopy groups of spheres are an OPEN problem in mathematics, in general
        // uncomputable, and our own engine provably cannot see π₃ here. We hold a generative LAW and the
        // 1-truncated OBJECT — not the full invariant catalog, which no finite engine can possess.
        let s2 = CubicalComplex::from_top_cells(vec![
            Cube { corner: vec![0, 0, 0], dirs: vec![1, 2] },
            Cube { corner: vec![1, 0, 0], dirs: vec![1, 2] },
            Cube { corner: vec![0, 0, 0], dirs: vec![0, 2] },
            Cube { corner: vec![0, 1, 0], dirs: vec![0, 2] },
            Cube { corner: vec![0, 0, 0], dirs: vec![0, 1] },
            Cube { corner: vec![0, 0, 1], dirs: vec![0, 1] },
        ]);
        assert_eq!(s2.betti(), vec![1, 0, 1], "the cube's surface is S²: β = [1,0,1]");
        // homology reports degree-3 emptiness; π₃(S²) = Z lives entirely outside what β can see.
        assert_eq!(s2.dim(), 2, "a 2-dimensional complex — β has no degree-3 term at all, yet π₃ ≠ 0");
    }

    #[test]
    fn the_ladder_one_more_process_is_one_more_homotopy_dimension() {
        // THE LADDER, as a single test. For d = 2, 3, 4 processes, forbidding the center cell of a 3^d
        // grid produces a (d−1)-sphere void: β_{d−1} = 1 and every other β_k = 0 (but β₀ = 1). Each
        // added process climbs exactly one homotopy rung — π₁, π₂, π₃ — and the limit of this staircase
        // is the ∞-groupoid. One engine, every rung, all computed honestly over GF(2).
        for d in 2..=4 {
            let lengths = vec![3usize; d];
            let beta = CubicalComplex::progress(&lengths, &center(d)).betti();
            assert_eq!(beta.len(), d + 1);
            assert_eq!(beta[0], 1, "connected (d={d})");
            for k in 1..=d {
                let expected = usize::from(k == d - 1);
                assert_eq!(beta[k], expected, "d={d}: β_{k} should be {expected} (only π_{{d-1}} ≠ 0)");
            }
        }
    }
}
