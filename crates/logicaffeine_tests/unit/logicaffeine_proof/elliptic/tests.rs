use super::*;

fn big(s: &str) -> BigInt {
    BigInt::parse_decimal(s).unwrap()
}

// Projective equality X1·Z2 ≡ X2·Z1 (mod n): two (X:Z) points are the same affine x.
fn proj_eq(a: &(BigInt, BigInt), b: &(BigInt, BigInt), n: &BigInt) -> bool {
    mulmod(&a.0, &b.1, n) == mulmod(&b.0, &a.1, n)
}

#[test]
fn ladder_is_a_consistent_scalar_multiplication() {
    // Over a prime field, the ladder must respect the group law: (a·b)·P = a·(b·P), and it must be
    // additive, k·P via the ladder equals stepping the differential-addition chain by hand.
    let n = big("1000003"); // prime
    let a24 = i(7);
    let p = (i(2), i(1));
    for (a, b) in [(7u64, 11u64), (13, 5), (100, 37), (1, 999), (255, 256)] {
        let lhs = ladder(a * b, &p.0, &p.1, &a24, &n);
        let rhs = ladder(
            a,
            &ladder(b, &p.0, &p.1, &a24, &n).0,
            &ladder(b, &p.0, &p.1, &a24, &n).1,
            &a24,
            &n,
        );
        assert!(proj_eq(&lhs, &rhs, &n), "(a·b)·P = a·(b·P) for a={a} b={b}");
    }
    // A hand-built addition chain: P, 2P, 3P=2P+P, 5P=3P+2P — must match the ladder.
    let p2 = xdbl(&p.0, &p.1, &a24, &n);
    let p3 = xadd(&p2.0, &p2.1, &p.0, &p.1, &p.0, &p.1, &n); // 2P + P, difference P
    let p5 = xadd(&p3.0, &p3.1, &p2.0, &p2.1, &p.0, &p.1, &n); // 3P + 2P, difference P
    assert!(proj_eq(&p2, &ladder(2, &p.0, &p.1, &a24, &n), &n));
    assert!(proj_eq(&p3, &ladder(3, &p.0, &p.1, &a24, &n), &n));
    assert!(proj_eq(&p5, &ladder(5, &p.0, &p.1, &a24, &n), &n));
}

#[test]
fn ecm_pulls_a_small_factor_from_a_semiprime() {
    // N = p·q with p ≈ 10⁶ (a factor ECM finds via a smooth curve order, where trial division would
    // grind through ~10⁶ candidates). The result must be a genuine nontrivial divisor.
    let p = big("1000003");
    let q = big("1000000000039");
    let n = p.mul(&q);
    let f = ecm_factor(&n, 50_000, 60, 12345).expect("ECM finds a factor");
    assert!(f != i(1) && f != n, "a nontrivial factor");
    assert!(
        n.div_rem(&f).unwrap().1.is_zero(),
        "and it actually divides N"
    );
    assert!(f == p || f == q, "recovering one of the true primes");
}

#[test]
fn ecm_pulls_a_factor_from_a_larger_modulus() {
    // A bigger modulus with a moderate factor — ECM's cost tracks the FACTOR size, not N.
    let p = big("100000007"); // ~27-bit prime
    let q = big("340282366920938463463374607431768211507"); // a large prime
    let n = p.mul(&q);
    let f = ecm_factor(&n, 100_000, 80, 999).expect("ECM finds the moderate factor");
    assert_eq!(f, p, "ECM pulls the smaller prime out of a huge modulus");
    assert!(n.div_rem(&f).unwrap().1.is_zero());
}

#[test]
fn ecm_declines_on_a_prime() {
    // A prime has no nontrivial factor; ECM must never fabricate one.
    let prime = big("1000000000039");
    assert_eq!(
        ecm_factor(&prime, 10_000, 40, 7),
        None,
        "no factor of a prime"
    );
}

#[test]
fn ecm_two_stage_and_driver_factor() {
    let p = big("100003"); // ~17-bit factor
    let q = big("1000000000039");
    let n = p.mul(&q);
    // Stage 1 + stage 2 (b2 ≫ b1) recovers the factor.
    let f = ecm_two_stage(&n, 500, 20_000, 40, 2024).expect("two-stage ECM finds the factor");
    assert!(
        f != i(1) && f != n && n.div_rem(&f).unwrap().1.is_zero(),
        "a real divisor"
    );
    assert!(f == p || f == q);
    // The escalating driver (stage 3) finds it with NO hand-tuned bounds.
    let g = ecm(&n, 60, 5).expect("the ECM driver factors by escalation");
    assert!(g != i(1) && g != n && n.div_rem(&g).unwrap().1.is_zero());
}

// A textbook curve y² = x³ + 2x + 2 over 𝔽₁₇, with the on-curve point (5,1).
fn curve17() -> (Curve, Point) {
    (Curve::new(i(2), i(2), i(17)), Point::Affine(i(5), i(1)))
}

#[test]
fn ecdlp_group_law_is_a_real_abelian_group() {
    let (c, p) = curve17();
    assert!(c.is_on_curve(&p));
    // Inverse: P + (−P) = O.
    assert_eq!(c.add(&p, &c.negate(&p)), Point::Infinity);
    // Doubling agrees with self-addition, and stays on the curve.
    let p2 = c.double(&p);
    assert_eq!(p2, c.add(&p, &p));
    assert!(c.is_on_curve(&p2));
    // Scalar mult = repeated addition.
    assert_eq!(c.mul(&i(3), &p), c.add(&c.add(&p, &p), &p));
    // Associativity across three multiples.
    let (q, r) = (c.mul(&i(2), &p), c.mul(&i(5), &p));
    assert_eq!(
        c.add(&c.add(&p, &q), &r),
        c.add(&p, &c.add(&q, &r)),
        "the group law is associative"
    );
    // Lagrange: the point order annihilates it.
    let ord = c.point_order(&p, 100).expect("a small order");
    assert_eq!(c.mul(&i(ord as i64), &p), Point::Infinity, "ord(P)·P = O");
}

#[test]
fn ecdlp_bsgs_recovers_the_discrete_log() {
    let (c, p) = curve17();
    let n = c.point_order(&p, 100).unwrap();
    for k in [3u64, 7, 11, 15] {
        let q = c.mul(&i(k as i64), &p);
        let recovered = ecdlp_bsgs(&c, &p, &q, n).expect("Q is in ⟨P⟩");
        assert_eq!(
            c.mul(&recovered, &p),
            q,
            "k·P = Q for the recovered k (k mod ord(P))"
        );
    }
}

#[test]
fn point_count_is_hasse_valid_and_annihilates_every_point() {
    let (c, p) = curve17();
    let order = c.count_points();
    // Hasse: |#E − (p+1)| ≤ 2√p. For p=17, #E ∈ [10, 26].
    assert!((10..=26).contains(&order), "#E = {order} within Hasse");
    // #E · P = O for any point (the group order annihilates).
    assert_eq!(c.mul(&i(order as i64), &p), Point::Infinity);
}

#[test]
fn curve_security_flags_exactly_the_weak_structures() {
    // Anomalous (#E = p): Smart's polynomial-time attack.
    assert_eq!(curve_security(23, 23), Some(CurveWeakness::Anomalous));
    // Supersingular (#E = p+1): MOV.
    assert_eq!(curve_security(23, 24), Some(CurveWeakness::Supersingular));
    // Smooth order 100 = 2²·5² (largest prime 5): Pohlig–Hellman.
    assert_eq!(
        curve_security(101, 100),
        Some(CurveWeakness::SmoothOrder {
            largest_prime_factor: 5
        })
    );
    // Prime order → only generic O(√n): no structural weakness.
    assert_eq!(
        curve_security(101, 103),
        None,
        "a prime-order curve resists"
    );

    // And it agrees with a REAL supersingular curve found by point counting.
    let mut found = false;
    'search: for pp in [11u64, 19, 23] {
        for a in 0..pp {
            for b in 0..pp {
                let c = Curve::new(i(a as i64), i(b as i64), i(pp as i64));
                // skip singular curves (discriminant 4a³+27b² ≡ 0)
                let disc = (4 * a * a * a + 27 * b * b) % pp;
                if disc == 0 {
                    continue;
                }
                if c.count_points() == pp + 1 {
                    assert_eq!(
                        curve_security(pp, pp + 1),
                        Some(CurveWeakness::Supersingular)
                    );
                    found = true;
                    break 'search;
                }
            }
        }
    }
    assert!(found, "supersingular curves exist and are detected");
}

// √a mod p for p ≡ 3 (mod 4): a^{(p+1)/4}, verified by squaring.
fn sqrt_mod(a: &BigInt, p: &BigInt) -> Option<BigInt> {
    let exp = p.add(&i(1)).div_rem(&i(4)).unwrap().0;
    let r = modpow(a, &exp, p);
    (mulmod(&r, &r, p) == rem_pos(a, p)).then_some(r)
}

fn rhs(c: &Curve, x: &BigInt) -> BigInt {
    let x3 = mulmod(&mulmod(x, x, &c.p), x, &c.p);
    addmod(&addmod(&x3, &mulmod(&c.a, x, &c.p), &c.p), &c.b, &c.p)
}

fn curve_points(c: &Curve) -> Vec<Point> {
    let pu = c.p.to_i64().unwrap() as u64;
    let mut v = vec![Point::Infinity];
    for xu in 0..pu {
        let x = i(xu as i64);
        let r = rhs(c, &x);
        if r.is_zero() {
            v.push(Point::Affine(x, i(0)));
        } else if let Some(y) = sqrt_mod(&r, &c.p) {
            v.push(Point::Affine(x.clone(), submod(&i(0), &y, &c.p)));
            v.push(Point::Affine(x, y));
        }
    }
    v
}

// Find a curve over 𝔽_p (p ≡ 3 mod 4) with a rational point of exact odd-prime order ℓ ∈ {3,5,7}.
fn find_ell_isogeny(pval: u64) -> Option<(Curve, Point, u64)> {
    let p = i(pval as i64);
    for a in 1..pval {
        for b in 1..pval {
            let c = Curve::new(i(a as i64), i(b as i64), p.clone());
            if c.j_invariant().is_none() {
                continue; // singular
            }
            let order = c.count_points();
            for ell in [3u64, 5, 7] {
                if order % ell != 0 {
                    continue;
                }
                let cof = order / ell;
                for pt in curve_points(&c) {
                    let q = c.mul(&i(cof as i64), &pt);
                    if q != Point::Infinity && c.mul(&i(ell as i64), &q) == Point::Infinity {
                        return Some((c, q, ell));
                    }
                }
            }
        }
    }
    None
}

#[test]
fn j_invariant_is_an_isomorphism_invariant() {
    let p = i(103);
    let c = Curve::new(i(2), i(3), p.clone());
    let j = c.j_invariant().unwrap();
    // The isomorphism (x,y) ↦ (u²x, u³y) sends (a,b) ↦ (u⁴a, u⁶b) and must preserve j.
    let u2 = mulmod(&i(5), &i(5), &p);
    let u4 = mulmod(&u2, &u2, &p);
    let u6 = mulmod(&u4, &u2, &p);
    let twist = Curve::new(mulmod(&u4, &i(2), &p), mulmod(&u6, &i(3), &p), p.clone());
    assert_eq!(
        twist.j_invariant().unwrap(),
        j,
        "j is invariant under isomorphism"
    );
    assert_ne!(
        Curve::new(i(1), i(1), p).j_invariant().unwrap(),
        j,
        "different curves, different j"
    );
}

// Find a curve over 𝔽_p (p ≡ 1 mod n, so μ_n ⊂ 𝔽_p) with FULL rational n-torsion E[n] ≅ (ℤ/n)²,
// returning a torsion basis (P, Q) of two independent order-n points.
fn find_full_torsion(primes: &[u64], n: u64) -> Option<(Curve, Point, Point)> {
    for &pval in primes {
        let p = i(pval as i64);
        for a in 1..pval {
            for b in 1..pval {
                let c = Curve::new(i(a as i64), i(b as i64), p.clone());
                if c.j_invariant().is_none() {
                    continue;
                }
                if c.count_points() % (n * n) != 0 {
                    continue;
                }
                let onp: Vec<Point> = curve_points(&c)
                    .into_iter()
                    .filter(|pt| {
                        *pt != Point::Infinity && c.mul(&i(n as i64), pt) == Point::Infinity
                    })
                    .collect();
                if (onp.len() as u64) < n * n - 1 {
                    continue; // not the full n-torsion
                }
                for pp in &onp {
                    let span: Vec<Point> = (0..n).map(|k| c.mul(&i(k as i64), pp)).collect();
                    if let Some(qq) = onp.iter().find(|q| !span.contains(q)) {
                        return Some((c.clone(), pp.clone(), qq.clone()));
                    }
                }
            }
        }
    }
    None
}

#[test]
fn weil_pairing_is_bilinear_alternating_and_nondegenerate() {
    let n = 3u64;
    let (c, pp, qq) =
        find_full_torsion(&[7u64, 13, 19, 31], n).expect("a curve with full n-torsion");
    let e = weil_pairing(&c, &pp, &qq, n).unwrap();
    // Non-degenerate: independent points give a PRIMITIVE nth root of unity.
    assert_ne!(e, i(1), "independent points ⟹ nontrivial pairing");
    assert_eq!(modpow(&e, &i(n as i64), &c.p), i(1), "e(P,Q) ∈ μ_n");
    // Alternating.
    assert_eq!(weil_pairing(&c, &pp, &pp, n).unwrap(), i(1), "e(P,P) = 1");
    // Antisymmetric: e(Q,P) = e(P,Q)⁻¹.
    assert_eq!(
        weil_pairing(&c, &qq, &pp, n).unwrap(),
        mod_inverse(&e, &c.p).unwrap()
    );
    // Bilinear: e(kP, Q) = e(P,Q)ᵏ.
    for k in 1..n {
        let ek = weil_pairing(&c, &c.mul(&i(k as i64), &pp), &qq, n).unwrap();
        assert_eq!(
            ek,
            modpow(&e, &i(k as i64), &c.p),
            "e({k}P, Q) = e(P,Q)^{k}"
        );
    }
}

#[test]
fn velu_isogeny_is_a_homomorphism_kills_its_kernel_and_preserves_order() {
    let (c, gen, ell) = find_ell_isogeny(103).expect("an ℓ-torsion instance exists");
    let iso = Isogeny::from_kernel(&c, &gen, ell).expect("Vélu builds the isogeny");
    assert_eq!(iso.degree, ell);
    let cod = &iso.codomain;

    // Tate's theorem: isogenous curves have the SAME number of points — a deep, exact oracle.
    assert_eq!(
        c.count_points(),
        cod.count_points(),
        "isogenous ⟹ equal order"
    );

    // φ annihilates the whole kernel ⟨gen⟩.
    let mut k = gen.clone();
    for _ in 0..ell {
        assert_eq!(iso.eval(&k), Point::Infinity, "φ kills its kernel");
        k = c.add(&k, &gen);
    }

    // φ is a genuine group homomorphism onto the codomain, over a sweep of points.
    let pts = curve_points(&c);
    for u in pts.iter().take(9) {
        for v in pts.iter().take(9) {
            let (fu, fv) = (iso.eval(u), iso.eval(v));
            assert!(cod.is_on_curve(&fu), "φ(P) lands on the codomain");
            assert_eq!(
                iso.eval(&c.add(u, v)),
                cod.add(&fu, &fv),
                "φ(P+Q) = φ(P)+φ(Q)"
            );
        }
    }
}

// The largest `a` for which the curve has a point of order exactly `ℓᵃ`, together with such a generator.
fn largest_prime_power_generator(curve: &Curve, ell: u64) -> Option<(Point, u32)> {
    (1u32..=4).rev().find_map(|a| {
        let n = ell.pow(a);
        all_affine_points(curve)
            .into_iter()
            .find(|pt| curve.point_order(pt, n + 1) == Some(n))
            .map(|g| (g, a))
    })
}

#[test]
fn torsion_image_generator_unfolds_into_the_secret_isogeny_path() {
    // Supersingular-shaped: p = 107 ≡ 3 (mod 4) (so √ works over 𝔽_p) and ≡ 2 (mod 3) (so x ↦ x³ is a
    // bijection ⟹ y² = x³ + 1 has #E = p+1 = 108 = 2²·3³), giving a deep 3-power isogeny chain.
    let p = big("107");
    let e = Curve::new(i(0), i(1), p.clone());
    assert_eq!(e.count_points(), 108, "#E = p + 1 (supersingular)");

    // The single reconstructed kernel generator — the datum the torsion images pin down.
    let (gen, a) = largest_prime_power_generator(&e, 3).expect("a 3-power torsion generator");
    assert!(
        a >= 2,
        "the 3-Sylow (order 27) forces a point of order ≥ 9 ⟹ a genuine multi-step chain"
    );

    // The whole secret path pops out of that one generator.
    let path = derive_isogeny_path(&e, &gen, 3, a).expect("a valid 3-isogeny chain");
    assert_eq!(
        path.len() as u32,
        a,
        "one ℓ-isogeny per unit of the exponent"
    );

    // Every step quotients a genuine order-3 subgroup, and the chain is connected domain → codomain.
    for (idx, step) in path.iter().enumerate() {
        assert_ne!(
            step.kernel,
            Point::Infinity,
            "step {idx} kernel is nontrivial"
        );
        assert_eq!(
            step.domain.mul(&i(3), &step.kernel),
            Point::Infinity,
            "step {idx} kernel has order 3"
        );
        assert!(
            step.domain.is_on_curve(&step.kernel),
            "the kernel point lies on the step's domain"
        );
        if idx > 0 {
            assert_eq!(
                step.domain,
                path[idx - 1].codomain,
                "the chain is connected"
            );
        }
    }

    // The composite is exactly the ℓᵃ-isogeny with kernel ⟨gen⟩: pushing gen through every step lands on
    // O, while a point outside ⟨gen⟩ survives (the composite is not the zero map).
    let outside = all_affine_points(&e)
        .into_iter()
        .find(|pt| (0..3u64.pow(a)).all(|k| e.mul(&i(k as i64), &gen) != *pt))
        .expect("a point outside ⟨gen⟩");
    let (mut in_img, mut out_img) = (gen.clone(), outside);
    for step in &path {
        let iso = Isogeny::from_kernel(&step.domain, &step.kernel, 3).unwrap();
        in_img = iso.eval(&in_img);
        out_img = iso.eval(&out_img);
    }
    assert_eq!(
        in_img,
        Point::Infinity,
        "gen generates the kernel of the whole composite"
    );
    assert_ne!(
        out_img,
        Point::Infinity,
        "a point outside ⟨gen⟩ survives the composite"
    );
}

#[test]
fn the_secret_scalar_selects_the_isogeny_through_a_torsion_basis() {
    // A genuine rank-2 basis E[3] = ⟨P, Q⟩ needs 3 | p−1 (the Weil pairing forces the cube roots of unity
    // into the field). Search 𝔽_p (p ≡ 3 mod 4 for √, p ≡ 1 mod 3 for full 3-torsion) for such a curve.
    let (c, p_pt, q_pt) = [19u64, 31, 43, 67, 79, 103]
        .iter()
        .find_map(|&pp| {
            let p = big(&pp.to_string());
            (0..pp).find_map(|a| {
                (1..pp).find_map(|b| {
                    // Skip singular curves (discriminant 4a³ + 27b² ≡ 0).
                    let disc = addmod(
                        &mulmod(
                            &i(4),
                            &mulmod(&mulmod(&i(a as i64), &i(a as i64), &p), &i(a as i64), &p),
                            &p,
                        ),
                        &mulmod(&i(27), &mulmod(&i(b as i64), &i(b as i64), &p), &p),
                        &p,
                    );
                    if disc.is_zero() {
                        return None;
                    }
                    let c = Curve::new(i(a as i64), i(b as i64), p.clone());
                    torsion_basis(&c, 3)
                        .filter(|(pt, qt)| {
                            c.point_order(pt, 4) == Some(3) && c.point_order(qt, 4) == Some(3)
                        })
                        .map(|(pt, qt)| (c.clone(), pt, qt))
                })
            })
        })
        .expect("a curve with a rank-2 3-torsion basis");

    // The secret s selects the kernel line ⟨P + [s]Q⟩; each of the four order-3 lines is a distinct,
    // valid 3-isogeny. This is the torsion-guided walk: one secret, one directed path.
    let mut lines = std::collections::HashSet::new();
    for s in 0..3u64 {
        let gen = kernel_generator(&c, &p_pt, &q_pt, &i(s as i64));
        assert!(c.is_on_curve(&gen), "P + [s]Q lies on the curve");
        assert_eq!(
            c.point_order(&gen, 4),
            Some(3),
            "P + [s]Q is a genuine order-3 kernel generator"
        );
        let path = derive_isogeny_path(&c, &gen, 3, 1).expect("secret s selects a valid 3-isogeny");
        assert_eq!(path.len(), 1, "a single secret step is one ℓ-isogeny");
        // Record the kernel line ⟨gen⟩ as a canonical set of its points.
        let mut line: Vec<Vec<u8>> = (0..3).map(|k| point_key(&c.mul(&i(k), &gen))).collect();
        line.sort();
        lines.insert(line);
    }
    assert!(
        lines.len() >= 2,
        "distinct secrets select distinct kernel lines ⟹ distinct isogenies"
    );
}
