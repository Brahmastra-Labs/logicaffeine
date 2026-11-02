use super::*;

// A tiny seeded SplitMix64 — reproducible, no wall-clock.
fn splitmix(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
    let mut z = *state;
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

fn count_invertible_modp_bruteforce(n: usize, p: u64) -> u128 {
    let cells = (n * n) as u32;
    let total = (p as u128).pow(cells);
    let mut count = 0u128;
    for idx in 0..total {
        let mut m = vec![vec![0u64; n]; n];
        let mut x = idx;
        for row in m.iter_mut() {
            for cell in row.iter_mut() {
                *cell = (x % p as u128) as u64;
                x /= p as u128;
            }
        }
        if is_invertible_modp(n, p, &m) {
            count += 1;
        }
    }
    count
}

/// **The symmetry break across the field: the invertible count is `|GL(n,p)|` over every `GF(p)`.**
/// Brute force over all `p^{n²}` matrices equals the orbit–stabilizer product `Π(pⁿ − pⁱ)` — the
/// invertible matrices are exactly the ordered bases, the simply-transitive orbit of `GL(n,p)`. Proven
/// exhaustively over several prime fields, with pinned group orders.
#[test]
fn gl_order_p_is_the_invertible_count_over_gf_p() {
    for &(n, p) in &[
        (1usize, 2u64),
        (1, 3),
        (2, 2),
        (2, 3),
        (2, 5),
        (2, 7),
        (3, 2),
    ] {
        assert_eq!(
            count_invertible_modp_bruteforce(n, p),
            gl_order_p(n as u32, p),
            "brute invertible count over GF({p}) must equal |GL({n},{p})| = Π(pⁿ−pⁱ)"
        );
    }
    assert_eq!(gl_order_p(2, 2), 6, "|GL(2,2)| = 6 ≅ S₃");
    assert_eq!(gl_order_p(2, 3), 48, "|GL(2,3)| = 48");
    assert_eq!(gl_order_p(2, 5), 480, "|GL(2,5)| = 480");
    assert_eq!(gl_order_p(3, 2), 168, "|GL(3,2)| = 168");
}

/// **The field size is a symmetry axis, and `GF(2)` is its densest-collision end.** The density
/// `Π(1−p⁻ʲ) = |GL(n,p)|/p^{n²}` is exact, increases strictly with `p` (bigger field ⟹ fewer linear
/// collisions ⟹ more likely invertible), and → 1 as `p → ∞`. The `p = 2` value is `φ(½)`, the smallest
/// — and it agrees with the dedicated GF(2) module (cross-check).
#[test]
fn the_field_size_is_a_symmetry_axis() {
    // Exact |GL|/p^(n²) == density — small n only, before the u128 product (≈ p^(n²)) overflows.
    for &p in &[2u64, 3, 5, 7] {
        for n in 1..=4u32 {
            let exact = gl_order_p(n, p) as f64 / (p as f64).powi((n * n) as i32);
            assert!(
                (invertibility_density_p(n, p) - exact).abs() < 1e-12,
                "density == |GL|/p^(n²) at n={n},p={p}"
            );
        }
    }
    // strictly increasing in p (denser field ⟹ closer to always-invertible), at fixed n
    let n = 6u32;
    let dens: Vec<f64> = [2u64, 3, 5, 7, 11]
        .iter()
        .map(|&p| invertibility_density_p(n, p))
        .collect();
    for w in dens.windows(2) {
        assert!(
            w[1] > w[0],
            "density increases with the field size: {dens:?}"
        );
    }
    // GF(2) is the densest-collision regime — the smallest constant, exactly φ(½)
    assert!(
        (invertibility_density_p(40, 2) - 0.288_788_095_1).abs() < 1e-9,
        "GF(2) density → φ(½)"
    );
    // cross-module agreement: the p=2 specialization is the gf2 module's own constant
    for n in 1..=10u32 {
        assert!(
            (invertibility_density_p(n, 2) - crate::gf2::invertibility_density(n)).abs() < 1e-12,
            "modp p=2 == gf2 at n={n}"
        );
        assert_eq!(
            gl_order_p(n, 2),
            crate::gf2::gl_order(n),
            "|GL(n,2)| agrees across modules at n={n}"
        );
    }
}

fn brute_force_sat(equations: &[ModpEquation], num_vars: usize, p: u64) -> bool {
    let total = (p as u128).pow(num_vars as u32);
    for code in 0..total {
        let mut a = vec![0u64; num_vars];
        let mut c = code;
        for slot in a.iter_mut() {
            *slot = (c % p as u128) as u64;
            c /= p as u128;
        }
        if satisfies(equations, &a, p) {
            return true;
        }
    }
    false
}

/// The certified mod-`p` cut, verified to the point of absurdity against brute force: over `GF(3)`
/// and `GF(5)`, on a fuzz of random systems, `solve`'s verdict always matches exhaustive search —
/// every `Sat` witness satisfies, every `Unsat` refutation independently re-checks.
#[test]
fn modp_gaussian_matches_brute_force() {
    for &p in &[2u64, 3, 5, 7] {
        let mut state = 0x1234_5678u64 ^ p;
        for _ in 0..40 {
            let num_vars = 2 + (splitmix(&mut state) % 3) as usize; // 2..4 variables
            let num_eqs = 1 + (splitmix(&mut state) % 5) as usize; // 1..5 equations
            let equations: Vec<ModpEquation> = (0..num_eqs)
                .map(|_| {
                    let coeffs: Vec<(usize, u64)> = (0..num_vars)
                        .map(|v| (v, splitmix(&mut state) % p))
                        .filter(|&(_, a)| a != 0)
                        .collect();
                    ModpEquation::new(coeffs, splitmix(&mut state) % p)
                })
                .collect();
            let brute = brute_force_sat(&equations, num_vars, p);
            match solve(&equations, num_vars, p) {
                ModpOutcome::Sat(a) => {
                    assert!(
                        brute,
                        "p={p}: solver Sat but brute force UNSAT: {equations:?}"
                    );
                    assert!(
                        satisfies(&equations, &a, p),
                        "p={p}: the model must satisfy: {a:?}"
                    );
                }
                ModpOutcome::Unsat(combo) => {
                    assert!(
                        !brute,
                        "p={p}: solver Unsat but a model exists: {equations:?}"
                    );
                    assert!(
                        is_refutation(&equations, num_vars, p, &combo),
                        "p={p}: the refutation must re-check: {combo:?}"
                    );
                }
            }
        }
    }
}

/// A concrete mod-3 inconsistency the GF(2) parity cut is **blind** to. The system `x+y+z ≡ 0` and
/// `x+y+z ≡ 2 (mod 3)` is inconsistent over `GF(3)` (subtract: `0 ≡ 2`). But reduce the right-hand
/// sides mod 2 and *both* become `x+y+z ≡ 0` — the very same GF(2) equation, perfectly consistent.
/// So a parity (GF(2)) solver sees no conflict; only the mod-3 cut refutes it, with a re-checkable
/// certificate. The new field genuinely reaches a class the old one cannot.
#[test]
fn mod3_inconsistency_is_invisible_to_gf2() {
    let p = 3;
    let eqs = vec![
        ModpEquation::new(vec![(0, 1), (1, 1), (2, 1)], 0),
        ModpEquation::new(vec![(0, 1), (1, 1), (2, 1)], 2),
    ];
    match solve(&eqs, 3, p) {
        ModpOutcome::Unsat(combo) => {
            assert!(
                is_refutation(&eqs, 3, p, &combo),
                "the mod-3 refutation re-checks: {combo:?}"
            );
        }
        other => panic!("expected the mod-3 system to be refuted, got {other:?}"),
    }
    // Over GF(2) both right-hand sides collapse to 0 — the same equation twice, satisfiable — so the
    // parity cut sees no conflict where the mod-3 cut crushes.
    let gf2_rhs: Vec<u64> = eqs.iter().map(|e| e.rhs % 2).collect();
    assert_eq!(
        gf2_rhs,
        vec![0, 0],
        "the GF(2) reduction has no conflict — parity is blind here"
    );
}

/// **The scalable mod-p crush — and the cross-field punch.** The cycle obstruction is refuted in
/// polynomial time exactly when `n` is not a multiple of `p`, with the all-ones combination as the
/// re-checkable witness — at every length. And a 4-cycle is satisfiable over `GF(2)` (the parity
/// cut sees nothing) yet refuted over `GF(3)`: the new field reaches a class the old one cannot.
#[test]
fn the_mod_p_cycle_obstruction_crushed_at_scale() {
    for &p in &[3u64, 5, 7] {
        for n in 2..=40 {
            let eqs = cycle_system(n, p);
            match solve(&eqs, n, p) {
                ModpOutcome::Unsat(combo) => {
                    assert_ne!(
                        n as u64 % p,
                        0,
                        "p={p} n={n}: refuted ⟹ n not a multiple of p"
                    );
                    assert!(
                        is_refutation(&eqs, n, p, &combo),
                        "p={p} n={n}: cycle refutation re-checks"
                    );
                }
                ModpOutcome::Sat(a) => {
                    assert_eq!(
                        n as u64 % p,
                        0,
                        "p={p} n={n}: satisfiable ⟹ n is a multiple of p"
                    );
                    assert!(
                        satisfies(&eqs, &a, p),
                        "p={p} n={n}: the model must satisfy"
                    );
                }
            }
        }
    }
    // The cross-field punch: a 4-cycle is SAT over GF(2) but UNSAT over GF(3).
    assert!(
        matches!(solve(&cycle_system(4, 2), 4, 2), ModpOutcome::Sat(_)),
        "4-cycle SAT over GF(2)"
    );
    assert!(
        matches!(solve(&cycle_system(4, 3), 4, 3), ModpOutcome::Unsat(_)),
        "4-cycle UNSAT over GF(3)"
    );
}

/// **`modp` over GF(2) *is* `xorsat`** — proven by a differential fuzz, not asserted. On 50 random
/// GF(2) systems the mod-2 Gaussian and the dedicated parity engine agree on every verdict, so the
/// new field is a faithful generalization of the old one (which is itself brute-force-verified).
#[test]
fn modp_over_gf2_agrees_with_xorsat() {
    use crate::xorsat::{self, XorEquation, XorOutcome};
    let mut state = 0x00AB_CDEFu64;
    for _ in 0..50 {
        let num_vars = 2 + (splitmix(&mut state) % 4) as usize;
        let num_eqs = 1 + (splitmix(&mut state) % 5) as usize;
        let systems: Vec<(Vec<usize>, bool)> = (0..num_eqs)
            .map(|_| {
                let vars: Vec<usize> = (0..num_vars)
                    .filter(|_| splitmix(&mut state) % 2 == 0)
                    .collect();
                (vars, splitmix(&mut state) % 2 == 1)
            })
            .collect();
        let xor_eqs: Vec<XorEquation> = systems
            .iter()
            .map(|(v, r)| XorEquation::new(v.clone(), *r))
            .collect();
        let modp_eqs: Vec<ModpEquation> = systems
            .iter()
            .map(|(v, r)| {
                ModpEquation::new(v.iter().map(|&x| (x, 1u64)).collect::<Vec<_>>(), *r as u64)
            })
            .collect();
        let xor_unsat = matches!(xorsat::solve(&xor_eqs, num_vars), XorOutcome::Unsat(_));
        let modp_unsat = matches!(solve(&modp_eqs, num_vars, 2), ModpOutcome::Unsat(_));
        assert_eq!(
            xor_unsat, modp_unsat,
            "modp(p=2) must match xorsat on {systems:?}"
        );
    }
}

/// The mod-`p` cut decides a satisfiable system and returns a real model.
#[test]
fn modp_solves_a_consistent_system() {
    // Over GF(5): x + 2y ≡ 3, 3y ≡ 4  ⟹  y ≡ 3 (3⁻¹=2, 2·4=8≡3), x ≡ 3 − 2·3 = −3 ≡ 2.
    let eqs = vec![
        ModpEquation::new(vec![(0, 1), (1, 2)], 3),
        ModpEquation::new(vec![(1, 3)], 4),
    ];
    match solve(&eqs, 2, 5) {
        ModpOutcome::Sat(a) => {
            assert!(satisfies(&eqs, &a, 5), "model must satisfy: {a:?}");
            assert_eq!(a, vec![2, 3], "the unique solution over GF(5)");
        }
        other => panic!("expected Sat, got {other:?}"),
    }
}

/// **The GF(p) lift is faithful at every prime.** For `p ∈ {3,5,7}` and several graph sizes/seeds,
/// `recover_from_cnf` reconstructs the system from the opaque one-hot CNF, and its verdict matches
/// both the hand-built supplied system *and* the family's declared UNSAT/SAT — on the inconsistent
/// (Tseitin) and the consistent forms alike. The recovery variables align with the edges, so the
/// recovered system is the same dimension as the supplied one. This is the equisatisfiability the
/// soundness of the dispatcher route rests on, proven across the field.
#[test]
fn recover_from_cnf_is_faithful_across_primes() {
    use crate::families::{mod_p_consistent_onehot, mod_p_tseitin_expander, ExpectedVerdict};
    for &p in &[3u64, 5, 7] {
        for &n in &[4usize, 6, 8] {
            for seed in 0..3u64 {
                for (supplied, cnf, expect) in [
                    mod_p_tseitin_expander(n, p, seed),
                    mod_p_consistent_onehot(n, p, seed),
                ] {
                    let rec = recover_from_cnf(cnf.num_vars, &cnf.clauses).unwrap_or_else(|| {
                        panic!("p={p} n={n} seed={seed}: must recover the GF(p) system")
                    });
                    assert_eq!(
                        rec.modulus, p,
                        "p={p} n={n} seed={seed}: recovered the right field"
                    );
                    let rec_unsat = matches!(
                        solve(&rec.equations, rec.num_vars, p),
                        ModpOutcome::Unsat(_)
                    );
                    let sup_unsat =
                        matches!(solve(&supplied, rec.num_vars, p), ModpOutcome::Unsat(_));
                    assert_eq!(
                        rec_unsat, sup_unsat,
                        "p={p} n={n} seed={seed}: recovered vs supplied"
                    );
                    assert_eq!(
                        rec_unsat,
                        matches!(expect, ExpectedVerdict::Unsat),
                        "p={p} n={n} seed={seed}: verdict matches the family's expectation"
                    );
                }
            }
        }
    }
}

/// **Soundness of declining.** The recoverer must guess nothing: it returns `None` on inputs that are
/// not a clean one-hot mod-`p` encoding — random 3-SAT (no groups), and an all-positive clause whose
/// pairwise at-most-one structure is incomplete (so "exactly one" is not actually enforced).
#[test]
fn recover_declines_on_inputs_that_are_not_a_one_hot_encoding() {
    use crate::cdcl::Lit;
    let rnd = crate::families::random_3sat(20, 80, 0xF00D);
    assert!(
        recover_from_cnf(rnd.num_vars, &rnd.clauses).is_none(),
        "random 3-SAT has no GF(p) structure"
    );
    let incomplete = vec![
        vec![Lit::pos(0), Lit::pos(1), Lit::pos(2)],
        vec![Lit::neg(0), Lit::neg(1)], // only one of the three at-most-one pairs
    ];
    assert!(
        recover_from_cnf(3, &incomplete).is_none(),
        "an incomplete at-most-one must not be mistaken for a one-hot group"
    );
}
