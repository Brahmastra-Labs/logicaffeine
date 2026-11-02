//! Reed-Solomon forward error correction — the erasure codec that lets a lossy
//! transport reconstruct dropped packets *without* a retransmit round-trip.
//!
//! Given `k` data shards we compute `m` parity shards; the `n = k + m` shards are
//! **Maximum Distance Separable (MDS)**: *any* `k` of the `n` shards reconstruct the
//! original data exactly. Losing up to `m` shards is fully recoverable.
//!
//! The construction is a *systematic* Reed-Solomon code over GF(2^8) (the [`crate::gf256`]
//! field). Start from an `n×k` Vandermonde matrix `V` on `n` distinct field nodes
//! `0,1,…,n-1`: every `k×k` submatrix of a Vandermonde on distinct nodes is invertible
//! (its determinant is `∏_{a<b}(x_b − x_a) ≠ 0`), which is exactly the MDS property.
//! Right-multiplying by the inverse of `V`'s top `k×k` block yields the generator
//! `G = V · V_top⁻¹`, whose top `k` rows are the identity — so the first `k` encoded
//! shards *are* the data (systematic), the rest are parity, and every `k`-row submatrix
//! stays invertible (invertible · invertible). Encoding is `G` applied per byte-column;
//! erasure recovery inverts the `k×k` submatrix of the surviving rows. All exact, no
//! floating point.

use crate::gf256;

/// A systematic Reed-Solomon erasure code with `k` data shards and `m = n − k` parity
/// shards. Cheap to clone (holds only the `n×k` generator matrix); construct once with
/// [`ReedSolomon::new`] and reuse across many messages of the same shape.
#[derive(Clone, Debug)]
pub struct ReedSolomon {
    k: usize,
    n: usize,
    /// The `n×k` systematic generator: rows `0..k` are the identity, rows `k..n` parity.
    generator: Vec<Vec<u8>>,
}

impl ReedSolomon {
    /// Build a code with `data_shards` data shards and `parity_shards` parity shards.
    /// Returns `None` if the parameters are out of range: both counts must be at least
    /// one and the total `n = data + parity` must fit the field's distinct nodes
    /// (`n ≤ 256`). The construction is deterministic — same parameters, same code.
    pub fn new(data_shards: usize, parity_shards: usize) -> Option<Self> {
        let k = data_shards;
        let n = k.checked_add(parity_shards)?;
        if k == 0 || parity_shards == 0 || n > 256 {
            return None;
        }
        // Vandermonde on nodes 0..n-1: row i, column j = node_i^j.
        let vander: Vec<Vec<u8>> = (0..n)
            .map(|i| {
                let node = i as u8;
                (0..k).map(|j| gf256::pow(node, j)).collect()
            })
            .collect();
        // Systematic: divide out the top k×k block so rows 0..k become the identity.
        let top: Vec<Vec<u8>> = vander[..k].to_vec();
        let top_inv = gf256::invert(&top)?;
        let generator = gf256::matmul(&vander, &top_inv);
        Some(Self { k, n, generator })
    }

    /// The number of data shards `k`.
    pub fn data_shards(&self) -> usize {
        self.k
    }

    /// The number of parity shards `m = n − k`.
    pub fn parity_shards(&self) -> usize {
        self.n - self.k
    }

    /// The total number of shards `n = k + m`.
    pub fn total_shards(&self) -> usize {
        self.n
    }

    /// Encode `k` equal-length data shards into `n` shards. The first `k` outputs are the
    /// data shards unchanged (the code is systematic); the remaining `m` are parity.
    /// Returns `None` unless exactly `k` shards of equal length are supplied.
    pub fn encode(&self, data: &[Vec<u8>]) -> Option<Vec<Vec<u8>>> {
        if data.len() != self.k {
            return None;
        }
        let len = data.first().map_or(0, |s| s.len());
        if data.iter().any(|s| s.len() != len) {
            return None;
        }
        Some(self.apply(&self.generator, data, len))
    }

    /// Reconstruct all `n` shards from a slice of `n` slots, each `Some(shard)` if the
    /// shard survived or `None` if it was erased. Succeeds when at least `k` shards
    /// survive (and share a length); returns `None` otherwise. The result is the full,
    /// byte-exact set of `n` shards — recovered data in `0..k`, regenerated parity after.
    pub fn reconstruct(&self, shards: &[Option<Vec<u8>>]) -> Option<Vec<Vec<u8>>> {
        if shards.len() != self.n {
            return None;
        }
        let present: Vec<usize> = (0..self.n).filter(|&i| shards[i].is_some()).collect();
        if present.len() < self.k {
            return None;
        }
        let len = shards[present[0]].as_ref().unwrap().len();
        if present
            .iter()
            .any(|&i| shards[i].as_ref().unwrap().len() != len)
        {
            return None;
        }
        // Any k surviving rows suffice; the k×k submatrix they form is invertible (MDS).
        let sel: Vec<usize> = present[..self.k].to_vec();
        let sub: Vec<Vec<u8>> = sel.iter().map(|&i| self.generator[i].clone()).collect();
        let sub_inv = gf256::invert(&sub)?;
        let survived: Vec<Vec<u8>> = sel
            .iter()
            .map(|&i| shards[i].as_ref().unwrap().clone())
            .collect();
        // data = (G_sel)⁻¹ · survived, per byte-column.
        let data = self.apply(&sub_inv, &survived, len);
        // Re-encode to fill in every shard (data + regenerated parity), all byte-exact.
        self.encode(&data)
    }

    /// Apply an `r×k` matrix to `k` shards of length `len`, producing `r` shards: output
    /// row `i`, byte `b` = `Σ_j matrix[i][j] · shards[j][b]` over GF(2^8).
    fn apply(&self, matrix: &[Vec<u8>], shards: &[Vec<u8>], len: usize) -> Vec<Vec<u8>> {
        matrix
            .iter()
            .map(|row| {
                // `out[b] = Σ_j row[j]·shards[j][b]` accumulated one whole shard at a time via the
                // SIMD-friendly region multiply — the vectorizable RS hot path. Zero coefficients
                // contribute nothing, so they are skipped (sparse rows get faster still).
                let mut out = vec![0u8; len];
                for (j, &coeff) in row.iter().enumerate() {
                    if coeff != 0 {
                        gf256::mul_add_region(coeff, &shards[j], &mut out);
                    }
                }
                out
            })
            .collect()
    }
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_base/fec/tests.rs"]
mod tests;
