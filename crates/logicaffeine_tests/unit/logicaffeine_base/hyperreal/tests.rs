use super::*;
use std::cmp::Ordering;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

fn qr(n: i64, d: i64) -> Rational {
    Rational::from_ratio_i64(n, d).unwrap()
}

fn h(n: i64) -> Hyperreal {
    Hyperreal::from_i64(n)
}

fn hq(n: i64, d: i64) -> Hyperreal {
    Hyperreal::from_rational(qr(n, d))
}

fn dt() -> Hyperreal {
    Hyperreal::dt()
}

fn hash_of(x: &Hyperreal) -> u64 {
    let mut s = DefaultHasher::new();
    x.hash(&mut s);
    s.finish()
}

fn xorshift(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

/// A random rational function of dt of small degree.
fn random_hyper(st: &mut u64) -> Hyperreal {
    let mut num = h(0);
    let mut power = h(1);
    for _ in 0..=(xorshift(st) % 3) {
        let c = hq((xorshift(st) % 11) as i64 - 5, 1 + (xorshift(st) % 3) as i64);
        num = num.add(&c.mul(&power));
        power = power.mul(&dt());
    }
    let mut den = h(1);
    if xorshift(st).is_multiple_of(2) {
        den = den.add(&hq((xorshift(st) % 7) as i64 - 3, 1).mul(&dt()));
    }
    if xorshift(st).is_multiple_of(4) {
        den = den.mul(&dt());
    }
    num.div(&den).unwrap()
}

// ── the infinitesimal itself ──────────────────────────────────────────────────────────────────

#[test]
fn dt_is_positive_and_below_every_positive_standard_number() {
    assert!(dt() > h(0));
    assert!(dt().neg() < h(0));
    for k in [1i64, 10, 1_000_000, 1_000_000_000_000_000_000] {
        assert!(dt() < hq(1, k), "dt < 1/{k}");
        assert!(dt().recip().unwrap() > h(k), "1/dt > {k}");
    }
    let tiny = Hyperreal::from_rational(Rational::new(BigInt::from_i64(1), BigInt::from_i64(10).pow(200)).unwrap());
    assert!(dt() < tiny);
    assert!(dt().mul(&dt()) < dt(), "dt² ≪ dt");
    assert!(dt().mul(&dt()).mul(&h(1_000_000)) < dt());
    assert!(dt().mul(&dt()) > h(0));
}

#[test]
fn infinitesimal_limited_and_standard_parts() {
    assert!(dt().is_infinitesimal());
    assert!(dt().mul(&dt()).add(&dt()).is_infinitesimal());
    assert!(h(0).is_infinitesimal());
    assert!(!h(3).is_infinitesimal());
    assert!(h(3).add(&dt()).is_limited());
    assert!(!dt().recip().unwrap().is_limited());
    assert_eq!(h(3).add(&h(2).mul(&dt())).standard_part(), Some(Algebraic::from_i64(3)));
    assert_eq!(dt().standard_part(), Some(Algebraic::from_i64(0)));
    assert_eq!(dt().recip().unwrap().standard_part(), None);
    assert_eq!(
        h(1).add(&dt()).div(&h(2).sub(&dt())).unwrap().standard_part(),
        Some(Algebraic::from_rational(qr(1, 2)))
    );
    let root2 = Algebraic::from_i64(2).sqrt().unwrap();
    assert_eq!(
        Hyperreal::from_algebraic(root2.clone()).add(&dt()).standard_part(),
        Some(root2)
    );
    assert!(h(3).add(&dt()).infinitely_close(&h(3)));
    assert!(!h(3).infinitely_close(&h(3).add(&hq(1, 1_000_000_000))));
    assert!(!dt().recip().unwrap().add(&h(1)).infinitely_close(&dt().recip().unwrap().add(&dt())));
    assert!(dt().recip().unwrap().add(&dt()).infinitely_close(&dt().recip().unwrap()));
}

#[test]
fn the_valuation_is_the_order_of_the_leading_power() {
    assert_eq!(dt().valuation(), Some(1));
    assert_eq!(h(5).valuation(), Some(0));
    assert_eq!(dt().recip().unwrap().valuation(), Some(-1));
    assert_eq!(dt().mul(&dt()).add(&dt().pow(5).unwrap()).valuation(), Some(2));
    assert_eq!(h(0).valuation(), None);
    assert_eq!(h(3).sub(&h(2).mul(&dt())).leading_coefficient(), Some(Algebraic::from_i64(3)));
    assert_eq!(dt().mul(&h(-7)).leading_coefficient(), Some(Algebraic::from_i64(-7)));
}

// ── the field ─────────────────────────────────────────────────────────────────────────────────

#[test]
fn the_field_laws_hold_exactly() {
    let mut st = 0x5151_a1a1_7272_b2b2;
    for _ in 0..150 {
        let (a, b, c) = (random_hyper(&mut st), random_hyper(&mut st), random_hyper(&mut st));
        assert_eq!(a.add(&b), b.add(&a));
        assert_eq!(a.mul(&b), b.mul(&a));
        assert_eq!(a.add(&b).add(&c), a.add(&b.add(&c)));
        assert_eq!(a.mul(&b).mul(&c), a.mul(&b.mul(&c)));
        assert_eq!(a.mul(&b.add(&c)), a.mul(&b).add(&a.mul(&c)));
        assert_eq!(a.sub(&a), h(0));
        if a != h(0) {
            assert_eq!(a.div(&a).unwrap(), h(1));
            assert_eq!(a.recip().unwrap().recip().unwrap(), a);
        }
    }
    assert!(h(1).div(&h(0)).is_none());
    assert_eq!(h(1).add(&dt()).mul(&h(1).sub(&dt())), h(1).sub(&dt().mul(&dt())));
    assert_eq!(h(1).div(&h(1).sub(&dt())).unwrap().sub(&h(1)), dt().div(&h(1).sub(&dt())).unwrap());
}

#[test]
fn equal_values_are_structurally_equal_and_hash_alike() {
    let a = h(1).add(&dt()).div(&h(2).sub(&dt())).unwrap();
    let b = h(2).add(&h(2).mul(&dt())).div(&h(4).sub(&h(2).mul(&dt()))).unwrap();
    let c = h(1).sub(&dt().mul(&dt())).div(&h(1).sub(&dt())).unwrap().div(&h(2).sub(&dt())).unwrap();
    assert_eq!(a, b);
    assert_eq!(a, c, "(1 − dt²)/(1 − dt) = 1 + dt cancels");
    assert_eq!(hash_of(&a), hash_of(&b));
    assert_eq!(hash_of(&a), hash_of(&c));
}

// ── order by transfer ─────────────────────────────────────────────────────────────────────────

#[test]
fn the_sign_at_dt_is_the_sign_at_every_small_enough_standard_value() {
    let mut st = 0x7e57_1234_abcd_0001;
    for _ in 0..200 {
        let x = random_hyper(&mut st);
        let small = Rational::new(BigInt::from_i64(1), BigInt::from_i64(1).shl_bits(80)).unwrap();
        let at = x.eval_at(&small);
        let want = match at {
            Some(v) => v.signum(),
            None => continue,
        };
        assert_eq!(x.signum(), want, "{x} at dt = 2^-80");
        let smaller = small.mul(&small);
        assert_eq!(x.eval_at(&smaller).map(|v| v.signum()), Some(want), "the sign is stable below 2^-80");
    }
}

#[test]
fn the_order_is_total_and_consistent_with_subtraction() {
    let mut st = 0x0dd5_ea5e_1357_2468;
    let xs: Vec<Hyperreal> = (0..30).map(|_| random_hyper(&mut st)).collect();
    for a in &xs {
        for b in &xs {
            let want = match b.sub(a).signum() {
                1 => Ordering::Less,
                0 => Ordering::Equal,
                _ => Ordering::Greater,
            };
            assert_eq!(a.cmp(b), want);
            assert_eq!(a.cmp(b), b.cmp(a).reverse());
        }
    }
}

// ── display ───────────────────────────────────────────────────────────────────────────────────

#[test]
fn values_print_as_exact_expressions_in_the_infinitesimal() {
    assert_eq!(dt().to_string(), "dt");
    assert_eq!(dt().neg().to_string(), "-dt");
    assert_eq!(h(3).add(&h(2).mul(&dt())).to_string(), "3 + 2·dt");
    assert_eq!(h(3).sub(&h(2).mul(&dt())).to_string(), "3 - 2·dt");
    assert_eq!(dt().mul(&dt()).to_string(), "dt^2");
    assert_eq!(dt().recip().unwrap().to_string(), "1/dt");
    assert_eq!(h(5).div(&dt()).unwrap().to_string(), "5/dt");
    assert_eq!(h(1).add(&dt()).div(&h(2).sub(&dt())).unwrap().to_string(), "(1 + dt)/(2 - dt)");
    assert_eq!(hq(1, 2).mul(&dt()).to_string(), "(1/2)·dt");
    assert_eq!(h(7).to_string(), "7");
    assert_eq!(h(0).to_string(), "0");
    let root2 = Hyperreal::from_algebraic(Algebraic::from_i64(2).sqrt().unwrap());
    assert_eq!(root2.mul(&dt()).add(&h(1)).to_string(), "1 + √2·dt");
    assert_eq!(dt().render("h"), "h");
    assert_eq!(h(1).add(&dt()).div(&dt()).unwrap().render("ε"), "(1 + ε)/ε");
}

/// A quotient's denominator that is a product is written in parentheses: `/` and `·` read left to
/// right, so `1/2·dt` would be `(1/2)·dt`, a different number from `1/(2·dt)`.
#[test]
fn a_product_in_the_denominator_reads_as_the_denominator() {
    assert_eq!(dt().mul(&h(2)).recip().unwrap().to_string(), "1/(2·dt)");
    assert_eq!(h(3).div(&h(2).mul(&dt())).unwrap().to_string(), "3/(2·dt)");
    assert_eq!(h(1).div(&h(2).mul(&dt()).mul(&dt())).unwrap().to_string(), "1/(2·dt^2)");
    assert_eq!(h(1).div(&dt().mul(&dt())).unwrap().to_string(), "1/dt^2");
    assert_eq!(h(2).mul(&dt()).div(&h(3)).unwrap().to_string(), "(2/3)·dt");
}

#[test]
fn evaluation_at_a_standard_value_substitutes_exactly() {
    let x = h(1).add(&dt()).div(&h(2).sub(&dt())).unwrap();
    assert_eq!(x.eval_at(&qr(1, 2)), Some(Algebraic::from_i64(1)));
    assert_eq!(x.eval_at(&Rational::from_i64(2)), None, "a pole");
    assert_eq!(dt().recip().unwrap().eval_at(&qr(1, 1000)), Some(Algebraic::from_i64(1000)));
}

#[test]
fn a_value_with_no_infinitesimal_part_is_its_standard_value() {
    assert_eq!(h(3).as_standard(), Some(Algebraic::from_i64(3)));
    assert_eq!(hq(-7, 2).as_standard(), Some(Algebraic::from_rational(qr(-7, 2))));
    assert_eq!(h(0).as_standard(), Some(Algebraic::from_i64(0)));
    assert_eq!(h(3).add(&dt()).as_standard(), None);
    assert_eq!(dt().recip().unwrap().as_standard(), None);
    assert_eq!(h(1).add(&dt()).sub(&dt()).as_standard(), Some(Algebraic::from_i64(1)), "dt cancels");
}

// ── powers ────────────────────────────────────────────────────────────────────────────────────

fn too_high(degree: u128) -> Result<Hyperreal, PowerError> {
    Err(PowerError::DegreeTooHigh { degree })
}

#[test]
fn a_power_whose_degree_passes_the_bound_is_refused_before_any_work() {
    assert_eq!(MAX_POWER_DEGREE, 1 << 12);
    assert_eq!(dt().pow(1 << 20), too_high(1 << 20));
    assert_eq!(dt().pow(-(1 << 20)), too_high(1 << 20));
    assert_eq!(dt().pow(i32::MIN), too_high(1 << 31), "the most negative exponent negates without overflow");
    assert_eq!(dt().pow(i32::MAX), too_high(i32::MAX as u128));
    assert_eq!(h(1).add(&dt()).mul(&dt()).pow(3000), too_high(6000), "a degree-2 base doubles the degree");
    assert_eq!(h(1).add(&dt()).div(&h(2).sub(&dt())).unwrap().pow(5000), too_high(5000));
    assert_eq!(h(1).div(&dt().mul(&dt())).unwrap().pow(-2049), too_high(4098), "a reciprocal counts its denominator");
}

#[test]
fn a_power_within_the_bound_is_exact() {
    let at_bound = dt().pow(MAX_POWER_DEGREE as i32).unwrap();
    assert_eq!(at_bound.valuation(), Some(MAX_POWER_DEGREE as i64));
    assert_eq!(dt().pow(-3).unwrap(), dt().recip().unwrap().mul(&dt().recip().unwrap()).mul(&dt().recip().unwrap()));
    let binomial = h(1).add(&dt()).pow(4).unwrap();
    let expanded = h(1)
        .add(&h(4).mul(&dt()))
        .add(&h(6).mul(&dt().mul(&dt())))
        .add(&h(4).mul(&dt().pow(3).unwrap()))
        .add(&dt().pow(4).unwrap());
    assert_eq!(binomial, expanded);
    assert_eq!(h(2).pow(-3), Ok(hq(1, 8)));
    assert_eq!(h(3).pow(100).unwrap().as_standard(), Some(Algebraic::from_rational(Rational::from_bigint(BigInt::from_i64(3).pow(100)))), "a standard base has no degree to bound");
    assert_eq!(h(0).pow(-1), Err(PowerError::ZeroToNegative));
    assert_eq!(h(0).pow(0), Ok(h(1)));
}

#[test]
fn a_refused_power_names_the_infinitesimal_and_the_bound() {
    assert_eq!(
        PowerError::DegreeTooHigh { degree: 6000 }.describe("dt"),
        "the power would have degree 6000 in dt; one power reaches at most degree 4,096"
    );
    assert_eq!(PowerError::ZeroToNegative.describe("dt"), "zero cannot be raised to a negative power");
    assert_eq!(PowerError::ZeroToNegative.to_string(), "zero cannot be raised to a negative power");
}

#[test]
fn a_polynomial_in_the_reciprocal_is_built_in_lowest_terms() {
    let inv = dt().recip().unwrap();
    let want = h(1).add(&h(2).mul(&inv)).add(&h(3).mul(&inv.mul(&inv)));
    let got = Hyperreal::from_reciprocal_powers(&[qr(1, 1), qr(2, 1), qr(3, 1)]);
    assert_eq!(got, want, "the same canonical form the field operations reach");
    assert_eq!(hash_of(&got), hash_of(&want));
    assert_eq!(Hyperreal::from_reciprocal_powers(&[qr(5, 2)]), hq(5, 2));
    assert_eq!(Hyperreal::from_reciprocal_powers(&[qr(0, 1), qr(0, 1), qr(-1, 3), qr(0, 1)]), hq(-1, 3).mul(&inv.mul(&inv)), "trailing zeros are not degree");
    assert_eq!(Hyperreal::from_reciprocal_powers(&[]), h(0));
    assert_eq!(Hyperreal::from_reciprocal_powers(&[qr(0, 1), qr(0, 1)]), h(0));
    assert_eq!(Hyperreal::from_reciprocal_powers(&[qr(4, 1), qr(0, 1), qr(7, 1)]).valuation(), Some(-2));
}

#[test]
fn a_polynomial_in_the_reciprocal_reads_back_its_coefficients() {
    let inv = dt().recip().unwrap();
    let p = h(1).add(&h(2).mul(&inv)).add(&h(3).mul(&inv.mul(&inv)));
    assert_eq!(p.as_reciprocal_powers(), Some(vec![qr(1, 1), qr(2, 1), qr(3, 1)]));
    assert_eq!(Hyperreal::from_reciprocal_powers(&p.as_reciprocal_powers().unwrap()), p);
    assert_eq!(h(1).add(&dt()).div(&dt()).unwrap().as_reciprocal_powers(), Some(vec![qr(1, 1), qr(1, 1)]), "(1 + dt)/dt = 1 + 1/dt");
    assert_eq!(hq(5, 2).as_reciprocal_powers(), Some(vec![qr(5, 2)]));
    assert_eq!(h(0).as_reciprocal_powers(), Some(vec![]));
    assert_eq!(hq(1, 2).div(&dt()).unwrap().as_reciprocal_powers(), Some(vec![qr(0, 1), qr(1, 2)]));
    assert_eq!(dt().as_reciprocal_powers(), None, "a positive power of dt");
    assert_eq!(h(1).add(&dt()).as_reciprocal_powers(), None);
    assert_eq!(h(1).div(&dt().mul(&h(1).add(&dt()))).unwrap().as_reciprocal_powers(), None, "1/(dt + dt²) is no polynomial in 1/dt");
    let root2 = Hyperreal::from_algebraic(Algebraic::from_i64(2).sqrt().unwrap());
    assert_eq!(root2.div(&dt()).unwrap().as_reciprocal_powers(), None, "an irrational coefficient");
}

// ── the principal part ────────────────────────────────────────────────────────────────────────

fn alg(n: i64) -> Algebraic {
    Algebraic::from_i64(n)
}

#[test]
fn a_limited_value_has_no_principal_part() {
    for x in [dt(), h(7), h(0), h(3).add(&dt()), h(1).div(&h(1).add(&dt())).unwrap()] {
        let (principal, rest) = x.principal_part();
        assert!(principal.is_empty(), "{x}");
        assert_eq!(rest, x);
    }
}

#[test]
fn the_principal_part_collects_the_negative_powers_of_dt() {
    let inv = dt().recip().unwrap();
    assert_eq!(inv.principal_part(), (vec![alg(1)], h(0)));
    let x = h(3).mul(&inv.mul(&inv)).add(&h(2).mul(&inv)).add(&h(5)).add(&dt());
    assert_eq!(x.principal_part(), (vec![alg(2), alg(3)], h(5).add(&dt())));
    assert_eq!(h(1).add(&dt()).div(&dt().mul(&dt())).unwrap().principal_part(), (vec![alg(1), alg(1)], h(0)));
    let root2 = Algebraic::from_i64(2).sqrt().unwrap();
    assert_eq!(Hyperreal::from_algebraic(root2.clone()).div(&dt()).unwrap().principal_part(), (vec![root2], h(0)));
}

#[test]
fn the_principal_part_of_a_series_keeps_its_exact_rest() {
    let x = h(1).div(&dt().add(&dt().mul(&dt()))).unwrap();
    let (principal, rest) = x.principal_part();
    assert_eq!(principal, vec![alg(1)], "1/(dt + dt²) = 1/dt − 1 + dt − …");
    assert_eq!(rest, h(-1).div(&h(1).add(&dt())).unwrap(), "the rest is exact, not a truncated series");
    assert!(rest.is_limited());
    let back = Hyperreal::from_reciprocal_powers(&[qr(0, 1), qr(1, 1)]).add(&rest);
    assert_eq!(back, x);
}

#[test]
fn a_double_pole_over_a_unit_takes_two_terms_of_its_series() {
    let x = h(1).div(&dt().mul(&dt()).mul(&h(1).add(&dt()))).unwrap();
    let (principal, rest) = x.principal_part();
    assert_eq!(principal, vec![alg(-1), alg(1)], "1/(dt²(1 + dt)) = 1/dt² − 1/dt + 1 − dt + …");
    assert_eq!(rest, h(1).div(&h(1).add(&dt())).unwrap(), "the rest is 1/(1 + dt)");
    let y = h(2).add(&h(3).mul(&dt())).div(&dt().mul(&dt()).mul(&h(1).sub(&h(2).mul(&dt())))).unwrap();
    let (p, r) = y.principal_part();
    assert_eq!(p, vec![alg(7), alg(2)], "(2 + 3dt)/(dt²(1 − 2dt)) = 2/dt² + 7/dt + …");
    assert!(r.is_limited());
    assert_eq!(Hyperreal::from_reciprocal_powers(&[qr(0, 1), qr(7, 1), qr(2, 1)]).add(&r), y);
}

#[test]
fn a_polynomial_in_dt_reads_back_its_coefficients() {
    let p = h(1).add(&h(2).mul(&dt())).add(&dt().mul(&dt()));
    assert_eq!(p.as_dt_polynomial(), Some(vec![alg(1), alg(2), alg(1)]));
    assert_eq!(h(0).as_dt_polynomial(), Some(vec![]));
    assert_eq!(h(1).div(&h(1).add(&dt())).unwrap().as_dt_polynomial(), None);
    assert_eq!(dt().recip().unwrap().as_dt_polynomial(), None);
}

fn al(n: i64) -> Algebraic {
    Algebraic::from_i64(n)
}

#[test]
fn a_monomial_is_any_power_of_dt_with_a_coefficient() {
    assert_eq!(Hyperreal::monomial(al(3), 2), dt().mul(&dt()).mul(&h(3)));
    assert_eq!(Hyperreal::monomial(al(2), -1), h(2).div(&dt()).unwrap());
    assert_eq!(Hyperreal::monomial(al(5), 0), h(5));
    assert!(Hyperreal::monomial(al(0), 4).is_zero());
}

#[test]
fn ramifying_replaces_dt_by_a_power_of_it_and_back() {
    let x = h(1).add(&dt()).div(&h(2).sub(&dt())).unwrap();
    let r = x.ramify(3);
    let dt3 = dt().pow(3).unwrap();
    assert_eq!(r, h(1).add(&dt3).div(&h(2).sub(&dt3)).unwrap(), "stays reduced and comonic");
    assert_eq!(r.unramify(3), Some(x.clone()));
    assert_eq!(r.unramify(2), None, "dt^3 is no function of dt^2");
    assert_eq!(x.ramify(1), x);
    assert_eq!(h(7).unramify(5), Some(h(7)));
}

#[test]
fn the_laurent_coefficients_run_through_a_pole() {
    let x = h(1).div(&dt().mul(&h(1).sub(&dt()))).unwrap();
    assert_eq!(x.laurent_coefficient(-2), al(0));
    assert_eq!(x.laurent_coefficient(-1), al(1), "1/(dt(1 − dt)) = 1/dt + 1 + dt + …");
    assert_eq!(x.laurent_coefficient(0), al(1));
    assert_eq!(x.laurent_coefficient(5), al(1));
}

#[test]
fn a_rational_function_is_recovered_from_enough_of_its_expansion() {
    let x = h(1).add(&dt()).div(&h(1).sub(&dt().mul(&h(2)))).unwrap();
    let series: Vec<Algebraic> = (0..6).map(|k| x.laurent_coefficient(k)).collect();
    assert_eq!(Hyperreal::pade(&series, 1, 1), Some(x.clone()));
    assert_eq!(Hyperreal::pade(&series, 2, 2), Some(x), "a larger type finds the same function");
    let exp_series: Vec<Algebraic> = [1, 1, 2, 6, 24].iter().map(|f| Algebraic::from_rational(qr(1, *f))).collect();
    let approx = Hyperreal::pade(&exp_series, 1, 1).expect("e^dt has a [1/1] approximant");
    assert_eq!(approx, h(2).add(&dt()).div(&h(2).sub(&dt())).unwrap(), "(2 + dt)/(2 − dt)");
    assert_eq!(Hyperreal::pade(&[al(0), al(1)], 0, 1), None, "dt is no constant over a denominator nonzero at 0");
    let poly = [al(3), al(0), al(4)];
    assert_eq!(Hyperreal::pade(&poly, 2, 0), Some(h(3).add(&dt().mul(&dt()).mul(&h(4)))));
}

#[test]
fn a_value_renders_its_powers_as_it_is_told() {
    let x = h(1).add(&dt().mul(&h(2))).add(&dt().pow(3).unwrap());
    assert_eq!(x.render_powers(&|k| format!("t{k}")), "1 + 2·t1 + t3");
    assert_eq!(x.render("dt"), "1 + 2·dt + dt^3");
    let y = h(1).div(&dt().add(&h(1))).unwrap();
    assert_eq!(y.render_powers(&|k| format!("s^{k}")), "1/(1 + s^1)");
}

#[test]
fn a_root_split_takes_out_every_nth_power_of_a_factor_and_a_constant_that_is_one() {
    let three_dt = h(3).add(&dt());
    let cube = three_dt.pow(3).unwrap();
    let (outside, inside) = cube.root_split(2).unwrap();
    assert_eq!(outside, three_dt);
    assert_eq!(inside, three_dt);
    let (outside, inside) = h(4).add(&dt().mul(&h(4))).root_split(2).unwrap();
    assert_eq!((outside, inside), (h(2), h(1).add(&dt())), "4 is a square");
    let (outside, inside) = h(2).add(&dt().mul(&h(2))).root_split(2).unwrap();
    assert_eq!((outside, inside), (h(1), h(2).add(&dt().mul(&h(2)))), "2 is no square and stays inside");
    let (outside, inside) = h(-8).sub(&dt().mul(&h(8))).root_split(3).unwrap();
    assert_eq!((outside, inside), (h(-2), h(1).add(&dt())), "an odd root of a negative constant");
    let reciprocal = h(1).div(&three_dt.pow(5).unwrap()).unwrap();
    let (outside, inside) = reciprocal.root_split(2).unwrap();
    assert_eq!(outside, h(1).div(&three_dt.pow(2).unwrap()).unwrap());
    assert_eq!(inside, h(1).div(&three_dt).unwrap());
    assert_eq!(dt().neg().root_split(2), None, "no even root of a negative value");
    assert_eq!(h(0).root_split(3), None);
    for (value, n) in [(cube, 2u32), (reciprocal, 2), (h(5).add(&dt()).pow(4).unwrap(), 3)] {
        let (outside, inside) = value.root_split(n).unwrap();
        assert_eq!(outside.pow(n as i32).unwrap().mul(&inside), value, "outside^n · inside is the value");
    }
}
