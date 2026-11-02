use super::*;

fn q(n: i64) -> Rational {
    Rational::from_i64(n)
}

fn qr(n: i64, d: i64) -> Rational {
    Rational::from_ratio_i64(n, d).unwrap()
}

fn p(coeffs: &[i64]) -> Poly {
    Poly::from_i64s(coeffs)
}

fn xorshift(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

fn small(state: &mut u64, span: i64) -> i64 {
    (xorshift(state) % (2 * span as u64 + 1)) as i64 - span
}

fn random_poly(state: &mut u64, max_deg: usize, span: i64) -> Poly {
    let deg = (xorshift(state) % (max_deg as u64 + 1)) as usize;
    let coeffs: Vec<Rational> = (0..=deg)
        .map(|_| {
            let n = small(state, span);
            let d = 1 + (xorshift(state) % 4) as i64;
            qr(n, d)
        })
        .collect();
    Poly::from_coeffs(coeffs)
}

fn product(factors: &[Poly]) -> Poly {
    factors.iter().fold(Poly::one(), |acc, f| acc.mul(f))
}

fn linear(root: &Rational) -> Poly {
    Poly::from_coeffs(vec![root.negated(), q(1)])
}

// ── representation ────────────────────────────────────────────────────────────────────────────

#[test]
fn trailing_zero_coefficients_are_trimmed() {
    let a = p(&[1, 2, 0, 0]);
    assert_eq!(a.degree(), Some(1));
    assert_eq!(a.coeffs().len(), 2);
    assert_eq!(a, p(&[1, 2]));
}

#[test]
fn the_zero_polynomial_has_no_degree_and_no_coefficients() {
    assert_eq!(Poly::zero().degree(), None);
    assert!(Poly::zero().is_zero());
    assert!(Poly::zero().coeffs().is_empty());
    assert_eq!(p(&[0, 0, 0]), Poly::zero());
    assert_eq!(Poly::zero().leading(), None);
}

#[test]
fn constants_x_and_one_have_the_expected_shape() {
    assert_eq!(Poly::one().degree(), Some(0));
    assert_eq!(Poly::x().degree(), Some(1));
    assert_eq!(Poly::x().coeff(1), q(1));
    assert_eq!(Poly::x().coeff(0), q(0));
    assert_eq!(Poly::x().coeff(7), q(0));
    assert_eq!(Poly::constant(qr(3, 4)).coeffs(), &[qr(3, 4)]);
    assert_eq!(Poly::constant(q(0)), Poly::zero());
}

#[test]
fn display_is_descending_with_signs_between_terms() {
    assert_eq!(p(&[-1, -1, 0, 1]).to_string(), "x^3 - x - 1");
    assert_eq!(p(&[1, 0, -1]).to_string(), "-x^2 + 1");
    assert_eq!(p(&[0, 3]).to_string(), "3x");
    assert_eq!(p(&[5]).to_string(), "5");
    assert_eq!(p(&[-5]).to_string(), "-5");
    assert_eq!(Poly::zero().to_string(), "0");
    assert_eq!(p(&[0, 0, -2]).to_string(), "-2x^2");
    assert_eq!(
        Poly::from_coeffs(vec![qr(1, 2), qr(-1, 3), q(1)]).to_string(),
        "x^2 - (1/3)x + 1/2"
    );
}

// ── ring laws ─────────────────────────────────────────────────────────────────────────────────

#[test]
fn addition_and_multiplication_obey_the_ring_laws_on_random_polynomials() {
    let mut st = 0x9e37_79b9_7f4a_7c15;
    for _ in 0..200 {
        let a = random_poly(&mut st, 5, 9);
        let b = random_poly(&mut st, 5, 9);
        let c = random_poly(&mut st, 5, 9);
        assert_eq!(a.add(&b).add(&c), a.add(&b.add(&c)));
        assert_eq!(a.add(&b), b.add(&a));
        assert_eq!(a.mul(&b).mul(&c), a.mul(&b.mul(&c)));
        assert_eq!(a.mul(&b), b.mul(&a));
        assert_eq!(a.mul(&b.add(&c)), a.mul(&b).add(&a.mul(&c)));
        assert_eq!(a.sub(&a), Poly::zero());
        assert_eq!(a.add(&a.neg()), Poly::zero());
        assert_eq!(a.mul(&Poly::one()), a);
        assert_eq!(a.mul(&Poly::zero()), Poly::zero());
        if let (Some(da), Some(db)) = (a.degree(), b.degree()) {
            assert_eq!(a.mul(&b).degree(), Some(da + db));
        }
    }
}

#[test]
fn scaling_by_a_rational_scales_every_coefficient() {
    let a = p(&[1, -2, 3]);
    assert_eq!(a.scale(&qr(1, 2)), Poly::from_coeffs(vec![qr(1, 2), q(-1), qr(3, 2)]));
    assert_eq!(a.scale(&q(0)), Poly::zero());
}

#[test]
fn pow_is_repeated_multiplication() {
    let a = p(&[1, 1]);
    assert_eq!(a.pow(0), Poly::one());
    assert_eq!(a.pow(1), a);
    assert_eq!(a.pow(4), p(&[1, 4, 6, 4, 1]));
}

// ── division and gcd ──────────────────────────────────────────────────────────────────────────

#[test]
fn division_with_remainder_reconstructs_the_dividend() {
    let mut st = 0x1234_5678_9abc_def1;
    for _ in 0..200 {
        let a = random_poly(&mut st, 7, 9);
        let d = random_poly(&mut st, 4, 9);
        if d.is_zero() {
            assert!(a.div_rem(&d).is_none());
            continue;
        }
        let (quot, rem) = a.div_rem(&d).unwrap();
        assert_eq!(quot.mul(&d).add(&rem), a);
        match (rem.degree(), d.degree()) {
            (Some(dr), Some(dd)) => assert!(dr < dd),
            (None, _) => {}
            (Some(_), None) => unreachable!(),
        }
    }
}

#[test]
fn gcd_is_monic_and_divides_both_and_recovers_a_planted_common_factor() {
    let mut st = 0xdead_beef_cafe_f00d;
    for _ in 0..120 {
        let a = random_poly(&mut st, 4, 7);
        let b = random_poly(&mut st, 4, 7);
        let c = random_poly(&mut st, 3, 7);
        if c.is_zero() || (a.is_zero() && b.is_zero()) {
            continue;
        }
        let g = a.mul(&c).gcd(&b.mul(&c));
        assert_eq!(g.leading(), Some(&q(1)), "gcd must be monic");
        assert!(a.mul(&c).div_rem(&g).unwrap().1.is_zero());
        assert!(b.mul(&c).div_rem(&g).unwrap().1.is_zero());
        assert!(g.div_rem(&c.monic()).unwrap().1.is_zero(), "the planted factor divides the gcd");
    }
    assert_eq!(Poly::zero().gcd(&Poly::zero()), Poly::zero());
    assert_eq!(p(&[0, 2]).gcd(&Poly::zero()), Poly::x());
    assert_eq!(p(&[1, 1]).gcd(&p(&[-1, 1])), Poly::one());
}

#[test]
fn monic_divides_by_the_leading_coefficient() {
    assert_eq!(p(&[2, 4]).monic(), Poly::from_coeffs(vec![qr(1, 2), q(1)]));
    assert_eq!(Poly::zero().monic(), Poly::zero());
}

// ── calculus and evaluation ───────────────────────────────────────────────────────────────────

#[test]
fn derivative_obeys_the_product_rule() {
    let mut st = 0x0bad_c0de_1234_5678;
    for _ in 0..100 {
        let a = random_poly(&mut st, 5, 9);
        let b = random_poly(&mut st, 5, 9);
        assert_eq!(
            a.mul(&b).derivative(),
            a.derivative().mul(&b).add(&a.mul(&b.derivative()))
        );
    }
    assert_eq!(p(&[7]).derivative(), Poly::zero());
    assert_eq!(p(&[1, 2, 3, 4]).derivative(), p(&[2, 6, 12]));
}

#[test]
fn evaluation_is_a_ring_homomorphism_and_composition_evaluates_inside_out() {
    let mut st = 0x5555_aaaa_3333_cccc;
    for _ in 0..100 {
        let a = random_poly(&mut st, 4, 9);
        let b = random_poly(&mut st, 4, 9);
        let x = qr(small(&mut st, 20), 1 + (xorshift(&mut st) % 5) as i64);
        assert_eq!(a.mul(&b).eval(&x), a.eval(&x).mul(&b.eval(&x)));
        assert_eq!(a.add(&b).eval(&x), a.eval(&x).add(&b.eval(&x)));
        assert_eq!(a.compose(&b).eval(&x), a.eval(&b.eval(&x)));
    }
}

#[test]
fn reverse_and_negated_variable_transform_the_roots() {
    let a = p(&[-6, 11, -6, 1]);
    assert_eq!(a.reverse(), p(&[1, -6, 11, -6]));
    for r in [1, 2, 3] {
        assert!(a.reverse().eval(&qr(1, r)).is_zero(), "1/r is a root of the reversal");
        assert!(a.neg_var().eval(&q(-r)).is_zero(), "-r is a root of p(-x)");
    }
    assert_eq!(p(&[0, 0, 1, 1]).reverse(), p(&[1, 1]));
}

#[test]
fn squarefree_part_keeps_each_root_once() {
    let a = p(&[-1, 1]).pow(3).mul(&p(&[2, 1]).pow(2)).mul(&p(&[-2, 0, 1]));
    let sf = a.squarefree_part();
    assert_eq!(sf, p(&[-1, 1]).mul(&p(&[2, 1])).mul(&p(&[-2, 0, 1])).monic());
    assert_eq!(Poly::zero().squarefree_part(), Poly::zero());
    assert_eq!(p(&[5]).squarefree_part(), Poly::one());
}

// ── integer content ───────────────────────────────────────────────────────────────────────────

#[test]
fn primitive_integer_form_clears_denominators_divides_content_and_makes_the_lead_positive() {
    let a = Poly::from_coeffs(vec![qr(-1, 2), q(0), qr(-3, 4)]);
    assert_eq!(
        a.primitive_int(),
        vec![BigInt::from_i64(2), BigInt::from_i64(0), BigInt::from_i64(3)]
    );
    assert_eq!(p(&[6, -12, 18]).primitive_int(), vec![1, -2, 3].into_iter().map(BigInt::from_i64).collect::<Vec<_>>());
    assert_eq!(Poly::from_bigints(&p(&[3, 0, 1]).primitive_int()), p(&[3, 0, 1]));
    assert!(Poly::zero().primitive_int().is_empty());
}

// ── real roots ────────────────────────────────────────────────────────────────────────────────

#[test]
fn sturm_counts_distinct_roots_in_half_open_intervals() {
    let a = p(&[-6, 11, -6, 1]);
    assert_eq!(a.count_real_roots(), 3);
    assert_eq!(a.count_real_roots_in(&q(0), &q(4)), 3);
    assert_eq!(a.count_real_roots_in(&q(1), &q(3)), 2, "(1, 3] holds 2 and 3, not 1");
    assert_eq!(a.count_real_roots_in(&qr(3, 2), &qr(5, 2)), 1);
    assert_eq!(a.count_real_roots_in(&q(4), &q(9)), 0);
    let repeated = p(&[-1, 1]).pow(2).mul(&p(&[2, 1]));
    assert_eq!(repeated.count_real_roots(), 2, "a double root counts once");
    assert_eq!(p(&[1, 0, 1]).count_real_roots(), 0);
    assert_eq!(p(&[-2, 0, 1]).count_real_roots(), 2);
    assert_eq!(p(&[-1, -1, 0, 1]).count_real_roots(), 1);
}

#[test]
fn sturm_counts_match_planted_rational_roots_on_random_intervals() {
    let mut st = 0x7777_1111_2222_9999;
    for _ in 0..80 {
        let k = 1 + (xorshift(&mut st) % 6) as usize;
        let roots: Vec<Rational> = (0..k)
            .map(|_| qr(small(&mut st, 30), 1 + (xorshift(&mut st) % 6) as i64))
            .collect();
        let f = product(&roots.iter().map(linear).collect::<Vec<_>>());
        let mut distinct = roots.clone();
        distinct.sort();
        distinct.dedup();
        assert_eq!(f.count_real_roots(), distinct.len());
        for _ in 0..10 {
            let a = qr(small(&mut st, 40), 1 + (xorshift(&mut st) % 7) as i64);
            let b = qr(small(&mut st, 40), 1 + (xorshift(&mut st) % 7) as i64);
            let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
            let want = distinct.iter().filter(|r| **r > lo && **r <= hi).count();
            assert_eq!(f.count_real_roots_in(&lo, &hi), want, "roots {distinct:?} in ({lo}, {hi}]");
        }
    }
}

#[test]
fn isolation_returns_sorted_disjoint_intervals_each_holding_exactly_one_root() {
    let mut st = 0x3141_5926_5358_9793;
    for _ in 0..60 {
        let k = 1 + (xorshift(&mut st) % 6) as usize;
        let roots: Vec<Rational> = (0..k)
            .map(|_| qr(small(&mut st, 25), 1 + (xorshift(&mut st) % 9) as i64))
            .collect();
        let f = product(&roots.iter().map(linear).collect::<Vec<_>>()).mul(&p(&[1, 0, 1]));
        let mut distinct = roots.clone();
        distinct.sort();
        distinct.dedup();
        let iso = f.isolate_real_roots();
        assert_eq!(iso.len(), distinct.len());
        for (iv, r) in iso.iter().zip(distinct.iter()) {
            assert!(iv.lo <= *r && *r <= iv.hi, "{r} not in [{}, {}]", iv.lo, iv.hi);
            if iv.lo == iv.hi {
                assert!(f.eval(&iv.lo).is_zero());
            } else {
                assert!(!f.eval(&iv.lo).is_zero() && !f.eval(&iv.hi).is_zero());
                assert_eq!(f.count_real_roots_in(&iv.lo, &iv.hi), 1);
            }
        }
        for w in iso.windows(2) {
            assert!(w[0].hi <= w[1].lo, "intervals overlap: {iso:?}");
            assert!(w[0].lo < w[1].lo);
        }
    }
}

#[test]
fn irrational_roots_are_isolated_and_refine_toward_the_true_value() {
    let f = p(&[-2, 0, 1]);
    let iso = f.isolate_real_roots();
    assert_eq!(iso.len(), 2);
    assert!(iso[0].hi <= q(0) && iso[1].lo >= q(0), "one root on each side of 0: {iso:?}");
    assert!(iso.iter().all(|iv| !f.eval(&iv.lo).is_zero() && !f.eval(&iv.hi).is_zero()));
    let mut iv = iso[1].clone();
    for _ in 0..60 {
        iv = f.refine(&iv);
    }
    let width = iv.hi.sub(&iv.lo);
    assert!(width < qr(1, 1_000_000_000_000), "refinement halves the interval: width {width}");
    assert!(iv.lo.mul(&iv.lo) < q(2) && iv.hi.mul(&iv.hi) > q(2), "√2 stays enclosed");
    assert!((iv.lo.to_f64() - std::f64::consts::SQRT_2).abs() < 1e-12);
}

#[test]
fn the_cauchy_bound_exceeds_every_root() {
    let mut st = 0xabcd_ef01_2345_6789;
    for _ in 0..60 {
        let roots: Vec<Rational> = (0..4).map(|_| q(small(&mut st, 50))).collect();
        let f = product(&roots.iter().map(linear).collect::<Vec<_>>()).scale(&qr(3, 7));
        let b = f.root_bound();
        for r in &roots {
            assert!(r.abs() < b, "|{r}| must be below the bound {b}");
        }
    }
}

// ── resultants ────────────────────────────────────────────────────────────────────────────────

#[test]
fn the_resultant_of_split_polynomials_is_the_product_of_root_differences() {
    let mut st = 0x1111_2222_3333_4444;
    for _ in 0..80 {
        let a_roots: Vec<Rational> = (0..1 + (xorshift(&mut st) % 4))
            .map(|_| qr(small(&mut st, 12), 1 + (xorshift(&mut st) % 3) as i64))
            .collect();
        let b_roots: Vec<Rational> = (0..1 + (xorshift(&mut st) % 4))
            .map(|_| qr(small(&mut st, 12), 1 + (xorshift(&mut st) % 3) as i64))
            .collect();
        let f = product(&a_roots.iter().map(linear).collect::<Vec<_>>());
        let g = product(&b_roots.iter().map(linear).collect::<Vec<_>>());
        let mut want = q(1);
        for a in &a_roots {
            for b in &b_roots {
                want = want.mul(&a.sub(b));
            }
        }
        assert_eq!(f.resultant(&g), want);
    }
}

#[test]
fn the_resultant_vanishes_exactly_on_a_common_root_and_scales_with_leading_coefficients() {
    assert!(!p(&[-2, 0, 1]).resultant(&p(&[-2, 0, 0, 0, 1])).is_zero());
    assert!(p(&[-2, 0, 1]).resultant(&p(&[-4, 0, 0, 0, 1])).is_zero());
    let f = p(&[-6, 11, -6, 1]);
    let g = p(&[1, 0, 1]);
    assert_eq!(f.scale(&q(2)).resultant(&g), f.resultant(&g).mul(&q(4)), "lc(f)^deg g");
    assert_eq!(f.resultant(&p(&[5])), q(125), "res(f, c) = c^deg f");
    assert_eq!(p(&[5]).resultant(&f), q(125));
}

// ── factorization over ℤ ──────────────────────────────────────────────────────────────────────

fn factors_of(f: &Poly) -> Vec<(Poly, u32)> {
    let out = f.factor_over_q();
    let rebuilt = out
        .iter()
        .fold(Poly::one(), |acc, (g, m)| acc.mul(&g.pow(*m)));
    assert_eq!(
        Poly::from_bigints(&rebuilt.primitive_int()),
        Poly::from_bigints(&f.primitive_int()),
        "the factors must multiply back to the primitive part of {f}"
    );
    for (g, _) in &out {
        assert_eq!(Poly::from_bigints(&g.primitive_int()), *g, "factor {g} is primitive with positive lead");
    }
    out
}

#[test]
fn x4_plus_1_is_irreducible_though_it_splits_modulo_every_prime() {
    let f = p(&[1, 0, 0, 0, 1]);
    assert_eq!(factors_of(&f), vec![(f.clone(), 1)]);
    assert!(f.is_irreducible());
}

#[test]
fn swinnerton_dyer_polynomials_are_irreducible() {
    let sd2 = p(&[1, 0, -10, 0, 1]);
    assert_eq!(factors_of(&sd2).len(), 1);
    let sd3 = p(&[576, 0, -960, 0, 352, 0, -40, 0, 1]);
    assert_eq!(factors_of(&sd3).len(), 1);
    assert!(sd3.is_irreducible());
}

#[test]
fn x_to_the_n_minus_1_splits_into_one_cyclotomic_per_divisor() {
    for n in 1..=24usize {
        let mut c = vec![0i64; n + 1];
        c[0] = -1;
        c[n] = 1;
        let f = p(&c);
        let divisors = (1..=n).filter(|d| n % d == 0).count();
        let fs = factors_of(&f);
        assert_eq!(fs.len(), divisors, "x^{n} - 1");
        assert!(fs.iter().all(|(_, m)| *m == 1));
    }
}

#[test]
fn non_monic_factors_and_multiplicities_are_recovered() {
    let a = p(&[1, 2]);
    let b = p(&[-1, 3]);
    let c = p(&[1, 0, 1]);
    let f = a.mul(&b).mul(&c.pow(2)).scale(&q(6));
    let fs = factors_of(&f);
    assert_eq!(fs, vec![(a.clone(), 1), (b.clone(), 1), (c.clone(), 2)]);
}

#[test]
fn content_and_rational_coefficients_do_not_change_the_factors() {
    let f = p(&[-6, 0, 6]);
    assert_eq!(factors_of(&f), vec![(p(&[-1, 1]), 1), (p(&[1, 1]), 1)]);
    let g = Poly::from_coeffs(vec![qr(-1, 3), q(0), qr(1, 3)]);
    assert_eq!(factors_of(&g), vec![(p(&[-1, 1]), 1), (p(&[1, 1]), 1)]);
}

#[test]
fn factor_order_is_by_degree_then_coefficients() {
    let f = p(&[-2, 0, 1]).mul(&p(&[3, 1])).mul(&p(&[-1, 1])).mul(&p(&[1, 0, 1]));
    let degs: Vec<usize> = factors_of(&f).iter().map(|(g, _)| g.degree().unwrap()).collect();
    assert_eq!(degs, vec![1, 1, 2, 2]);
    assert_eq!(factors_of(&f), factors_of(&f.scale(&q(-5))), "deterministic and sign-blind");
}

#[test]
fn random_products_of_irreducibles_factor_back_into_the_same_pieces() {
    let pieces = [
        p(&[1, 0, 0, 0, 1]),
        p(&[1, 0, -10, 0, 1]),
        p(&[-2, 0, 1]),
        p(&[-3, 0, 1]),
        p(&[1, 1, 1]),
        p(&[-1, -1, 0, 1]),
        p(&[2, 0, 0, 1]),
        p(&[1, 2]),
        p(&[5, -7]),
        p(&[-1, 0, 0, 0, 0, 1]).div_rem(&p(&[-1, 1])).unwrap().0,
    ];
    let canonical: Vec<Poly> = pieces.iter().map(|g| Poly::from_bigints(&g.primitive_int())).collect();
    let mut st = 0x2718_2818_2845_9045;
    for _ in 0..25 {
        let mut chosen: Vec<(usize, u32)> = Vec::new();
        for i in 0..canonical.len() {
            if xorshift(&mut st).is_multiple_of(3) {
                chosen.push((i, 1 + (xorshift(&mut st) % 2) as u32));
            }
        }
        if chosen.is_empty() {
            continue;
        }
        let f = chosen
            .iter()
            .fold(Poly::one(), |acc, (i, m)| acc.mul(&canonical[*i].pow(*m)));
        let got = factors_of(&f);
        let mut want: Vec<(Poly, u32)> = chosen.iter().map(|(i, m)| (canonical[*i].clone(), *m)).collect();
        want.sort_by(|a, b| Poly::canonical_order(&a.0, &b.0));
        assert_eq!(got, want);
        for (g, _) in &got {
            assert!(g.is_irreducible(), "{g}");
        }
    }
}

#[test]
fn linear_and_constant_polynomials_factor_trivially() {
    assert_eq!(factors_of(&p(&[3, 6])), vec![(p(&[1, 2]), 1)]);
    assert!(p(&[7]).factor_over_q().is_empty());
    assert!(p(&[0, 1]).is_irreducible());
    assert!(!p(&[0, 0, 1]).is_irreducible());
    assert_eq!(factors_of(&p(&[0, 0, 0, 1])), vec![(p(&[0, 1]), 3)]);
}
