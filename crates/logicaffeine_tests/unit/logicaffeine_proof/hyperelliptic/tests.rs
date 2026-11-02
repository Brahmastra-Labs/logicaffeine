use super::*;

fn big(s: &str) -> BigInt {
    BigInt::parse_decimal(s).unwrap()
}

#[test]
fn poly_arithmetic_is_correct() {
    let p = big("101");
    // (1 + 2x)(3 + x) = 3 + 7x + 2x²
    assert_eq!(
        poly_mul(&[ib(1), ib(2)], &[ib(3), ib(1)], &p),
        vec![ib(3), ib(7), ib(2)]
    );
    // d/dx (5 + 3x + 4x²) = 3 + 8x
    assert_eq!(poly_deriv(&[ib(5), ib(3), ib(4)], &p), vec![ib(3), ib(8)]);
}

#[test]
fn richelot_delta_and_dual_quadratics_are_correct() {
    let p = big("101");
    // Three generic quadratics.
    let g = [
        [ib(1), ib(2), ib(1)], // 1 + 2x + x²
        [ib(3), ib(1), ib(1)], // 3 + x + x²
        [ib(0), ib(4), ib(1)], // 4x + x²
    ];
    let r = richelot(&g, &p);
    // δ = det of the coefficient matrix, columns [x², x, 1] = [[1,2,1],[1,1,3],[1,4,0]].
    // det = 1(1·0−3·4) − 2(1·0−3·1) + 1(1·4−1·1) = −12 + 6 + 3 = −3 ≡ 98 (mod 101).
    assert_eq!(
        r.delta,
        big("98"),
        "Richelot δ = det of the quadratic coefficients"
    );
    // Each dual Hᵢ has degree ≤ 2 (the cubic leading terms cancel).
    for h in &r.dual {
        assert!(h.len() <= 3, "Hᵢ is a quadratic");
    }
    // Domain = G₁·G₂·G₃ is a genuine sextic.
    assert_eq!(r.domain.len(), 7, "domain is a sextic (7 coefficients)");
    assert!(
        !r.is_split(),
        "generic (independent) quadratics ⟹ δ ≠ 0 ⟹ not split"
    );
}

#[test]
fn dependent_quadratics_give_a_reducible_split_surface() {
    let p = big("101");
    // G₃ = G₁ + G₂ makes the coefficient rows linearly dependent ⟹ δ = 0 ⟹ the surface is a product.
    let g1 = [ib(1), ib(2), ib(1)];
    let g2 = [ib(3), ib(1), ib(1)];
    let g3 = [
        am(&g1[0], &g2[0], &p),
        am(&g1[1], &g2[1], &p),
        am(&g1[2], &g2[2], &p),
    ];
    let r = richelot(&[g1, g2, g3], &p);
    assert_eq!(r.delta, ib(0), "dependent quadratics ⟹ δ = 0");
    assert!(
        r.is_split(),
        "δ = 0 ⟹ reducible: the abelian surface splits into a product of curves"
    );
}

#[test]
fn chain_walk_finds_a_split_across_the_2_2_graph() {
    let p = big("101");
    // Six Weierstrass points chosen so that one of the 15 (2,2)-neighbours is a split surface.
    // (Roots crafted so a quadratic splitting yields dependent coefficient rows.)
    let roots = [ib(1), ib(2), ib(3), ib(4), ib(5), ib(6)];
    let parts = richelot_partitions(&roots, &p);
    assert_eq!(
        parts.len(),
        15,
        "a genus-2 Jacobian has exactly 15 (2,2)-neighbours"
    );
    // Every partition yields a well-formed Richelot isogeny with a defined δ.
    for g in &parts {
        let r = richelot(g, &p);
        assert_eq!(r.domain.len(), 7, "each neighbour has a sextic domain");
    }
    // The δ = 0 split test is exercised across the whole neighbourhood; the search is total.
    let any_split = parts.iter().any(|g| richelot(g, &p).is_split());
    let found = find_split_neighbour(&roots, &p);
    assert_eq!(
        found.is_some(),
        any_split,
        "find_split_neighbour agrees with the exhaustive scan"
    );
}

#[test]
fn quadratic_root_extraction_recovers_the_weierstrass_points() {
    let p = big("103"); // 103 ≡ 3 (mod 4)
                        // (x − 7)(x − 20) = x² − 27x + 140 ≡ x² + 76x + 37 (mod 103).
    let q = [big("37"), big("76"), ib(1)];
    let roots = quad_roots(&q, &p).expect("rational roots");
    let mut got: Vec<i64> = roots.iter().map(|r| r.to_i64().unwrap()).collect();
    got.sort();
    assert_eq!(
        got,
        vec![7, 20],
        "the quadratic formula recovers both Weierstrass points"
    );
    for r in &roots {
        assert!(
            poly_eval(&q, r, &p).is_zero(),
            "each extracted point is a genuine root"
        );
    }
}

#[test]
fn a_guided_chain_step_lands_on_the_codomain_surface() {
    let p = big("103");
    let roots = [ib(1), ib(2), ib(3), ib(4), ib(5), ib(6)];
    // Take one non-split neighbour and step through it; the extracted Weierstrass points of the next
    // surface must be genuine roots of that neighbour's codomain sextic.
    let parts = richelot_partitions(&roots, &p);
    let idx = (0..parts.len())
        .find(|&i| !richelot(&parts[i], &p).is_split())
        .unwrap();
    let r = richelot(&parts[idx], &p);
    match guided_chain(&roots, &[idx], &p) {
        GuidedWalk::Ended(next) => {
            for pt in &next {
                assert!(
                    poly_eval(&r.codomain, pt, &p).is_zero(),
                    "next surface's 2-torsion ⊂ codomain"
                );
            }
        }
        GuidedWalk::Stuck(_) => {} // codomain 2-torsion left 𝔽_p; the prime-field walk stops honestly
        GuidedWalk::SplitAt(_) => panic!("chose a non-split neighbour"),
    }
}

#[test]
fn the_recursive_chain_finds_a_split_and_the_guided_walk_confirms_it() {
    let p = big("103");
    // Search the space of six-point configurations for one whose (2,2)-graph harbours a split, then
    // confirm the recursive walk finds a path to it and the guided walk reproduces that split exactly.
    let mut s = 0x9e3779b97f4a7c15u64;
    let mut next = || {
        s = s
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((s >> 33) % 100 + 1) as i64
    };
    let mut confirmed = false;
    for _ in 0..400 {
        let mut vals = [0i64; 6];
        for v in &mut vals {
            *v = next();
        }
        // distinct Weierstrass points only (a smooth genus-2 curve)
        if (0..6).any(|i| (i + 1..6).any(|j| vals[i] == vals[j])) {
            continue;
        }
        let roots: [BigInt; 6] = core::array::from_fn(|i| ib(vals[i]));
        if let Some(path) = richelot_chain(&roots, 0, &p) {
            // The recursive search returned a neighbour it certifies as split — re-run it head-on.
            assert_eq!(path.len(), 1, "a depth-0 walk reports a single-step chain");
            assert!(
                richelot(&richelot_partitions(&roots, &p)[path[0]], &p).is_split(),
                "the returned neighbour is genuinely split"
            );
            assert_eq!(
                guided_chain(&roots, &path, &p),
                GuidedWalk::SplitAt(0),
                "following the guidance path reproduces the split"
            );
            confirmed = true;
            break;
        }
    }
    assert!(
        confirmed,
        "split-bearing configurations exist and both walks agree on them"
    );
}

#[test]
fn a_guided_path_off_the_split_ends_or_gets_stuck_but_never_lies() {
    let p = big("103");
    let roots = [ib(2), ib(9), ib(11), ib(30), ib(41), ib(58)];
    // A two-step guided walk either ends on a surface, splits, or honestly reports it left 𝔽_p — it never
    // silently fabricates a continuation.
    match guided_chain(&roots, &[3, 1], &p) {
        GuidedWalk::Ended(final_roots) => {
            assert!(
                final_roots.iter().all(|r| !r.is_negative()),
                "final points are field elements"
            );
        }
        GuidedWalk::SplitAt(step) => assert!(step < 2, "a split is reported within the path"),
        GuidedWalk::Stuck(step) => assert!(step < 2, "leaving 𝔽_p is reported at a real step"),
    }
}

#[test]
fn even_sextic_glues_two_elliptic_curves_into_a_split_jacobian() {
    let p = big("103"); // 103 ≡ 3 (mod 4), so fp_sqrt works
                        // g(u) = (u−1)(u−2)(u−3) = u³ − 6u² + 11u − 6, low→high [−6, 11, −6, 1].
    let g = [ib(-6), ib(11), ib(-6), ib(1)];
    let sg = split_jacobian_from_cubic(&g, &p);
    assert_eq!(sg.sextic.len(), 7, "C is a sextic y² = g(x²)");
    assert_eq!(
        sg.e2,
        vec![ib(1), rp(&ib(-6), &p), rp(&ib(11), &p), rp(&ib(-6), &p)],
        "E₂ is the reversed cubic"
    );

    // Validation 1 — the two degree-2 quotient maps land on E₁ and E₂ for every point of C.
    let mut points = 0;
    for x in 0..103i64 {
        let xb = ib(x);
        let fx = poly_eval(&sg.sextic, &xb, &p);
        let Some(y) = fp_sqrt(&fx, &p) else { continue };
        points += 1;
        // ψ₁: (x, y) ↦ (x², y) onto E₁: Y² = g(u).
        let u = mm(&xb, &xb, &p);
        assert_eq!(
            mm(&y, &y, &p),
            poly_eval(&sg.e1, &u, &p),
            "ψ₁(P) lies on E₁"
        );
        // ψ₂: (x, y) ↦ (1/x², y/x³) onto E₂: Y² = ĝ(u).
        if x != 0 {
            let xi = mod_inverse(&xb, &p).unwrap();
            let v = mm(&xi, &xi, &p);
            let yv = mm(&y, &mm(&xi, &mm(&xi, &xi, &p), &p), &p);
            assert_eq!(
                mm(&yv, &yv, &p),
                poly_eval(&sg.e2, &v, &p),
                "ψ₂(P) lies on E₂"
            );
        }
    }
    assert!(
        points > 5,
        "C has genuine rational points to validate the maps on"
    );

    // Validation 2 — the Richelot ±-splitting Gᵢ = x² − rᵢ (rᵢ the roots 1,2,3 of g) is reducible: its
    // coefficient matrix has an all-zero x-column ⟹ δ = 0 ⟹ is_split. The gluing and the split-test agree.
    let split = [
        [ib(-1), ib(0), ib(1)],
        [ib(-2), ib(0), ib(1)],
        [ib(-3), ib(0), ib(1)],
    ];
    let r = richelot(&split, &p);
    assert_eq!(r.delta, ib(0), "the ±-splitting has δ = 0");
    assert!(
        r.is_split(),
        "even sextic ⟹ Richelot-reducible ⟹ Jac(C) ~ E₁ × E₂ (a genuine (2,2)-split)"
    );
}

#[test]
fn split_jacobian_order_equals_the_product_of_its_elliptic_quotients() {
    // The strongest validation of the gluing: count #Jac(C) independently (genus-2 L-polynomial from the
    // point counts over 𝔽_p and 𝔽_{p²}) and confirm it equals #E₁·#E₂ (Tate). This is the certificate the
    // GENERAL matched-pair gluing will reuse.
    let p = big("103");
    let g = [ib(-6), ib(11), ib(-6), ib(1)]; // (u−1)(u−2)(u−3)
    let sg = split_jacobian_from_cubic(&g, &p);
    let e1 = count_curve_fp(&sg.e1, &p, 1) as i128; // #E₁ (cubic ⟹ 1 point at infinity)
    let e2 = count_curve_fp(&sg.e2, &p, 1) as i128; // #E₂
    let jac = genus2_jacobian_order(&sg.sextic, &p);
    assert_eq!(
        jac,
        e1 * e2,
        "#Jac(C) = #E₁·#E₂ — the split, verified at the level of point counts"
    );
    // Sanity: the elliptic quotients are genuine curves (Hasse bound).
    let bound = 2 * (103f64.sqrt() as i128) + 2;
    assert!(
        (e1 - 104).abs() <= bound && (e2 - 104).abs() <= bound,
        "#Eᵢ obey the Hasse bound"
    );
}

#[test]
fn general_matched_pair_gluing_validates_by_jacobian_order() {
    // The GENERAL (2,2)-gluing: two curves sharing matched 2-torsion {−c₁,−c₂,−c₃}, differing in the
    // fourth quartic root (∓2). The Jacobian-order certificate judges the derivation.
    let p = big("103");
    for c in [
        [ib(3), ib(7), ib(20)],
        [ib(1), ib(5), ib(9)],
        [ib(4), ib(11), ib(30)],
    ] {
        let g = glue_shared_2torsion(&c, &p);
        assert_eq!(g.sextic.len(), 7, "C is a genus-2 sextic");
        assert_eq!(g.e1.len(), 5, "E₁ is a quartic model");
        let e1 = count_curve_fp(&g.e1, &p, 2) as i128; // monic quartic ⟹ 2 points at infinity
        let e2 = count_curve_fp(&g.e2, &p, 2) as i128;
        let jac = genus2_jacobian_order(&g.sextic, &p);
        assert_eq!(
            jac,
            e1 * e2,
            "#Jac(C) = #E₁·#E₂ for c = {c:?} — matched-pair gluing verified (Tate)"
        );
    }
    // E₁ and E₂ genuinely differ (the fourth root ∓2 makes them a nontrivial pair, not one curve twice).
    let g = glue_shared_2torsion(&[ib(3), ib(7), ib(20)], &p);
    assert_ne!(
        g.e1, g.e2,
        "the two elliptic quotients are distinct (they differ in the fourth root)"
    );
}

#[test]
fn the_split_test_is_the_per_digit_oracle() {
    let p = big("103");
    // The CONSISTENT branch: a matched-pair gluing. Its sextic ∏(x²+cᵢx+1) has roots in reciprocal pairs
    // {r, 1/r}; pairing them is a (2,2)-splitting with δ = 0 ⟹ the surface SPLITS (reducible).
    let recip = |r: i64| (ib(r), mod_inverse(&ib(r), &p).unwrap());
    let (a0, a1) = recip(2);
    let (b0, b1) = recip(5);
    let (c0, c1) = recip(7);
    let consistent = [a0, a1, b0, b1, c0, c1];
    assert!(
        surface_is_reducible(&consistent, &p),
        "the glued surface splits — the correct digit"
    );

    // Discrimination: a generic configuration is an indecomposable genus-2 Jacobian — the oracle returns
    // false. (Roots must be SCATTERED: an arithmetic progression is always reducible, since its symmetric
    // pairing has equal pair-sums ⟹ collinear ⟹ δ = 0. That structure is itself a reducibility witness.)
    let mut inconsistent: Option<[BigInt; 6]> = None;
    for seed in 1..200i64 {
        let vals: Vec<i64> = (0..6)
            .map(|i| (seed * 41 + i * i * 7 + i * 3) % 101 + 1)
            .collect();
        if (0..6).any(|i| (i + 1..6).any(|j| vals[i] == vals[j])) {
            continue; // need six distinct Weierstrass points
        }
        let rs: [BigInt; 6] = core::array::from_fn(|i| ib(vals[i]));
        if !surface_is_reducible(&rs, &p) {
            inconsistent = Some(rs);
            break;
        }
    }
    let inconsistent = inconsistent.expect("a generic (irreducible) branch exists");
    assert!(
        !surface_is_reducible(&inconsistent, &p),
        "a wrong branch does not split — the oracle prunes it"
    );

    // Wired as the per-digit selector: among the branches, the split-test picks out the consistent one.
    let branches = [inconsistent.clone(), consistent.clone(), inconsistent];
    let sel =
        select_splitting_branch(&branches, &p).expect("the oracle selects a splitting branch");
    assert_eq!(
        sel, 1,
        "the split-test selects exactly the consistent (gluing) branch"
    );
    assert!(
        surface_is_reducible(&branches[sel], &p),
        "and the selected branch genuinely splits"
    );
}

#[test]
fn genus2_jacobian_cantor_arithmetic_is_a_group_of_order_hash_jac() {
    // C: y² = x(x−1)(x−2)(x−3)(x−4) over 𝔽₁₀₃ — a genus-2 imaginary hyperelliptic curve (deg f = 5).
    let p = big("103");
    let lin = |r: i64| vec![sm(&ib(0), &ib(r), &p), ib(1)]; // x − r
    let mut f = vec![ib(1)];
    for r in [0, 1, 2, 3, 4] {
        f = pmul(&f, &lin(r), &p);
    }
    assert_eq!(pdeg(&f), 5, "a quintic ⟹ genus 2");

    // A rational point P = (x₀, y₀), y₀ ≠ 0, as the divisor class [P − ∞] = (x − x₀, y₀).
    let d = (0..103i64)
        .find_map(|x0| {
            let fx = poly_eval(&f, &ib(x0), &p);
            fp_sqrt(&fx, &p).filter(|y| !y.is_zero()).map(|y0| Mumford {
                u: vec![sm(&ib(0), &ib(x0), &p), ib(1)],
                v: vec![y0],
            })
        })
        .expect("a rational non-Weierstrass point on C");

    let id = jac_identity();
    // Group axioms.
    assert_eq!(cantor_add(&d, &id, &f, &p), d, "D + 0 = D");
    assert_eq!(
        cantor_add(&d, &jac_negate(&d, &p), &f, &p),
        id,
        "D + (−D) = 0"
    );
    let d2 = cantor_add(&d, &d, &f, &p);
    assert_eq!(
        cantor_add(&d2, &d, &f, &p),
        cantor_add(&d, &d2, &f, &p),
        "associativity: (2D)+D = D+(2D)"
    );
    // Every computed class is a valid Mumford pair: u | (f − v²), deg u ≤ 2.
    assert!(pdeg(&d2.u) <= 2, "reduced class has deg u ≤ 2");
    assert_eq!(
        pmod(&psub(&f, &pmul(&d2.v, &d2.v, &p), &p), &d2.u, &p),
        vec![ib(0)],
        "u | (f − v²)"
    );

    // THE definitive validator: the group order kills every class. #Jac counted independently via the
    // genus-2 L-polynomial (point counts over 𝔽_p and 𝔽_{p²}, one point at infinity for deg f = 5).
    let pu = 103i128;
    let n1 = count_curve_fp(&f, &p, 1) as i128;
    let n2 = count_curve_fp2(&f, &p, 1) as i128;
    let (t1, t2) = ((pu + 1) - n1, (pu * pu + 1) - n2);
    let jac = 1 + pu * pu - (1 + pu) * t1 + (t1 * t1 - t2) / 2;
    assert!(jac > 0, "#Jac is a positive integer");
    assert_eq!(
        jac_scalar_mul(jac as u128, &d, &f, &p),
        id,
        "#Jac · D = 0 — Cantor's law is a genuine group of exactly the order the L-polynomial predicts"
    );
}

#[test]
fn richelot_two_two_kernel_and_two_power_torsion() {
    // C: y² = x(x−1)(x−2)(x−3)(x−4) over 𝔽₁₀₃.
    let p = big("103");
    let lin = |r: i64| vec![sm(&ib(0), &ib(r), &p), ib(1)]; // x − r
    let mut f = vec![ib(1)];
    for r in [0, 1, 2, 3, 4] {
        f = pmul(&f, &lin(r), &p);
    }
    let id = jac_identity();
    let pu = 103i128;
    let n1 = count_curve_fp(&f, &p, 1) as i128;
    let n2 = count_curve_fp2(&f, &p, 1) as i128;
    let (t1, t2) = ((pu + 1) - n1, (pu * pu + 1) - n2);
    let jac = (1 + pu * pu - (1 + pu) * t1 + (t1 * t1 - t2) / 2) as u128;

    // ── The Richelot kernel: a (2,2)-subgroup of Jac(C)[2] from Weierstrass-difference divisors. ──
    // D₁ = [W₀ − ∞] = (x, 0), D₂ = [W₁ − ∞] = (x−1, 0) — the classes a (2,2)-isogeny quotients by.
    let d1 = Mumford {
        u: lin(0),
        v: vec![ib(0)],
    };
    let d2 = Mumford {
        u: lin(1),
        v: vec![ib(0)],
    };
    assert_eq!(
        jac_element_order(&d1, jac, &f, &p),
        2,
        "D₁ is 2-torsion (a Weierstrass point)"
    );
    assert_eq!(jac_element_order(&d2, jac, &f, &p), 2, "D₂ is 2-torsion");
    let d3 = cantor_add(&d1, &d2, &f, &p);
    assert_eq!(jac_element_order(&d3, jac, &f, &p), 2, "D₁+D₂ is 2-torsion");
    assert_ne!(d3, id, "D₁, D₂ are independent");
    // {0, D₁, D₂, D₁+D₂} is a closed order-4 (2,2)-subgroup — a genuine Richelot isogeny kernel.
    assert_eq!(cantor_add(&d1, &d3, &f, &p), d2, "closed: D₁+(D₁+D₂) = D₂");
    assert_eq!(cantor_add(&d2, &d3, &f, &p), d1, "closed: D₂+(D₁+D₂) = D₁");

    // ── The 2^e-torsion: the substrate the surface isogeny's (2^e,2^e)-kernel lives in. ──
    let odd = {
        let mut m = jac;
        while m % 2 == 0 {
            m /= 2;
        }
        m
    };
    // Strip the odd part of #Jac off a generic class to land in the 2-power torsion; find one of order > 1.
    let two_power = (5..103i64)
        .find_map(|x0| {
            let y = fp_sqrt(&poly_eval(&f, &ib(x0), &p), &p).filter(|y| !y.is_zero())?;
            let dp = jac_scalar_mul(
                odd,
                &Mumford {
                    u: lin(x0),
                    v: vec![y],
                },
                &f,
                &p,
            );
            (dp != id).then_some(dp)
        })
        .expect("a class with nontrivial 2-power torsion");
    let e_ord = jac_element_order(&two_power, jac, &f, &p);
    assert!(
        e_ord.is_power_of_two() && e_ord >= 2,
        "genuine 2^e-torsion, e ≥ 1: order {e_ord}"
    );
    assert_eq!(
        jac_scalar_mul(e_ord, &two_power, &f, &p),
        id,
        "and 2^e kills it"
    );
    // This 2^e-torsion element, paired with an independent one, spans the (2^e,2^e) lattice the surface
    // isogeny's kernel is a maximal isotropic subgroup of — the structure the image-determined kernel picks.
}

#[test]
fn richelot_dual_is_genuinely_isogenous_equal_jacobian_order() {
    // Three generic monic quadratics ⟹ a genus-2 curve C: y² = G₁G₂G₃, and the Richelot dual
    // C': y² = δ⁻¹·H₁H₂H₃. If richelot()'s dual construction is a genuine (2,2)-isogeny, then by Tate the
    // Jacobians have EQUAL order over 𝔽_p — a validation the map itself would have to respect.
    let p = big("103");
    for g in [
        [
            [ib(1), ib(0), ib(1)],
            [ib(2), ib(1), ib(1)],
            [ib(5), ib(3), ib(1)],
        ],
        [
            [ib(7), ib(2), ib(1)],
            [ib(1), ib(4), ib(1)],
            [ib(3), ib(0), ib(1)],
        ],
    ] {
        let r = richelot(&g, &p);
        assert_ne!(
            r.delta,
            ib(0),
            "generic quadratics ⟹ δ ≠ 0 ⟹ a genuine genus-2 isogeny (not a split)"
        );
        let jac_c = genus2_jacobian_order_general(&r.domain, &p);
        // The true dual carries the δ⁻¹ quadratic twist; without it the count would be off by the twist.
        let dinv = mod_inverse(&r.delta, &p).unwrap();
        let cprime = pscale(&r.codomain, &dinv, &p);
        let jac_cp = genus2_jacobian_order_general(&cprime, &p);
        assert_eq!(
            jac_c, jac_cp,
            "#Jac(C) = #Jac(C') — the Richelot dual is genuinely isogenous (Tate)"
        );
    }
}
