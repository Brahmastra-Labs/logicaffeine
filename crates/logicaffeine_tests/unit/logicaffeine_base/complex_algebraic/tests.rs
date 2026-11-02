use super::*;

fn a(n: i64) -> Algebraic {
    Algebraic::from_i64(n)
}

fn q(n: i64, d: i64) -> Algebraic {
    Algebraic::from_rational(Rational::from_ratio_i64(n, d).unwrap())
}

fn c(re: Algebraic, im: Algebraic) -> ComplexAlgebraic {
    ComplexAlgebraic::new(re, im)
}

fn i() -> ComplexAlgebraic {
    ComplexAlgebraic::imag(a(1))
}

// ── arithmetic ────────────────────────────────────────────────────────────────────────────────

#[test]
fn complex_arithmetic_is_exact() {
    assert_eq!(i().mul(&i()), ComplexAlgebraic::from_i64(-1));
    let one_plus_i = c(a(1), a(1));
    assert_eq!(one_plus_i.mul(&one_plus_i.conj()), ComplexAlgebraic::from_i64(2));
    assert_eq!(one_plus_i.recip().unwrap(), c(q(1, 2), q(-1, 2)));
    assert_eq!(one_plus_i.div(&one_plus_i).unwrap(), ComplexAlgebraic::one());
    assert_eq!(one_plus_i.pow(4), ComplexAlgebraic::from_i64(-4));
    assert_eq!(ComplexAlgebraic::zero().recip(), None);
    let root2 = a(2).sqrt().unwrap();
    let w = c(root2.mul(&q(1, 2)), root2.mul(&q(1, 2)));
    assert_eq!(w.pow(8), ComplexAlgebraic::one(), "a primitive eighth root of unity");
}

#[test]
fn a_polynomial_is_evaluated_at_a_complex_point() {
    let p = Poly::from_i64s(&[1, 0, 1]);
    assert!(ComplexAlgebraic::eval(&p, &i()).is_zero());
    assert_eq!(ComplexAlgebraic::eval(&p, &c(a(1), a(1))), c(a(1), a(2)));
}

#[test]
fn complex_numbers_print_in_the_usual_way() {
    assert_eq!(i().to_string(), "i");
    assert_eq!(i().neg().to_string(), "-i");
    assert_eq!(c(a(1), a(2)).to_string(), "1 + 2·i");
    assert_eq!(c(a(1), a(-2)).to_string(), "1 - 2·i");
    assert_eq!(c(q(-1, 2), a(3).sqrt().unwrap().mul(&q(1, 2))).to_string(), "-1/2 + (√3/2)·i");
    assert_eq!(ComplexAlgebraic::from_i64(5).to_string(), "5");
}

// ── roots ─────────────────────────────────────────────────────────────────────────────────────

fn check_roots(p: &Poly, expect_real: usize, expect_total: usize) -> Vec<ComplexAlgebraic> {
    let roots = complex_roots(p);
    assert_eq!(roots.len(), expect_total, "{p:?}");
    assert_eq!(roots.iter().filter(|z| z.is_real()).count(), expect_real);
    for z in &roots {
        assert!(ComplexAlgebraic::eval(p, z).is_zero(), "{z} is a root of {p:?}");
    }
    for (k, z) in roots.iter().enumerate() {
        assert!(!roots[..k].contains(z), "each root once");
    }
    roots
}

#[test]
fn the_roots_of_quadratics_are_the_formula() {
    assert_eq!(check_roots(&Poly::from_i64s(&[1, 0, 1]), 0, 2), vec![i(), i().neg()]);
    let third = check_roots(&Poly::from_i64s(&[1, 1, 1]), 0, 2);
    let half_root3 = a(3).sqrt().unwrap().mul(&q(1, 2));
    assert_eq!(third[0], c(q(-1, 2), half_root3.clone()));
    assert_eq!(third[1], c(q(-1, 2), half_root3.neg()));
    assert_eq!(check_roots(&Poly::from_i64s(&[-2, 0, 1]), 2, 2)[0], ComplexAlgebraic::real(a(2).sqrt().unwrap().neg()));
}

#[test]
fn a_cubic_with_one_real_root_has_a_conjugate_pair() {
    let roots = check_roots(&Poly::from_i64s(&[-2, 0, 0, 1]), 1, 3);
    let cbrt2 = a(2).nth_root(3).unwrap();
    assert_eq!(roots[0], ComplexAlgebraic::real(cbrt2.clone()));
    assert_eq!(roots[1].re, cbrt2.mul(&q(-1, 2)));
    assert_eq!(roots[1].im, cbrt2.mul(&a(3).sqrt().unwrap()).mul(&q(1, 2)));
    assert_eq!(roots[2], roots[1].conj());
}

#[test]
fn cyclotomic_roots_are_found_and_checked() {
    let eighth = check_roots(&Poly::from_i64s(&[1, 0, 0, 0, 1]), 0, 4);
    let h = a(2).sqrt().unwrap().mul(&q(1, 2));
    assert!(eighth.contains(&c(h.clone(), h.clone())));
    assert!(eighth.contains(&c(h.neg(), h.neg())));
    let fifth = check_roots(&Poly::from_i64s(&[1, 1, 1, 1, 1]), 0, 4);
    for z in &fifth {
        assert_eq!(z.pow(5), ComplexAlgebraic::one(), "{z} is a fifth root of unity");
    }
    let seventh = check_roots(&Poly::from_i64s(&[1, 1, 1, 1, 1, 1, 1]), 0, 6);
    for z in &seventh {
        assert_eq!(z.pow(7), ComplexAlgebraic::one());
    }
}

#[test]
fn a_reducible_polynomial_gives_each_distinct_root_once() {
    let p = Poly::from_i64s(&[1, 0, 1]).mul(&Poly::from_i64s(&[1, 0, 1])).mul(&Poly::from_i64s(&[-3, 1]));
    assert_eq!(check_roots(&p, 1, 3), vec![ComplexAlgebraic::from_i64(3), i(), i().neg()]);
    let mixed = Poly::from_i64s(&[-2, 0, 1]).mul(&Poly::from_i64s(&[1, 1, 1]));
    check_roots(&mixed, 2, 4);
}

#[test]
fn a_quartic_with_two_complex_pairs_pairs_each_real_part_with_its_own_imaginary_part() {
    let p = Poly::from_i64s(&[2, 0, 1]).mul(&Poly::from_i64s(&[2, 2, 1]));
    let roots = check_roots(&p, 0, 4);
    assert!(roots.contains(&c(a(0), a(2).sqrt().unwrap())));
    assert!(roots.contains(&c(a(-1), a(1))));
    assert!(!roots.contains(&c(a(-1), a(2).sqrt().unwrap())), "no mismatched pair");
}
