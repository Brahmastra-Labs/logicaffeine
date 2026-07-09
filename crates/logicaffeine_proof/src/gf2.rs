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
    let mut aug: Vec<u64> =
        rows.iter().zip(rhs).map(|(&c, &b)| (c & coeff_mask) | ((b as u64) << n)).collect();
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
    Some(SolutionSpace { num_vars: n, particular, kernel_basis })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **It is exactly what it ought to be: the invertible count IS `|GL(n,2)|`.** Brute force over all
    /// `2^{n²}` matrices equals the orbit–stabilizer product `Π(2ⁿ − 2ⁱ)` — i.e. the invertible matrices
    /// are precisely the ordered bases, the simply-transitive orbit of `GL(n,2)`. Proven exhaustively for
    /// n = 1..4.
    #[test]
    fn the_invertible_count_is_the_general_linear_group() {
        for n in 1..=4u32 {
            let brute = count_invertible_gf2_bruteforce(n);
            let group = gl_order(n);
            assert_eq!(brute, group, "n={n}: brute-force invertible count must equal |GL(n,2)| = Π(2ⁿ−2ⁱ)");
        }
        // The factorization is the symmetry break: each factor counts vectors avoiding the span so far.
        for n in 1..=10u32 {
            let full = 1u128 << n;
            let stepwise: u128 = (0..n).map(|i| full - (1u128 << i)).product();
            assert_eq!(gl_order(n), stepwise, "|GL(n,2)| is the step-by-step basis enumeration");
        }
        // A couple of pinned values: |GL(2,2)| = 6 (≅ S₃), |GL(3,2)| = 168.
        assert_eq!(gl_order(2), 6);
        assert_eq!(gl_order(3), 168);
    }

    /// **The invertibility density is the Euler partial product, and its limit is the reciprocal
    /// SAT-threshold sum.** `|GL(n,2)| / 2^{n²} = Π_{j=1}^n(1 − 2⁻ʲ) → φ(½)`, and `−log₂ φ(½)` equals
    /// `Σ_{k≥1} 1/α*_k` exactly. The loop closes: the counting thresholds and the GF(2) group density are
    /// the same constant, viewed from two sides.
    #[test]
    fn the_invertibility_density_is_the_reciprocal_threshold_constant() {
        // density matches |GL|/2^{n²} for small n (where the u128 ratio is exact)
        for n in 1..=10u32 {
            let exact = gl_order(n) as f64 / 2f64.powi((n * n) as i32);
            assert!((invertibility_density(n) - exact).abs() < 1e-12, "density == |GL(n,2)|/2^(n²) at n={n}");
        }
        // the limit is φ(½), the GF(2) invertibility constant
        let phi_half = invertibility_density(60);
        assert!((phi_half - 0.288_788_095_1).abs() < 1e-9, "Π(1−2⁻ʲ) → φ(½) = 0.28879");
        // and −log₂ φ(½) is the reciprocal first-moment threshold sum, computed independently
        let ln2 = std::f64::consts::LN_2;
        let recip_sum: f64 = (1..=60u32).map(|k| 1.0 / crate::families::ksat_threshold_first_moment_upper(k)).sum();
        assert!((recip_sum + phi_half.ln() / ln2).abs() < 1e-6, "Σ 1/α*_k = −log₂ φ(½) — the loop closes");
        assert!((recip_sum - 1.791_916_824_7).abs() < 1e-6, "and it is ≈ 1.79192");
    }

    /// **Our own GF(2) substrate exhibits the constant.** Sampled random matrices are invertible at the
    /// Euler rate `Π(1−2⁻ʲ)` — the parity rung's reality matching the closed form. (Decorrelated seeds:
    /// never sample along SplitMix64's own increment γ, per the seed-collapse lesson.)
    #[test]
    fn random_gf2_matrices_are_invertible_at_the_euler_rate() {
        let n = 6u32;
        let trials = 4000u64;
        let mut rng = 0x1234_5678_9ABC_DEF0u64;
        let mut next = || {
            rng = rng.wrapping_add(0xD1B5_4A32_D192_ED03);
            let mut z = rng;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        };
        let mask = (1u64 << n) - 1;
        let invertible = (0..trials)
            .filter(|_| {
                let rows: Vec<u64> = (0..n).map(|_| next() & mask).collect();
                is_invertible_gf2(n, &rows)
            })
            .count();
        let rate = invertible as f64 / trials as f64;
        let expected = invertibility_density(n);
        assert!((rate - expected).abs() < 0.03, "empirical invertibility rate {rate:.4} ≈ Π(1−2⁻ʲ) = {expected:.4}");
    }

    // A row is satisfied by x (bit-packed) iff the parity over its support matches the rhs.
    fn satisfies(row: u64, rhs: bool, x: u64) -> bool {
        (row & x).count_ones() % 2 == rhs as u32
    }
    fn pack(x: &[bool]) -> u64 {
        x.iter().enumerate().fold(0u64, |a, (i, &b)| a | ((b as u64) << i))
    }

    /// **Symmetry-break HARD, then find every solution.** Over a fuzz of random GF(2) systems, the
    /// symmetry-broken `SolutionSpace` (one particular solution + a kernel basis) regenerates the *exact*
    /// full solution set — matching brute force corner-for-corner — and the count is `2^{nullity}`
    /// (rank–nullity). All `2^{n−rank}` solutions are recovered from `1 + (n−rank)` vectors: the kernel is
    /// the symmetry, and enumerating its orbit of the witness is "finding the solutions".
    #[test]
    fn the_kernel_break_recovers_every_solution() {
        let n = 12usize;
        let mut rng = 0xCAFE_F00D_1234_5678u64;
        let mut next = || {
            rng = rng.wrapping_add(0xD1B5_4A32_D192_ED03);
            let mut z = rng;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        };
        let mask = (1u64 << n) - 1;
        let mut saw_consistent = 0usize;
        let mut saw_inconsistent = 0usize;
        let mut saw_multiple = 0usize;
        for _ in 0..400 {
            let m = 3 + (next() as usize % 12); // 3..14 equations — under- and over-determined
            let rows: Vec<u64> = (0..m).map(|_| next() & mask).collect();
            let rhs: Vec<bool> = (0..m).map(|_| next() & 1 == 0).collect();

            // brute-force solution set
            let brute: std::collections::BTreeSet<u64> = (0u64..(1u64 << n))
                .filter(|&x| rows.iter().zip(&rhs).all(|(&r, &b)| satisfies(r, b, x)))
                .collect();

            match solve_gf2(n, &rows, &rhs) {
                None => {
                    assert!(brute.is_empty(), "solve_gf2 said inconsistent but solutions exist");
                    saw_inconsistent += 1;
                }
                Some(space) => {
                    saw_consistent += 1;
                    if space.kernel_basis.len() > 0 {
                        saw_multiple += 1;
                    }
                    // every generated solution is genuine
                    let gen: std::collections::BTreeSet<u64> =
                        space.enumerate().iter().map(|x| pack(x)).collect();
                    for &x in &gen {
                        assert!(rows.iter().zip(&rhs).all(|(&r, &b)| satisfies(r, b, x)), "generated non-solution");
                    }
                    // the orbit is EXACTLY the solution set, and the count is 2^nullity
                    assert_eq!(gen, brute, "kernel orbit must equal the full solution set");
                    assert_eq!(space.count(), brute.len() as u128, "count = #solutions");
                    assert_eq!(space.count(), 1u128 << space.kernel_basis.len(), "count = 2^(dim kernel)");
                }
            }
        }
        assert!(saw_consistent > 20 && saw_inconsistent > 20 && saw_multiple > 20,
            "fuzz must exercise consistent ({saw_consistent}), inconsistent ({saw_inconsistent}), and multi-solution ({saw_multiple}) systems");
    }

    /// **No symmetry ⟹ a unique witness — and that is the φ(½) event.** When the coefficient matrix is
    /// invertible (square, full rank), the kernel is trivial: exactly one solution, no break possible.
    /// Rank–nullity is the orbit–stabilizer law for the solution coset: `#solutions = 2^{n−rank}`. So the
    /// invertibility constant φ(½) is precisely the rate at which a random square system pins down a
    /// *unique* witness — rigidity and uniqueness are the same thing, here exactly and constructively.
    #[test]
    fn invertible_system_has_the_unique_rigid_witness() {
        let n = 8usize;
        let mut rng = 0x0BADC0DE_5EED_1234u64;
        let mut next = || {
            rng = rng.wrapping_add(0xD1B5_4A32_D192_ED03);
            let mut z = rng;
            z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
            z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
            z ^ (z >> 31)
        };
        let mask = (1u64 << n) - 1;
        let mut unique = 0usize;
        let mut total = 0usize;
        for _ in 0..2000 {
            let rows: Vec<u64> = (0..n).map(|_| next() & mask).collect();
            let rhs: Vec<bool> = (0..n).map(|_| next() & 1 == 0).collect();
            if is_invertible_gf2(n as u32, &rows) {
                // invertible ⟹ always consistent, with exactly one (rigid) solution
                let space = solve_gf2(n, &rows, &rhs).expect("invertible ⟹ consistent");
                assert_eq!(space.kernel_basis.len(), 0, "invertible ⟹ trivial kernel");
                assert_eq!(space.count(), 1, "invertible ⟹ a unique, rigid witness");
                unique += 1;
            } else {
                // singular ⟹ either inconsistent (None) or many solutions (nontrivial kernel)
                match solve_gf2(n, &rows, &rhs) {
                    None => {}
                    Some(space) => {
                        assert!(!space.kernel_basis.is_empty(), "consistent singular ⟹ a symmetry to break");
                        assert!(space.count() >= 2, "consistent singular ⟹ multiple solutions");
                    }
                }
            }
            total += 1;
        }
        // the unique-solution rate IS the GL(n,2) density φ(½) ≈ Π(1−2⁻ʲ)
        let rate = unique as f64 / total as f64;
        assert!((rate - invertibility_density(n as u32)).abs() < 0.03,
            "unique-witness rate {rate:.4} ≈ φ(½) = {:.4}", invertibility_density(n as u32));
    }
}
