use super::*;
use std::collections::hash_map::DefaultHasher;

fn dt() -> Hyperreal {
    Hyperreal::dt()
}

fn h(n: i64) -> Hyperreal {
    Hyperreal::from_i64(n)
}

fn q(n: i64, d: i64) -> Rational {
    Rational::from_ratio_i64(n, d).unwrap()
}

fn field(x: Hyperreal) -> Closed {
    Closed::Field(x)
}

fn sqrt(x: &Closed) -> Closed {
    x.root(2).unwrap()
}

fn shows(x: &Closed) -> String {
    x.render("dt")
}

/// The first term of each real root of `p`, in `dt`'s units.
fn leading_terms(p: &KPoly) -> Vec<(Rational, Algebraic, u32)> {
    let mut out: Vec<(Rational, Algebraic, u32)> = branches(p, 1)
        .into_iter()
        .map(|mut b| {
            let first = b.through(&Rational::from_i64(5)).into_iter().next().expect("a nonzero root");
            (first.0, first.1, b.q)
        })
        .collect();
    out.sort_by(|a, b| a.1.cmp(&b.1).then(a.0.cmp(&b.0)));
    out
}

#[test]
fn newton_s_polygon_finds_each_real_root_by_its_leading_term() {
    let p = KPoly::from(vec![dt().neg(), h(-1), h(0), h(1)]);
    let leads = leading_terms(&p);
    let a = |n: i64| Algebraic::from_i64(n);
    assert_eq!(
        leads,
        vec![(q(0, 1), a(-1), 1), (q(1, 1), a(-1), 1), (q(0, 1), a(1), 1)],
        "y³ − y − dt has roots near −1, −dt and 1"
    );
    let no_real = KPoly::from(vec![dt(), h(0), h(1)]);
    assert!(branches(&no_real, 1).is_empty(), "y² + dt has no real root");
}

#[test]
fn a_fractional_valuation_refines_the_root_of_dt() {
    let p = KPoly::from(vec![dt().pow(3).unwrap().neg(), h(0), h(1)]);
    let leads = leading_terms(&p);
    assert_eq!(leads.len(), 2);
    assert!(leads.iter().all(|(e, _, r)| *e == q(3, 2) && *r == 2), "±dt^(3/2): {leads:?}");
}

#[test]
fn roots_that_share_a_leading_term_separate_at_the_next() {
    let r1 = KPoly::linear(&h(1).add(&dt()));
    let r2 = KPoly::linear(&h(1).add(&dt().mul(&h(2))));
    let p = r1.mul(&r2);
    let mut seconds: Vec<Algebraic> = branches(&p, 1)
        .into_iter()
        .map(|mut b| b.through(&Rational::from_i64(1))[1].1.clone())
        .collect();
    seconds.sort();
    assert_eq!(seconds, vec![Algebraic::from_i64(1), Algebraic::from_i64(2)]);
}

#[test]
fn the_square_root_of_dt_squares_back_to_dt() {
    let r = sqrt(&field(dt()));
    assert_eq!(shows(&r), "√dt");
    assert_eq!(r.mul(&r), field(dt()));
    assert_eq!(r.valuation(), Some(q(1, 2)));
    assert!(r.is_infinitesimal());
    assert_eq!(r.compare(&field(dt())), Ordering::Greater);
    assert_eq!(r.compare(&field(Hyperreal::from_rational(q(1, 1000000)))), Ordering::Less);
    assert_eq!(shows(&r.recip().unwrap()), "1/√dt");
    assert!(!r.recip().unwrap().is_limited());
}

#[test]
fn a_polynomial_in_the_root_of_dt_is_held_as_one() {
    let r = sqrt(&field(dt()));
    let x = field(h(1)).add(&r);
    assert_eq!(shows(&x.mul(&x)), "1 + 2·√dt + dt");
    assert_eq!(shows(&r.pow(3).unwrap()), "dt^(3/2)");
    assert_eq!(shows(&x.mul(&x).sub(&field(h(1).add(&dt())))), "2·√dt");
    assert_eq!(x.mul(&x).sub(&x.mul(&x)), field(h(0)));
}

#[test]
fn the_square_root_of_one_plus_dt_prints_as_a_radical() {
    let r = sqrt(&field(h(1).add(&dt())));
    assert_eq!(shows(&r), "√(1 + dt)");
    assert_eq!(r.mul(&r), field(h(1).add(&dt())));
    assert_eq!(shows(&r.neg()), "-√(1 + dt)");
    assert_eq!(r.standard_part(), Some(Algebraic::from_i64(1)));
    assert_eq!(shows(&sqrt(&field(h(4).add(&dt().mul(&h(4)))))), "2·√(1 + dt)");
    assert_eq!(shows(&sqrt(&field(dt().mul(&h(2))))), "√2·√dt");
    let third = field(h(8).add(&dt().mul(&h(8)))).root(3).unwrap();
    assert_eq!(shows(&third), "2·∛(1 + dt)");
}

#[test]
fn a_value_neither_rational_nor_radical_prints_its_leading_terms() {
    let r = sqrt(&field(h(2).add(&dt()))).add(&sqrt(&field(h(2).sub(&dt()))));
    assert_eq!(shows(&r), "2√2 - (√2/16)·dt^2 + O(dt^3)");
    let two_root_two = Algebraic::from_i64(8).sqrt().unwrap();
    assert_eq!(r.standard_part(), Some(two_root_two.clone()));
    assert_eq!(r.compare(&field(Hyperreal::from_algebraic(two_root_two))), Ordering::Less);
}

#[test]
fn equal_values_reached_two_ways_are_equal_and_hash_alike() {
    let a = sqrt(&field(h(2).add(&dt().mul(&h(2)))));
    let root_two = field(Hyperreal::from_algebraic(Algebraic::from_i64(2).sqrt().unwrap()));
    let b = root_two.mul(&sqrt(&field(h(1).add(&dt()))));
    assert_eq!(a, b);
    assert_eq!(a.sub(&b), field(h(0)));
    let hash = |x: &Closed| {
        let mut s = DefaultHasher::new();
        x.hash(&mut s);
        s.finish()
    };
    assert_eq!(hash(&a), hash(&b));
}

#[test]
fn roots_that_agree_to_a_high_order_are_still_told_apart() {
    let d3 = dt().pow(3).unwrap();
    let a = sqrt(&field(h(1).add(&d3)));
    let b = sqrt(&field(h(1).add(&d3).add(&dt().pow(4).unwrap())));
    assert_eq!(a.compare(&b), Ordering::Less);
    assert_ne!(a, b);
    assert!(b.sub(&a).is_infinitesimal());
}

#[test]
fn a_negative_value_has_an_odd_root_and_no_even_one() {
    assert_eq!(field(dt().neg()).root(2).unwrap_err(), "square root of a negative number");
    assert_eq!(field(h(1).neg().add(&dt())).root(4).unwrap_err(), "an even root (4th) of a negative number");
    assert_eq!(shows(&field(dt().neg()).root(3).unwrap()), "-∛dt");
    assert_eq!(field(h(0)).root(2).unwrap(), field(h(0)));
}

#[test]
fn a_series_value_survives_its_bytes() {
    for x in [
        sqrt(&field(dt())),
        sqrt(&field(h(1).add(&dt()))),
        sqrt(&field(h(2).add(&dt()))).add(&sqrt(&field(h(2).sub(&dt())))),
        field(h(3).add(&dt())),
    ] {
        let mut bytes = Vec::new();
        x.encode(&mut bytes);
        let mut pos = 0;
        let back = Closed::decode(&bytes, &mut pos).expect("decodes");
        assert_eq!(pos, bytes.len());
        assert_eq!(back, x);
        assert_eq!(shows(&back), shows(&x));
    }
    assert!(Closed::decode(&[9], &mut 0).is_none());
}

#[test]
fn the_rest_of_an_expansion_is_bounded_on_its_own_grid_or_by_the_next_term() {
    let a = |n: i64| Algebraic::from_i64(n);
    let integral = [(q(0, 1), a(2)), (q(2, 1), a(-1))];
    assert_eq!(remainder_order(&integral, &q(4, 1)), q(3, 1), "the order after dt^2 on the integers");
    let halves = [(q(1, 2), a(1)), (q(3, 2), a(1))];
    assert_eq!(remainder_order(&halves, &q(5, 2)), q(2, 1), "the order after dt^(3/2) on the halves");
    assert_eq!(remainder_order(&halves, &q(5, 3)), q(5, 3), "a finer grid's next term comes sooner");
}

#[test]
fn a_power_of_an_infinite_root_keeps_every_term_its_factors_lower_it_to() {
    let inverse = field(h(1).div(&dt()).unwrap());
    assert_eq!(shows(&inverse.root(2).unwrap().pow(3).unwrap()), "1/dt^(3/2)");
    assert_eq!(shows(&inverse.root(2).unwrap().pow(7).unwrap()), "1/dt^(7/2)");
    assert_eq!(shows(&sqrt(&field(dt())).pow(-7).unwrap()), "1/dt^(7/2)");
    let infinite_sum = sqrt(&field(h(1).div(&dt()).unwrap().add(&h(1))));
    let cube = infinite_sum.pow(3).unwrap();
    assert_eq!(cube.valuation(), Some(q(-3, 2)));
    assert_eq!(cube.compare(&infinite_sum.mul(&infinite_sum).mul(&infinite_sum)), Ordering::Equal);
}

/// A negated series extends past what it had computed as the negation of the value: its rest is the
/// rest at `−z`, not at `z`.
#[test]
fn a_negated_series_extends_as_the_negation_of_its_value() {
    let poly = KPoly::from(vec![h(-2).sub(&dt()), h(0), h(1)]);
    let bs = branches(&poly, 1);
    let separation = separation_of(&bs);
    let positive = bs.into_iter().find(|b| b.terms[0].1.signum() > 0).expect("the positive root of 2 + dt");
    let p = Puiseux::picked_as(poly, 1, positive, separation);
    let negated = p.negated();
    let five = Rational::from_i64(5);
    let far = negated.through(&five);
    assert!(far.len() > 2, "the negation extended past its prefix: {far:?}");
    assert_eq!(far, series_neg(&p.through(&five)));
}

/// A series is read on its branch's own grid: its ramification, its leading order there, and each
/// coefficient, found as far as it is asked for.
#[test]
fn a_series_reads_its_coefficients_on_its_branch_s_grid() {
    let Closed::Series(root) = Closed::from_hyperreal(Hyperreal::dt()).root(2).unwrap() else {
        panic!("√dt is no rational function of dt")
    };
    assert_eq!((root.ramification(), root.order()), (2, 1));
    let read: Vec<String> = (0..4).map(|k| root.coefficient(k).to_string()).collect();
    assert_eq!(read, ["0", "1", "0", "0"]);
    let one_plus = Hyperreal::from_i64(1).add(&Hyperreal::dt());
    let Closed::Series(appreciable) = Closed::from_hyperreal(one_plus).root(2).unwrap() else {
        panic!("√(1 + dt) is no rational function of dt")
    };
    assert_eq!((appreciable.ramification(), appreciable.order()), (1, 0));
    let read: Vec<String> = (0..5).map(|k| appreciable.coefficient(k).to_string()).collect();
    assert_eq!(read, ["1", "1/2", "-1/8", "1/16", "-5/128"]);
}

/// One term of an expansion: a coefficient that is a sum or a quotient is parenthesized, the sign of
/// one that is not a sum stands outside, and the rest of an expansion from the standard order is `O(1)`.
#[test]
fn an_expansion_term_parenthesizes_a_sum_or_a_quotient_and_keeps_a_sign_outside() {
    let e = |n: i64, d: i64| Rational::from_ratio_i64(n, d).unwrap();
    assert_eq!(expansion_term("dt", "1", &e(1, 1)), "dt");
    assert_eq!(expansion_term("dt", "-1", &e(1, 2)), "-√dt");
    assert_eq!(expansion_term("dt", "2", &e(3, 2)), "2·dt^(3/2)");
    assert_eq!(expansion_term("dt", "-1/2", &e(2, 1)), "-(1/2)·dt^2");
    assert_eq!(expansion_term("dt", "1/2", &e(2, 1)), "(1/2)·dt^2");
    assert_eq!(expansion_term("dt", "-(1/2)·e", &e(1, 1)), "-(1/2)·e·dt");
    assert_eq!(expansion_term("dt", "(1/2)·e", &e(1, 1)), "(1/2)·e·dt");
    assert_eq!(expansion_term("dt", "e^2", &e(1, 1)), "e^2·dt");
    assert_eq!(expansion_term("dt", "√2/16", &e(2, 1)), "(√2/16)·dt^2");
    assert_eq!(expansion_term("dt", "-e + 1", &e(1, 1)), "(-e + 1)·dt", "a sum keeps its own sign inside");
    assert_eq!(expansion_term("dt", "e", &e(-1, 1)), "e/dt");
    assert_eq!(expansion_term("dt", "1", &e(-1, 2)), "1/√dt");
    assert_eq!(expansion_term("dt", "-1/2", &e(-1, 1)), "-(1/2)/dt");
    assert_eq!(expansion_term("h", "3", &e(0, 1)), "3");
    assert_eq!(remainder("dt", &e(0, 1)), "O(1)");
    assert_eq!(remainder("dt", &e(2, 3)), "O(dt^(2/3))");
    assert_eq!(remainder("h", &e(2, 1)), "O(h^2)");
}

/// Two rational functions of roots of `dt` meet on the least root both are functions of, and their
/// arithmetic is that field's own: no resultant, and the one form a value always has.
#[test]
fn rational_functions_of_roots_of_dt_compute_in_their_own_field() {
    let half = sqrt(&field(dt()));
    let third = field(dt()).root(3).unwrap();
    assert_eq!(half.field().map(|(_, q)| q), Some(2));
    assert_eq!(shows(&half.add(&third)), "∛dt + √dt");
    assert_eq!(shows(&half.mul(&third)), "dt^(5/6)");
    assert_eq!(half.compare(&third), Ordering::Less, "√dt lies below ∛dt");
    assert_eq!(third.compare(&half), Ordering::Greater);
    assert_eq!(half.add(&third).sub(&third), half, "a difference lands in the one form");
    assert!(matches!(half.mul(&half), Closed::Field(_)), "√dt·√dt is dt itself");
    assert_eq!(shows(&half.pow(3).unwrap()), "dt^(3/2)");
    let appreciable = sqrt(&field(h(1).add(&dt())));
    assert_eq!(appreciable.field(), None, "√(1 + dt) is no rational function of a root of dt");
    assert_eq!(shows(&appreciable.mul(&appreciable)), "1 + dt");
}

/// A power past the degree bound names the root of `dt` its degree counts, and zero to a negative
/// power is refused in the hyperreals' own words.
#[test]
fn a_power_past_the_degree_bound_names_the_root_it_counts_in() {
    let one_plus = field(h(1)).add(&sqrt(&field(dt())));
    assert_eq!(
        one_plus.pow(5000).unwrap_err().describe("dt"),
        "the power would have degree 5000 in √dt; one power reaches at most degree 4,096"
    );
    assert_eq!(
        field(h(1)).add(&field(dt())).pow(5000).unwrap_err().describe("h"),
        "the power would have degree 5000 in h; one power reaches at most degree 4,096"
    );
    assert_eq!(field(h(0)).pow(-1).unwrap_err().describe("dt"), "zero cannot be raised to a negative power");
}
