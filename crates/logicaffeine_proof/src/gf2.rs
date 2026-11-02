//! The general linear group **GL(n,2)** and the invertibility constant — where the reciprocal
//! SAT-threshold sum comes home.
//!
//! A uniformly random n×n matrix over GF(2) is invertible with probability `Π_{j=1}^n (1 − 2⁻ʲ)`, which
//! → `φ(½) = 0.2887880951`. The reciprocal first-moment SAT-threshold sum is `−log₂ φ(½)` exactly, so the
//! two meet at this one constant. And it is *not* a coincidence — it is a **symmetry break**:
//!
//! - The invertible matrices are exactly the group `GL(n,2)`.
//! - `GL(n,2)` acts **simply transitively on ordered bases** of `GF(2)ⁿ` (a matrix is invertible iff its
//!   rows are a basis). So by orbit–stabilizer with a *trivial* stabilizer, `|GL(n,2)|` = the number of
//!   ordered bases = `Π_{i=0}^{n-1} (2ⁿ − 2ⁱ)` — each factor counting the vectors that *avoid the span
//!   built so far*. That step-by-step quotient is the symmetry-broken enumeration; dividing by `2^{n²}`
//!   gives the Euler partial product.

/// `|GL(n,2)|` via the orbit–stabilizer (ordered-basis) factorization `Π_{i=0}^{n-1}(2ⁿ − 2ⁱ)`. Each
/// factor `2ⁿ − 2ⁱ` is the count of vectors outside the i-dimensional span of the basis chosen so far —
/// the symmetry-broken enumeration of the bases on which `GL(n,2)` acts simply transitively. Valid up to
/// `n = 10` before the `u128` product overflows.
pub fn gl_order(n: u32) -> u128 {
    let full = 1u128 << n;
    (0..n).map(|i| full - (1u128 << i)).product()
}

/// Is an n×n GF(2) matrix (each row packed as the low `n` bits of a `u64`) invertible? Gaussian
/// elimination over GF(2): invertible iff it reduces to full rank `n`.
pub fn is_invertible_gf2(n: u32, rows: &[u64]) -> bool {
    let mut rows = rows.to_vec();
    let mut rank = 0usize;
    for col in 0..n {
        if let Some(p) = (rank..rows.len()).find(|&r| (rows[r] >> col) & 1 == 1) {
            rows.swap(rank, p);
            for r in 0..rows.len() {
                if r != rank && (rows[r] >> col) & 1 == 1 {
                    rows[r] ^= rows[rank];
                }
            }
            rank += 1;
        }
    }
    rank == n as usize
}

/// Brute-force count of invertible n×n GF(2) matrices, over all `2^{n²}` of them. Feasible for `n ≤ 4`.
pub fn count_invertible_gf2_bruteforce(n: u32) -> u128 {
    let cells = n * n;
    let mask = (1u64 << n) - 1;
    let mut count = 0u128;
    for bits in 0u64..(1u64 << cells) {
        let rows: Vec<u64> = (0..n).map(|r| (bits >> (r * n)) & mask).collect();
        if is_invertible_gf2(n, &rows) {
            count += 1;
        }
    }
    count
}

/// The Euler partial product `Π_{j=1}^n (1 − 2⁻ʲ)` — the exact invertibility *density* `|GL(n,2)| / 2^{n²}`.
pub fn invertibility_density(n: u32) -> f64 {
    (1..=n).map(|j| 1.0 - 2f64.powi(-(j as i32))).product()
}

/// The **complete solution space** of a GF(2) linear system `A x = b`, in *symmetry-broken* form: one
/// particular solution `x₀` plus a basis of the kernel. Every solution is `x₀ ⊕ (a GF(2) combination of
/// the kernel basis)`, so the entire affine coset — all `2^{n−rank}` solutions — is generated from this
/// compressed witness. The kernel is the **symmetry group** of the solution set (the translations that
/// preserve `Ax`), `x₀` is one witness, and the basis are its generators: this is the exact, polynomial,
/// linear analog of `hypercube::model_orbit`. The harder the break (the bigger the kernel), the more
/// solutions; an *invertible* system has a trivial kernel — no symmetry, a unique witness.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SolutionSpace {
    pub num_vars: usize,
    pub particular: Vec<bool>,
    pub kernel_basis: Vec<Vec<bool>>,
}

impl SolutionSpace {
    /// The number of solutions: `2^{dim kernel}` (rank–nullity) — the orbit size of the solution coset.
    pub fn count(&self) -> u128 {
        1u128 << self.kernel_basis.len()
    }

    /// Generate **every** solution by XORing the particular solution with each subset of the kernel basis
    /// — the orbit of `x₀` under the kernel's translation symmetry.
    pub fn enumerate(&self) -> Vec<Vec<bool>> {
        let k = self.kernel_basis.len();
        (0u64..(1u64 << k))
            .map(|mask| {
                let mut x = self.particular.clone();
                for b in 0..k {
                    if (mask >> b) & 1 == 1 {
                        for v in 0..self.num_vars {
                            x[v] ^= self.kernel_basis[b][v];
                        }
                    }
                }
                x
            })
            .collect()
    }
}

/// Solve a GF(2) system `A x = b` (each row a coefficient bit-vector in the low `n` bits, `rhs` the
/// right-hand sides) for its **entire** solution space via Gaussian elimination to reduced row echelon
/// form. Returns the symmetry-broken [`SolutionSpace`] (particular solution + kernel basis), or `None`
/// iff the system is inconsistent. Generalizes [`crate::xorsat::solve`], which returns just one witness.
pub fn solve_gf2(n: usize, rows: &[u64], rhs: &[bool]) -> Option<SolutionSpace> {
    let coeff_mask = if n == 64 { u64::MAX } else { (1u64 << n) - 1 };
    let mut aug: Vec<u64> = rows
        .iter()
        .zip(rhs)
        .map(|(&c, &b)| (c & coeff_mask) | ((b as u64) << n))
        .collect();
    let mut pivot_col_of_row: Vec<usize> = Vec::new();
    let mut rank = 0usize;
    for col in 0..n {
        if let Some(p) = (rank..aug.len()).find(|&r| (aug[r] >> col) & 1 == 1) {
            aug.swap(rank, p);
            for r in 0..aug.len() {
                if r != rank && (aug[r] >> col) & 1 == 1 {
                    aug[r] ^= aug[rank];
                }
            }
            pivot_col_of_row.push(col);
            rank += 1;
        }
    }
    // Inconsistent: a fully-reduced row with no coefficients but a 1 on the right (0 = 1).
    for r in 0..aug.len() {
        if aug[r] & coeff_mask == 0 && (aug[r] >> n) & 1 == 1 {
            return None;
        }
    }
    let mut is_pivot = vec![false; n];
    for &c in &pivot_col_of_row {
        is_pivot[c] = true;
    }
    // Particular solution: free variables 0, each pivot variable = its row's right-hand side.
    let mut particular = vec![false; n];
    for (r, &pc) in pivot_col_of_row.iter().enumerate() {
        particular[pc] = (aug[r] >> n) & 1 == 1;
    }
    // Kernel basis: one vector per free column f — set x_f = 1, each pivot var = its row's f-coefficient.
    let mut kernel_basis = Vec::new();
    for f in 0..n {
        if is_pivot[f] {
            continue;
        }
        let mut kv = vec![false; n];
        kv[f] = true;
        for (r, &pc) in pivot_col_of_row.iter().enumerate() {
            if (aug[r] >> f) & 1 == 1 {
                kv[pc] = true;
            }
        }
        kernel_basis.push(kv);
    }
    Some(SolutionSpace {
        num_vars: n,
        particular,
        kernel_basis,
    })
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/gf2/tests.rs"]
mod tests;
