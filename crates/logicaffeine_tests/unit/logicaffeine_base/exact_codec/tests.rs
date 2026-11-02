use super::*;
use crate::algebraic::Algebraic;
use crate::exact_real::ExactReal;
use crate::hypernatural::Hypernatural;
use crate::hyperreal::Hyperreal;
use crate::limit_value::{Depends, ResidueType};
use crate::numeric::{BigInt, Rational};
use crate::ordinal::Ordinal;
use crate::poly::Poly;
use std::cmp::Ordering;

fn round_trip<T: ExactCodec + std::fmt::Debug>(v: &T) -> T {
    let bytes = v.to_bytes();
    let back = T::from_bytes(&bytes).unwrap_or_else(|| panic!("{v:?} did not decode"));
    assert_eq!(back.to_bytes(), bytes, "{v:?}: the encoding is canonical");
    back
}

fn alg(n: i64) -> Algebraic {
    Algebraic::from_i64(n)
}

fn sqrt(n: i64) -> Algebraic {
    alg(n).sqrt().unwrap()
}

// ── round trips ───────────────────────────────────────────────────────────────────────────────

#[test]
fn integers_rationals_and_polynomials_round_trip() {
    for n in [0i64, 1, -1, 127, 128, -129, i64::MAX, i64::MIN] {
        let b = BigInt::from_i64(n);
        assert_eq!(round_trip(&b), b);
    }
    let huge = BigInt::from_i64(1).shl_bits(500).sub(&BigInt::from_i64(7));
    assert_eq!(round_trip(&huge), huge);
    assert_eq!(round_trip(&huge.negated()), huge.negated());
    for (n, d) in [(0i64, 1i64), (1, 3), (-22, 7), (10, 4)] {
        let r = Rational::from_ratio_i64(n, d).unwrap();
        assert_eq!(round_trip(&r), r);
    }
    let p = Poly::from_coeffs(vec![Rational::from_ratio_i64(1, 2).unwrap(), Rational::zero(), Rational::from_i64(-3)]);
    assert_eq!(round_trip(&p), p);
    assert_eq!(round_trip(&Poly::zero()), Poly::zero());
}

#[test]
fn algebraic_numbers_round_trip_through_their_minimal_polynomial_and_index() {
    let values = [
        alg(0),
        alg(-5),
        Algebraic::from_rational(Rational::from_ratio_i64(7, 3).unwrap()),
        sqrt(2),
        sqrt(2).neg(),
        alg(1).add(&sqrt(5)).div(&alg(2)).unwrap(),
        alg(2).nth_root(3).unwrap(),
        sqrt(2).add(&sqrt(3)),
        Algebraic::root_of(&Poly::from_i64s(&[1, 0, -10, 0, 1]), 2).unwrap(),
    ];
    for v in &values {
        let back = round_trip(v);
        assert_eq!(&back, v);
        assert_eq!(back.to_string(), v.to_string());
        assert_eq!(back.cmp(v), Ordering::Equal);
    }
}

#[test]
fn exact_reals_round_trip_in_every_form() {
    let p = ExactReal::pi();
    let e = ExactReal::e();
    let values = [
        ExactReal::from_i64(3),
        ExactReal::from_algebraic(sqrt(2)),
        e.clone(),
        ExactReal::from_i64(2).exp().add(&ExactReal::from_i64(1)),
        ExactReal::from_i64(1).cos(),
        ExactReal::from_algebraic(sqrt(2)).sin().mul(&ExactReal::from_i64(3)),
        p.clone(),
        p.mul(&p).add(&ExactReal::from_i64(1)),
        p.recip().unwrap(),
        p.add(&e),
        p.add(&e).log().unwrap().exp().sqrt().unwrap(),
        ExactReal::from_i64(2).log().unwrap(),
        p.sub(&e).recip().unwrap().sin().cos(),
    ];
    for v in &values {
        let back = round_trip(v);
        assert_eq!(back.describe(), v.describe());
        assert_eq!(back.to_decimal(25), v.to_decimal(25));
    }
}

#[test]
fn hyperreals_infinite_naturals_ordinals_and_residue_values_round_trip() {
    let dt = Hyperreal::dt();
    for h in [
        Hyperreal::from_i64(4),
        dt.clone(),
        dt.recip().unwrap(),
        Hyperreal::from_i64(1).add(&dt).div(&Hyperreal::from_i64(2).sub(&dt)).unwrap(),
        Hyperreal::from_algebraic(sqrt(2)).mul(&dt).add(&Hyperreal::from_i64(1)),
    ] {
        assert_eq!(round_trip(&h), h);
    }
    let ty = ResidueType::congruent(0, 2);
    assert_eq!(round_trip(&ty), ty);
    assert_eq!(round_trip(&ResidueType::any()), ResidueType::any());
    let any = ResidueType::any();
    let h = Hypernatural::generator(&any);
    let third = h.scaled_ceil(&Rational::from_ratio_i64(1, 3).unwrap()).unwrap();
    let square_plus_five = h.mul(&h).unwrap().add(&Hypernatural::from_integer(BigInt::from_i64(5), &any)).unwrap();
    for n in [h.clone(), third, square_plus_five] {
        assert_eq!(round_trip(&n), n);
    }
    let w = Ordinal::omega();
    for o in [Ordinal::zero(), Ordinal::from_u64(9), w.clone(), Ordinal::omega_pow(w.add(&Ordinal::one())).mul(&Ordinal::from_u64(3)).add(&w)] {
        assert_eq!(round_trip(&o), o);
    }
    let d = Depends::from_fn(4, &ResidueType::congruent(0, 2), |r| BigInt::from_u64(r * 10)).unwrap();
    assert_eq!(round_trip(&d), d);
}

// ── hostile input ─────────────────────────────────────────────────────────────────────────────

#[test]
fn truncated_and_trailing_input_is_refused() {
    let bytes = sqrt(2).to_bytes();
    for cut in 0..bytes.len() {
        assert!(Algebraic::from_bytes(&bytes[..cut]).is_none(), "a {cut}-byte prefix decoded");
    }
    let mut extra = bytes.clone();
    extra.push(0);
    assert!(Algebraic::from_bytes(&extra).is_none(), "trailing bytes are refused");
    assert!(ExactReal::from_bytes(&[0xff]).is_none(), "an unknown form tag is refused");
}

#[test]
fn non_canonical_encodings_are_refused() {
    // x² − 2 squared is not a minimal polynomial; a reducible "minimal polynomial" would break
    // the canonical name every equality test relies on.
    let mut forged = Vec::new();
    Poly::from_i64s(&[4, 0, -4, 0, 1]).write(&mut forged);
    1u64.write(&mut forged);
    assert!(Algebraic::from_bytes(&forged).is_none());
    // An index past the real roots.
    let mut out_of_range = Vec::new();
    Poly::from_i64s(&[-2, 0, 1]).write(&mut out_of_range);
    3u64.write(&mut out_of_range);
    assert!(Algebraic::from_bytes(&out_of_range).is_none());
    // An unreduced rational.
    let mut unreduced = Vec::new();
    BigInt::from_i64(2).write(&mut unreduced);
    BigInt::from_i64(4).write(&mut unreduced);
    assert!(Rational::from_bytes(&unreduced).is_none());
}

#[test]
fn a_minimal_polynomial_beyond_the_degree_bound_is_refused_before_any_work() {
    let mut c = vec![0i64; MAX_WIRE_DEGREE + 2];
    c[0] = -2;
    c[MAX_WIRE_DEGREE + 1] = 1;
    let mut big = Vec::new();
    Poly::from_i64s(&c).write(&mut big);
    1u64.write(&mut big);
    assert!(Algebraic::from_bytes(&big).is_none());
}

#[test]
fn an_infinite_integer_integral_only_on_its_admitted_classes_round_trips() {
    let even = ResidueType::congruent(0, 2);
    let half = Hypernatural::from_polynomial(
        Poly::from_coeffs(vec![Rational::zero(), Rational::from_ratio_i64(1, 2).unwrap()]),
        &even,
    )
    .unwrap();
    assert_eq!(round_trip(&half), half);
    let six = ResidueType::congruent(0, 6);
    let third = Hypernatural::generator(&six)
        .scaled_ceil(&Rational::from_ratio_i64(1, 3).unwrap())
        .unwrap();
    assert_eq!(round_trip(&third), third);
}
