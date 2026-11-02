use super::*;
use crate::algebraic::Algebraic;
use crate::limit_value::ResidueType;
use crate::multi::Multi;
use crate::ordered_field::OrderedField;
use crate::tower::Scale;

fn n(k: i64) -> Multi {
    <Multi as OrderedField>::from_i64(k)
}

fn r(a: i64, b: i64) -> Multi {
    Multi::Standard(Algebraic::from_rational(Rational::from_ratio_i64(a, b).unwrap()))
}

fn dt() -> Multi {
    Multi::generator(&Arc::new(Scale::new("dt", ResidueType::any())))
}

fn terms(e: &Expansion<Multi>, through: i64) -> Vec<Multi> {
    (e.floor()..=through).map(|k| e.coefficient(k).unwrap()).collect()
}

/// `1/(1 − x)` is the geometric series, and `x/(x² − x³)` starts at `x^(-1)`.
#[test]
fn a_fraction_expands_by_power_series_division() {
    let geometric = Expansion::fraction(vec![n(1)], vec![n(1), n(-1)], 1).unwrap();
    assert_eq!(terms(&geometric, 4), vec![n(1); 5]);
    let pole = Expansion::fraction(vec![n(0), n(1)], vec![n(0), n(0), n(1), n(-1)], 1).unwrap();
    assert_eq!(pole.floor(), -1);
    assert_eq!(terms(&pole, 2), vec![n(1); 4]);
    assert_eq!(Expansion::<Multi>::fraction(vec![n(1)], vec![n(0)], 1).unwrap_err(), ExpansionError::DivisionByZero);
}

/// Coefficients that are values of a larger infinitesimal: `1/(dt − x)` is `Σ x^k/dt^(k+1)`.
#[test]
fn coefficients_may_be_values_of_a_larger_infinitesimal() {
    let e = Expansion::fraction(vec![n(1)], vec![dt(), n(-1)], 1).unwrap();
    let inv = OrderedField::recip(&dt()).unwrap();
    for k in 0..4 {
        assert_eq!(e.coefficient(k).unwrap(), inv.pow(k as i32 + 1).unwrap(), "order {k}");
    }
}

#[test]
fn sums_products_and_reciprocals_meet_on_one_grid() {
    let one_plus = Expansion::fraction(vec![n(1), n(1)], vec![n(1)], 1).unwrap();
    let root_term = Expansion::fraction(vec![n(0), n(1)], vec![n(1)], 2).unwrap();
    let sum = one_plus.add(&root_term);
    assert_eq!(sum.ramification(), 2);
    assert_eq!(terms(&sum, 2), vec![n(1), n(1), n(1)], "1 + x^(1/2) + x");
    let square = one_plus.mul(&one_plus);
    assert_eq!(terms(&square, 3), vec![n(1), n(2), n(1), n(0)]);
    let inverse = one_plus.recip(0, n(1));
    assert_eq!(terms(&inverse, 3), vec![n(1), n(-1), n(1), n(-1)]);
    assert_eq!(terms(&one_plus.sub(&one_plus), 2), vec![n(0); 3]);
}

/// Miller's recurrence: `√(1 + x) = 1 + x/2 − x²/8 + x³/16 − …`, and `√(4x²) = 2x`.
#[test]
fn a_root_follows_the_binomial_series() {
    let one_plus = Expansion::fraction(vec![n(1), n(1)], vec![n(1)], 1).unwrap();
    let root = one_plus.root(2, 0, n(1)).unwrap();
    assert_eq!(terms(&root, 3), vec![n(1), r(1, 2), r(-1, 8), r(1, 16)]);
    let square = Expansion::fraction(vec![n(0), n(0), n(4)], vec![n(1)], 1).unwrap();
    let root = square.root(2, 2, n(4)).unwrap();
    assert_eq!(root.floor(), 1);
    assert_eq!(terms(&root, 3), vec![n(2), n(0), n(0)]);
    let odd = Expansion::fraction(vec![n(0), n(1)], vec![n(1)], 1).unwrap();
    let half = odd.root(2, 1, n(1)).unwrap();
    assert_eq!(half.ramification(), 2, "√x lies on the grid of x^(1/2)");
    assert_eq!(half.floor(), 1);
    assert_eq!(odd.neg().root(2, 1, n(-1)).unwrap_err(), ExpansionError::NegativeRoot(2));
    assert_eq!(terms(&odd.neg().root(3, 1, n(-1)).unwrap(), 1), vec![n(-1)]);
}

#[test]
fn the_leading_term_is_found_within_its_bound() {
    let late = Expansion::fraction(vec![n(0), n(0), n(0), n(5)], vec![n(1)], 1).unwrap();
    assert_eq!(late.leading_through(2).unwrap().map(|(k, _)| k), None);
    assert_eq!(late.leading_through(3).unwrap(), Some((3, n(5))));
}
