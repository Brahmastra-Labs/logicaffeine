use super::*;
use crate::hyperreal::Hyperreal;
use crate::limit_value::{ResidueError, ResidueType, MAX_MODULUS};
use std::cmp::Ordering;

fn qr(n: i64, d: i64) -> Rational {
    Rational::from_ratio_i64(n, d).unwrap()
}

fn any() -> ResidueType {
    ResidueType::any()
}

fn big_h(ty: &ResidueType) -> Hypernatural {
    Hypernatural::generator(ty)
}

fn n(k: i64, ty: &ResidueType) -> Hypernatural {
    Hypernatural::from_integer(BigInt::from_i64(k), ty)
}

fn k(v: u64) -> BigInt {
    BigInt::from_u64(v)
}

// ── order ─────────────────────────────────────────────────────────────────────────────────────

#[test]
fn the_generator_exceeds_every_standard_natural() {
    let ty = any();
    let h = big_h(&ty);
    for k in [0i64, 1, 1_000, 1_000_000_000_000_000_000] {
        assert_eq!(h.compare(&n(k, &ty)).unwrap().single(), Some(&Ordering::Greater), "H > {k}");
    }
    assert_eq!(h.add(&n(1, &ty)).unwrap().compare(&h).unwrap().single(), Some(&Ordering::Greater));
    assert_eq!(
        h.mul(&n(2, &ty)).unwrap().compare(&h.add(&n(1000, &ty)).unwrap()).unwrap().single(),
        Some(&Ordering::Greater)
    );
    assert_eq!(h.compare(&h).unwrap().single(), Some(&Ordering::Equal));
    assert_eq!(h.is_infinite().single(), Some(&true));
    assert_eq!(h.sub(&h).unwrap().add(&n(5, &ty)).unwrap().is_infinite().single(), Some(&false));
    assert_eq!(n(5, &ty).sub(&h).unwrap().is_negative().single(), Some(&true), "5 − H is a negative infinite integer");
}

// ── residues ──────────────────────────────────────────────────────────────────────────────────

#[test]
fn residues_of_polynomials_in_the_generator() {
    let ty = any();
    let h = big_h(&ty);
    let parity = h.rem(&k(2)).unwrap();
    assert_eq!(parity.modulus(), 2);
    assert_eq!(parity.at(7), Some(&BigInt::from_i64(1)));
    assert_eq!(
        h.mul(&n(2, &ty)).unwrap().add(&n(1, &ty)).unwrap().rem(&k(2)).unwrap().single(),
        Some(&BigInt::from_i64(1))
    );
    assert_eq!(h.mul(&h).unwrap().rem(&k(2)).unwrap(), parity, "H² has the parity of H");
    let triangular = h.mul(&h.add(&n(1, &ty)).unwrap()).unwrap().scaled_floor(&qr(1, 2)).unwrap();
    let t2 = triangular.rem(&k(2)).unwrap();
    assert_eq!(t2.modulus(), 4);
    for x in 0..12u64 {
        assert_eq!(t2.at(x), Some(&BigInt::from_u64((x * (x + 1) / 2) % 2)), "H = {x}");
    }
    let even = ResidueType::congruent(0, 2);
    assert_eq!(big_h(&even).rem(&k(2)).unwrap().single(), Some(&BigInt::from_i64(0)));
    assert_eq!(
        big_h(&even).rem(&k(4)).unwrap().render("H", |x| x.to_string()),
        "0 when H ≡ 0 (mod 4), 2 when H ≡ 2 (mod 4)"
    );
}

#[test]
fn a_rational_multiple_rounds_by_residue() {
    let ty = any();
    let third = big_h(&ty).scaled_ceil(&qr(1, 3)).unwrap();
    assert_eq!(third.render("H"), "(1/3)·H when H ≡ 0 (mod 3), (1/3)·H + 2/3 when H ≡ 1 (mod 3), (1/3)·H + 1/3 when H ≡ 2 (mod 3)");
    for x in [3u64, 4, 5, 300, 301, 302] {
        let want = x.div_ceil(3);
        assert_eq!(third.eval_at(x), Some(BigInt::from_u64(want)), "⌈{x}/3⌉");
    }
    let six = ResidueType::congruent(0, 6);
    assert_eq!(big_h(&six).scaled_ceil(&qr(1, 3)).unwrap().render("H"), "(1/3)·H");
    let floor = big_h(&ty).scaled_floor(&qr(2, 5)).unwrap();
    for x in [10u64, 11, 12, 13, 14] {
        assert_eq!(floor.eval_at(x), Some(BigInt::from_u64(2 * x / 5)));
    }
}

// ── to hyperreals and display ─────────────────────────────────────────────────────────────────

#[test]
fn the_generator_is_the_reciprocal_of_the_infinitesimal() {
    let ty = any();
    let h = big_h(&ty);
    assert_eq!(h.to_hyperreal().single(), Some(&Hyperreal::dt().recip().unwrap()));
    assert_eq!(
        h.mul(&n(2, &ty)).unwrap().add(&n(1, &ty)).unwrap().to_hyperreal().single(),
        Some(&Hyperreal::from_i64(2).div(&Hyperreal::dt()).unwrap().add(&Hyperreal::from_i64(1)))
    );
    assert_eq!(h.to_hyperreal().single().unwrap().mul(&Hyperreal::dt()), Hyperreal::from_i64(1), "H·dt = 1");
}

#[test]
fn hypernaturals_print_as_polynomials_in_the_generator() {
    let ty = any();
    let h = big_h(&ty);
    assert_eq!(h.render("H"), "H");
    let two_h_plus_one = h.mul(&n(2, &ty)).unwrap().add(&n(1, &ty)).unwrap();
    assert_eq!(two_h_plus_one.render("H"), "2·H + 1");
    assert_eq!(h.mul(&h).unwrap().add(&h.mul(&n(3, &ty)).unwrap()).unwrap().render("H"), "H^2 + 3·H");
    assert_eq!(n(5, &ty).sub(&h).unwrap().render("H"), "-H + 5");
    assert_eq!(h.render_reciprocal("dt"), "1/dt");
    assert_eq!(two_h_plus_one.render_reciprocal("dt"), "2/dt + 1");
    assert_eq!(h.mul(&h).unwrap().render_reciprocal("dt"), "1/dt^2");
    assert_eq!(n(7, &ty).render("H"), "7");
}

#[test]
fn fractional_coefficients_print_over_the_infinitesimal() {
    let six = ResidueType::congruent(0, 6);
    assert_eq!(big_h(&six).scaled_ceil(&qr(1, 3)).unwrap().render_reciprocal("dt"), "1/(3·dt)");
    let ty = any();
    assert_eq!(
        big_h(&ty).scaled_ceil(&qr(1, 3)).unwrap().render_reciprocal("dt"),
        "1/(3·dt) when ⌈1/dt⌉ ≡ 0 (mod 3), 1/(3·dt) + 2/3 when ⌈1/dt⌉ ≡ 1 (mod 3), 1/(3·dt) + 1/3 when ⌈1/dt⌉ ≡ 2 (mod 3)"
    );
}

// ── residue periods ───────────────────────────────────────────────────────────────────────────

/// `C(H, j)` built as `H(H − 1)…(H − j + 1) / j!`.
fn binomial(j: u64, ty: &ResidueType) -> Hypernatural {
    let h = big_h(ty);
    let mut falling = n(1, ty);
    for i in 0..j {
        falling = falling.mul(&h.sub(&n(i as i64, ty)).unwrap()).unwrap();
    }
    let factorial = (1..=j).fold(k(1), |acc, i| acc.mul(&k(i)));
    falling
        .scaled_floor(&Rational::new(k(1), factorial).unwrap())
        .unwrap()
}

#[test]
fn a_high_power_of_the_generator_reads_its_residue_from_a_short_period() {
    let ty = any();
    let h = big_h(&ty);
    let mut p = h.clone();
    for _ in 1..25 {
        p = p.mul(&h).unwrap();
    }
    assert_eq!(p.rem(&k(2)).unwrap(), h.rem(&k(2)).unwrap(), "H^25 has the parity of H");
    assert_eq!(p.rem(&k(3)).unwrap(), h.rem(&k(3)).unwrap(), "x^25 ≡ x (mod 3) by Fermat");
    assert_eq!(p.rem(&k(5)).unwrap(), h.rem(&k(5)).unwrap(), "x^25 ≡ x (mod 5) by Fermat");
    let mut p64 = h.clone();
    for _ in 1..64 {
        p64 = p64.mul(&h).unwrap();
    }
    let parity = p64.rem(&k(2)).unwrap();
    assert_eq!(parity.modulus(), 2, "H^64 needs no 64! classes");
    let mod_four = p64.rem(&k(4)).unwrap();
    for x in 0..8u64 {
        let want = if x % 2 == 0 { 0 } else { 1 };
        assert_eq!(mod_four.at(x), Some(&k(want)), "{x}^64 mod 4");
    }
}

#[test]
fn a_binomial_in_the_generator_has_the_period_lucas_gives_it() {
    let ty = any();
    let c = binomial(25, &ty);
    let parity = c.rem(&k(2)).unwrap();
    assert_eq!(parity.modulus(), 32, "C(H, 25) mod 2 reads the low five bits of H");
    for x in 0..96u64 {
        assert_eq!(parity.at(x), Some(&k(u64::from(x & 25 == 25))), "C({x}, 25) is odd exactly when 25 ⊆ {x}");
    }
    assert_eq!(c.eval_at(30), Some(k(142_506)), "C(30, 25)");
    let c4 = binomial(4, &ty);
    let mod_three = c4.rem(&k(3)).unwrap();
    assert_eq!(mod_three.modulus(), 9, "C(H, 4) mod 3 has period 3^(1 + ⌊log₃ 4⌋)");
    for x in 0..27u64 {
        let want = (x * x.saturating_sub(1) * x.saturating_sub(2) * x.saturating_sub(3) / 24) % 3;
        assert_eq!(mod_three.at(x), Some(&k(want)), "C({x}, 4) mod 3");
    }
}

#[test]
fn a_multiple_of_the_modulus_has_no_residue_to_read() {
    let ty = any();
    let h = big_h(&ty);
    let mut falling = n(1, &ty);
    for i in 0..25 {
        falling = falling.mul(&h.sub(&n(i, &ty)).unwrap()).unwrap();
    }
    let factorial = (1..=25u64).fold(k(1), |acc, i| acc.mul(&k(i)));
    assert_eq!(falling.rem(&factorial).unwrap().single(), Some(&k(0)), "25! divides every product of 25 consecutive integers");
    assert_eq!(h.mul(&n(6, &ty)).unwrap().rem(&k(3)).unwrap().single(), Some(&k(0)));
}

#[test]
fn residues_track_the_class_each_branch_lives_on() {
    let ty = any();
    let third = big_h(&ty).scaled_ceil(&qr(1, 3)).unwrap();
    let parity = third.rem(&k(2)).unwrap();
    for x in 0..60u64 {
        assert_eq!(parity.at(x), Some(&k(x.div_ceil(3) % 2)), "⌈{x}/3⌉ mod 2");
    }
    let ninth = big_h(&ty).scaled_floor(&qr(1, 9)).unwrap();
    let sq = ninth.mul(&ninth).unwrap();
    let r = sq.rem(&k(4)).unwrap();
    for x in 0..200u64 {
        assert_eq!(r.at(x), Some(&k((x / 9) * (x / 9) % 4)), "⌊{x}/9⌋² mod 4");
    }
}

#[test]
fn a_residue_finer_than_the_bound_is_refused_not_built() {
    let ty = any();
    let h = big_h(&ty);
    assert_eq!(
        h.rem(&k(1_000_003)),
        Err(ResidueError::TooFine {
            classes: k(1_000_003)
        })
    );
    assert_eq!(h.rem(&k(MAX_MODULUS)).unwrap().modulus(), MAX_MODULUS);
    let huge = k(1).shl_bits(80);
    assert!(matches!(
        h.scaled_floor(&Rational::new(k(1), huge.clone()).unwrap()),
        Err(ResidueError::TooFine { .. })
    ));
    assert!(matches!(h.rem(&huge), Err(ResidueError::TooFine { .. })), "a modulus past u64 is refused, not truncated");
    let p = big_h(&ResidueType::congruent(0, 65_521));
    let q = big_h(&ResidueType::congruent(0, 65_519));
    assert!(matches!(p.add(&q), Err(ResidueError::TooFine { .. })));
}

// ── powers ────────────────────────────────────────────────────────────────────────────────────

#[test]
fn a_power_of_an_infinite_integer_squares_its_way_up_and_stops_at_the_bound() {
    use crate::hyperreal::{PowerError, MAX_POWER_DEGREE};
    let ty = any();
    let h = big_h(&ty);
    assert_eq!(h.degree(), 1);
    assert_eq!(n(4, &ty).degree(), 0);
    assert_eq!(h.mul(&h).unwrap().add(&h).unwrap().degree(), 2);
    assert_eq!(h.pow(3).unwrap(), h.mul(&h).unwrap().mul(&h).unwrap());
    assert_eq!(h.pow(0).unwrap(), n(1, &ty));
    assert_eq!(h.pow(MAX_POWER_DEGREE).unwrap().degree(), MAX_POWER_DEGREE as usize);
    assert_eq!(n(3, &ty).pow(40).unwrap().eval_at(0), Some(BigInt::from_i64(3).pow(40)), "a standard base has no degree to bound");
    assert_eq!(h.pow(u64::MAX), Err(PowerError::DegreeTooHigh { degree: u64::MAX as u128 }), "refused before any work");
    assert_eq!(h.mul(&h).unwrap().pow(3000), Err(PowerError::DegreeTooHigh { degree: 6000 }));
    let third = big_h(&ty).scaled_floor(&qr(1, 3)).unwrap();
    let cube = third.pow(3).unwrap();
    for x in [0u64, 1, 2, 3, 4, 5, 299, 300, 301] {
        assert_eq!(cube.eval_at(x), Some(BigInt::from_u64((x / 3).pow(3))), "⌊{x}/3⌋³");
    }
}

#[test]
fn an_infinite_integer_of_any_degree_becomes_a_hyperreal() {
    use crate::hyperreal::MAX_POWER_DEGREE;
    let ty = any();
    let h = big_h(&ty);
    let mut high = n(1, &ty);
    for _ in 0..=MAX_POWER_DEGREE {
        high = high.mul(&h).unwrap();
    }
    let as_hyper = high.to_hyperreal();
    let x = as_hyper.single().unwrap();
    assert_eq!(x.valuation(), Some(-(MAX_POWER_DEGREE as i64) - 1), "built step by step, past the bound one power may reach");
    assert_eq!(x.leading_coefficient(), Some(crate::algebraic::Algebraic::from_i64(1)));
}

#[test]
fn an_integer_valued_polynomial_on_every_admitted_class_is_an_infinite_integer() {
    let ty = any();
    let triangular = Poly::from_coeffs(vec![qr(0, 1), qr(1, 2), qr(1, 2)]);
    let t = Hypernatural::from_polynomial(triangular, &ty).unwrap();
    assert_eq!(t.eval_at(4), Some(k(10)));
    assert_eq!(t, big_h(&ty).mul(&big_h(&ty).add(&n(1, &ty)).unwrap()).unwrap().scaled_floor(&qr(1, 2)).unwrap());
    let half = Poly::from_coeffs(vec![qr(0, 1), qr(1, 2)]);
    assert_eq!(Hypernatural::from_polynomial(half.clone(), &ty), None, "H/2 is not an integer when H is odd");
    let even = ResidueType::congruent(0, 2);
    let h_half = Hypernatural::from_polynomial(half.clone(), &even).unwrap();
    assert_eq!(h_half.eval_at(6), Some(k(3)));
    assert_eq!(Hypernatural::from_polynomial(half, &ResidueType::congruent(1, 2)), None);
    assert_eq!(Hypernatural::from_polynomial(Poly::constant(qr(1, 3)), &ty), None);
    assert_eq!(Hypernatural::from_polynomial(Poly::constant(qr(7, 1)), &ty), Some(n(7, &ty)));
    let sixth = Poly::from_coeffs(vec![qr(0, 1), qr(1, 6)]);
    assert!(Hypernatural::from_polynomial(sixth.clone(), &ResidueType::congruent(0, 6)).is_some());
    assert!(Hypernatural::from_polynomial(sixth, &ResidueType::congruent(0, 3)).is_none());
}

#[test]
fn a_residue_is_read_only_along_the_classes_the_type_admits() {
    let even = ResidueType::congruent(0, 2);
    let half = Hypernatural::from_polynomial(Poly::from_coeffs(vec![qr(0, 1), qr(1, 2)]), &even).unwrap();
    let parity = half.rem(&k(2)).unwrap();
    for x in (0..40u64).step_by(2) {
        assert_eq!(parity.at(x), Some(&k((x / 2) % 2)), "{x}/2 mod 2");
    }
    assert_eq!(parity.at(3), None, "an odd H is not admitted");
    let six = ResidueType::congruent(0, 6);
    let third = big_h(&six).scaled_ceil(&qr(1, 3)).unwrap();
    assert_eq!(third.rem(&k(2)).unwrap().single(), Some(&k(0)), "H/3 = 2j for H = 6j");
    assert_eq!(third.rem(&k(4)).unwrap().render("H", |x| x.to_string()), "0 when H ≡ 0 (mod 4), 2 when H ≡ 2 (mod 4)", "H = 12i gives 0, H = 12i + 6 gives 2, and mod 4 already tells them apart");
}

// ── floors of hyperreals ──────────────────────────────────────────────────────────────────────

fn hy(n: i64) -> Hyperreal {
    Hyperreal::from_i64(n)
}

fn floor(x: &Hyperreal, ty: &ResidueType) -> Hypernatural {
    Hypernatural::floor_of(x, ty).unwrap()
}

#[test]
fn the_floor_of_a_limited_hyperreal_is_a_standard_integer() {
    let ty = any();
    let dt = Hyperreal::dt();
    assert_eq!(floor(&dt, &ty), n(0, &ty));
    assert_eq!(floor(&dt.neg(), &ty), n(-1, &ty), "just below zero");
    assert_eq!(floor(&hy(3).add(&dt), &ty), n(3, &ty));
    assert_eq!(floor(&hy(3).sub(&dt), &ty), n(2, &ty), "just below three");
    assert_eq!(floor(&hy(3), &ty), n(3, &ty));
    let root2 = Hyperreal::from_algebraic(crate::algebraic::Algebraic::from_i64(2).sqrt().unwrap());
    assert_eq!(floor(&root2.sub(&dt), &ty), n(1, &ty), "no infinitesimal crosses an irrational");
    assert_eq!(floor(&Hyperreal::from_rational(qr(-7, 2)), &ty), n(-4, &ty));
}

#[test]
fn the_floor_of_an_infinite_hyperreal_is_an_infinite_integer() {
    let ty = any();
    let dt = Hyperreal::dt();
    let inv = dt.recip().unwrap();
    assert_eq!(floor(&inv, &ty), big_h(&ty));
    assert_eq!(floor(&inv.add(&Hyperreal::from_rational(qr(1, 2))), &ty), big_h(&ty));
    assert_eq!(floor(&inv.sub(&dt), &ty), big_h(&ty).sub(&n(1, &ty)).unwrap(), "H − dt is just below H");
    let half = inv.mul(&Hyperreal::from_rational(qr(1, 2)));
    assert_eq!(floor(&half, &ty), big_h(&ty).scaled_floor(&qr(1, 2)).unwrap());
    let series = hy(1).div(&dt.add(&dt.mul(&dt))).unwrap();
    assert_eq!(floor(&series, &ty), big_h(&ty).sub(&n(1, &ty)).unwrap(), "H²/(H + 1) = H − 1 + 1/(H + 1)");
    let even = ResidueType::congruent(0, 2);
    assert_eq!(floor(&half, &even).depends().modulus(), 1, "H/2 is whole once H is even");
}

#[test]
fn the_ceiling_mirrors_the_floor() {
    let ty = any();
    let dt = Hyperreal::dt();
    let third = dt.recip().unwrap().mul(&Hyperreal::from_rational(qr(1, 3)));
    assert_eq!(Hypernatural::ceil_of(&third, &ty).unwrap(), big_h(&ty).scaled_ceil(&qr(1, 3)).unwrap());
    assert_eq!(Hypernatural::ceil_of(&hy(3).sub(&dt), &ty).unwrap(), n(3, &ty));
    assert_eq!(Hypernatural::ceil_of(&hy(3).add(&dt), &ty).unwrap(), n(4, &ty));
}

#[test]
fn an_irrational_multiple_of_the_generator_has_no_polynomial_floor() {
    let ty = any();
    let root2 = Hyperreal::from_algebraic(crate::algebraic::Algebraic::from_i64(2).sqrt().unwrap());
    let x = root2.div(&Hyperreal::dt()).unwrap();
    assert_eq!(Hypernatural::floor_of(&x, &ty), Err(FloorError::Irrational));
    assert_eq!(
        FloorError::Irrational.describe("dt"),
        "the integer part of a value with an irrational multiple of a power of 1/dt is no polynomial in 1/dt"
    );
}

#[test]
fn a_dependence_of_polynomials_is_an_infinite_integer_when_each_is_integral_on_its_class() {
    let ty = any();
    let third = big_h(&ty).scaled_ceil(&qr(1, 3)).unwrap();
    assert_eq!(Hypernatural::from_branches(third.depends().clone()), Some(third));
    let half = crate::limit_value::Depends::constant(Poly::from_coeffs(vec![qr(0, 1), qr(1, 2)]), &ty);
    assert_eq!(Hypernatural::from_branches(half), None);
}

// ── integer-valued polynomials ────────────────────────────────────────────────────────────────

/// Whether `p` is an integer at `first + step·t` for every `t` below `count`, by evaluating it.
fn integer_at_each(p: &Poly, first: u64, step: u64, count: u64) -> bool {
    (0..count).all(|t| p.eval(&Rational::from_bigint(k(first + step * t))).is_integer())
}

/// The residue test agrees with evaluation on every small polynomial with denominators up to 12 and
/// every progression it is asked about: integer-valued exactly where every member, far past the
/// degree, takes an integer.
#[test]
fn a_polynomial_is_integer_valued_by_its_residues_exactly_where_its_values_are_integers() {
    let denominators = [1i64, 2, 3, 4, 6, 12];
    let mut checked = 0;
    for &d in &denominators {
        for a in -3i64..=3 {
            for b in -3i64..=3 {
                for c in -2i64..=2 {
                    let p = Poly::from_coeffs(vec![qr(c, d), qr(b, d), qr(a, d)]);
                    for (first, step) in [(0u64, 1u64), (1, 2), (0, 2), (2, 3), (5, 6), (3, 4)] {
                        assert_eq!(
                            integer_on(&p, first, step),
                            integer_at_each(&p, first, step, 60),
                            "{p:?} on {first} + {step}·t"
                        );
                        checked += 1;
                    }
                }
            }
        }
    }
    assert_eq!(checked, 6 * 7 * 7 * 5 * 6);
}

#[test]
fn the_classic_integer_valued_polynomials_are_read_as_integers() {
    let any = any();
    let even = ResidueType::congruent(0, 2);
    let h_choose_2 = Poly::from_coeffs(vec![qr(0, 1), qr(-1, 2), qr(1, 2)]);
    assert!(Hypernatural::from_polynomial(h_choose_2, &any).is_some(), "H(H − 1)/2");
    let h_cubed_minus_h_over_6 = Poly::from_coeffs(vec![qr(0, 1), qr(-1, 6), qr(0, 1), qr(1, 6)]);
    assert!(Hypernatural::from_polynomial(h_cubed_minus_h_over_6, &any).is_some(), "(H³ − H)/6");
    let half = Poly::from_coeffs(vec![qr(0, 1), qr(1, 2)]);
    assert!(Hypernatural::from_polynomial(half.clone(), &any).is_none(), "H/2 for any H");
    assert!(Hypernatural::from_polynomial(half, &even).is_some(), "H/2 for an even H");
    let third_plus = Poly::from_coeffs(vec![qr(1, 3), qr(1, 3)]);
    assert!(Hypernatural::from_polynomial(third_plus.clone(), &ResidueType::congruent(2, 3)).is_some(), "(H + 1)/3 for H ≡ 2");
    assert!(Hypernatural::from_polynomial(third_plus, &ResidueType::congruent(1, 3)).is_none(), "(H + 1)/3 for H ≡ 1");
}

/// A polynomial with integer coefficients is an integer everywhere at once, whatever its degree: `H^1000`
/// is read in no time, where evaluating it at a thousand and one points took seconds.
#[test]
fn a_high_power_with_integer_coefficients_is_read_at_once() {
    let mut c = vec![qr(0, 1); 1001];
    c[1000] = qr(1, 1);
    let start = std::time::Instant::now();
    assert!(Hypernatural::from_polynomial(Poly::from_coeffs(c), &any()).is_some());
    assert!(start.elapsed() < std::time::Duration::from_secs(1), "took {:?}", start.elapsed());
    let mut halves = vec![qr(0, 1); 401];
    halves[400] = qr(1, 2);
    halves[399] = qr(1, 2);
    let start = std::time::Instant::now();
    assert!(Hypernatural::from_polynomial(Poly::from_coeffs(halves), &any()).is_some(), "(H^400 + H^399)/2");
    assert!(start.elapsed() < std::time::Duration::from_secs(1), "took {:?}", start.elapsed());
}
