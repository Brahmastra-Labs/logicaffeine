use super::*;
use crate::algebraic::Algebraic;
use std::cmp::Ordering;

fn qr(n: i64, d: i64) -> Rational {
    Rational::from_ratio_i64(n, d).unwrap()
}

fn r(n: i64) -> ExactReal {
    ExactReal::from_i64(n)
}

fn rq(n: i64, d: i64) -> ExactReal {
    ExactReal::from_rational(qr(n, d))
}

/// Numeric equality as `compare` decides it: `Equal` only when an exact form proves it.
fn same(x: &ExactReal, y: &ExactReal) -> bool {
    x.compare(y, 256).unwrap() == Ordering::Equal
}

fn ten_pow(k: u32) -> BigInt {
    BigInt::from_i64(10).pow(k)
}

/// ⌊√n⌋ for n ≥ 0, by Newton's method on integers — an oracle independent of the crate.
fn isqrt(n: &BigInt) -> BigInt {
    if n.is_zero() {
        return BigInt::zero();
    }
    let mut x = BigInt::from_i64(1).shl_bits(n.bit_len() / 2 + 1);
    loop {
        let y = x.add(&n.div_rem(&x).unwrap().0).div_rem(&BigInt::from_i64(2)).unwrap().0;
        if y >= x {
            return x;
        }
        x = y;
    }
}

/// The first `digits` decimal digits of π after "3.", by the Gauss–Legendre iteration in
/// fixed-point integers with 20 guard digits.
fn pi_digits_oracle(digits: u32) -> String {
    let scale = ten_pow(digits + 20);
    let two = BigInt::from_i64(2);
    let mut a = scale.clone();
    let mut b = isqrt(&scale.mul(&scale).div_rem(&two).unwrap().0);
    let mut t = scale.div_rem(&BigInt::from_i64(4)).unwrap().0;
    let mut p = BigInt::from_i64(1);
    for _ in 0..((digits as f64).log2() as u32 + 4) {
        let an = a.add(&b).div_rem(&two).unwrap().0;
        let bn = isqrt(&a.mul(&b));
        let d = a.sub(&an);
        t = t.sub(&p.mul(&d.mul(&d)).div_rem(&scale).unwrap().0);
        p = p.mul(&two);
        a = an;
        b = bn;
    }
    let s = a.add(&b);
    let pi = s.mul(&s).div_rem(&t.mul(&BigInt::from_i64(4))).unwrap().0;
    let text = pi.to_string();
    text[1..1 + digits as usize].to_string()
}

/// The first `digits` decimal digits of e after "2.", from Σ 1/k! in fixed point.
fn e_digits_oracle(digits: u32) -> String {
    let scale = ten_pow(digits + 20);
    let mut term = scale.clone();
    let mut sum = BigInt::zero();
    let mut k = 1i64;
    while !term.is_zero() {
        sum = sum.add(&term);
        term = term.div_rem(&BigInt::from_i64(k)).unwrap().0;
        k += 1;
    }
    let text = sum.to_string();
    text[1..1 + digits as usize].to_string()
}

fn sqrt2_digits_oracle(digits: u32) -> String {
    let s = isqrt(&BigInt::from_i64(2).mul(&ten_pow(2 * (digits + 20))));
    s.to_string()[1..1 + digits as usize].to_string()
}

/// The digits after the decimal point of a `truncated_decimal` string.
fn fraction_digits(s: &str) -> &str {
    s.split('.').nth(1).unwrap()
}

// ── constants ─────────────────────────────────────────────────────────────────────────────────

#[test]
fn pi_e_and_root_two_match_their_published_first_fifty_digits() {
    assert_eq!(
        ExactReal::pi().to_decimal(51),
        "3.14159265358979323846264338327950288419716939937511"
    );
    assert_eq!(
        ExactReal::e().to_decimal(51),
        "2.71828182845904523536028747135266249775724709369996"
    );
    assert_eq!(
        r(2).sqrt().unwrap().to_decimal(51),
        "1.41421356237309504880168872420969807856967187537695"
    );
}

#[test]
fn a_thousand_digits_agree_with_independent_integer_oracles() {
    let pi = ExactReal::pi().truncated_decimal(1000).unwrap();
    assert_eq!(fraction_digits(&pi), pi_digits_oracle(1000));
    let e = ExactReal::e().truncated_decimal(1000).unwrap();
    assert_eq!(fraction_digits(&e), e_digits_oracle(1000));
    let s = r(2).sqrt().unwrap().truncated_decimal(1000).unwrap();
    assert_eq!(fraction_digits(&s), sqrt2_digits_oracle(1000));
}

#[test]
fn enclosures_contain_the_value_and_meet_the_requested_width() {
    for bits in [1u32, 10, 64, 200, 1000] {
        let (lo, hi) = ExactReal::pi().enclosure(bits).unwrap();
        let width = hi.sub(&lo);
        let eps = Rational::new(BigInt::from_i64(1), BigInt::from_i64(1).shl_bits(bits as u64)).unwrap();
        assert!(width <= eps, "width {width} at {bits} bits");
        assert!(lo < qr(314_159_266, 100_000_000) && hi > qr(314_159_265, 100_000_000));
    }
}

// ── exact forms decide equality ───────────────────────────────────────────────────────────────

#[test]
fn rationals_and_algebraic_numbers_stay_exact() {
    assert_eq!(rq(1, 3).exact_algebraic(), Some(Algebraic::from_rational(qr(1, 3))));
    let s = r(2).sqrt().unwrap();
    assert_eq!(s.exact_algebraic(), Algebraic::from_i64(2).sqrt());
    assert!(same(&s.mul(&s), &r(2)));
    assert_eq!(s.mul(&s).to_string(), "2");
    assert_eq!(s.to_string(), "√2");
    assert_eq!(rq(-7, 2).to_string(), "-7/2");
    assert!(same(&ExactReal::from_f64(0.5).unwrap(), &rq(1, 2)));
}

#[test]
fn lindemann_weierstrass_decides_exponential_identities() {
    let e2 = r(2).exp();
    let em2 = r(-2).exp();
    assert!(same(&e2.mul(&em2), &r(1)), "e²·e⁻² = 1 exactly");
    assert!(ExactReal::e().sub(&r(1).exp()).is_exactly_zero());
    assert!(same(&ExactReal::e().mul(&ExactReal::e()), &e2));
    assert_eq!(e2.compare(&r(7), 64).unwrap(), Ordering::Greater);
    assert_eq!(e2.compare(&r(8), 64).unwrap(), Ordering::Less);
    let s = r(2).sqrt().unwrap();
    assert!(same(&s.exp().mul(&s.neg().exp()), &r(1)), "e^√2 · e^-√2 = 1");
    assert!(same(&s.exp().pow(2).unwrap(), &s.mul(&r(2)).exp()), "(e^√2)² = e^(2√2)");
    assert!(e2.add(&em2).sub(&r(2).cosh().mul(&r(2))).is_exactly_zero(), "e² + e⁻² = 2·cosh 2");
    assert!(!e2.sub(&r(7)).is_exactly_zero());
}

#[test]
fn sine_and_cosine_of_algebraic_arguments_satisfy_their_identities_exactly() {
    for x in [r(1), rq(1, 3), r(2).sqrt().unwrap(), r(-5)] {
        let (s, c) = (x.sin(), x.cos());
        assert!(same(&s.mul(&s).add(&c.mul(&c)), &r(1)), "sin²({x}) + cos²({x}) = 1");
        assert!(same(&x.mul(&r(2)).sin(), &r(2).mul(&s).mul(&c)), "sin(2x) = 2 sin x cos x");
        assert!(same(&x.neg().cos(), &c), "cosine is even");
        assert!(same(&x.neg().sin(), &s.neg()), "sine is odd");
    }
    assert!(same(&r(0).sin(), &r(0)));
    assert!(same(&r(0).cos(), &r(1)));
    assert_eq!(r(1).cos().to_decimal(30), "0.540302305868139717400936607443");
    assert_eq!(r(1).sin().to_decimal(30), "0.841470984807896506652502321630");
}

#[test]
fn exact_forms_print_symbolically() {
    assert_eq!(ExactReal::pi().to_string(), "π");
    assert_eq!(ExactReal::e().to_string(), "e");
    assert_eq!(r(2).exp().to_string(), "e^2");
    assert_eq!(r(-1).exp().to_string(), "e^(-1)");
    assert_eq!(rq(1, 2).exp().to_string(), "e^(1/2)");
    assert_eq!(r(2).sqrt().unwrap().exp().to_string(), "e^(√2)");
    assert_eq!(r(3).mul(&r(2).exp()).to_string(), "3·e^2");
    assert_eq!(r(2).exp().add(&r(1)).to_string(), "e^2 + 1");
    assert_eq!(r(1).sub(&r(2).exp()).to_string(), "-e^2 + 1");
    assert_eq!(r(1).cos().to_string(), "cos(1)");
    assert_eq!(r(1).sin().to_string(), "sin(1)");
    assert_eq!(r(2).mul(&r(3).sin()).to_string(), "2·sin(3)");
    assert_eq!(r(1).exp().mul(&r(2).cos()).to_string(), "e·cos(2)");
    assert_eq!(ExactReal::pi().mul(&r(2)).to_string(), "2·π");
    assert_eq!(ExactReal::pi().mul(&ExactReal::pi()).add(&r(1)).to_string(), "π^2 + 1");
}

#[test]
fn polynomials_in_pi_have_decidable_equality() {
    let p = ExactReal::pi();
    assert!(same(&p.add(&p), &p.mul(&r(2))));
    assert!(p.mul(&p).sub(&p.mul(&p)).is_exactly_zero());
    assert!(!p.mul(&p).sub(&r(10)).is_exactly_zero());
    assert_eq!(p.mul(&p).compare(&r(10), 64).unwrap(), Ordering::Less);
    assert_eq!(p.compare(&rq(22, 7), 64).unwrap(), Ordering::Less);
    assert_eq!(p.compare(&rq(333, 106), 64).unwrap(), Ordering::Greater);
}

// ── general computable reals ──────────────────────────────────────────────────────────────────

#[test]
fn logarithms_invert_exponentials_and_general_values_print_as_decimals() {
    assert!(same(&r(3).exp().log().unwrap(), &r(3)), "log e³ = 3 exactly");
    assert!(same(&r(2).log().unwrap().exp(), &r(2)), "e^(log 2) = 2 exactly");
    assert_eq!(r(2).log().unwrap().to_string(), "0.693147180559945309417232121458");
    assert_eq!(r(10).log().unwrap().to_decimal(20), "2.3025850929940456840");
    assert!(r(0).log().is_err());
    assert!(r(-1).log().is_err());
}

#[test]
fn mixed_transcendentals_are_compared_by_refinement() {
    let s = ExactReal::pi().add(&ExactReal::e());
    assert_eq!(s.to_decimal(20), "5.8598744820488384738");
    assert_eq!(s.compare(&rq(586, 100), 64).unwrap(), Ordering::Less);
    assert_eq!(s.compare(&rq(585, 100), 64).unwrap(), Ordering::Greater);
    let golden = r(1).add(&r(5).sqrt().unwrap()).div(&r(2)).unwrap();
    assert_eq!(ExactReal::pi().sub(&golden).compare(&rq(152, 100), 64).unwrap(), Ordering::Greater);
}

#[test]
fn comparing_values_that_may_be_equal_fails_loudly_at_the_budget() {
    let x = ExactReal::pi().add(&ExactReal::e());
    let y = ExactReal::e().add(&ExactReal::pi());
    let err = x.compare(&y, 128).unwrap_err();
    assert!(err.to_string().contains("cannot separate"), "{err}");
    assert!(err.to_string().contains("2^-128"), "{err}");
    assert!(err.to_string().contains("π + e"), "the values are named structurally: {err}");
}

#[test]
fn comparison_within_a_tolerance_is_total() {
    let p = ExactReal::pi();
    let near = rq(355, 113);
    assert_eq!(p.compare_within(&near, &qr(1, 1_000_000)).unwrap(), Closeness::Close);
    assert_eq!(p.compare_within(&near, &qr(1, 100_000_000)).unwrap(), Closeness::Less);
    assert_eq!(near.compare_within(&p, &qr(1, 100_000_000)).unwrap(), Closeness::Greater);
    let x = ExactReal::pi().add(&ExactReal::e());
    let y = ExactReal::e().add(&ExactReal::pi());
    assert_eq!(x.compare_within(&y, &qr(1, 1_000_000_000_000)).unwrap(), Closeness::Close);
}

#[test]
fn division_by_an_exact_zero_is_refused_and_by_a_general_zero_fails_at_the_budget() {
    assert!(r(1).div(&r(0)).is_err());
    let zeroish = ExactReal::pi().add(&ExactReal::e()).sub(&ExactReal::e().add(&ExactReal::pi()));
    let q = r(1).div(&zeroish).unwrap();
    assert!(q.enclosure(64).is_err());
    assert!(q.enclosure(64).unwrap_err().to_string().contains("cannot separate"));
}

#[test]
fn floor_of_a_transcendental_is_exact() {
    assert_eq!(ExactReal::pi().floor(64).unwrap(), BigInt::from_i64(3));
    assert_eq!(ExactReal::pi().neg().floor(64).unwrap(), BigInt::from_i64(-4));
    assert_eq!(r(5).exp().floor(64).unwrap(), BigInt::from_i64(148));
    assert_eq!(r(7).floor(64).unwrap(), BigInt::from_i64(7));
    assert_eq!(rq(-7, 2).floor(64).unwrap(), BigInt::from_i64(-4));
}

#[test]
fn to_decimal_rounds_correctly_and_handles_signs_and_magnitudes() {
    assert_eq!(rq(1, 3).to_decimal(5), "0.33333");
    assert_eq!(rq(2, 3).to_decimal(5), "0.66667");
    assert_eq!(rq(-2, 3).to_decimal(5), "-0.66667");
    assert_eq!(rq(1, 8).to_decimal(2), "0.13", "ties round half away from zero");
    assert_eq!(r(123_456).to_decimal(3), "123000");
    assert_eq!(rq(1, 1_000_000).to_decimal(2), "0.0000010");
    assert_eq!(ExactReal::pi().mul(&r(1000)).to_decimal(6), "3141.59");
    assert_eq!(r(0).to_decimal(5), "0");
}

#[test]
fn the_field_operations_agree_with_rational_arithmetic_on_rationals() {
    let mut st = 0x9999_1111_8888_2222u64;
    let mut next = || {
        st ^= st << 13;
        st ^= st >> 7;
        st ^= st << 17;
        st
    };
    for _ in 0..200 {
        let a = qr((next() % 61) as i64 - 30, 1 + (next() % 7) as i64);
        let b = qr((next() % 61) as i64 - 30, 1 + (next() % 7) as i64);
        let (x, y) = (ExactReal::from_rational(a.clone()), ExactReal::from_rational(b.clone()));
        assert!(same(&x.add(&y), &ExactReal::from_rational(a.add(&b))));
        assert!(same(&x.mul(&y), &ExactReal::from_rational(a.mul(&b))));
        assert!(same(&x.sub(&y), &ExactReal::from_rational(a.sub(&b))));
        assert_eq!(x.compare(&y, 64).unwrap(), a.cmp(&b));
        if !b.is_zero() {
            assert!(same(&x.div(&y).unwrap(), &ExactReal::from_rational(a.div(&b).unwrap())));
        }
    }
}

// ── soundness: every enclosure meets an independent oracle's bracket ─────────────────────────

const ORACLE_DIGITS: u32 = 400;

/// `x·10^ORACLE_DIGITS` truncated, as the bracket `[n, n + 1]/10^ORACLE_DIGITS` around `x`.
fn bracket(scaled: BigInt) -> (Rational, Rational) {
    let den = ten_pow(ORACLE_DIGITS);
    (
        Rational::new(scaled.clone(), den.clone()).unwrap(),
        Rational::new(scaled.add(&BigInt::from_i64(1)), den).unwrap(),
    )
}

/// Σ_{k ≥ 0} t_k in fixed point at `ORACLE_DIGITS + 60` digits, where `next(t_{k−1}, k)` produces
/// each term from the previous; returns ⌊x·10^ORACLE_DIGITS⌋ (the 60 guard digits absorb every
/// truncation).
fn series(first: BigInt, next: impl Fn(&BigInt, i64) -> BigInt) -> BigInt {
    let mut term = first;
    let mut sum = BigInt::zero();
    let mut k = 1i64;
    while !term.is_zero() {
        sum = sum.add(&term);
        term = next(&term, k);
        k += 1;
    }
    let (q, r) = sum.div_rem(&ten_pow(60)).unwrap();
    if r.is_negative() { q.sub(&BigInt::from_i64(1)) } else { q }
}

fn oracle_scale() -> BigInt {
    ten_pow(ORACLE_DIGITS + 60)
}

fn assert_meets(x: &ExactReal, oracle: (Rational, Rational), what: &str) {
    for bits in [64u32, 300, 600] {
        let (lo, hi) = x.enclosure(bits).unwrap();
        assert!(lo <= oracle.1 && hi >= oracle.0, "{what}: enclosure at {bits} bits misses the true value");
    }
}

#[test]
fn transcendental_enclosures_contain_independently_computed_values() {
    let k = |n: i64| BigInt::from_i64(n);
    let ln2 = series(oracle_scale().div_rem(&k(2)).unwrap().0, |t, j| {
        t.mul(&k(j)).div_rem(&k(2 * (j + 1))).unwrap().0
    });
    assert_meets(&r(2).log().unwrap(), bracket(ln2), "ln 2 = Σ 1/(k·2^k)");
    let inv_e = series(oracle_scale(), |t, j| t.negated().div_rem(&k(j)).unwrap().0);
    assert_meets(&r(-1).exp(), bracket(inv_e), "e⁻¹ = Σ (−1)^k/k!");
    let e10 = series(oracle_scale(), |t, j| t.mul(&k(10)).div_rem(&k(j)).unwrap().0);
    assert_meets(&r(10).exp(), bracket(e10), "e¹⁰ = Σ 10^k/k!");
    let sin1 = series(oracle_scale(), |t, j| t.negated().div_rem(&k((2 * j) * (2 * j + 1))).unwrap().0);
    assert_meets(&r(1).sin(), bracket(sin1), "sin 1");
    let cos1 = series(oracle_scale(), |t, j| t.negated().div_rem(&k((2 * j - 1) * (2 * j))).unwrap().0);
    assert_meets(&r(1).cos(), bracket(cos1), "cos 1");
    let guard = ten_pow(ORACLE_DIGITS + 60 + 50);
    let sin100 = series(guard.mul(&k(100)), |t, j| {
        t.mul(&k(-10_000)).div_rem(&k((2 * j) * (2 * j + 1))).unwrap().0
    });
    let sin100 = sin100.div_rem(&ten_pow(50)).unwrap().0;
    let sin100 = if sin100.is_negative() { sin100.sub(&BigInt::from_i64(1)) } else { sin100 };
    assert_meets(&r(100).sin(), bracket(sin100), "sin 100 (reduced modulo 2π)");
    let ln10 = series(oracle_scale().mul(&k(9)).div_rem(&k(10)).unwrap().0, |t, j| {
        t.mul(&k(9)).mul(&k(j)).div_rem(&k(10 * (j + 1))).unwrap().0
    });
    assert_meets(&r(10).log().unwrap(), bracket(ln10), "ln 10 = Σ (9/10)^k/k");
}

#[test]
fn general_operations_keep_enclosures_sound() {
    let pi_digits: String = std::iter::once('3').chain(pi_digits_oracle(ORACLE_DIGITS).chars()).collect();
    let pi_scaled = BigInt::parse_decimal(&pi_digits).unwrap();
    assert_meets(&ExactReal::pi(), bracket(pi_scaled.clone()), "π");
    let p = ExactReal::pi();
    let e = ExactReal::e();
    for x in [p.add(&e), p.mul(&e), p.div(&e).unwrap(), p.sub(&e), p.add(&e).sqrt().unwrap(), p.exp(), e.log().unwrap().add(&p)] {
        let (lo, hi) = x.enclosure(200).unwrap();
        let (lo2, hi2) = x.enclosure(400).unwrap();
        assert!(lo <= lo2 && hi2 <= hi || (lo2 <= hi && hi2 >= lo), "{}: narrower enclosures stay inside", x.describe());
        assert!(hi2.sub(&lo2) <= hi.sub(&lo));
    }
    assert_eq!(p.add(&e).describe(), "π + e");
    assert_eq!(p.sub(&e).describe(), "π - e");
    assert_eq!(p.add(&e).recip().unwrap().describe(), "1/(π + e)");
    assert_eq!(p.add(&e).log().unwrap().describe(), "log(π + e)");
}

#[test]
fn rational_rounding_ties_and_negative_values() {
    assert_eq!(rq(5, 2).to_decimal(1), "3", "2.5 rounds away from zero");
    assert_eq!(rq(-5, 2).to_decimal(1), "-3");
    assert_eq!(rq(15, 1000).to_decimal(1), "0.02");
    assert_eq!(rq(999, 1000).to_decimal(2), "1.0", "rounding up carries into a new digit");
    assert_eq!(rq(9995, 10).to_decimal(3), "1000");
    assert_eq!(ExactReal::pi().neg().to_decimal(5), "-3.1416");
    assert_eq!(ExactReal::pi().neg().truncated_decimal(4).unwrap(), "-3.1415");
    assert_eq!(rq(7, 2).truncated_decimal(0).unwrap(), "3");
}

// ── raw primitives: every counted error term is load-bearing ──────────────────────────────────
//
// Each oracle is an exact rational partial sum whose omitted tail is below 2^-1000, so
// `[S − 2^-(w+60), S + 2^-(w+60)]` holds the true value for every working precision `w` tested
// (at most a few hundred bits) while staying far narrower than the enclosure under test — so
// dropping any counted error term makes the enclosure miss it.

/// `[S − 2^-(w+60), S + 2^-(w+60)]` for the partial sum `S` of `terms`.
fn truth(terms: impl Iterator<Item = Rational>, w: i64) -> (Rational, Rational) {
    let s = terms.fold(Rational::zero(), |acc, t| acc.add(&t));
    let slack = Rational::new(BigInt::from_i64(1), BigInt::from_i64(1).shl_bits((w + 60) as u64)).unwrap();
    (s.sub(&slack), s.add(&slack))
}

fn scaled(x: &(Rational, Rational), w: i64) -> (Rational, Rational) {
    let k = Rational::from_bigint(BigInt::from_i64(1).shl_bits(w as u64));
    (x.0.mul(&k), x.1.mul(&k))
}

fn int_bracket_holds(lo: &BigInt, hi: &BigInt, t: &(Rational, Rational)) -> bool {
    Rational::from_bigint(lo.clone()) <= t.0 && Rational::from_bigint(hi.clone()) >= t.1
}

fn iv_holds(iv: &Iv, t: &(Rational, Rational)) -> bool {
    iv.lo.to_rational() <= t.0 && iv.hi.to_rational() >= t.1
}

/// `Σ_{k<n} sign^k · x^{a·k+b} / (scale(k))` as exact rationals.
fn power_series(x: &Rational, n: usize, step: impl Fn(usize) -> Rational) -> Vec<Rational> {
    let mut out = Vec::with_capacity(n);
    let mut term = Rational::one();
    for k in 0..n {
        out.push(term.clone());
        term = term.mul(&step(k)).mul(x);
    }
    out
}

#[test]
fn the_atan_series_bracket_contains_the_true_value() {
    for (n, w) in [(5i64, 64i64), (5, 300), (239, 64), (239, 400), (7, 100)] {
        let z = Rational::from_ratio_i64(1, n).unwrap();
        let z2 = z.mul(&z);
        let terms = (0..400i64).map(|k| {
            let sign = if k % 2 == 0 { Rational::one() } else { Rational::from_i64(-1) };
            z.mul(&z2.pow(k as i32).unwrap()).mul(&sign).div(&Rational::from_i64(2 * k + 1)).unwrap()
        });
        let t = scaled(&truth(terms, w), w);
        let (lo, hi) = atan_recip_scaled(n, w);
        assert!(int_bracket_holds(&lo, &hi, &t), "atan(1/{n}) at 2^-{w}");
    }
}

#[test]
fn the_atanh_bracket_contains_the_true_value() {
    for (zn, zd, w) in [(1i64, 3i64, 64i64), (1, 3, 300), (1, 5, 80), (2, 7, 120)] {
        let z = Rational::from_ratio_i64(zn, zd).unwrap();
        let z2 = z.mul(&z);
        let terms = (0..400i64).map(|k| z.mul(&z2.pow(k as i32).unwrap()).div(&Rational::from_i64(2 * k + 1)).unwrap());
        let t = scaled(&truth(terms, w), w);
        let (lo, hi) = atanh_scaled(&BigInt::from_i64(zn), &BigInt::from_i64(zd), w);
        assert!(int_bracket_holds(&lo, &hi, &t), "atanh({zn}/{zd}) at 2^-{w}");
    }
}

#[test]
fn the_raw_exponential_contains_the_true_value() {
    for (x, w) in [
        (Dyadic::int(1), 64i64),
        (Dyadic::int(-1), 64),
        (Dyadic::new(BigInt::from_i64(3), -2), 96),
        (Dyadic::int(5), 80),
        (Dyadic::int(1), 300),
    ] {
        let xr = x.to_rational();
        let terms = power_series(&xr, 260, |k| Rational::from_ratio_i64(1, k as i64 + 1).unwrap());
        let t = truth(terms.into_iter(), w);
        assert!(iv_holds(&exp_raw(&x, w), &t), "exp({xr}) at working precision {w}");
    }
}

#[test]
fn the_raw_sine_and_cosine_contain_the_true_values() {
    for (x, w) in [(Dyadic::int(1), 64i64), (Dyadic::new(BigInt::from_i64(5), -1), 80), (Dyadic::int(-3), 72), (Dyadic::int(1), 300)] {
        let xr = x.to_rational();
        let x2 = xr.mul(&xr);
        let sin_terms = (0..120i64).map(|k| {
            let sign = if k % 2 == 0 { Rational::one() } else { Rational::from_i64(-1) };
            let fact = (1..=(2 * k + 1)).fold(Rational::one(), |a, j| a.mul(&Rational::from_i64(j)));
            xr.mul(&x2.pow(k as i32).unwrap()).mul(&sign).div(&fact).unwrap()
        });
        let cos_terms = (0..120i64).map(|k| {
            let sign = if k % 2 == 0 { Rational::one() } else { Rational::from_i64(-1) };
            let fact = (1..=(2 * k)).fold(Rational::one(), |a, j| a.mul(&Rational::from_i64(j)));
            x2.pow(k as i32).unwrap().mul(&sign).div(&fact).unwrap()
        });
        let (s, c) = sin_cos_raw(&x, w);
        assert!(iv_holds(&s, &truth(sin_terms, w)), "sin({xr}) at working precision {w}");
        assert!(iv_holds(&c, &truth(cos_terms, w)), "cos({xr}) at working precision {w}");
    }
}

#[test]
fn reciprocal_enclosures_round_outward() {
    let mut st = 0xabad_1dea_5eed_0001u64;
    for _ in 0..500 {
        st ^= st << 13;
        st ^= st >> 7;
        st ^= st << 17;
        let m = (st % 100_000) as i64 + 1;
        let sign = if st & 1 == 0 { 1 } else { -1 };
        let a = Dyadic::new(BigInt::from_i64(sign * m), -(((st >> 20) % 30) as i64));
        let b = a.add(&Dyadic::new(BigInt::from_i64(1), -40));
        let x = Iv { lo: a, hi: b };
        let r = recip_iv(&x, 50);
        let (true_lo, true_hi) = (x.hi.to_rational().recip().unwrap(), x.lo.to_rational().recip().unwrap());
        assert!(r.lo.to_rational() <= true_lo && r.hi.to_rational() >= true_hi, "1/{x:?}");
    }
}

// ── targeted checks (each one kills a mutant the corpus above let live) ──────────────────────

#[test]
fn fractional_and_compound_coefficients_are_parenthesised() {
    assert_eq!(rq(1, 2).mul(&r(2).exp()).to_string(), "(1/2)·e^2");
    let one_plus_root2 = r(1).add(&r(2).sqrt().unwrap());
    assert_eq!(one_plus_root2.mul(&ExactReal::e()).to_string(), "(1 + √2)·e");
    assert_eq!(r(2).sqrt().unwrap().div(&r(2)).unwrap().mul(&ExactReal::pi()).to_string(), "(√2/2)·π");
    assert_eq!(rq(-3, 4).mul(&r(1).cos()).to_string(), "-(3/4)·cos(1)");
}

#[test]
fn the_logarithm_of_a_general_exponential_is_its_exponent() {
    let p = ExactReal::pi();
    assert_eq!(p.exp().log().unwrap().describe(), "π");
    let s = ExactReal::pi().add(&ExactReal::e());
    assert_eq!(s.exp().log().unwrap().describe(), "π + e");
}

#[test]
fn arithmetic_nodes_meet_their_working_precision_directly() {
    let p = ExactReal::pi();
    let e = ExactReal::e();
    for x in [p.add(&e), p.mul(&e), p.mul(&e).mul(&p), p.sub(&e).neg()] {
        let node = x.node();
        for w in [8i64, 40, 120] {
            let iv = node.enclose(w, 4096).unwrap();
            let bound = Dyadic::new(BigInt::from_i64(1), 1 - w);
            assert!(iv.width().cmp(&bound) != Ordering::Greater, "{} at {w}", x.describe());
        }
    }
}

// ── flows of polynomial differential equations ──────────────────────────────────────────────

fn monomial(c: Rational, powers: &[u32]) -> crate::ode::Monomial {
    crate::ode::Monomial {
        coefficient: c,
        powers: powers.to_vec(),
    }
}

/// `x' = x − x²`.
fn logistic_field() -> crate::ode::Field {
    crate::ode::Field {
        components: vec![vec![monomial(qr(1, 1), &[1]), monomial(qr(-1, 1), &[2])]],
    }
}

#[test]
fn the_logistic_flow_prints_its_certified_digits() {
    let x = ExactReal::flow(&logistic_field(), &[rq(1, 2)], &qr(1, 1)).unwrap();
    assert_eq!(x.len(), 1);
    assert_eq!(x[0].to_decimal(30), "0.731058578630004879251159241822");
    assert_eq!(x[0].describe(), "x1 of a flow at time 1");
}

#[test]
fn a_nonlinear_rotation_lands_on_cosine_and_sine() {
    let field = crate::ode::Field {
        components: vec![
            vec![monomial(qr(-1, 1), &[2, 1]), monomial(qr(-1, 1), &[0, 3])],
            vec![monomial(qr(1, 1), &[3, 0]), monomial(qr(1, 1), &[1, 2])],
        ],
    };
    let xy = ExactReal::flow(&field, &[r(1), r(0)], &qr(1, 1)).unwrap();
    assert_eq!(xy[0].to_decimal(30), r(1).cos().to_decimal(30));
    assert_eq!(xy[1].to_decimal(30), r(1).sin().to_decimal(30));
    assert_eq!(xy[0].compare(&rq(1, 2), 256), Ok(Ordering::Greater));
}

#[test]
fn a_flow_that_leaves_every_bound_is_a_domain_error() {
    let field = crate::ode::Field {
        components: vec![vec![monomial(qr(1, 1), &[2])]],
    };
    let x = ExactReal::flow(&field, &[r(1)], &qr(2, 1)).unwrap();
    assert!(matches!(x[0].enclosure(8), Err(RealError::Domain(why)) if why.contains("leaves every bound")));
}

#[test]
fn a_flow_with_a_mismatched_shape_or_negative_time_is_refused() {
    assert!(ExactReal::flow(&logistic_field(), &[r(1), r(2)], &qr(1, 1)).is_none());
    assert!(ExactReal::flow(&logistic_field(), &[r(1)], &qr(-1, 1)).is_none());
    assert!(ExactReal::flow(&logistic_field(), &[], &qr(1, 1)).is_none());
}

#[test]
fn a_flow_round_trips_through_its_encoding() {
    use crate::exact_codec::ExactCodec;
    let x = ExactReal::flow(&logistic_field(), &[rq(1, 2)], &qr(1, 1)).unwrap().remove(0);
    let mut bytes = Vec::new();
    x.write(&mut bytes);
    let mut pos = 0;
    let back = ExactReal::read(&bytes, &mut pos).expect("decodes");
    assert_eq!(pos, bytes.len());
    assert_eq!(back.to_decimal(30), x.to_decimal(30));
    let mut again = Vec::new();
    back.write(&mut again);
    assert_eq!(again, bytes);
}

// ── real numbers a program defines ────────────────────────────────────────────────────────────

use crate::exact_codec::ExactCodec;

/// A function of `n` as a definition reads it, counting how often it was asked.
struct Counted<F: Fn(u64) -> Result<Rational, String> + Send + Sync> {
    f: F,
    calls: std::sync::atomic::AtomicUsize,
}

impl<F: Fn(u64) -> Result<Rational, String> + Send + Sync> Approximant for Counted<F> {
    fn call(&self, n: u64) -> Result<Rational, String> {
        self.calls.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        (self.f)(n)
    }
}

fn counted<F: Fn(u64) -> Result<Rational, String> + Send + Sync + 'static>(f: F) -> Arc<Counted<F>> {
    Arc::new(Counted {
        f,
        calls: std::sync::atomic::AtomicUsize::new(0),
    })
}

/// The digits of 1/7 in base 10, digit 0 the units.
fn seventh(i: u64) -> Result<Rational, String> {
    let cycle = [1, 4, 2, 8, 5, 7];
    Ok(Rational::from_i64(if i == 0 { 0 } else { cycle[((i - 1) % 6) as usize] }))
}

/// √2 from below within `2^-n`, by bisection on `[1, 2]`.
fn root_two(n: u64) -> Result<Rational, String> {
    let (mut lo, mut hi) = (Rational::from_i64(1), Rational::from_i64(2));
    for _ in 0..=n {
        let mid = lo.add(&hi).div(&Rational::from_i64(2)).unwrap();
        if mid.mul(&mid) <= Rational::from_i64(2) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    Ok(lo)
}

#[test]
fn a_real_defined_by_its_digits_prints_compares_and_floors_as_its_number() {
    let x = ExactReal::defined(Definition::Digits { base: 10 }, "seventh", counted(seventh));
    assert_eq!(x.to_string(), "0.142857142857142857142857142857");
    assert_eq!(x.compare(&rq(1, 8), 256).unwrap(), Ordering::Greater);
    assert_eq!(x.compare(&rq(1, 6), 256).unwrap(), Ordering::Less);
    assert_eq!(x.mul(&r(1_000_000)).floor(256).unwrap(), BigInt::from_i64(142_857));
    assert_eq!(x.describe(), "the real number whose digits in base 10 are seventh");
    assert!(!x.is_exact_form() && !x.is_transmissible());
    let err = x.compare(&rq(1, 7), 64).unwrap_err();
    assert_eq!(
        err.to_string(),
        "cannot separate the real number whose digits in base 10 are seventh and 1/7 within 2^-64; they may be equal"
    );
}

#[test]
fn a_real_defined_by_its_approximations_is_its_number_and_reads_each_precision_once() {
    let source = counted(root_two);
    let x = ExactReal::defined(Definition::Approximations, "rootTwo", source.clone());
    assert_eq!(x.to_string(), "1.41421356237309504880168872421");
    let asked = source.calls.load(std::sync::atomic::Ordering::Relaxed);
    assert_eq!(x.to_string(), "1.41421356237309504880168872421");
    assert_eq!(
        source.calls.load(std::sync::atomic::Ordering::Relaxed),
        asked,
        "a second printing reads no precision the first did not"
    );
    assert_eq!(x.mul(&x).to_decimal(20), "2.0000000000000000000");
    assert_eq!(x.sub(&rq(1414, 1000)).compare(&r(0), 256).unwrap(), Ordering::Greater);
}

#[test]
fn a_digit_outside_its_base_is_refused_naming_its_place() {
    let bits = |i: u64| Ok(Rational::from_i64(if i == 2 { 3 } else { 1 }));
    let x = ExactReal::defined(Definition::Digits { base: 2 }, "bits", counted(bits));
    let err = x.compare(&r(5), 256).unwrap_err();
    assert_eq!(
        err.to_string(),
        "digit 2 of the real number whose digits in base 2 are bits is 3: a digit in base 2 is a whole number from 0 to 1"
    );
    let halves = |_: u64| Ok(Rational::from_ratio_i64(1, 2).unwrap());
    let y = ExactReal::defined(Definition::Digits { base: 3 }, "halves", counted(halves));
    assert!(y.floor(256).unwrap_err().to_string().contains("digit 0 of the real number whose digits in base 3 are halves is 1/2"));
}

#[test]
fn approximations_further_apart_than_their_precisions_allow_are_refused_naming_both() {
    // Zero to every precision below 40 and one past it: no single answer is wrong, two are.
    let jumpy = |n: u64| Ok(Rational::from_i64(i64::from(n >= 40)));
    let x = ExactReal::defined(Definition::Approximations, "jumpy", counted(jumpy));
    let tiny = ExactReal::from_rational(Rational::new(BigInt::from_i64(1), BigInt::from_i64(1 << 60)).unwrap());
    let err = x.compare(&tiny, 256).unwrap_err().to_string();
    assert!(err.contains("jumpy approximates no one real number"), "{err}");
    assert!(err.contains("jumpy(") && err.contains("further apart than 2^-"), "{err}");
}

#[test]
fn a_function_s_own_failure_is_the_reals_failure() {
    let broken = |n: u64| Err(format!("broken at {n}"));
    let x = ExactReal::defined(Definition::Approximations, "broken", counted(broken));
    assert!(x.compare(&r(0), 256).unwrap_err().to_string().starts_with("broken at "));
}

#[test]
fn a_real_built_from_a_defined_one_cannot_travel_and_its_encoding_names_it() {
    let x = ExactReal::defined(Definition::Approximations, "rootTwo", counted(root_two));
    let y = x.add(&ExactReal::pi());
    assert!(!y.is_transmissible());
    assert!(ExactReal::pi().add(&ExactReal::e()).is_transmissible());
    let bytes = y.to_bytes();
    assert!(ExactReal::from_bytes(&bytes).is_none(), "a defined real is never rebuilt from bytes");
    let z = ExactReal::defined(Definition::Approximations, "rootTwo", counted(root_two));
    assert_eq!(x.to_bytes(), z.to_bytes(), "two definitions by one function encode alike");
}

#[test]
fn a_deep_product_of_a_general_real_refines_each_link_once_per_precision() {
    // `3 · (3 · (… · r))` forty deep: each link asks its factor for a rough magnitude and then for
    // its value, so without a shared enclosure every path to the leaf is retraced — 2^40 of them.
    let x = ExactReal::defined(Definition::Approximations, "rootTwo", counted(root_two));
    let mut a = x.clone();
    for _ in 0..40 {
        a = r(3).mul(&a);
    }
    let three_40 = BigInt::from_i64(3).pow(40);
    let expected = isqrt(&three_40.mul(&three_40).mul(&BigInt::from_i64(2)));
    assert_eq!(a.floor(4096).unwrap(), expected);
}

/// An `n`-th root is exact on an algebraic number and on a single real exponential term, whose root
/// is the root of its coefficient times the exponential of its exponent over `n`; an even root of a
/// value held exactly as negative is refused in the words every exact type uses.
#[test]
fn a_root_is_exact_on_algebraic_numbers_and_on_one_exponential_term() {
    let e = ExactReal::e();
    assert_eq!(e.nth_root(2).unwrap().to_string(), "e^(1/2)");
    assert_eq!(e.sqrt().unwrap().to_string(), "e^(1/2)", "sqrt is the second root");
    assert_eq!(r(8).mul(&r(3).exp()).nth_root(3).unwrap().to_string(), "2·e");
    assert_eq!(e.recip().unwrap().nth_root(2).unwrap().to_string(), "e^(-1/2)");
    assert!(same(&e.nth_root(2).unwrap().pow(2).unwrap(), &e), "(√e)² = e exactly");
    assert_eq!(r(-8).nth_root(3).unwrap().to_string(), "-2");
    assert_eq!(r(27).nth_root(1).unwrap().to_string(), "27");
    assert_eq!(e.neg().nth_root(3).unwrap().to_string(), "-e^(1/3)");
    assert_eq!(r(-8).nth_root(2).unwrap_err().to_string(), "square root of a negative number");
    assert_eq!(e.neg().nth_root(4).unwrap_err().to_string(), "an even root (4th) of a negative number");
}

/// A root of a value with no exact root is a computable real: the square root evaluated as one, a
/// higher root as `e^(ln|x|/n)` with the sign of `x`.
#[test]
fn a_root_of_a_transcendental_is_computed_to_any_precision() {
    let p = ExactReal::pi();
    assert!(!p.sqrt().unwrap().is_exact_form());
    let cube = p.nth_root(3).unwrap();
    assert_eq!(cube.pow(3).unwrap().to_decimal(30), p.to_decimal(30), "(∛π)³ = π");
    let negative = p.neg().nth_root(3).unwrap();
    assert_eq!(negative.to_decimal(30), cube.neg().to_decimal(30), "∛(−π) = −∛π");
    assert_eq!(p.neg().nth_root(4).unwrap_err().to_string(), "an even root (4th) of a negative number");
}
