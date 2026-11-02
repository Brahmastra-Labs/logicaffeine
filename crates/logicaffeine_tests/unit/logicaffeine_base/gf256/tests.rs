use super::*;

#[test]
fn mul_table_matches_scalar_multiplication_for_every_constant() {
    for c in 0u16..=255 {
        let c = c as u8;
        let t = mul_table(c);
        for b in 0u16..=255 {
            assert_eq!(t[b as usize], mul(c, b as u8), "mul_table[{c}][{b}] ≠ c·b");
        }
    }
}

#[test]
fn mul_add_region_handles_edge_lengths_and_the_identity_and_zero_coefficients() {
    // Empty region is a no-op.
    let mut dst: Vec<u8> = vec![];
    mul_add_region(42, &[], &mut dst);
    assert!(dst.is_empty());
    // Coefficient 0 contributes nothing (dst unchanged); coefficient 1 XORs src straight in.
    let src = vec![9u8, 200, 255, 1, 0, 77];
    let mut z = vec![5u8; src.len()];
    mul_add_region(0, &src, &mut z);
    assert_eq!(z, vec![5u8; src.len()], "×0 accumulates nothing");
    let mut one = vec![0u8; src.len()];
    mul_add_region(1, &src, &mut one);
    assert_eq!(one, src, "×1 into a zeroed region is the identity copy");
    // A single-byte region, and accumulation across two coefficients equals the scalar sum.
    let mut acc = vec![3u8; 4];
    let s1 = [1u8, 2, 3, 4];
    let s2 = [5u8, 6, 7, 8];
    mul_add_region(7, &s1, &mut acc);
    mul_add_region(11, &s2, &mut acc);
    let expected: Vec<u8> = (0..4).map(|i| 3 ^ mul(7, s1[i]) ^ mul(11, s2[i])).collect();
    assert_eq!(
        acc, expected,
        "chained region MACs equal the scalar accumulation"
    );
}

#[test]
fn mul_add_region_equals_the_byte_by_byte_scalar_loop() {
    let mut rng = R(0x5EED_5A1A_D0FF_1234);
    for _ in 0..3000 {
        let c = rng.byte();
        let len = 1 + rng.upto(64);
        let src: Vec<u8> = (0..len).map(|_| rng.byte()).collect();
        let mut dst: Vec<u8> = (0..len).map(|_| rng.byte()).collect();
        let mut expected = dst.clone();
        for (e, &s) in expected.iter_mut().zip(src.iter()) {
            *e ^= mul(c, s);
        }
        mul_add_region(c, &src, &mut dst);
        assert_eq!(
            dst, expected,
            "the region multiply-accumulate must equal the scalar field loop (c={c}, len={len})"
        );
    }
}

/// Deterministic splitmix64 — reproducible fuzzing, no external RNG.
struct R(u64);
impl R {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }
    fn byte(&mut self) -> u8 {
        self.next() as u8
    }
    fn upto(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// An INDEPENDENT multiplication oracle: bit-by-bit "Russian peasant" polynomial
/// multiply reduced modulo `0x11d`. It shares no code path with the log/exp table,
/// so agreeing with `mul` on all 65 536 pairs proves the table is uncorrupted.
fn ref_mul(mut a: u8, mut b: u8) -> u8 {
    let mut p: u8 = 0;
    for _ in 0..8 {
        if b & 1 != 0 {
            p ^= a;
        }
        let hi = a & 0x80;
        a <<= 1;
        if hi != 0 {
            a ^= 0x1d; // reduce (0x11d mod 2^8 = 0x1d) after the implicit carry out of bit 8
        }
        b >>= 1;
    }
    p
}

fn identity(k: usize) -> Vec<Vec<u8>> {
    (0..k)
        .map(|i| (0..k).map(|j| if i == j { 1 } else { 0 }).collect())
        .collect()
}

fn random_matrix(rng: &mut R, rows: usize, cols: usize) -> Vec<Vec<u8>> {
    (0..rows)
        .map(|_| (0..cols).map(|_| rng.byte()).collect())
        .collect()
}

fn random_invertible(rng: &mut R, k: usize) -> Vec<Vec<u8>> {
    loop {
        let m = random_matrix(rng, k, k);
        if invert(&m).is_some() {
            return m;
        }
    }
}

#[test]
fn exp_log_tables_are_true_inverses_and_the_group_is_cyclic_of_order_255() {
    let g = gf();
    for a in 1u16..=255 {
        let a = a as u8;
        // log then exp round-trips every nonzero element.
        assert_eq!(g.exp[g.log[a as usize] as usize], a, "exp∘log ≠ id at {a}");
    }
    // The generator 2 has full multiplicative order 255 (primitive element).
    assert_eq!(g.exp[0], 1, "generator^0 must be 1");
    assert_eq!(
        g.exp[255], 1,
        "generator^255 must wrap to 1 (order divides 255)"
    );
    for e in 1..255 {
        assert_ne!(g.exp[e], 1, "generator^{e} = 1 would mean order {e} < 255");
    }
}

#[test]
fn a_zero_divisor_yields_the_safe_zero_sentinel_not_garbage() {
    // 0 has no multiplicative inverse; div/inv must return the defined 0 sentinel (never the
    // silent nonzero garbage a raw log-table lookup gives — inv(0) once returned 1) so a caller
    // that violates the nonzero precondition fails safe, in release as well as debug.
    assert_eq!(inv(0), 0, "inv(0) must be the 0 sentinel");
    for a in 0u16..=255 {
        assert_eq!(div(a as u8, 0), 0, "div({a}, 0) must be the 0 sentinel");
    }
    // And on its valid domain the inverse is genuine: a · a⁻¹ = 1 for every nonzero a.
    for a in 1u16..=255 {
        let a = a as u8;
        assert_eq!(mul(a, inv(a)), 1, "a · inv(a) ≠ 1 at a={a}");
    }
}

#[test]
fn mul_matches_an_independent_bitwise_reference_across_the_entire_field() {
    for a in 0u16..=255 {
        for b in 0u16..=255 {
            let (a, b) = (a as u8, b as u8);
            assert_eq!(
                mul(a, b),
                ref_mul(a, b),
                "table mul disagrees with bitwise reference at a={a} b={b}"
            );
        }
    }
}

#[test]
fn addition_is_xor_and_is_its_own_inverse() {
    for a in 0u16..=255 {
        for b in 0u16..=255 {
            let (a, b) = (a as u8, b as u8);
            assert_eq!(add(a, b), a ^ b);
            assert_eq!(sub(a, b), add(a, b), "char-2: sub == add");
            assert_eq!(add(add(a, b), b), a, "adding b twice cancels");
        }
    }
}

#[test]
fn multiplication_has_identity_zero_and_is_commutative() {
    for a in 0u16..=255 {
        let a = a as u8;
        assert_eq!(mul(a, 0), 0, "0 absorbs");
        assert_eq!(mul(a, 1), a, "1 is the identity");
        for b in 0u16..=255 {
            let b = b as u8;
            assert_eq!(mul(a, b), mul(b, a), "mul commutes at a={a} b={b}");
        }
    }
}

#[test]
fn every_nonzero_element_has_a_unique_two_sided_inverse() {
    for a in 1u16..=255 {
        let a = a as u8;
        let ai = inv(a);
        assert_eq!(mul(a, ai), 1, "a·a⁻¹ = 1 at a={a}");
        assert_eq!(mul(ai, a), 1, "a⁻¹·a = 1 at a={a}");
        assert_eq!(inv(ai), a, "inverse is an involution at a={a}");
    }
}

#[test]
fn division_is_multiplication_by_the_inverse() {
    for a in 0u16..=255 {
        for b in 1u16..=255 {
            let (a, b) = (a as u8, b as u8);
            assert_eq!(div(a, b), mul(a, inv(b)), "div(a,b)=a·b⁻¹ at a={a} b={b}");
            assert_eq!(mul(div(a, b), b), a, "(a/b)·b = a at a={a} b={b}");
        }
    }
}

#[test]
fn multiplication_is_associative_and_distributes_over_addition() {
    let mut rng = R(0xF1E1_D1C1_B1A1_9181);
    for _ in 0..300_000 {
        let a = rng.byte();
        let b = rng.byte();
        let c = rng.byte();
        assert_eq!(
            mul(mul(a, b), c),
            mul(a, mul(b, c)),
            "associativity a={a} b={b} c={c}"
        );
        assert_eq!(
            mul(a, add(b, c)),
            add(mul(a, b), mul(a, c)),
            "distributivity a={a} b={b} c={c}"
        );
    }
}

#[test]
fn pow_matches_repeated_multiplication() {
    for base in (0u16..=255).step_by(7) {
        let base = base as u8;
        let mut acc = 1u8;
        for e in 0..40usize {
            assert_eq!(pow(base, e), acc, "pow({base},{e})");
            acc = mul(acc, base);
        }
    }
}

#[test]
fn matrix_inverse_round_trips_both_sides_and_rejects_singular() {
    let mut rng = R(0x00C0_FFEE_D00D_1234);
    for k in 1..=9usize {
        for _ in 0..40 {
            let m = random_invertible(&mut rng, k);
            let mi = invert(&m).expect("random_invertible produced an invertible matrix");
            assert_eq!(matmul(&m, &mi), identity(k), "M·M⁻¹ = I (k={k})");
            assert_eq!(matmul(&mi, &m), identity(k), "M⁻¹·M = I (k={k})");
        }
        if k >= 2 {
            // A duplicated row is singular.
            let mut m = random_matrix(&mut rng, k, k);
            m[1] = m[0].clone();
            assert!(
                invert(&m).is_none(),
                "duplicate-row matrix must be singular (k={k})"
            );
            // A zero row is singular.
            let mut z = random_matrix(&mut rng, k, k);
            z[k - 1] = vec![0u8; k];
            assert!(
                invert(&z).is_none(),
                "zero-row matrix must be singular (k={k})"
            );
        }
    }
}

// ── The pit-of-success invariants for the linear solver ──────────────────────────

#[test]
fn solve_recovers_the_unique_solution_of_a_full_rank_square_system() {
    let mut rng = R(0x5150_A5A5_1234_9999);
    for n in 1..=10usize {
        for _ in 0..60 {
            let a = random_invertible(&mut rng, n);
            let x: Vec<u8> = (0..n).map(|_| rng.byte()).collect();
            let b = matvec(&a, &x);
            let got = solve(&a, &b).expect("full-rank system is solvable");
            assert_eq!(got, x, "solve must recover the unique x (n={n})");
        }
    }
}

#[test]
fn any_solution_solve_returns_actually_satisfies_the_system() {
    // Rectangular and rank-deficient systems included: whatever solve hands back
    // must be a genuine solution, and it must only decline truly inconsistent ones.
    let mut rng = R(0xABCD_0123_7777_4242);
    for _ in 0..4000 {
        let m = 1 + rng.upto(6);
        let n = 1 + rng.upto(6);
        let a = random_matrix(&mut rng, m, n);
        let b: Vec<u8> = (0..m).map(|_| rng.byte()).collect();
        match solve(&a, &b) {
            Some(x) => {
                assert_eq!(x.len(), n);
                assert_eq!(matvec(&a, &x), b, "returned x must satisfy A·x = b");
            }
            None => {
                // Declined ⇒ b must be outside the column space: no x can satisfy it.
                // Verify via rank(A) < rank([A|b]).
                let aug: Vec<Vec<u8>> = a
                    .iter()
                    .zip(b.iter())
                    .map(|(row, &r)| {
                        let mut rr = row.clone();
                        rr.push(r);
                        rr
                    })
                    .collect();
                assert!(
                    rank(&aug) > rank(&a),
                    "solve declined a consistent system (rank did not grow)"
                );
            }
        }
    }
}

#[test]
fn solve_constructs_and_confirms_a_solution_whenever_one_exists() {
    // Build b = A·x from a known x so the system is guaranteed consistent; solve must
    // return SOME solution (not necessarily x for rank-deficient A), and it must check.
    let mut rng = R(0x9999_1111_2222_3333);
    for _ in 0..4000 {
        let m = 1 + rng.upto(6);
        let n = 1 + rng.upto(6);
        let a = random_matrix(&mut rng, m, n);
        let x: Vec<u8> = (0..n).map(|_| rng.byte()).collect();
        let b = matvec(&a, &x);
        let got = solve(&a, &b).expect("A·x=b is consistent by construction");
        assert_eq!(
            matvec(&a, &got),
            b,
            "constructed solution must satisfy the system"
        );
    }
}

#[test]
fn solve_declines_a_provably_inconsistent_system() {
    // x = 0 and x = 1 over one variable: no solution.
    assert_eq!(solve(&[vec![1], vec![1]], &[0, 1]), None);
    // x + y = 0 twice but with different RHS is inconsistent.
    assert_eq!(solve(&[vec![1, 1], vec![1, 1]], &[3, 7]), None);
}

// ── The pit-of-success invariants for null-space extraction (the XL kernel) ───────

#[test]
fn nullspace_vectors_are_annihilated_span_the_kernel_and_are_independent() {
    let mut rng = R(0x7A7A_5C5C_3E3E_1010);
    for _ in 0..3000 {
        let m = 1 + rng.upto(6);
        let n = 1 + rng.upto(6);
        let a = random_matrix(&mut rng, m, n);
        let basis = nullspace(&a);
        let r = rank(&a);
        // Dimension: |basis| = n − rank(A).
        assert_eq!(basis.len(), n - r, "nullity must be n − rank (m={m} n={n})");
        // Membership: every basis vector is annihilated by A.
        for v in &basis {
            assert_eq!(
                matvec(&a, v),
                vec![0u8; m],
                "A·v ≠ 0 for a null-space vector"
            );
        }
        // Independence: the basis stacked as rows has full rank equal to its size.
        if !basis.is_empty() {
            assert_eq!(
                rank(&basis),
                basis.len(),
                "null-space basis must be independent"
            );
        }
    }
}

#[test]
fn particular_solution_plus_any_kernel_vector_is_still_a_solution() {
    let mut rng = R(0x1357_9BDF_2468_ACE0);
    for _ in 0..3000 {
        let m = 1 + rng.upto(6);
        let n = 1 + rng.upto(6);
        let a = random_matrix(&mut rng, m, n);
        let x: Vec<u8> = (0..n).map(|_| rng.byte()).collect();
        let b = matvec(&a, &x);
        let p = solve(&a, &b).expect("consistent by construction");
        for v in nullspace(&a) {
            let shifted: Vec<u8> = p.iter().zip(v.iter()).map(|(&pi, &vi)| pi ^ vi).collect();
            assert_eq!(
                matvec(&a, &shifted),
                b,
                "p + kernel vector must still solve A·x=b"
            );
        }
    }
}
