//! Union-Find (disjoint set) with path compression and union by rank.
//!
//! Shared by the kernel's congruence closure (`logicaffeine_kernel::cc`)
//! and the compiler's equality-saturation e-graph — one equivalence engine
//! underneath both the proof system and the optimizer.

/// Union-Find over `usize` element ids.
///
/// Supports near-constant amortized time for `find` (with path compression)
/// and `union` (by rank).
pub struct UnionFind {
    /// Parent pointer for each element (element is its own parent if root).
    parent: Vec<usize>,
    /// Rank (approximate tree depth) for union by rank optimization.
    rank: Vec<usize>,
}

impl UnionFind {
    pub fn new() -> Self {
        UnionFind {
            parent: Vec::new(),
            rank: Vec::new(),
        }
    }

    /// Add a new element, returns its ID.
    pub fn make_set(&mut self) -> usize {
        let id = self.parent.len();
        self.parent.push(id);
        self.rank.push(0);
        id
    }

    /// Number of elements ever created (not the number of classes).
    pub fn len(&self) -> usize {
        self.parent.len()
    }

    pub fn is_empty(&self) -> bool {
        self.parent.is_empty()
    }

    /// Find representative with path compression.
    pub fn find(&mut self, x: usize) -> usize {
        if self.parent[x] != x {
            self.parent[x] = self.find(self.parent[x]);
        }
        self.parent[x]
    }

    /// Union by rank, returns true if a merge occurred.
    pub fn union(&mut self, x: usize, y: usize) -> bool {
        let rx = self.find(x);
        let ry = self.find(y);
        if rx == ry {
            return false;
        }

        if self.rank[rx] < self.rank[ry] {
            self.parent[rx] = ry;
        } else if self.rank[rx] > self.rank[ry] {
            self.parent[ry] = rx;
        } else {
            self.parent[ry] = rx;
            self.rank[rx] += 1;
        }
        true
    }
}

impl Default for UnionFind {
    fn default() -> Self {
        Self::new()
    }
}
