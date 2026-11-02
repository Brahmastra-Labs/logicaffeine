use super::*;

fn bi(x: i64) -> BigInt {
    BigInt::from_i64(x)
}

#[test]
fn negacyclic_multiplication_reduces_x_to_the_n_as_minus_one() {
    let n = 8;
    // X · X^{n-1} = Xⁿ = −1.
    let x = Cyclo::monomial(n, 1, bi(1));
    let xnm1 = Cyclo::monomial(n, n - 1, bi(1));
    assert_eq!(x.mul(&xnm1), Cyclo::one(n).neg(), "Xⁿ ≡ −1 (negacyclic)");
    // X^{n+2} folds to −X².
    assert_eq!(
        Cyclo::monomial(n, n + 2, bi(1)),
        Cyclo::monomial(n, 2, bi(-1))
    );
    // One is the multiplicative identity; multiplication commutes and distributes.
    let a = Cyclo::from_ints(n, &[1, -2, 3, 0, -1, 5, 0, 2]);
    let b = Cyclo::from_ints(n, &[0, 1, -1, 4, 2, 0, -3, 1]);
    let c = Cyclo::from_ints(n, &[2, 0, 1, -1, 0, 3, 1, -2]);
    assert_eq!(a.mul(&Cyclo::one(n)), a, "1 is the identity");
    assert_eq!(a.mul(&b), b.mul(&a), "multiplication commutes");
    assert_eq!(
        a.mul(&b.add(&c)),
        a.mul(&b).add(&a.mul(&c)),
        "distributivity"
    );
    assert_eq!(a.mul(&b).mul(&c), a.mul(&b.mul(&c)), "associativity");
}

#[test]
fn every_galois_map_is_a_ring_automorphism() {
    let n = 8;
    let a = Cyclo::from_ints(n, &[1, -2, 3, 0, -1, 5, 0, 2]);
    let b = Cyclo::from_ints(n, &[0, 1, -1, 4, 2, 0, -3, 1]);
    for &t in &galois_group(n) {
        // σ_t preserves + and ·, so it is a ring homomorphism (and, being invertible, an automorphism).
        assert_eq!(
            a.add(&b).galois(t),
            a.galois(t).add(&b.galois(t)),
            "σ_{t} preserves addition"
        );
        assert_eq!(
            a.mul(&b).galois(t),
            a.galois(t).mul(&b.galois(t)),
            "σ_{t} preserves multiplication"
        );
        // σ_t fixes the rational constant ℤ ⊂ R.
        assert_eq!(Cyclo::one(n).galois(t), Cyclo::one(n), "σ_{t} fixes 1");
    }
}

#[test]
fn the_galois_group_is_z_mod_2n_star_of_order_n() {
    let n = 8; // 2n = 16, (ℤ/16)^× has order φ(16) = 8 = n
    let group = galois_group(n);
    assert_eq!(group.len(), n, "there are exactly φ(2n) = n automorphisms");

    // σ_1 = id, and the group law σ_s ∘ σ_t = σ_{st mod 2n} holds on a witness element.
    let a = Cyclo::from_ints(n, &[3, 1, -2, 0, 4, -1, 2, 5]);
    assert_eq!(a.galois(1), a, "σ_1 is the identity");
    let twon = (2 * n) as u64;
    for &s in &group {
        for &t in &group {
            assert_eq!(
                a.galois(t).galois(s),
                a.galois((s * t) % twon),
                "σ_s ∘ σ_t = σ_{{st}}"
            );
        }
    }

    // The automorphisms are pairwise distinct (their action on X already separates them).
    let x = Cyclo::monomial(n, 1, bi(1));
    let images: Vec<Cyclo> = group.iter().map(|&t| x.galois(t)).collect();
    for i in 0..images.len() {
        for j in (i + 1)..images.len() {
            assert_ne!(images[i], images[j], "distinct t ⟹ distinct automorphism");
        }
    }

    // {σ_{-1}, σ_3} generate the whole group — the closure of {2n−1, 3} under × mod 2n is all n residues.
    let mut closure = vec![1u64];
    let gens = [twon - 1, 3];
    loop {
        let mut grew = false;
        for &g in &gens {
            for k in 0..closure.len() {
                let p = (closure[k] * g) % twon;
                if !closure.contains(&p) {
                    closure.push(p);
                    grew = true;
                }
            }
        }
        if !grew {
            break;
        }
    }
    closure.sort_unstable();
    let mut all = group.clone();
    all.sort_unstable();
    assert_eq!(
        closure, all,
        "⟨σ_{{-1}}, σ_3⟩ generates the full Galois group"
    );
}

#[test]
fn conjugation_is_the_order_two_automorphism() {
    let n = 8;
    let a = Cyclo::from_ints(n, &[3, 1, -2, 0, 4, -1, 2, 5]);
    // σ_{-1} is an involution.
    assert_eq!(a.conjugate().conjugate(), a, "σ_{{-1}}² = id");
    // σ_{-1}(X) = X^{-1} = X^{2n-1} = −X^{n-1}.
    let x = Cyclo::monomial(n, 1, bi(1));
    assert_eq!(
        x.conjugate(),
        Cyclo::monomial(n, n - 1, bi(-1)),
        "σ_{{-1}}(X) = −X^{{n-1}}"
    );
    // Conjugation fixes the constant term and negates the "imaginary" part, as complex conjugation must.
    assert_eq!(
        a.conjugate().coeffs[0],
        a.coeffs[0],
        "the rational trace part is fixed"
    );
}

#[test]
fn norm_and_trace_are_rational_integers() {
    let n = 8;
    let a = Cyclo::from_ints(n, &[3, 1, -2, 0, 4, -1, 2, 5]);
    let b = Cyclo::from_ints(n, &[1, 0, 1, -1, 2, 1, 0, -2]);
    // The Galois product/sum land in ℤ ⊂ R — every higher coefficient vanishes.
    assert!(
        a.galois_product().coeffs[1..].iter().all(|c| c.is_zero()),
        "N(a) is a rational integer"
    );
    // Norm is multiplicative, trace is additive — the defining properties.
    assert_eq!(
        a.mul(&b).norm(),
        a.norm().mul(&b.norm()),
        "N(ab) = N(a)·N(b)"
    );
    assert_eq!(
        a.add(&b).trace(),
        a.trace().add(&b.trace()),
        "Tr(a+b) = Tr(a)+Tr(b)"
    );
    assert_eq!(Cyclo::one(n).norm(), bi(1), "N(1) = 1");
    assert_eq!(Cyclo::one(n).trace(), bi(n as i64), "Tr(1) = n");
}

#[test]
fn cyclotomic_units_have_norm_plus_minus_one() {
    let n = 8;
    // u = 1 + X + X² = (1 − X³)/(1 − X) is a cyclotomic unit: N(1−X³) = N(1−X) = 2, so N(u) = 1.
    let u = Cyclo::from_ints(n, &[1, 1, 1]);
    assert_eq!(u.norm(), bi(1), "the cyclotomic unit 1+X+X² has norm 1");
    assert!(u.is_unit(), "hence it is a unit of R");
    // Every Galois image of a unit is a unit (the automorphisms permute the units).
    for &t in &galois_group(n) {
        assert!(u.galois(t).is_unit(), "σ_t(u) is again a unit");
    }
    // 1 + X is NOT a unit: N(1+X) = ∏(1+ζ) = 2. Ramified prime above 2, not a unit.
    let non_unit = Cyclo::from_ints(n, &[1, 1]);
    assert_eq!(non_unit.norm(), bi(2), "N(1+X) = 2");
    assert!(
        !non_unit.is_unit(),
        "1+X is not a unit (it generates the ramified prime above 2)"
    );
}

#[test]
fn unit_inverse_is_an_exact_ring_inverse() {
    let n = 8;
    let u = Cyclo::from_ints(n, &[1, 1, 1]); // the cyclotomic unit 1+X+X²
    let inv = u.unit_inverse().expect("a unit has a ring inverse");
    assert_eq!(u.mul(&inv), Cyclo::one(n), "u · u⁻¹ = 1 exactly in R");
    // A non-unit has no ring inverse.
    assert!(
        Cyclo::from_ints(n, &[1, 1]).unit_inverse().is_none(),
        "1+X is not invertible in R"
    );
}

#[test]
fn cdpr_strips_the_unit_and_recovers_the_short_generator() {
    let n = 8;
    // The Soliloquy-style secret: a SHORT generator g of a principal ideal.
    let g = Cyclo::from_ints(n, &[1, -1, 0, 0, 0, 0, 0, 0]); // 1 − X
                                                             // The adversary publishes h = g·u, hiding g behind a large cyclotomic unit u.
    let units = cyclotomic_units(n);
    let u = units[0].mul(&units[1]); // b₃ · b₅ — a genuine, nontrivial unit
    assert!(u.is_unit(), "the mask is a unit");
    let h = g.mul(&u);
    assert!(
        h.coeff_norm() > g.coeff_norm(),
        "the public generator h is long (unit-masked)"
    );

    // Recover: strip the unit in the log-unit lattice, divide it out exactly.
    let rec = recover_short_generator(&h).expect("CDPR recovery succeeds");

    // The recovery lands on a generator as short as the planted secret — the unit is stripped. By
    // construction rec = h·u'⁻¹ with u' a unit, so (rec) = (h) = (g): it is a genuine short generator of
    // the secret ideal (differing from g by a unit — CDPR recovers *a* short generator, the break).
    assert_eq!(
        rec.coeff_norm(),
        g.coeff_norm(),
        "recovered a generator as short as the secret"
    );
    let recn = rec.norm();
    assert!(
        recn == bi(2) || recn == bi(-2),
        "|N(rec)| = |N(g)| — a genuine generator of the secret ideal"
    );
    assert!(
        h.coeff_norm() >= 2 * rec.coeff_norm(),
        "and far shorter than the unit-masked public h"
    );
    // Sanity: the recovered generator is not a unit — it generates the proper secret ideal, not all of R.
    assert!(
        !rec.is_unit(),
        "rec generates the secret ideal (norm 2), not R"
    );
}

#[test]
fn the_log_unit_geometry_audit_is_sane_and_the_scale_grows_with_n() {
    for n in [8usize, 16, 32] {
        let a = audit_log_unit(n);
        assert_eq!(
            a.rank,
            n / 2 - 1,
            "unit rank = n/2 − 1 (Dirichlet unit theorem)"
        );
        assert!(
            a.gs_lengths.iter().all(|&l| l > 1e-9),
            "the cyclotomic-unit log-basis is nondegenerate"
        );
        assert!(a.covering_radius_bound > 0.0, "a positive decoding scale");
    }
    // The decoding scale (the log-unit covering radius) grows with n — the geometry the wall lives in.
    let (m8, m16, m32) = (
        audit_log_unit(8).covering_radius_bound,
        audit_log_unit(16).covering_radius_bound,
        audit_log_unit(32).covering_radius_bound,
    );
    assert!(
        m8 < m16 && m16 < m32,
        "μ(Λ) grows with n: {m8} < {m16} < {m32}"
    );
}

#[test]
fn recovery_margin_locates_the_short_generator_wall() {
    let n = 8;
    // 1 (trivial generator of R) has Log(1) = 0 ⟹ margin 0.
    assert!(
        recovery_margin(&Cyclo::one(n)) < 1e-6,
        "Log(1) = 0 ⟹ margin 0"
    );

    // The short secret 1 − X we actually recovered sits INSIDE the decoding region.
    let g = Cyclo::from_ints(n, &[1, -1, 0, 0, 0, 0, 0, 0]);
    let m_g = recovery_margin(&g);
    assert!(
        m_g < 1.0,
        "the short principal-ideal generator is inside the wall (margin {m_g} < 1)"
    );

    // A cyclotomic unit's whole log lies in span(Λ), so it sits far out along exactly the directions the
    // recovery quotients away — its margin exceeds the short generator's.
    let u = cyclotomic_units(n)[1].clone();
    assert!(
        recovery_margin(&u) > m_g,
        "a unit sits farther out along the log-unit directions than g"
    );

    // The honest headline, as a number: short generators are INSIDE the wall (recoverable). So the
    // log-unit collapse is not Module-LWE's defense — that is upstream (no generator handed over, module
    // rank ≥ 2, the Ideal-SVP approximation gap). The auditor measures the rung and points past it.
}

#[test]
fn the_upstream_wall_the_approximation_scale_grows_with_n() {
    // The decoding factor the log-unit collapse achieves would have to stay POLYNOMIAL to threaten
    // Module-LWE; measured directly, it grows with the field dimension n — the real wall.
    let ns = [8usize, 16, 32, 64];
    let scales: Vec<f64> = ns.iter().map(|&n| approximation_scale(n)).collect();
    for (n, s) in ns.iter().zip(&scales) {
        eprintln!("approximation_scale(n={n}) = {s:.4}");
        assert!(
            s.is_finite() && *s > 0.0,
            "a finite positive decoding scale"
        );
    }
    // Over the measured range the scale clearly grows — the gap to a polynomial factor widens with n.
    assert!(
        scales[3] > scales[0],
        "the approximation scale grows with n: {scales:?}"
    );
}
