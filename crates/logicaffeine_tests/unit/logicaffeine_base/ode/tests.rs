use super::*;
use crate::interval::{Dyadic, Iv};
use crate::numeric::{BigInt, Rational};
use crate::poly::Poly;

fn r(n: i64, d: i64) -> Rational {
    Rational::from_ratio_i64(n, d).unwrap()
}

fn point(x: &Rational, q: i64) -> Iv {
    Iv {
        lo: Dyadic::from_rational_floor(x, q),
        hi: Dyadic::from_rational_ceil(x, q),
    }
}

fn mono(c: Rational, powers: &[u32]) -> Monomial {
    Monomial {
        coefficient: c,
        powers: powers.to_vec(),
    }
}

/// `x' = x`.
fn growth() -> Field {
    Field {
        components: vec![vec![mono(r(1, 1), &[1])]],
    }
}

/// `x' = x − x²`.
fn logistic() -> Field {
    Field {
        components: vec![vec![mono(r(1, 1), &[1]), mono(r(-1, 1), &[2])]],
    }
}

/// `x' = −y(x² + y²)`, `y' = x(x² + y²)`.
fn rotation() -> Field {
    Field {
        components: vec![
            vec![mono(r(-1, 1), &[2, 1]), mono(r(-1, 1), &[0, 3])],
            vec![mono(r(1, 1), &[3, 0]), mono(r(1, 1), &[1, 2])],
        ],
    }
}

/// `x' = x²`.
fn blow_up() -> Field {
    Field {
        components: vec![vec![mono(r(1, 1), &[2])]],
    }
}

/// The enclosure contains the decimal bracket `[a, b]` drawn within `10^-38` of a known constant,
/// so it contains the constant, and it is thinner than `2^-bits`.
fn holds(iv: &Iv, below: &str, above: &str, bits: i64) {
    let parse = |s: &str| {
        let (whole, frac) = s.split_once('.').unwrap();
        Rational::new(
            BigInt::parse_decimal(&format!("{whole}{frac}")).unwrap(),
            BigInt::from_i64(10).pow(frac.len() as u32),
        )
        .unwrap()
    };
    assert!(iv.lo.to_rational() <= parse(below), "{iv:?} starts above {below}");
    assert!(iv.hi.to_rational() >= parse(above), "{iv:?} ends below {above}");
    assert!(iv.width().mag_bits() <= -bits, "{iv:?} is wider than 2^-{bits}");
}

#[test]
fn exponential_growth_is_enclosed_around_e() {
    let x = flow(&growth(), &[point(&r(1, 1), 96)], &r(1, 1), 96, 40).unwrap();
    holds(&x[0], "2.7182818284590452353602874713526624977472", "2.7182818284590452353602874713526624977672", 60);
}

#[test]
fn the_logistic_solution_is_enclosed_around_its_sigmoid() {
    let x = flow(&logistic(), &[point(&r(1, 2), 96)], &r(1, 1), 96, 40).unwrap();
    holds(&x[0], "0.7310585786300048792511592418218362743551", "0.7310585786300048792511592418218362743751", 60);
}

#[test]
fn a_nonlinear_rotation_stays_on_its_circle() {
    let start = [point(&r(1, 1), 96), point(&r(0, 1), 96)];
    let x = flow(&rotation(), &start, &r(1, 1), 96, 40).unwrap();
    holds(&x[0], "0.5403023058681397174009366074429766037223", "0.5403023058681397174009366074429766037423", 50);
    holds(&x[1], "0.8414709848078965066525023216302989996126", "0.8414709848078965066525023216302989996326", 50);
}

#[test]
fn a_time_that_is_not_dyadic_is_reached_exactly() {
    let x = flow(&growth(), &[point(&r(1, 1), 96)], &r(1, 3), 96, 40).unwrap();
    holds(&x[0], "1.3956124250860895286281253196025868375879", "1.3956124250860895286281253196025868376079", 60);
}

#[test]
fn time_zero_is_the_start() {
    let start = [point(&r(1, 3), 64)];
    let x = flow(&growth(), &start, &r(0, 1), 64, 40).unwrap();
    assert_eq!(x[0].lo, start[0].lo);
    assert_eq!(x[0].hi, start[0].hi);
}

#[test]
fn a_solution_that_leaves_every_bound_escapes() {
    assert_eq!(
        flow(&blow_up(), &[point(&r(1, 1), 64)], &r(2, 1), 64, 30).err(),
        Some(FlowError::Escapes)
    );
}

#[test]
fn a_solution_before_its_blow_up_is_enclosed() {
    let x = flow(&blow_up(), &[point(&r(1, 1), 96)], &r(1, 2), 96, 40).unwrap();
    holds(&x[0], "1.9999999999999999999999999999999999999900", "2.0000000000000000000000000000000000000100", 50);
}

#[test]
fn a_mismatched_shape_is_refused() {
    assert_eq!(
        flow(&rotation(), &[point(&r(1, 1), 64)], &r(1, 1), 64, 30).err(),
        Some(FlowError::Shape)
    );
    let ragged = Field {
        components: vec![vec![mono(r(1, 1), &[1, 0])]],
    };
    assert_eq!(flow(&ragged, &[point(&r(1, 1), 64)], &r(1, 1), 64, 30).err(), Some(FlowError::Shape));
}

/// `x² + c` as a polynomial in one variable.
fn square_plus(c: Rational) -> Poly {
    Poly::from_coeffs(vec![c, Rational::zero(), Rational::one()])
}

#[test]
fn the_escape_time_of_x_squared_is_bounded_above_its_exact_value() {
    let bound = escape_time_bound(&square_plus(Rational::zero()), &r(1, 1), 24).unwrap();
    assert!(bound >= r(1, 1), "∫₁^∞ dx/x² = 1 ≤ {bound}");
    assert!(bound < r(2, 1), "{bound}");
}

#[test]
fn a_root_above_the_start_has_no_escape_time() {
    let p = Poly::from_coeffs(vec![r(-4, 1), Rational::zero(), Rational::one()]);
    assert_eq!(escape_time_bound(&p, &r(1, 1), 24), None, "x² − 4 vanishes at 2");
    assert_eq!(escape_time_bound(&p, &r(-3, 1), 24), None, "x² − 4 is negative at 0");
}

#[test]
fn a_field_of_degree_one_never_escapes_in_finite_time() {
    let p = Poly::from_coeffs(vec![Rational::one(), Rational::one()]);
    assert_eq!(escape_time_bound(&p, &r(1, 1), 24), None);
}

#[test]
fn a_start_beyond_the_spread_needs_only_the_tail() {
    let bound = escape_time_bound(&square_plus(Rational::one()), &r(4, 1), 24).unwrap();
    assert!(bound >= r(1, 4).sub(&r(1, 100)), "∫₄^∞ dx/(x²+1) ≈ 0.245 ≤ {bound}");
    assert!(bound <= r(1, 2), "{bound}");
}
