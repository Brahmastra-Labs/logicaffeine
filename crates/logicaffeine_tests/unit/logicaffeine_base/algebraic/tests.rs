use super::*;
use crate::poly::Poly;
use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

fn q(n: i64) -> Rational {
    Rational::from_i64(n)
}

fn qr(n: i64, d: i64) -> Rational {
    Rational::from_ratio_i64(n, d).unwrap()
}

fn a(n: i64) -> Algebraic {
    Algebraic::from_i64(n)
}

fn ar(n: i64, d: i64) -> Algebraic {
    Algebraic::from_rational(qr(n, d))
}

fn sqrt(n: i64) -> Algebraic {
    a(n).sqrt().unwrap()
}

fn phi() -> Algebraic {
    a(1).add(&sqrt(5)).div(&a(2)).unwrap()
}

fn hash_of(x: &Algebraic) -> u64 {
    let mut h = DefaultHasher::new();
    x.hash(&mut h);
    h.finish()
}

fn xorshift(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}

/// A small corpus mixing rationals, square roots and their combinations.
fn corpus() -> Vec<Algebraic> {
    vec![
        a(0),
        a(1),
        a(-3),
        ar(7, 2),
        ar(-5, 3),
        sqrt(2),
        sqrt(3).neg(),
        sqrt(5),
        phi(),
        a(2).nth_root(3).unwrap(),
        sqrt(2).add(&a(1)),
        sqrt(3).div(&a(2)).unwrap(),
    ]
}

// ── rationals are degree-1 algebraic numbers ──────────────────────────────────────────────────

#[test]
fn rationals_have_linear_minimal_polynomials_and_print_as_rationals() {
    assert_eq!(a(5).degree(), 1);
    assert_eq!(a(5).to_string(), "5");
    assert_eq!(a(-5).to_string(), "-5");
    assert_eq!(ar(3, 2).to_string(), "3/2");
    assert_eq!(ar(-3, 2).to_string(), "-3/2");
    assert_eq!(a(0).to_string(), "0");
    assert_eq!(ar(3, 2).minimal_polynomial(), &Poly::from_i64s(&[-3, 2]));
    assert_eq!(ar(3, 2).to_rational(), Some(qr(3, 2)));
    assert_eq!(sqrt(2).to_rational(), None);
}

#[test]
fn rational_arithmetic_through_algebraic_matches_rational_arithmetic() {
    let mut st = 0x1357_9bdf_2468_ace0;
    for _ in 0..200 {
        let x = qr((xorshift(&mut st) % 41) as i64 - 20, 1 + (xorshift(&mut st) % 9) as i64);
        let y = qr((xorshift(&mut st) % 41) as i64 - 20, 1 + (xorshift(&mut st) % 9) as i64);
        let (ax, ay) = (Algebraic::from_rational(x.clone()), Algebraic::from_rational(y.clone()));
        assert_eq!(ax.add(&ay).to_rational(), Some(x.add(&y)));
        assert_eq!(ax.sub(&ay).to_rational(), Some(x.sub(&y)));
        assert_eq!(ax.mul(&ay).to_rational(), Some(x.mul(&y)));
        assert_eq!(ax.div(&ay).map(|v| v.to_rational()), x.div(&y).map(Some));
        assert_eq!(ax.cmp(&ay), x.cmp(&y));
    }
}

// ── square roots and display ──────────────────────────────────────────────────────────────────

#[test]
fn square_roots_print_in_simplified_radical_form() {
    assert_eq!(sqrt(2).to_string(), "√2");
    assert_eq!(sqrt(2).neg().to_string(), "-√2");
    assert_eq!(sqrt(8).to_string(), "2√2");
    assert_eq!(sqrt(12).neg().to_string(), "-2√3");
    assert_eq!(ar(1, 2).sqrt().unwrap().to_string(), "√2/2");
    assert_eq!(sqrt(2).recip().unwrap().to_string(), "√2/2");
    assert_eq!(ar(3, 4).sqrt().unwrap().to_string(), "√3/2");
    assert_eq!(sqrt(9).to_string(), "3");
    assert_eq!(sqrt(0).to_string(), "0");
    assert_eq!(sqrt(18).div(&a(4)).unwrap().to_string(), "3√2/4");
}

#[test]
fn quadratic_irrationals_with_a_rational_part_print_as_a_fraction_of_the_radical() {
    assert_eq!(phi().to_string(), "(1 + √5)/2");
    assert_eq!(a(1).sub(&sqrt(5)).div(&a(2)).unwrap().to_string(), "(1 - √5)/2");
    assert_eq!(a(1).add(&sqrt(2)).to_string(), "1 + √2");
    assert_eq!(a(-1).sub(&sqrt(3)).to_string(), "-1 - √3");
    assert_eq!(a(3).sub(&sqrt(8)).to_string(), "3 - 2√2");
    assert_eq!(a(2).add(&sqrt(3)).div(&a(3)).unwrap().to_string(), "(2 + √3)/3");
}

#[test]
fn higher_degree_numbers_print_as_an_indexed_root_of_their_minimal_polynomial() {
    let plastic = Algebraic::root_of(&Poly::from_i64s(&[-1, -1, 0, 1]), 1).unwrap();
    assert_eq!(plastic.to_string(), "root 1 of x^3 - x - 1");
    assert_eq!(a(2).nth_root(3).unwrap().to_string(), "root 1 of x^3 - 2");
    let sd = sqrt(2).add(&sqrt(3));
    assert_eq!(sd.minimal_polynomial(), &Poly::from_i64s(&[1, 0, -10, 0, 1]));
    assert_eq!(sd.to_string(), "root 4 of x^4 - 10x^2 + 1");
    assert_eq!(sd.neg().to_string(), "root 1 of x^4 - 10x^2 + 1");
}

#[test]
fn root_of_picks_the_indexed_real_root_and_cuts_to_the_minimal_polynomial() {
    let f = Poly::from_i64s(&[-2, 0, 1]).mul(&Poly::from_i64s(&[-3, 1]));
    assert_eq!(Algebraic::root_of(&f, 1).unwrap(), sqrt(2).neg());
    assert_eq!(Algebraic::root_of(&f, 2).unwrap(), sqrt(2));
    assert_eq!(Algebraic::root_of(&f, 3).unwrap(), a(3));
    assert!(Algebraic::root_of(&f, 4).is_none());
    assert!(Algebraic::root_of(&f, 0).is_none());
    assert!(Algebraic::root_of(&Poly::from_i64s(&[1, 0, 1]), 1).is_none(), "x² + 1 has no real root");
    assert!(Algebraic::root_of(&Poly::zero(), 1).is_none());
    let repeated = Poly::from_i64s(&[-2, 0, 1]).pow(3);
    assert_eq!(Algebraic::root_of(&repeated, 2).unwrap(), sqrt(2));
}

// ── canonical identity ────────────────────────────────────────────────────────────────────────

#[test]
fn equal_values_reached_by_different_routes_are_equal_and_hash_alike() {
    let routes = [
        sqrt(2),
        sqrt(8).div(&a(2)).unwrap(),
        a(2).div(&sqrt(2)).unwrap(),
        sqrt(2).mul(&sqrt(3)).div(&sqrt(3)).unwrap(),
        a(1).add(&sqrt(2)).sub(&a(1)),
        a(2).nth_root(4).unwrap().pow(2).unwrap(),
    ];
    for r in &routes {
        assert_eq!(r, &routes[0]);
        assert_eq!(hash_of(r), hash_of(&routes[0]));
        assert_eq!(r.to_string(), "√2");
    }
}

#[test]
fn exact_identities_hold() {
    assert_eq!(sqrt(2).mul(&sqrt(2)), a(2));
    assert_eq!(sqrt(2).mul(&sqrt(2)).degree(), 1);
    assert_eq!(phi().mul(&phi()), phi().add(&a(1)), "φ² = φ + 1");
    let s = sqrt(2).add(&sqrt(3));
    assert_eq!(s.mul(&s), a(5).add(&a(2).mul(&sqrt(6))));
    assert_eq!(sqrt(2).pow(10).unwrap(), a(32));
    assert_eq!(sqrt(2).pow(-2).unwrap(), ar(1, 2));
    assert_eq!(sqrt(3).sub(&sqrt(3)), a(0));
    assert_eq!(a(-8).nth_root(3).unwrap(), a(-2));
    assert_eq!(a(2).nth_root(3).unwrap().pow(3).unwrap(), a(2));
}

#[test]
fn powers_of_the_golden_ratio_follow_fibonacci() {
    let mut fib = vec![0i64, 1];
    for i in 2..30 {
        fib.push(fib[i - 1] + fib[i - 2]);
    }
    for n in 2..30usize {
        let want = phi().mul(&a(fib[n])).add(&a(fib[n - 1]));
        assert_eq!(phi().pow(n as i32).unwrap(), want, "φ^{n} = F(n)φ + F(n−1)");
    }
}

// ── field laws ────────────────────────────────────────────────────────────────────────────────

#[test]
fn the_field_laws_hold_on_the_corpus() {
    let c = corpus();
    for x in &c {
        assert_eq!(x.add(&a(0)), *x);
        assert_eq!(x.mul(&a(1)), *x);
        assert_eq!(x.sub(x), a(0));
        assert_eq!(x.neg().neg(), *x);
        if !x.is_zero() {
            assert_eq!(x.div(x).unwrap(), a(1));
            assert_eq!(x.recip().unwrap().recip().unwrap(), *x);
        } else {
            assert!(x.recip().is_none());
            assert!(a(1).div(x).is_none());
        }
    }
    for x in c.iter().take(8) {
        for y in c.iter().take(8) {
            assert_eq!(x.add(y), y.add(x));
            assert_eq!(x.mul(y), y.mul(x));
            assert_eq!(x.add(y).sub(y), *x);
        }
    }
    let (x, y, z) = (sqrt(2), phi(), ar(-5, 3));
    assert_eq!(x.add(&y).add(&z), x.add(&y.add(&z)));
    assert_eq!(x.mul(&y).mul(&z), x.mul(&y.mul(&z)));
    assert_eq!(x.mul(&y.add(&z)), x.mul(&y).add(&x.mul(&z)));
}

// ── order ─────────────────────────────────────────────────────────────────────────────────────

#[test]
fn the_order_agrees_with_the_sign_of_the_difference_and_with_floating_point() {
    let c = corpus();
    for x in &c {
        for y in &c {
            let by_sign = x.sub(y).signum();
            let want = match by_sign {
                -1 => Ordering::Less,
                0 => Ordering::Equal,
                _ => Ordering::Greater,
            };
            assert_eq!(x.cmp(y), want, "{x} vs {y}");
            let (fx, fy) = (x.to_f64(), y.to_f64());
            if (fx - fy).abs() > 1e-9 {
                assert_eq!(x.cmp(y), fx.partial_cmp(&fy).unwrap(), "{x} vs {y}");
            }
        }
    }
}

#[test]
fn close_neighbours_are_ordered_exactly() {
    assert!(sqrt(2) < ar(1_414_214, 1_000_000));
    assert!(sqrt(2) > ar(1_414_213, 1_000_000));
    assert!(sqrt(2) < ar(3, 2) && ar(3, 2) < sqrt(3));
    assert!(sqrt(2).add(&sqrt(3)) < sqrt(10));
    assert!(sqrt(2).add(&sqrt(3)) > ar(314, 100));
    let tiny = sqrt(2).sub(&ar(1_414_213_562_373, 1_000_000_000_000));
    assert_eq!(tiny.signum(), 1);
}

#[test]
fn floor_ceil_and_signum_are_exact() {
    assert_eq!(sqrt(2).floor(), BigInt::from_i64(1));
    assert_eq!(sqrt(2).ceil(), BigInt::from_i64(2));
    assert_eq!(sqrt(2).neg().floor(), BigInt::from_i64(-2));
    assert_eq!(sqrt(2).neg().ceil(), BigInt::from_i64(-1));
    assert_eq!(phi().floor(), BigInt::from_i64(1));
    assert_eq!(ar(7, 2).floor(), BigInt::from_i64(3));
    assert_eq!(a(4).floor(), BigInt::from_i64(4));
    assert_eq!(a(4).ceil(), BigInt::from_i64(4));
    assert_eq!(sqrt(1_000_001).floor(), BigInt::from_i64(1000));
    assert_eq!(a(0).signum(), 0);
    assert_eq!(sqrt(3).neg().signum(), -1);
}

#[test]
fn to_f64_is_within_an_ulp_of_the_true_value() {
    assert_eq!(sqrt(2).to_f64(), std::f64::consts::SQRT_2);
    assert!((phi().to_f64() - 1.618_033_988_749_895).abs() < 1e-15);
    assert_eq!(ar(1, 4).to_f64(), 0.25);
    assert!((a(2).nth_root(3).unwrap().to_f64() - 2f64.cbrt()).abs() < 1e-15);
}

#[test]
fn square_roots_of_negatives_and_even_roots_of_negatives_do_not_exist() {
    assert!(a(-2).sqrt().is_none());
    assert!(a(-16).nth_root(4).is_none());
    assert!(a(2).nth_root(0).is_none());
    assert_eq!(a(16).nth_root(4).unwrap(), a(2));
    assert_eq!(a(5).nth_root(1).unwrap(), a(5));
}

#[test]
fn from_f64_is_the_exact_dyadic_value() {
    assert_eq!(Algebraic::from_f64(0.1).unwrap().to_rational().unwrap(), crate::numeric::rational_from_f64_exact(0.1).unwrap());
    assert_eq!(Algebraic::from_f64(-2.5).unwrap(), ar(-5, 2));
    assert!(Algebraic::from_f64(f64::NAN).is_none());
    assert!(Algebraic::from_f64(f64::INFINITY).is_none());
}

#[test]
fn refining_the_enclosure_narrows_it_around_the_value() {
    let x = sqrt(2);
    let (lo, hi) = x.enclosure(64);
    assert!(lo <= hi);
    let eps = Rational::new(BigInt::from_i64(1), BigInt::from_i64(1).shl_bits(64)).unwrap();
    assert!(hi.sub(&lo) <= eps, "enclosure(64) is at most 2^-64 wide");
    assert!(lo.mul(&lo) <= q(2) && hi.mul(&hi) >= q(2));
    let (rlo, rhi) = ar(1, 3).enclosure(10);
    assert_eq!((rlo, rhi), (qr(1, 3), qr(1, 3)), "a rational encloses itself exactly");
}

// ── targeted checks (each one kills a mutant the corpus above let live) ──────────────────────

#[test]
fn conjugates_share_a_minimal_polynomial_but_are_distinct_values() {
    assert_ne!(sqrt(2), sqrt(2).neg());
    assert_ne!(hash_of(&sqrt(2)), hash_of(&sqrt(2).neg()));
    assert_eq!(sqrt(2).minimal_polynomial(), sqrt(2).neg().minimal_polynomial());
    let f = Poly::from_i64s(&[1, 0, -10, 0, 1]);
    let roots: Vec<Algebraic> = (1..=4).map(|k| Algebraic::root_of(&f, k).unwrap()).collect();
    for i in 0..4 {
        for j in 0..4 {
            assert_eq!(roots[i] == roots[j], i == j);
            assert_eq!(roots[i].cmp(&roots[j]), i.cmp(&j));
        }
    }
}

#[test]
fn a_sum_whose_conjugates_crowd_together_still_picks_its_own_root() {
    let x = a(1).add(&sqrt(2).div(&a(1000)).unwrap());
    let y = a(1).add(&sqrt(3).div(&a(1000)).unwrap());
    let s = x.add(&y);
    let want = 2.0 + (2f64.sqrt() + 3f64.sqrt()) / 1000.0;
    assert!((s.to_f64() - want).abs() < 1e-12, "{} vs {want}", s.to_f64());
    let d = x.sub(&y);
    let want = (2f64.sqrt() - 3f64.sqrt()) / 1000.0;
    assert!((d.to_f64() - want).abs() < 1e-15, "{} vs {want}", d.to_f64());
    assert_eq!(s.root_index(), 4, "the largest of 2 ± √2/1000 ± √3/1000");
    assert_eq!(d.root_index(), 2, "(√2 − √3)/1000 is the second of (±√2 ± √3)/1000");
    assert_eq!(s, Algebraic::root_of(s.minimal_polynomial(), 4).unwrap());
    let p = x.mul(&y);
    let want = (1.0 + 2f64.sqrt() / 1000.0) * (1.0 + 3f64.sqrt() / 1000.0);
    assert!((p.to_f64() - want).abs() < 1e-12);
}

#[test]
fn floor_and_ceil_refine_an_interval_that_straddles_an_integer() {
    let root2 = Algebraic::root_of(&Poly::from_i64s(&[-2, 0, 1]), 2).unwrap();
    assert_eq!(root2.floor(), BigInt::from_i64(1));
    assert_eq!(root2.ceil(), BigInt::from_i64(2));
    let neg = Algebraic::root_of(&Poly::from_i64s(&[-2, 0, 1]), 1).unwrap();
    assert_eq!(neg.floor(), BigInt::from_i64(-2));
    assert_eq!(neg.ceil(), BigInt::from_i64(-1));
    let big = Algebraic::root_of(&Poly::from_i64s(&[-1_000_001, 0, 1]), 2).unwrap();
    assert_eq!(big.floor(), BigInt::from_i64(1000));
    assert_eq!(big.ceil(), BigInt::from_i64(1001));
}

#[test]
fn interval_helpers_are_sound() {
    assert_eq!(interval_pow(&q(-1), &q(2), 2), (q(0), q(4)), "a straddling even power starts at 0");
    assert_eq!(interval_pow(&q(-3), &q(-2), 2), (q(4), q(9)));
    assert_eq!(interval_pow(&q(-2), &q(3), 3), (q(-8), q(27)));
    assert_eq!(interval_mul(&(q(-2), q(3)), &(q(-5), q(1))), (q(-15), q(10)));
    for n in [0i64, 1, 2, 3, 4, 15, 16, 17, 99, 100, 101, 1 << 40] {
        let r = integer_root_floor(&BigInt::from_i64(n), 2);
        assert!(r.mul(&r) <= BigInt::from_i64(n));
        let r1 = r.add(&BigInt::from_i64(1));
        assert!(r1.mul(&r1) > BigInt::from_i64(n), "⌊√{n}⌋");
    }
    assert_eq!(integer_root_floor(&BigInt::from_i64(26), 3), BigInt::from_i64(2));
    assert_eq!(integer_root_floor(&BigInt::from_i64(27), 3), BigInt::from_i64(3));
    let (l, u) = rational_root_bounds(&q(2), 2, 30);
    assert!(l.mul(&l) <= q(2) && u.mul(&u) >= q(2) && u.sub(&l) <= qr(1, 1 << 29));
    assert_eq!(split_square(&BigInt::from_i64(72)), (BigInt::from_i64(6), BigInt::from_i64(2)));
    assert_eq!(split_square(&BigInt::from_i64(49)), (BigInt::from_i64(7), BigInt::from_i64(1)));
    let p = BigInt::from_i64(1_000_003);
    assert_eq!(split_square(&p.mul(&p).mul(&BigInt::from_i64(5))), (p.clone(), BigInt::from_i64(5)),
        "a square of a large prime is found as the perfect-square cofactor");
}

#[test]
fn operands_with_coarse_isolating_intervals_still_select_the_right_conjugate() {
    let x = Algebraic::root_of(&Poly::from_i64s(&[999_998, -2_000_000, 1_000_000]), 2).unwrap();
    let y = Algebraic::root_of(&Poly::from_i64s(&[999_997, -2_000_000, 1_000_000]), 2).unwrap();
    assert_eq!(x, a(1).add(&sqrt(2).div(&a(1000)).unwrap()), "x = 1 + √2/1000");
    assert_eq!(y, a(1).add(&sqrt(3).div(&a(1000)).unwrap()), "y = 1 + √3/1000");
    let s = x.add(&y);
    assert_eq!(s.root_index(), 4, "2 + (√2 + √3)/1000 is the largest conjugate");
    assert_eq!(s.minimal_polynomial().count_real_roots(), 4);
    let d = x.sub(&y);
    assert_eq!(d.root_index(), 2);
    let p = x.mul(&y);
    assert_eq!(p.root_index(), 4);
    assert!(s > d && p > a(1));
}

#[test]
fn the_real_roots_of_a_rational_polynomial_come_in_increasing_order() {
    let roots = Algebraic::real_roots(&[a(-2), a(0), a(1)]);
    assert_eq!(roots, vec![(sqrt(2).neg(), 1), (sqrt(2), 1)]);
    assert!(Algebraic::real_roots(&[a(1), a(0), a(1)]).is_empty(), "x² + 1 has no real root");
    assert!(Algebraic::real_roots(&[a(5)]).is_empty());
    assert!(Algebraic::real_roots(&[a(5), a(0), a(0)]).is_empty(), "a trailing zero is no degree");
}

#[test]
fn a_repeated_root_carries_its_multiplicity() {
    let roots = Algebraic::real_roots(&[a(-1), a(3), a(-3), a(1)]);
    assert_eq!(roots, vec![(a(1), 3)], "(x − 1)³");
    let roots = Algebraic::real_roots(&[a(0), a(0), a(-2), a(0), a(1)]);
    assert_eq!(roots, vec![(sqrt(2).neg(), 1), (a(0), 2), (sqrt(2), 1)], "x²(x² − 2)");
}

#[test]
fn a_polynomial_with_algebraic_coefficients_has_its_own_roots_and_not_its_conjugates() {
    let roots = Algebraic::real_roots(&[a(0), sqrt(2).neg(), a(1)]);
    assert_eq!(roots, vec![(a(0), 1), (sqrt(2), 1)], "x² − √2·x: not −√2, a root of its conjugate");
    let two_root_two = sqrt(2).mul(&a(2));
    let roots = Algebraic::real_roots(&[a(2), two_root_two.neg(), a(1)]);
    assert_eq!(roots, vec![(sqrt(2), 2)], "(x − √2)²");
    let roots = Algebraic::real_roots(&[sqrt(3).neg(), a(0), sqrt(2)]);
    let expected = sqrt(3).div(&sqrt(2)).unwrap().sqrt().unwrap();
    assert_eq!(roots, vec![(expected.neg(), 1), (expected, 1)], "√2·x² − √3");
    let roots = Algebraic::real_roots(&[sqrt(2).add(&sqrt(3)).neg(), a(1)]);
    assert_eq!(roots, vec![(sqrt(2).add(&sqrt(3)), 1)], "two independent irrationals in one coefficient");
}

/// `a + b·√d` built from `√d` by the rational shortcuts alone, the oracle for the quadratic field.
fn in_field(a: Rational, b: Rational, d: i64) -> Algebraic {
    sqrt(d).mul(&Algebraic::from_rational(b)).add(&Algebraic::from_rational(a))
}

/// Two numbers of one quadratic field add and multiply as `a + b·√d`, landing in the one form any
/// route gives: `(a₁ + b₁√d) + (a₂ + b₂√d) = (a₁ + a₂) + (b₁ + b₂)√d` and
/// `(a₁ + b₁√d)(a₂ + b₂√d) = (a₁a₂ + b₁b₂d) + (a₁b₂ + a₂b₁)√d`.
#[test]
fn numbers_of_one_quadratic_field_compute_in_that_field() {
    let values = [qr(0, 1), qr(1, 1), qr(-2, 1), qr(3, 4), qr(-5, 3)];
    let nonzero = [qr(1, 1), qr(-1, 1), qr(2, 3), qr(-7, 5)];
    for d in [2, 3, 5, 6, 7, 10] {
        let dr = q(d);
        for a1 in &values {
            for b1 in &nonzero {
                for a2 in &values {
                    for b2 in &nonzero {
                        let x = in_field(a1.clone(), b1.clone(), d);
                        let y = in_field(a2.clone(), b2.clone(), d);
                        let sum = in_field(a1.add(a2), b1.add(b2), d);
                        assert_eq!(x.add(&y), sum, "({a1} + {b1}√{d}) + ({a2} + {b2}√{d})");
                        let a = a1.mul(a2).add(&b1.mul(b2).mul(&dr));
                        let b = a1.mul(b2).add(&a2.mul(b1));
                        assert_eq!(x.mul(&y), in_field(a, b, d), "({a1} + {b1}√{d})·({a2} + {b2}√{d})");
                    }
                }
            }
        }
    }
}

#[test]
fn a_quadratic_field_s_arithmetic_collapses_to_rationals_where_it_should() {
    let one_plus = in_field(q(1), q(1), 2);
    let one_minus = in_field(q(1), q(-1), 2);
    assert_eq!(one_plus.mul(&one_minus), a(-1));
    assert_eq!(one_plus.add(&one_minus), a(2));
    assert_eq!(sqrt(2).mul(&sqrt(2)), a(2));
    assert_eq!(sqrt(8).mul(&sqrt(2)), a(4), "√8 = 2√2 lies in the field of √2");
    assert_eq!(sqrt(8).add(&sqrt(2)).to_string(), "3√2");
    assert_eq!(phi().mul(&phi()), phi().add(&a(1)), "φ² = φ + 1 in the field of √5");
}

#[test]
fn numbers_of_two_quadratic_fields_still_meet_through_their_resultant() {
    let s = sqrt(2).add(&sqrt(3));
    assert_eq!(s.degree(), 4);
    assert_eq!(s.mul(&s), sqrt(6).mul(&a(2)).add(&a(5)), "(√2 + √3)² = 5 + 2√6");
    assert_eq!(sqrt(2).mul(&sqrt(3)), sqrt(6));
}

/// Two thousand sums in one quadratic field take well under the time a resultant for each would.
#[test]
fn a_thousand_sums_in_a_quadratic_field_are_cheap() {
    let r = sqrt(2);
    let start = std::time::Instant::now();
    let mut total = a(0);
    for i in 1..=1000 {
        total = total.add(&r.mul(&a(i)));
        total = total.add(&r).sub(&r);
    }
    assert_eq!(total, sqrt(2).mul(&a(500500)));
    assert!(start.elapsed() < std::time::Duration::from_secs(5), "took {:?}", start.elapsed());
}

/// A thousand products in one quadratic field take well under the time a resultant for each would:
/// `(1 + √2)^1000`, its coefficients from `(a + b√2)(1 + √2) = (a + 2b) + (a + b)√2`.
#[test]
fn a_thousand_products_in_a_quadratic_field_are_cheap() {
    let unit = in_field(q(1), q(1), 2);
    let start = std::time::Instant::now();
    let mut power = a(1);
    let (mut pa, mut pb) = (q(1), q(0));
    for _ in 0..1000 {
        power = power.mul(&unit);
        let next_a = pa.add(&pb.add(&pb));
        pb = pa.add(&pb);
        pa = next_a;
    }
    let elapsed = start.elapsed();
    assert_eq!(power, in_field(pa, pb, 2));
    assert!(elapsed < std::time::Duration::from_secs(5), "took {elapsed:?}");
}
