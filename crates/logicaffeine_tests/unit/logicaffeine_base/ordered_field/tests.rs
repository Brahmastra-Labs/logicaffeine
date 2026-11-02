use super::*;
use crate::numeric::Rational;

fn a(n: i64) -> Algebraic {
    Algebraic::from_i64(n)
}

fn ar(n: i64, d: i64) -> Algebraic {
    Algebraic::from_rational(Rational::from_ratio_i64(n, d).unwrap())
}

fn p(c: &[i64]) -> PolyOver<Algebraic> {
    PolyOver::from(c.iter().map(|n| a(*n)).collect())
}

#[test]
fn a_polynomial_drops_its_trailing_zeros_and_reads_its_degree_and_order() {
    let x = p(&[0, 0, 3, 0, 0]);
    assert_eq!(x.degree(), Some(2));
    assert_eq!(x.order(), Some(2));
    assert_eq!(x.coeff(7), a(0));
    assert!(p(&[0, 0]).is_zero());
    assert_eq!(p(&[0, 0]).degree(), None);
    assert_eq!(PolyOver::<Algebraic>::variable(), p(&[0, 1]));
}

#[test]
fn euclids_division_leaves_a_remainder_of_lower_degree() {
    let f = p(&[-1, 0, 0, 1]);
    let d = p(&[-1, 1]);
    let (q, r) = f.div_rem(&d);
    assert_eq!(q, p(&[1, 1, 1]), "x³ − 1 = (x − 1)(x² + x + 1)");
    assert!(r.is_zero());
    let (q, r) = p(&[1, 0, 1]).div_rem(&p(&[0, 2]));
    assert_eq!((q, r), (PolyOver::from(vec![a(0), ar(1, 2)]), p(&[1])));
}

#[test]
fn the_gcd_is_monic_and_divides_both() {
    let f = p(&[-1, 0, 1]).mul(&p(&[2, 1]));
    let g = p(&[-1, 0, 1]).mul(&p(&[3, 1]));
    assert_eq!(f.gcd(&g), p(&[-1, 0, 1]));
    assert_eq!(p(&[0, 4]).gcd(&p(&[0, 0, 6])), p(&[0, 1]));
    assert_eq!(p(&[1, 1]).gcd(&p(&[2, 1])), p(&[1]), "coprime factors meet in one");
}

#[test]
fn yun_splits_a_polynomial_into_its_square_free_powers() {
    let f = p(&[1, 1]).mul(&p(&[-2, 1])).mul(&p(&[-2, 1])).scale(&a(3));
    let (lc, parts) = f.squarefree_parts();
    assert_eq!(lc, a(3));
    assert_eq!(parts, vec![(p(&[1, 1]), 1), (p(&[-2, 1]), 2)]);
}

#[test]
fn evaluation_spreading_and_the_derivative_are_the_polynomial_s_own() {
    let f = p(&[1, -3, 2]);
    assert_eq!(f.eval(&a(2)), a(3));
    assert_eq!(f.spread(3), p(&[1, 0, 0, -3, 0, 0, 2]));
    assert_eq!(f.derivative(), p(&[-3, 4]));
    assert_eq!(f.neg().add(&f), PolyOver::zero());
    assert_eq!(f.monic(), PolyOver::from(vec![ar(1, 2), ar(-3, 2), a(1)]));
}

#[test]
fn an_algebraic_number_is_an_ordered_field() {
    let r = Algebraic::from_i64(2).sqrt().unwrap();
    assert_eq!(<Algebraic as OrderedField>::signum(&r.neg()), -1);
    assert_eq!(OrderedField::mul(&r, &r), a(2));
    assert_eq!(OrderedField::recip(&a(0)), None);
    assert_eq!(<Algebraic as OrderedField>::sub(&a(5), &a(7)), a(-2));
}

/// `Res_z(P(z), y − z^k)` by interpolation through `deg P + 1` points, the definition the power
/// polynomial is checked against.
fn power_by_resultant(poly: &PolyOver<Algebraic>, k: u32) -> PolyOver<Algebraic> {
    let degree = poly.degree().unwrap();
    PolyOver::interpolate(degree, |j| {
        let mut c = vec![Algebraic::zero(); k as usize + 1];
        c[0] = Algebraic::from_i64(j);
        c[k as usize] = Algebraic::from_i64(-1);
        poly.resultant(&PolyOver::from(c))
    })
}

/// The power polynomial read through Newton's identities has the roots of the resultant's, each as
/// often, for polynomials with and without a zero root, repeated roots, irrational and rational roots,
/// a leading coefficient other than one, binomials and powers that make roots meet.
#[test]
fn the_power_polynomial_is_the_resultant_s_for_every_power() {
    let polys = [
        p(&[1, -3, 1]),
        p(&[-2, 0, 0, 1]),
        p(&[1, 1, 0, 1]),
        p(&[6, -1, -4, 1]),
        p(&[0, -2, 0, 1]),
        p(&[4, -4, 1]),
        p(&[-3, 0, 2]),
        p(&[1, 0, -5, 0, 4]),
    ];
    for poly in &polys {
        for k in 1..=5 {
            let fast = PolyOver::power_poly(poly, k).monic();
            let slow = power_by_resultant(poly, k).monic();
            assert_eq!(fast.squarefree(), slow.squarefree(), "power {k} of {poly:?}");
            if (1..poly.degree().unwrap()).any(|i| !poly.coeff(i).is_zero()) {
                assert_eq!(fast, slow, "power {k} of {poly:?}, each root as often");
            }
        }
    }
}

/// A binomial's powers are read off: `z³ − 2` to the sixth is `y − 4`, to the fourth `y³ − 16`.
#[test]
fn a_binomial_s_power_is_read_off() {
    let cube = p(&[-2, 0, 0, 1]);
    assert_eq!(PolyOver::power_poly(&cube, 6), p(&[-4, 1]));
    assert_eq!(PolyOver::power_poly(&cube, 4), p(&[-16, 0, 0, 1]));
    assert_eq!(PolyOver::power_poly(&p(&[-3, 0, 2]), 2), PolyOver::from(vec![ar(-3, 2), a(1)]));
}

/// The composed sums and products read through power sums have the roots of their resultants, each
/// pair of roots once.
#[test]
fn composed_sums_and_products_are_their_resultants() {
    let polys = [p(&[1, -3, 1]), p(&[-2, 0, 0, 1]), p(&[6, -1, -4, 1]), p(&[-3, 0, 2]), p(&[2, 1])];
    for x in &polys {
        for y in &polys {
            let dy = y.degree().unwrap();
            let degree = x.degree().unwrap() * dy;
            let reflected = y.neg_var();
            let sum = PolyOver::interpolate(degree, |j| x.resultant(&reflected.shift(&Algebraic::from_i64(-j))));
            assert_eq!(PolyOver::sum_poly(x, y), sum.monic(), "sums of roots of {x:?} and {y:?}");
            let product = PolyOver::interpolate(degree, |j| {
                let j = Algebraic::from_i64(j);
                let mut c = vec![Algebraic::zero(); dy + 1];
                let mut power = Algebraic::from_i64(1);
                for i in 0..=dy {
                    c[dy - i] = y.coeff(i).mul(&power);
                    power = power.mul(&j);
                }
                x.resultant(&PolyOver::from(c))
            });
            assert_eq!(PolyOver::product_poly(x, y), product.monic(), "products of roots of {x:?} and {y:?}");
        }
    }
}
