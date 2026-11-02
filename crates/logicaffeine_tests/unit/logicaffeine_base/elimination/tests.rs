use super::*;

fn q(n: i64) -> Rational {
    Rational::from_i64(n)
}

fn c(n: i64) -> Polynomial {
    Polynomial::constant(q(n))
}

fn x(i: usize) -> Polynomial {
    Polynomial::var(i)
}

#[test]
fn the_ring_operations_expand_and_cancel() {
    let s = x(0).add(&x(1));
    assert_eq!(s.pow(2), x(0).pow(2).add(&x(0).mul(&x(1)).scale(&q(2))).add(&x(1).pow(2)));
    assert!(s.sub(&s).is_zero());
    assert_eq!(s.pow(2).degree_in(0), 2);
    assert_eq!(s.pow(3).coefficient_in(1, 2), x(0).scale(&q(3)));
    assert_eq!(s.pow(3).leading_in(0), c(1));
    assert_eq!(Polynomial::zero().as_constant(), Some(q(0)));
    assert_eq!(x(2).variables().into_iter().collect::<Vec<_>>(), [2]);
}

#[test]
fn a_variable_takes_a_value_or_a_polynomial() {
    let p = x(0).pow(2).add(&x(0).mul(&x(1))).add(&c(1));
    assert_eq!(p.substitute(0, &q(2)), x(1).scale(&q(2)).add(&c(5)));
    assert_eq!(p.compose(1, &x(0)), x(0).pow(2).scale(&q(2)).add(&c(1)));
    assert_eq!(p.derivative(0), x(0).scale(&q(2)).add(&x(1)));
    assert_eq!(p.substitute(1, &q(0)).to_univariate(0), Some(Poly::from_i64s(&[1, 0, 1])));
    assert_eq!(p.to_univariate(0), None);
}

#[test]
fn a_resultant_of_one_variable_is_the_univariate_resultant() {
    let a = Poly::from_i64s(&[-2, 0, 1]);
    let b = Poly::from_i64s(&[1, -3, 0, 2]);
    let r = Polynomial::from_univariate(&a, 0).resultant(&Polynomial::from_univariate(&b, 0), 0);
    assert_eq!(r.as_constant(), Some(a.resultant(&b)));
}

#[test]
fn a_line_meets_the_circle_where_the_resultant_vanishes() {
    let circle = x(1).pow(2).add(&x(0).pow(2)).sub(&c(1));
    let line = x(0).sub(&x(1));
    assert_eq!(circle.resultant(&line, 1), x(0).pow(2).scale(&q(2)).sub(&c(1)));
}

#[test]
fn eliminating_an_algebraic_number_gives_its_minimal_polynomial() {
    let sqrt2 = x(1).pow(2).sub(&c(2));
    let sqrt3 = x(2).pow(2).sub(&c(3));
    let sum = x(0).sub(&x(1)).sub(&x(2));
    let inner = sqrt3.resultant(&sum, 2);
    let norm = sqrt2.resultant(&inner, 1).to_univariate(0).expect("only x0 is left");
    assert_eq!(norm, Poly::from_i64s(&[1, 0, -10, 0, 1]), "√2 + √3 is a root of x⁴ − 10x² + 1");
}

#[test]
fn a_point_where_a_leading_coefficient_vanishes_is_passed_over() {
    let a = x(0).mul(&x(1)).sub(&c(1));
    let b = x(1).pow(2).sub(&c(2));
    assert_eq!(a.resultant(&b, 1), c(1).sub(&x(0).pow(2).scale(&q(2))));
    assert_eq!(c(3).resultant(&b, 1), c(9), "a constant to the other's degree");
    let lead_vanishes = x(0).mul(&x(1).pow(2)).add(&x(1)).add(&c(1));
    let line = x(1).sub(&c(2));
    assert_eq!(
        lead_vanishes.resultant(&line, 1),
        x(0).scale(&q(4)).add(&c(3)),
        "the formal resultant, though at x0 = 0 the first polynomial drops a degree and the resultant there is -3"
    );
    assert!(Polynomial::zero().resultant(&b, 1).is_zero());
}

#[test]
fn a_polynomial_prints_from_its_highest_terms_down() {
    let p = x(0).pow(2).scale(&q(3)).sub(&x(0).mul(&x(1))).add(&c(1));
    assert_eq!(p.to_string(), "3·x0^2 - x0·x1 + 1");
    assert_eq!(Polynomial::zero().to_string(), "0");
}
