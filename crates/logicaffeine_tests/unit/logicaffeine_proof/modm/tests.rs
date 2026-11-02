use super::*;

fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn brute_force_sat(equations: &[ModpEquation], num_vars: usize, m: u64) -> bool {
    let total = (m as u128).pow(num_vars as u32);
    for code in 0..total {
        let mut a = vec![0u64; num_vars];
        let mut c = code;
        for slot in a.iter_mut() {
            *slot = (c % m as u128) as u64;
            c /= m as u128;
        }
        if satisfies(equations, &a, m) {
            return true;
        }
    }
    false
}

/// **Squarefree factorization is exact, and rejects non-squarefree moduli.** The CRT decision
/// procedure is complete precisely on squarefree `m`, so the gate must be precise.
#[test]
fn squarefree_primes_is_exact() {
    assert_eq!(squarefree_primes(6), Some(vec![2, 3]));
    assert_eq!(squarefree_primes(30), Some(vec![2, 3, 5]));
    assert_eq!(squarefree_primes(15), Some(vec![3, 5]));
    assert_eq!(squarefree_primes(7), Some(vec![7]));
    assert_eq!(squarefree_primes(105), Some(vec![3, 5, 7])); // 3·5·7
    assert_eq!(squarefree_primes(1), None);
    assert_eq!(squarefree_primes(4), None); // 2²
    assert_eq!(squarefree_primes(12), None); // 2²·3
    assert_eq!(squarefree_primes(9), None); // 3²
    assert_eq!(squarefree_primes(60), None); // 2²·3·5
}

/// **The composite cut, verified to the point of absurdity against brute force.** Over squarefree
/// moduli `m ∈ {6, 10, 15, 30}`, on a fuzz of random systems, `solve_squarefree`'s verdict always
/// matches exhaustive search over all `m^vars` assignments — every `Sat` recombination re-checks
/// mod `m`, every `Unsat` re-checks as a GF(prime) refutation at its witnessing prime.
#[test]
fn solve_squarefree_matches_brute_force_over_composites() {
    for &m in &[6u64, 10, 15, 30] {
        let mut state = 0xC0DE_1234u64 ^ m;
        for _ in 0..40 {
            let num_vars = 2 + (splitmix(&mut state) % 2) as usize; // 2..3 vars
            let num_eqs = 1 + (splitmix(&mut state) % 4) as usize; // 1..4 eqs
            let equations: Vec<ModpEquation> = (0..num_eqs)
                .map(|_| {
                    let coeffs: Vec<(usize, u64)> = (0..num_vars)
                        .map(|v| (v, splitmix(&mut state) % m))
                        .filter(|&(_, a)| a != 0)
                        .collect();
                    ModpEquation::new(coeffs, splitmix(&mut state) % m)
                })
                .collect();
            let brute = brute_force_sat(&equations, num_vars, m);
            match solve_squarefree(&equations, num_vars, m).expect("m is squarefree") {
                ModmOutcome::Sat(a) => {
                    assert!(brute, "m={m}: Sat but brute force UNSAT: {equations:?}");
                    assert!(
                        satisfies(&equations, &a, m),
                        "m={m}: the model must satisfy mod m: {a:?}"
                    );
                    assert!(a.iter().all(|&v| v < m), "m={m}: residues lie in 0..m");
                }
                ModmOutcome::Unsat { modulus, combo } => {
                    assert!(!brute, "m={m}: Unsat but a model exists: {equations:?}");
                    assert!(
                        is_refutation(&equations, num_vars, modulus, &combo),
                        "m={m}: the witness must re-check over ℤ/{modulus}: {combo:?}"
                    );
                    assert_eq!(
                        m % modulus,
                        0,
                        "m={m}: the witnessing modulus must divide m"
                    );
                }
            }
        }
    }
}

/// **The composite obstruction is found via a prime field GF(2) is blind to.** The mod-6 4-cycle is
/// inconsistent (4 is not a multiple of 6). By CRT this is invisible to the GF(2) component (4 ≡ 0
/// mod 2 ⟹ that field is consistent) and is caught only in the GF(3) component (4 ≡ 1 mod 3 ⟹
/// `0 ≡ 1`). The witness is the GF(3) refutation, and it re-checks. The multiplicative split reaches
/// a composite obstruction neither prime field reaches alone.
#[test]
fn the_mod_6_cycle_obstruction_is_caught_through_the_gf3_factor() {
    let eqs = cycle_system(4, 6);
    match solve_squarefree(&eqs, 4, 6).expect("6 is squarefree") {
        ModmOutcome::Unsat { modulus, combo } => {
            assert_eq!(
                modulus, 3,
                "the mod-6 obstruction lives in the GF(3) factor"
            );
            assert!(
                is_refutation(&eqs, 4, modulus, &combo),
                "the GF(3) refutation re-checks"
            );
        }
        other => panic!("the mod-6 4-cycle must be UNSAT, got {other:?}"),
    }
    // The GF(2) factor on its own is perfectly consistent — the parity cut sees nothing here.
    assert!(
        matches!(modp::solve(&eqs, 4, 2), ModpOutcome::Sat(_)),
        "GF(2) factor is consistent"
    );
    // And a 6-cycle (n = 6, a multiple of 6) is satisfiable mod 6, recombined across both fields.
    let eqs6 = cycle_system(6, 6);
    match solve_squarefree(&eqs6, 6, 6).expect("6 is squarefree") {
        ModmOutcome::Sat(a) => {
            assert!(satisfies(&eqs6, &a, 6), "the 6-cycle model satisfies mod 6")
        }
        other => panic!("the mod-6 6-cycle must be SAT, got {other:?}"),
    }
}

/// The CRT recombination is genuine: a system consistent over each prime factor but with *different*
/// residues per factor is solved by stitching them into one value mod m. `x ≡ 1 (mod 2)` and
/// `x ≡ 2 (mod 3)` ⟹ `x ≡ 5 (mod 6)`.
#[test]
fn crt_recombines_distinct_residues_across_factors() {
    // Single variable pinned to 5 mod 6 (1 mod 2, 2 mod 3).
    let eqs = vec![ModpEquation::new(vec![(0, 1)], 5)];
    match solve_squarefree(&eqs, 1, 6).expect("6 is squarefree") {
        ModmOutcome::Sat(a) => {
            assert_eq!(a, vec![5], "CRT(1 mod 2, 2 mod 3) = 5 mod 6");
            assert!(satisfies(&eqs, &a, 6));
        }
        other => panic!("expected Sat, got {other:?}"),
    }
    // Direct CRT check on the primitive.
    assert_eq!(crt(&[(1, 2), (2, 3)]), 5);
    assert_eq!(crt(&[(2, 3), (4, 5)]), 14); // 14 ≡ 2 mod 3, ≡ 4 mod 5
    assert_eq!(crt(&[(0, 2), (0, 3), (0, 5)]), 0);
    // CRT over PRIME POWERS (coprime), the new closure.
    assert_eq!(crt(&[(3, 4), (2, 9)]), 11); // 11 ≡ 3 mod 4, ≡ 2 mod 9
}

/// **The residue-ring solver, verified to the point of absurdity against brute force.** Over genuine
/// rings `ℤ/pᵏ` (`k ≥ 2`, where `p` is a zero divisor and field Gaussian does not apply), on a fuzz
/// of random systems the integer-Smith solver always matches exhaustive search: every `Sat` model
/// satisfies mod `pᵏ`, every `Unsat` witness re-checks as a refutation over `ℤ/pᵏ`.
#[test]
fn solve_prime_power_matches_brute_force_over_residue_rings() {
    for &(p, k) in &[(2u64, 2u32), (2, 3), (2, 4), (3, 2), (3, 3), (5, 2)] {
        let q = p.pow(k);
        let mut state = 0xBEEF_0001u64 ^ q;
        for _ in 0..40 {
            let num_vars = 2 + (splitmix(&mut state) % 2) as usize;
            let num_eqs = 1 + (splitmix(&mut state) % 4) as usize;
            let equations: Vec<ModpEquation> = (0..num_eqs)
                .map(|_| {
                    let coeffs: Vec<(usize, u64)> = (0..num_vars)
                        .map(|v| (v, splitmix(&mut state) % q))
                        .filter(|&(_, a)| a != 0)
                        .collect();
                    ModpEquation::new(coeffs, splitmix(&mut state) % q)
                })
                .collect();
            let brute = brute_force_sat(&equations, num_vars, q);
            match solve_prime_power(&equations, num_vars, p, k).expect("within the growth cap") {
                ModmOutcome::Sat(a) => {
                    assert!(brute, "q={q}: Sat but brute force UNSAT: {equations:?}");
                    assert!(
                        satisfies(&equations, &a, q),
                        "q={q}: the ring model must satisfy: {a:?}"
                    );
                    assert!(a.iter().all(|&val| val < q), "q={q}: residues lie in 0..q");
                }
                ModmOutcome::Unsat { modulus, combo } => {
                    assert!(!brute, "q={q}: Unsat but a model exists: {equations:?}");
                    assert_eq!(modulus, q, "q={q}: the witness modulus is the prime power");
                    assert!(
                        is_refutation(&equations, num_vars, modulus, &combo),
                        "q={q}: the ring refutation must re-check: {combo:?}"
                    );
                }
            }
        }
    }
}

/// **The full closure over every composite.** Squarefree, prime-power, and mixed moduli alike — the
/// general `solve` (CRT over the prime-power components) matches brute force on a fuzz of systems.
#[test]
fn solve_matches_brute_force_over_all_composites() {
    for &m in &[4u64, 8, 9, 12, 18, 24, 36] {
        let mut state = 0xABCD_0002u64 ^ m;
        for _ in 0..30 {
            let num_vars = 2 + (splitmix(&mut state) % 2) as usize;
            let num_eqs = 1 + (splitmix(&mut state) % 4) as usize;
            let equations: Vec<ModpEquation> = (0..num_eqs)
                .map(|_| {
                    let coeffs: Vec<(usize, u64)> = (0..num_vars)
                        .map(|v| (v, splitmix(&mut state) % m))
                        .filter(|&(_, a)| a != 0)
                        .collect();
                    ModpEquation::new(coeffs, splitmix(&mut state) % m)
                })
                .collect();
            let brute = brute_force_sat(&equations, num_vars, m);
            match solve(&equations, num_vars, m).expect("m ≥ 2 and within the cap") {
                ModmOutcome::Sat(a) => {
                    assert!(brute, "m={m}: Sat but brute force UNSAT: {equations:?}");
                    assert!(
                        satisfies(&equations, &a, m),
                        "m={m}: the model must satisfy mod m: {a:?}"
                    );
                }
                ModmOutcome::Unsat { modulus, combo } => {
                    assert!(!brute, "m={m}: Unsat but a model exists: {equations:?}");
                    assert_eq!(
                        m % modulus,
                        0,
                        "m={m}: the witnessing modulus must divide m"
                    );
                    assert!(
                        is_refutation(&equations, num_vars, modulus, &combo),
                        "m={m}: the witness must re-check over ℤ/{modulus}: {combo:?}"
                    );
                }
            }
        }
    }
}

/// **The prime-power obstruction needs the RING, not the field.** `2x ≡ 1 (mod 4)` is unsatisfiable —
/// the left side is always even — yet this is a `ℤ/4` fact: the witness `2·(2x − 1) = 4x − 2 ≡ −2 ≢ 0
/// (mod 4)` annihilates the coefficient while leaving a nonzero residue, the residue-ring refutation.
/// `2x ≡ 2 (mod 4)` is satisfiable (`x = 1`). Field reasoning over GF(2) cannot tell these apart
/// (both reduce to `0 ≡ ·`); the local ring decides.
#[test]
fn the_mod_4_obstruction_needs_the_ring_not_the_field() {
    let unsat = vec![ModpEquation::new(vec![(0, 2)], 1)];
    match solve(&unsat, 1, 4).unwrap() {
        ModmOutcome::Unsat { modulus, combo } => {
            assert_eq!(modulus, 4);
            assert!(
                is_refutation(&unsat, 1, 4, &combo),
                "the ℤ/4 refutation re-checks: {combo:?}"
            );
        }
        other => panic!("2x ≡ 1 (mod 4) is UNSAT, got {other:?}"),
    }
    let sat = vec![ModpEquation::new(vec![(0, 2)], 2)];
    match solve(&sat, 1, 4).unwrap() {
        ModmOutcome::Sat(a) => assert!(satisfies(&sat, &a, 4), "2x ≡ 2 (mod 4) has a model"),
        other => panic!("2x ≡ 2 (mod 4) is SAT, got {other:?}"),
    }
    assert_eq!(prime_power_factorize(36), Some(vec![(2, 2), (3, 2)]));
    assert_eq!(prime_power_factorize(8), Some(vec![(2, 3)]));
    assert_eq!(
        prime_power_factorize(30),
        Some(vec![(2, 1), (3, 1), (5, 1)])
    );
}
