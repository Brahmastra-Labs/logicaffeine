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
            faces.push(Cube {
                corner: self.corner.clone(),
                dirs: sub.clone(),
            });
            let mut up = self.corner.clone();
            up[a] += 1;
            faces.push(Cube {
                corner: up,
                dirs: sub,
            });
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
                top.push(Cube {
                    corner: idx.clone(),
                    dirs: (0..d).collect(),
                });
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
        let lidx: HashMap<Cube, usize> =
            lower.into_iter().enumerate().map(|(i, c)| (c, i)).collect();
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
        (0..=self.dim)
            .map(|k| self.cells[k].len() - ranks[k] - ranks[k + 1])
            .collect()
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
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/cubical/tests.rs"]
mod tests;
