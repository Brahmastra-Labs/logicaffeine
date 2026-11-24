use z3::ast::{Bool, Dynamic, Int};
use z3::{SatResult, Solver};

use super::*;

fn n() -> Int {
    Int::new_const("n")
}

fn poly(t: &Int) -> Poly {
    polynomial(&Dynamic::from_ast(t)).expect("a polynomial")
}

fn divides(t: &Int, c: i64) -> bool {
    always_divides(&poly(t), &BigInt::from_i64(c))
}

#[test]
fn a_product_of_consecutive_integers_is_divided_by_the_count_of_its_factors_factorial() {
    let n = n();
    assert!(divides(&(&n * &(&n + 1i64)), 2));
    assert!(divides(&(&(&n * &(&n + 1i64)) * &(&(&n * 2i64) + 1i64)), 6), "n(n+1)(2n+1) is six times Σ k²");
    assert!(divides(&(&(&(&n - 1i64) * &n) * &(&n + 1i64)), 6));
    let (x, y) = (Int::new_const("x"), Int::new_const("y"));
    assert!(divides(&(&(&x * &y) * &(&x + &y)), 2), "two variables: every residue pair is checked");
}

#[test]
fn a_polynomial_that_misses_one_residue_is_not_always_divided() {
    let n = n();
    assert!(!divides(&(&n * &n), 2), "an odd n squared is odd");
    assert!(!divides(&(&(&n * &(&n + 1i64)) + 1i64), 2));
    assert!(!divides(&(&n * &(&n + 1i64)), 3), "1·2 is not a multiple of three");
}

#[test]
fn a_term_that_is_not_a_polynomial_is_not_read_as_one() {
    let n = n();
    assert!(polynomial(&Dynamic::from_ast(&n.div(Int::from_i64(2)))).is_none());
    assert!(polynomial(&Dynamic::from_ast(&n.ge(Int::from_i64(0)).ite(&n, &Int::from_i64(0)))).is_none());
    assert_eq!(poly(&(&(&n - &n) + 3i64)), Poly::from([(BTreeMap::new(), BigInt::from_i64(3))]), "terms cancel");
}

/// The lemma is what the solver lacked: with it, `2·(n(n+1) div 2) = n² + n` is proved; a division its polynomial does
/// not always meet gives no lemma.
#[test]
fn the_lemma_proves_an_exact_halving_and_none_is_given_for_an_inexact_one() {
    let n = n();
    let half = (&n * &(&n + 1i64)).div(Int::from_i64(2));
    let given = lemmas(&[Dynamic::from_ast(&half)]);
    assert_eq!(given.len(), 1);
    let solver = Solver::new();
    for l in &given {
        solver.assert(l);
    }
    let claim: Bool = (&half * 2i64).eq(&(&(&n * &n) + &n));
    solver.assert(&claim.not());
    assert_eq!(solver.check(), SatResult::Unsat);
    assert!(lemmas(&[Dynamic::from_ast(&(&n * &n).div(Int::from_i64(2)))]).is_empty());
    assert!(lemmas(&[Dynamic::from_ast(&(&n * &(&n + 1i64)).div(Int::from_i64(-2)))]).is_empty(), "a negative divisor");
}
