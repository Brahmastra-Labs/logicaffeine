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
        assert_eq!(
            brute, group,
            "n={n}: brute-force invertible count must equal |GL(n,2)| = Π(2ⁿ−2ⁱ)"
        );
    }
    // The factorization is the symmetry break: each factor counts vectors avoiding the span so far.
    for n in 1..=10u32 {
        let full = 1u128 << n;
        let stepwise: u128 = (0..n).map(|i| full - (1u128 << i)).product();
        assert_eq!(
            gl_order(n),
            stepwise,
            "|GL(n,2)| is the step-by-step basis enumeration"
        );
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
        assert!(
            (invertibility_density(n) - exact).abs() < 1e-12,
            "density == |GL(n,2)|/2^(n²) at n={n}"
        );
    }
    // the limit is φ(½), the GF(2) invertibility constant
    let phi_half = invertibility_density(60);
    assert!(
        (phi_half - 0.288_788_095_1).abs() < 1e-9,
        "Π(1−2⁻ʲ) → φ(½) = 0.28879"
    );
    // and −log₂ φ(½) is the reciprocal first-moment threshold sum, computed independently
    let ln2 = std::f64::consts::LN_2;
    let recip_sum: f64 = (1..=60u32)
        .map(|k| 1.0 / crate::families::ksat_threshold_first_moment_upper(k))
        .sum();
    assert!(
        (recip_sum + phi_half.ln() / ln2).abs() < 1e-6,
        "Σ 1/α*_k = −log₂ φ(½) — the loop closes"
    );
    assert!(
        (recip_sum - 1.791_916_824_7).abs() < 1e-6,
        "and it is ≈ 1.79192"
    );
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
    assert!(
        (rate - expected).abs() < 0.03,
        "empirical invertibility rate {rate:.4} ≈ Π(1−2⁻ʲ) = {expected:.4}"
    );
}

// A row is satisfied by x (bit-packed) iff the parity over its support matches the rhs.
fn satisfies(row: u64, rhs: bool, x: u64) -> bool {
    (row & x).count_ones() % 2 == rhs as u32
}
fn pack(x: &[bool]) -> u64 {
    x.iter()
        .enumerate()
        .fold(0u64, |a, (i, &b)| a | ((b as u64) << i))
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
                assert!(
                    brute.is_empty(),
                    "solve_gf2 said inconsistent but solutions exist"
                );
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
                    assert!(
                        rows.iter().zip(&rhs).all(|(&r, &b)| satisfies(r, b, x)),
                        "generated non-solution"
                    );
                }
                // the orbit is EXACTLY the solution set, and the count is 2^nullity
                assert_eq!(gen, brute, "kernel orbit must equal the full solution set");
                assert_eq!(space.count(), brute.len() as u128, "count = #solutions");
                assert_eq!(
                    space.count(),
                    1u128 << space.kernel_basis.len(),
                    "count = 2^(dim kernel)"
                );
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
                    assert!(
                        !space.kernel_basis.is_empty(),
                        "consistent singular ⟹ a symmetry to break"
                    );
                    assert!(
                        space.count() >= 2,
                        "consistent singular ⟹ multiple solutions"
                    );
                }
            }
        }
        total += 1;
    }
    // the unique-solution rate IS the GL(n,2) density φ(½) ≈ Π(1−2⁻ʲ)
    let rate = unique as f64 / total as f64;
    assert!(
        (rate - invertibility_density(n as u32)).abs() < 0.03,
        "unique-witness rate {rate:.4} ≈ φ(½) = {:.4}",
        invertibility_density(n as u32)
    );
}
