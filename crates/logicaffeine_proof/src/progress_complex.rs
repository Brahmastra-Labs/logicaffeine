//! **Genuine higher homotopy, at last** — `π₁ ≠ 0` from real concurrency, the hole *is* the deadlock.
//!
//! Everything below `proof_rewrite` was a 1-type or contractible: a *discrete* symmetry gives `K(G,1)`,
//! a *deterministic* trace gives a contractible cube complex. Genuine non-trivial homotopy needs a
//! **forbidden region** — and concurrency with shared resources is exactly where one appears
//! (Fajstrup–Goubault–Raussen, *directed algebraic topology*).
//!
//! Model two processes of lengths `n` and `m` by their **progress complex**: a grid of states `(i, j)`
//! ("`P` has run `i` steps, `Q` has run `j` steps"), unit edges for single steps, and a filled square
//! at every cell where the two steps commute — *except* cells in the **forbidden region** (both
//! processes inside their critical section at once, which mutual exclusion forbids). Removing those
//! cells leaves a **hole**, and the hole is not decoration: it is the synchronization obstruction,
//! the deadlock, made into topology.
//!
//! We compute the homology rigorously — `β₀` by union-find, `β₁ = #E − #V + β₀ − rank ∂₂` and
//! `β₂ = #C − rank ∂₂` with the boundary rank taken over `GF(2)` by Gaussian elimination — and we
//! cross-check every case against the Euler–Poincaré identity `β₀ − β₁ + β₂ = χ`. No contention gives
//! a contractible square (`β₁ = 0`, determinism). One mutex gives `β₁ = 1`: the execution space is a
//! circle, and the two directed ways around the hole (`P`-first vs `Q`-first) are genuinely
//! inequivalent schedules — **the hole is precisely where the scheduler symmetry can no longer be
//! broken to one canonical class.** Higher homotopy = the obstruction to symmetry breaking.

use std::collections::{BTreeSet, HashMap};

/// `GF(2)` rank of a set of rows (each a bitset of column indices) by Gaussian elimination.
fn gf2_rank(mut rows: Vec<u128>) -> usize {
    let mut rank = 0;
    let mut col = 0;
    while col < 128 && rank < rows.len() {
        let bit = 1u128 << col;
        if let Some(pivot) = (rank..rows.len()).find(|&r| rows[r] & bit != 0) {
            rows.swap(rank, pivot);
            let pr = rows[rank];
            for r in 0..rows.len() {
                if r != rank && rows[r] & bit != 0 {
                    rows[r] ^= pr;
                }
            }
            rank += 1;
        }
        col += 1;
    }
    rank
}

/// `GF(2)` rank for wide bitsets (more than 128 columns), each row a `Vec<u64>` of `words` limbs.
pub(crate) fn gf2_rank_wide(mut rows: Vec<Vec<u64>>, ncols: usize) -> usize {
    let words = ncols.div_ceil(64);
    for r in &mut rows {
        r.resize(words, 0);
    }
    let mut rank = 0;
    for col in 0..ncols {
        let (w, bit) = (col / 64, 1u64 << (col % 64));
        if let Some(pivot) = (rank..rows.len()).find(|&r| rows[r][w] & bit != 0) {
            rows.swap(rank, pivot);
            let pr = rows[rank].clone();
            for r in 0..rows.len() {
                if r != rank && rows[r][w] & bit != 0 {
                    for t in 0..words {
                        rows[r][t] ^= pr[t];
                    }
                }
            }
            rank += 1;
            if rank == rows.len() {
                break;
            }
        }
    }
    rank
}

/// The progress complex of two processes of lengths `n`, `m` (an `n × m` grid of cells), with a set of
/// **forbidden cells** (the mutual-exclusion region) left unfilled.
pub struct ProgressComplex {
    pub n: usize,
    pub m: usize,
    forbidden: BTreeSet<(usize, usize)>,
}

impl ProgressComplex {
    pub fn new(n: usize, m: usize, forbidden: &[(usize, usize)]) -> Self {
        ProgressComplex {
            n,
            m,
            forbidden: forbidden.iter().copied().collect(),
        }
    }

    fn vid(&self, i: usize, j: usize) -> usize {
        i * (self.m + 1) + j
    }

    /// The four boundary edges of cell `(i, j)`, each as an ordered vertex-id pair.
    fn cell_edges(&self, i: usize, j: usize) -> [(usize, usize); 4] {
        let v = |a: usize, b: usize| self.vid(a, b);
        let order = |a: usize, b: usize| (a.min(b), a.max(b));
        [
            order(v(i, j), v(i + 1, j)),         // bottom
            order(v(i, j + 1), v(i + 1, j + 1)), // top
            order(v(i, j), v(i, j + 1)),         // left
            order(v(i + 1, j), v(i + 1, j + 1)), // right
        ]
    }

    fn allowed_cells(&self) -> Vec<(usize, usize)> {
        let mut cells = Vec::new();
        for i in 0..self.n {
            for j in 0..self.m {
                if !self.forbidden.contains(&(i, j)) {
                    cells.push((i, j));
                }
            }
        }
        cells
    }

    /// The Betti numbers `(β₀, β₁, β₂)` of the **closure of the allowed cells** — vertices and edges are
    /// included exactly when they bound some allowed cell, so removing a forbidden region opens a real
    /// hole. Homology over `GF(2)`, fully computed (no shortcuts).
    pub fn betti(&self) -> (usize, usize, usize) {
        let cells = self.allowed_cells();

        let mut edge_set: BTreeSet<(usize, usize)> = BTreeSet::new();
        let mut vert_set: BTreeSet<usize> = BTreeSet::new();
        for &(i, j) in &cells {
            for e in self.cell_edges(i, j) {
                edge_set.insert(e);
                vert_set.insert(e.0);
                vert_set.insert(e.1);
            }
        }

        let edges: Vec<(usize, usize)> = edge_set.into_iter().collect();
        let edge_index: HashMap<(usize, usize), usize> =
            edges.iter().enumerate().map(|(k, &e)| (e, k)).collect();
        let verts: Vec<usize> = vert_set.into_iter().collect();
        let vert_index: HashMap<usize, usize> =
            verts.iter().enumerate().map(|(k, &v)| (v, k)).collect();
        let (nv, ne, nc) = (verts.len(), edges.len(), cells.len());
        assert!(
            ne <= 128,
            "GF(2) rows are u128 — keep grids small (#edges = {ne})"
        );

        // β₀ — connected components of the 1-skeleton by union-find.
        let mut parent: Vec<usize> = (0..nv).collect();
        fn find(parent: &mut [usize], x: usize) -> usize {
            let mut r = x;
            while parent[r] != r {
                r = parent[r];
            }
            let mut c = x;
            while parent[c] != r {
                let nx = parent[c];
                parent[c] = r;
                c = nx;
            }
            r
        }
        for &(a, b) in &edges {
            let (ra, rb) = (
                find(&mut parent, vert_index[&a]),
                find(&mut parent, vert_index[&b]),
            );
            if ra != rb {
                parent[ra] = rb;
            }
        }
        let b0 = (0..nv).filter(|&x| find(&mut parent, x) == x).count();

        // ∂₂ over GF(2): each allowed cell ↦ the bitset of its four boundary edges.
        let d2: Vec<u128> = cells
            .iter()
            .map(|&(i, j)| {
                self.cell_edges(i, j)
                    .iter()
                    .fold(0u128, |row, e| row ^ (1u128 << edge_index[e]))
            })
            .collect();
        let rank2 = gf2_rank(d2);

        let b2 = nc - rank2;
        // β₁ = dim Z₁ − dim B₁ = (#E − rank ∂₁) − rank ∂₂, with rank ∂₁ = #V − β₀.
        let b1 = ne + b0 - nv - rank2;
        (b0, b1, b2)
    }

    /// `χ = #V − #E + #C` over the closure of the allowed cells.
    pub fn euler(&self) -> i64 {
        let cells = self.allowed_cells();
        let mut edge_set: BTreeSet<(usize, usize)> = BTreeSet::new();
        let mut vert_set: BTreeSet<usize> = BTreeSet::new();
        for &(i, j) in &cells {
            for e in self.cell_edges(i, j) {
                edge_set.insert(e);
                vert_set.insert(e.0);
                vert_set.insert(e.1);
            }
        }
        vert_set.len() as i64 - edge_set.len() as i64 + cells.len() as i64
    }
}

/// A 3D coordinate (vertex) of the three-process progress complex.
type V3 = (usize, usize, usize);

/// The progress complex of **three** processes of lengths `n`, `m`, `p` — a 3D grid of states, filled
/// with solid 3-cells (cubes) where all three steps commute, *except* forbidden cells. Climbing one
/// homotopy dimension: a forbidden core now leaves a hollow **2-sphere**, so `β₂ = π₂ ≠ 0` — the first
/// genuine `π₂` the tower *produces* (not just admits). Adding a process climbs a rung; the limit of
/// "one more process, one more dimension" is the `∞`-groupoid the ladder points at.
pub struct ProgressComplex3 {
    pub n: usize,
    pub m: usize,
    pub p: usize,
    forbidden: BTreeSet<V3>,
}

impl ProgressComplex3 {
    pub fn new(n: usize, m: usize, p: usize, forbidden: &[V3]) -> Self {
        ProgressComplex3 {
            n,
            m,
            p,
            forbidden: forbidden.iter().copied().collect(),
        }
    }

    /// The eight corners of cell `(i, j, l)`.
    fn corners(i: usize, j: usize, l: usize) -> [V3; 8] {
        let mut c = [(0, 0, 0); 8];
        for (b, slot) in c.iter_mut().enumerate() {
            *slot = (i + (b & 1), j + ((b >> 1) & 1), l + ((b >> 2) & 1));
        }
        c
    }

    fn differ_in_one(a: V3, b: V3) -> bool {
        let d = (a.0 != b.0) as u8 + (a.1 != b.1) as u8 + (a.2 != b.2) as u8;
        d == 1
    }

    /// The twelve edges of a cell — corner pairs differing in exactly one coordinate.
    fn cell_edges(i: usize, j: usize, l: usize) -> Vec<[V3; 2]> {
        let cs = Self::corners(i, j, l);
        let mut es = Vec::new();
        for a in 0..8 {
            for b in (a + 1)..8 {
                if Self::differ_in_one(cs[a], cs[b]) {
                    let (mut u, mut v) = (cs[a], cs[b]);
                    if v < u {
                        std::mem::swap(&mut u, &mut v);
                    }
                    es.push([u, v]);
                }
            }
        }
        es
    }

    /// The six faces of a cell, each the four corners sharing one fixed coordinate (axis, value), sorted.
    fn cell_faces(i: usize, j: usize, l: usize) -> Vec<[V3; 4]> {
        let cs = Self::corners(i, j, l);
        let mut fs = Vec::new();
        for axis in 0..3 {
            for val in 0..2 {
                let mut quad: Vec<V3> = cs
                    .iter()
                    .copied()
                    .filter(|&c| [c.0, c.1, c.2][axis] == [i, j, l][axis] + val)
                    .collect();
                quad.sort_unstable();
                fs.push([quad[0], quad[1], quad[2], quad[3]]);
            }
        }
        fs
    }

    /// The four boundary edges of a face (its four corners, adjacent pairs).
    fn face_edges(face: &[V3; 4]) -> Vec<[V3; 2]> {
        let mut es = Vec::new();
        for a in 0..4 {
            for b in (a + 1)..4 {
                if Self::differ_in_one(face[a], face[b]) {
                    let (mut u, mut v) = (face[a], face[b]);
                    if v < u {
                        std::mem::swap(&mut u, &mut v);
                    }
                    es.push([u, v]);
                }
            }
        }
        es
    }

    fn allowed_cells(&self) -> Vec<V3> {
        let mut cells = Vec::new();
        for i in 0..self.n {
            for j in 0..self.m {
                for l in 0..self.p {
                    if !self.forbidden.contains(&(i, j, l)) {
                        cells.push((i, j, l));
                    }
                }
            }
        }
        cells
    }

    /// Betti numbers `(β₀, β₁, β₂, β₃)` of the closure of the allowed 3-cells — full `GF(2)` homology
    /// through `∂₃`. `β₂` is the genuine `π₂` (2-voids); `β₃` detects enclosed 3-voids (none here).
    pub fn betti(&self) -> (usize, usize, usize, usize) {
        let cells = self.allowed_cells();

        // collect closure: every vertex/edge/face that bounds an allowed 3-cell
        let mut verts: BTreeSet<V3> = BTreeSet::new();
        let mut edges: BTreeSet<[V3; 2]> = BTreeSet::new();
        let mut faces: BTreeSet<[V3; 4]> = BTreeSet::new();
        for &(i, j, l) in &cells {
            for c in Self::corners(i, j, l) {
                verts.insert(c);
            }
            for e in Self::cell_edges(i, j, l) {
                edges.insert(e);
            }
            for f in Self::cell_faces(i, j, l) {
                faces.insert(f);
            }
        }
        let verts: Vec<V3> = verts.into_iter().collect();
        let edges: Vec<[V3; 2]> = edges.into_iter().collect();
        let faces: Vec<[V3; 4]> = faces.into_iter().collect();
        let vidx: HashMap<V3, usize> = verts.iter().enumerate().map(|(k, &v)| (v, k)).collect();
        let eidx: HashMap<[V3; 2], usize> =
            edges.iter().enumerate().map(|(k, &e)| (e, k)).collect();
        let fidx: HashMap<[V3; 4], usize> =
            faces.iter().enumerate().map(|(k, &f)| (f, k)).collect();
        let (nv, ne, nf, nc) = (verts.len(), edges.len(), faces.len(), cells.len());

        // β₀ — components of the 1-skeleton.
        let mut parent: Vec<usize> = (0..nv).collect();
        fn find(parent: &mut [usize], x: usize) -> usize {
            let mut r = x;
            while parent[r] != r {
                r = parent[r];
            }
            let mut c = x;
            while parent[c] != r {
                let nx = parent[c];
                parent[c] = r;
                c = nx;
            }
            r
        }
        for e in &edges {
            let (ra, rb) = (
                find(&mut parent, vidx[&e[0]]),
                find(&mut parent, vidx[&e[1]]),
            );
            if ra != rb {
                parent[ra] = rb;
            }
        }
        let b0 = (0..nv).filter(|&x| find(&mut parent, x) == x).count();

        // ∂₂ : faces → edges (4 each), ∂₃ : cells → faces (6 each), ranks over GF(2).
        let d2: Vec<Vec<u64>> = faces
            .iter()
            .map(|f| {
                let mut row = vec![0u64; ne.div_ceil(64)];
                for e in Self::face_edges(f) {
                    let idx = eidx[&e];
                    row[idx / 64] ^= 1u64 << (idx % 64);
                }
                row
            })
            .collect();
        let d3: Vec<Vec<u64>> = cells
            .iter()
            .map(|&(i, j, l)| {
                let mut row = vec![0u64; nf.div_ceil(64)];
                for f in Self::cell_faces(i, j, l) {
                    let idx = fidx[&f];
                    row[idx / 64] ^= 1u64 << (idx % 64);
                }
                row
            })
            .collect();
        let rank2 = gf2_rank_wide(d2, ne);
        let rank3 = gf2_rank_wide(d3, nf);

        let b1 = ne + b0 - nv - rank2;
        let b2 = nf - rank2 - rank3;
        let b3 = nc - rank3;
        (b0, b1, b2, b3)
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/progress_complex/tests.rs"]
mod tests;
