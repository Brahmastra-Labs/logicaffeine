use super::*;

fn r(n: i64) -> Rational {
    Rational::from_i64(n)
}

fn q(n: i64, d: i64) -> Rational {
    Rational::from_ratio_i64(n, d).unwrap()
}

fn m(rows: &[&[i64]]) -> Vec<Vec<Rational>> {
    rows.iter().map(|row| row.iter().map(|v| r(*v)).collect()).collect()
}

fn dec(x: &ExactReal) -> f64 {
    x.to_decimal(25).parse().unwrap()
}

/// `⌈1/dt⌉` Euler steps of `x′ = a·x` from `x = 1`.
fn euler(a: i64) -> Germ {
    let flow = Flow::new(vec![m(&[&[a]])], r(1), r(0), vec![Germ::from_i64(1)]).unwrap();
    Germ::flow(&flow, 0)
}

// ── flows ─────────────────────────────────────────────────────────────────────────────────────

#[test]
fn the_euler_flow_of_growth_has_standard_part_e() {
    let x = euler(1);
    assert_eq!(x.standard_part().unwrap().unwrap().to_string(), "e");
    assert_eq!(euler(-1).standard_part().unwrap().unwrap().to_string(), "e^(-1)");
    assert_eq!(euler(0).standard_part().unwrap().unwrap().to_string(), "1");
}

#[test]
fn the_euler_flow_series_is_the_series_of_one_plus_dt_to_the_one_over_dt() {
    let x = euler(1);
    let e = std::f64::consts::E;
    for (k, want) in [(0, e), (1, -e / 2.0), (2, 11.0 * e / 24.0), (3, -7.0 * e / 16.0), (4, 2447.0 * e / 5760.0)] {
        let got = dec(&x.coefficient(k).unwrap());
        assert!((got - want).abs() < 1e-12, "dt^{k}: {got} vs {want}");
    }
}

#[test]
fn a_flow_over_a_trip_count_with_a_remainder_moves_its_first_order_term() {
    let flow = Flow::new(vec![m(&[&[1]])], r(1), q(1, 2), vec![Germ::from_i64(1)]).unwrap();
    let x = Germ::flow(&flow, 0);
    assert_eq!(x.standard_part().unwrap().unwrap().to_string(), "e");
    let e = std::f64::consts::E;
    assert!((dec(&x.coefficient(1).unwrap()) - 0.0).abs() < 1e-12, "δ = 1/2 cancels the −e/2 of the plain count: e·(δ − 1/2) = {}", e * 0.0);
}

#[test]
fn the_harmonic_oscillator_flows_to_cosine_and_sine() {
    let flow = Flow::new(vec![m(&[&[0, 1], &[-1, 0]])], r(1), r(0), vec![Germ::from_i64(1), Germ::from_i64(0)]).unwrap();
    let x = Germ::flow(&flow, 0);
    let v = Germ::flow(&flow, 1);
    assert_eq!(x.standard_part().unwrap().unwrap().to_string(), "cos(1)");
    assert_eq!(v.standard_part().unwrap().unwrap().to_string(), "-sin(1)");
    let first = dec(&x.coefficient(1).unwrap());
    assert!(first.is_finite(), "a one-matrix flow has every coefficient");
}

#[test]
fn a_flow_whose_terms_do_not_commute_refuses_past_its_standard_part() {
    let terms = vec![m(&[&[0, 1], &[-1, 0]]), m(&[&[0, 0], &[0, -1]])];
    let flow = Flow::new(terms, r(1), r(0), vec![Germ::from_i64(1), Germ::from_i64(0)]).unwrap();
    let x = Germ::flow(&flow, 0);
    assert_eq!(x.standard_part().unwrap().unwrap().to_string(), "cos(1)");
    assert_eq!(x.coefficient(1).err(), Some(GermError::NotComputed { order: r(1) }));
    assert_eq!(
        GermError::NotComputed { order: r(1) }.to_string(),
        "the dt^1 term of this flow is not computed exactly: its update matrices do not commute"
    );
}

#[test]
fn a_flow_from_an_infinite_start_carries_its_pole() {
    let start = Germ::from_hyperreal(&Hyperreal::dt().recip().unwrap());
    let flow = Flow::new(vec![m(&[&[1]])], r(1), r(0), vec![start]).unwrap();
    let x = Germ::flow(&flow, 0);
    assert_eq!(x.floor(), -1);
    assert_eq!(x.coefficient(-1).unwrap().to_string(), "e");
    assert!(x.standard_part().unwrap().is_none(), "e/dt is infinite");
}

// ── arithmetic, signs and display ─────────────────────────────────────────────────────────────

#[test]
fn a_rational_function_embeds_with_its_laurent_series() {
    let h = Hyperreal::from_i64(1).div(&Hyperreal::from_i64(1).add(&Hyperreal::dt())).unwrap();
    let g = Germ::from_hyperreal(&h);
    for k in 0..6 {
        let want = if k % 2 == 0 { 1 } else { -1 };
        assert_eq!(g.coefficient(k).unwrap().to_string(), want.to_string(), "1/(1 + dt) at dt^{k}");
    }
    let pole = Germ::from_hyperreal(&Hyperreal::from_i64(3).div(&Hyperreal::dt()).unwrap().add(&Hyperreal::from_i64(2)));
    assert_eq!(pole.coefficient(-1).unwrap().to_string(), "3");
    assert_eq!(pole.coefficient(0).unwrap().to_string(), "2");
    assert!(pole.standard_part().unwrap().is_none());
}

#[test]
fn germs_add_multiply_and_divide_as_series() {
    let x = euler(1);
    let y = euler(-1);
    let product = x.mul(&y);
    assert_eq!(dec(&product.coefficient(0).unwrap()), 1.0, "e·e^(-1) = 1 at order 0");
    let ratio = x.div(&x).unwrap();
    assert!((dec(&ratio.coefficient(0).unwrap()) - 1.0).abs() < 1e-20);
    assert!(dec(&ratio.coefficient(1).unwrap()).abs() < 1e-20, "x/x = 1 exactly, order by order");
    let diff = x.sub(&x);
    assert_eq!(diff.signum(), Err(GermError::Flat), "x − x vanishes in every order looked at");
    assert_eq!(diff.recip().err(), Some(GermError::DivisionByZero));
}

#[test]
fn a_germ_takes_the_sign_of_its_first_nonzero_order() {
    assert_eq!(euler(1).signum(), Ok(1));
    let below_e = euler(1).sub(&Germ::from_hyperreal(&Hyperreal::from_i64(3)));
    assert_eq!(below_e.signum(), Ok(-1), "e < 3");
    let e_flat = euler(1).sub(&euler(1).add(&Germ::from_hyperreal(&Hyperreal::dt())));
    assert_eq!(e_flat.signum(), Ok(-1), "x − (x + dt) is −dt");
    assert_eq!(Germ::from_hyperreal(&Hyperreal::dt().neg()).signum(), Ok(-1));
}

#[test]
fn a_germ_prints_its_two_leading_orders() {
    assert_eq!(euler(1).render("dt"), "e - (1/2)·e·dt + O(dt^2)");
    assert_eq!(Germ::from_hyperreal(&Hyperreal::dt()).render("dt"), "dt + O(dt^2)");
    let shifted = Germ::from_hyperreal(&Hyperreal::from_i64(5).add(&Hyperreal::dt().mul(&Hyperreal::dt())));
    assert_eq!(shifted.render("h"), "5 + h^2 + O(h^3)");
}

// ── encoding ──────────────────────────────────────────────────────────────────────────────────

#[test]
fn a_germ_round_trips_through_its_construction() {
    use crate::exact_codec::ExactCodec;
    let flow = Flow::new(vec![m(&[&[0, 1], &[-1, 0]])], r(1), q(1, 3), vec![Germ::from_i64(1), Germ::from_hyperreal(&Hyperreal::dt())]).unwrap();
    let samples = vec![
        euler(1),
        Germ::flow(&flow, 1),
        euler(1).mul(&euler(-1)).add(&Germ::from_hyperreal(&Hyperreal::dt())),
        euler(1).recip().unwrap().neg(),
    ];
    for g in samples {
        let bytes = g.to_bytes();
        let back = Germ::from_bytes(&bytes).expect("decodes");
        assert_eq!(back.to_bytes(), bytes, "canonical");
        for k in 0..3 {
            assert_eq!(back.coefficient(k).unwrap().to_decimal(20), g.coefficient(k).unwrap().to_decimal(20), "dt^{k}");
        }
    }
}

#[test]
fn a_malformed_flow_is_refused() {
    use crate::exact_codec::ExactCodec;
    let flow = Flow::new(vec![m(&[&[1]])], r(1), r(0), vec![Germ::from_i64(1)]).unwrap();
    let mut bytes = Germ::flow(&flow, 0).to_bytes();
    let last = bytes.len() - 1;
    bytes[last] = 7;
    assert!(Germ::from_bytes(&bytes).is_none(), "a component past the width");
    assert!(Germ::from_bytes(&[9]).is_none(), "an unknown tag");
    let zero = Germ::from_i64(1).sub(&Germ::from_i64(1));
    let mut recip = vec![4u8];
    recip.extend(zero.to_bytes());
    assert!(Germ::from_bytes(&recip).is_none(), "the reciprocal of zero");
}

#[test]
fn a_computable_real_meets_the_infinitesimal_as_a_germ() {
    use crate::exact_codec::ExactCodec;
    let pi = Germ::constant(&ExactReal::pi());
    let near = pi.add(&Germ::from_hyperreal(&Hyperreal::dt()));
    assert_eq!(near.render("dt"), "π + dt + O(dt^2)");
    assert_eq!(near.standard_part().unwrap().unwrap().to_string(), "π");
    assert_eq!(near.sub(&pi).signum(), Ok(1), "π + dt exceeds π by an infinitesimal");
    assert!(near.as_hyperreal().is_none());
    assert!(Germ::from_hyperreal(&Hyperreal::dt()).as_hyperreal().is_some());
    let back = Germ::from_bytes(&near.to_bytes()).unwrap();
    assert_eq!(back.render("dt"), "π + dt + O(dt^2)");
}

// ── nonlinear flows: standard parts and unlimited values ─────────────────────────────────────

#[test]
fn a_standard_germ_knows_only_its_standard_part() {
    let x = Germ::standard(&ExactReal::from_i64(3).sqrt().unwrap());
    assert_eq!(x.coefficient(0).unwrap().to_string(), "√3");
    assert_eq!(x.coefficient(-1).unwrap().to_string(), "0", "a standard germ is limited");
    assert_eq!(x.coefficient(1).err(), Some(GermError::StandardOnly { order: r(1) }));
    assert!(x.coefficient(1).unwrap_err().to_string().contains("known by its standard part"));
    assert_eq!(x.standard_part().unwrap().unwrap().to_string(), "√3");
    assert_eq!(x.signum(), Ok(1));
    assert_eq!(x.render("dt"), "√3 + O(dt)");
    assert_eq!(Germ::standard(&ExactReal::from_i64(-2)).signum(), Ok(-1));
}

#[test]
fn an_unlimited_germ_has_a_sign_and_no_standard_part() {
    let up = Germ::unlimited(1);
    let down = Germ::unlimited(-7);
    assert_eq!(up.signum(), Ok(1));
    assert_eq!(down.signum(), Ok(-1), "only the sign of the argument counts");
    assert!(matches!(up.standard_part(), Ok(None)));
    assert_eq!(up.render("dt"), "unlimited");
    assert_eq!(down.render("dt"), "-unlimited");
    assert_eq!(up.neg().signum(), Ok(-1));
}

#[test]
fn unlimited_dominates_what_is_limited() {
    let up = Germ::unlimited(1);
    let three = Germ::from_i64(3);
    assert_eq!(up.sub(&three).signum(), Ok(1), "unlimited − 3 is still beyond every bound");
    assert_eq!(three.sub(&up).signum(), Ok(-1));
    assert_eq!(up.add(&Germ::unlimited(1)).signum(), Ok(1));
    assert_eq!(up.mul(&Germ::from_i64(-2)).signum(), Ok(-1), "a negative factor flips it");
    assert_eq!(
        up.mul(&Germ::standard(&ExactReal::from_i64(5))).unlimited_sign(),
        Some(1)
    );
    assert!(matches!(up.sub(&three).standard_part(), Ok(None)));
}

#[test]
fn opposite_unlimited_values_cancel_to_something_unknown() {
    let sum = Germ::unlimited(1).add(&Germ::unlimited(-1));
    assert_eq!(sum.unlimited_sign(), None, "∞ − ∞ is not decided by signs alone");
    assert!(sum.signum().is_err());
}

#[test]
fn an_unlimited_value_times_an_infinitesimal_is_not_decided() {
    let product = Germ::unlimited(1).mul(&Germ::from_hyperreal(&Hyperreal::dt()));
    assert_eq!(product.unlimited_sign(), None, "dt has standard part 0, so the order is unknown");
}

#[test]
fn standard_and_unlimited_germs_round_trip_through_their_encoding() {
    use crate::exact_codec::ExactCodec;
    for g in [
        Germ::standard(&ExactReal::from_i64(2).sqrt().unwrap()),
        Germ::unlimited(1),
        Germ::unlimited(-1),
    ] {
        let mut bytes = Vec::new();
        g.write(&mut bytes);
        let mut pos = 0;
        let back = Germ::read(&bytes, &mut pos).expect("decodes");
        assert_eq!(pos, bytes.len());
        assert_eq!(back.render("dt"), g.render("dt"));
        let mut again = Vec::new();
        back.write(&mut again);
        assert_eq!(again, bytes);
    }
    let mut pos = 0;
    assert!(Germ::read(&[8, 2], &mut pos).is_none(), "an unlimited value's sign byte is 0 or 1");
}

// ── fractional orders ─────────────────────────────────────────────────────────────────────────

fn dt_germ() -> Germ {
    Germ::from_hyperreal(&Hyperreal::dt())
}

fn series(h: Hyperreal) -> Germ {
    Germ::from_hyperreal(&h)
}

fn h(n: i64) -> Hyperreal {
    Hyperreal::from_i64(n)
}

fn dt_pow(k: i32) -> Hyperreal {
    Hyperreal::dt().pow(k).unwrap()
}

/// The `n`-th root of `x` in the real closure, read as a germ.
fn root_of(x: Hyperreal, n: u32) -> Germ {
    match crate::puiseux::Closed::from_hyperreal(x).root(n).unwrap() {
        crate::puiseux::Closed::Series(p) => Germ::from_puiseux(&p),
        crate::puiseux::Closed::Field(f) => panic!("{f:?} is a rational function of dt"),
    }
}

fn shown(g: &Germ, orders: std::ops::Range<i64>) -> Vec<String> {
    orders.map(|k| g.coefficient(k).unwrap().to_string()).collect()
}

/// A value of the real closure is read on its branch's own grid, its leading order its floor.
#[test]
fn a_value_of_the_real_closure_is_read_on_its_own_grid() {
    let half = root_of(Hyperreal::dt(), 2);
    assert_eq!((half.ramification(), half.floor()), (2, 1));
    assert_eq!(shown(&half, 0..4), ["0", "1", "0", "0"]);
    assert_eq!(half.render("dt"), "√dt + O(dt)");
    let third = root_of(Hyperreal::dt(), 3);
    assert_eq!((third.ramification(), third.floor()), (3, 1));
    assert_eq!(third.render("dt"), "∛dt + O(dt^(2/3))");
    let appreciable = root_of(h(1).add(&Hyperreal::dt()), 2);
    assert_eq!(appreciable.ramification(), 1, "√(1 + dt) is a series in dt itself");
    assert_eq!(shown(&appreciable, 0..4), ["1", "1/2", "-1/8", "1/16"]);
    assert_eq!(appreciable.render("dt"), "1 + (1/2)·dt + O(dt^2)");
    let mixed = root_of(Hyperreal::dt().add(&dt_pow(2)), 2);
    assert_eq!(shown(&mixed, 0..6), ["0", "1", "0", "1/2", "0", "-1/8"], "√(dt + dt²) = √dt·√(1 + dt)");
}

/// Two germs meet on the least grid both lie on, each refined to it, and every operation reads
/// them there.
#[test]
fn germs_on_two_grids_meet_on_the_least_both_lie_on() {
    let half = root_of(Hyperreal::dt(), 2);
    let third = root_of(Hyperreal::dt(), 3);
    let sum = half.add(&third);
    assert_eq!((sum.ramification(), sum.floor()), (6, 2));
    assert_eq!(shown(&sum, 0..5), ["0", "0", "1", "1", "0"]);
    assert_eq!(sum.render("dt"), "∛dt + √dt + O(dt^(2/3))");
    let product = half.mul(&dt_germ());
    assert_eq!((product.ramification(), product.floor()), (2, 1));
    assert_eq!(product.render("dt"), "dt^(3/2) + O(dt^2)");
    let near_pi = half.add(&Germ::constant(&ExactReal::pi()));
    assert_eq!(near_pi.render("dt"), "π + √dt + O(dt)");
    assert_eq!(near_pi.standard_part().unwrap().unwrap().to_string(), "π");
    assert_eq!(near_pi.sub(&Germ::constant(&ExactReal::pi())).signum(), Ok(1));
    assert_eq!(half.sub(&dt_germ()).signum(), Ok(1), "√dt exceeds dt");
    assert_eq!(dt_germ().sub(&half).signum(), Ok(-1));
    let pole = half.recip().unwrap();
    assert_eq!((pole.ramification(), pole.floor()), (2, -1));
    assert_eq!(pole.render("dt"), "1/√dt + O(1)");
    assert!(pole.standard_part().unwrap().is_none(), "1/√dt is infinite");
    let quotient = dt_germ().div(&half).unwrap();
    assert_eq!(shown(&quotient, 0..3), ["0", "1", "0"], "dt/√dt = √dt");
}

// ── the exponential, the logarithm and roots ──────────────────────────────────────────────────

#[test]
fn the_exponential_of_a_limited_germ_is_its_series() {
    let e = dt_germ().exp().unwrap();
    assert_eq!(shown(&e, 0..5), ["1", "1", "1/2", "1/6", "1/24"]);
    assert_eq!(e.render("dt"), "1 + dt + O(dt^2)");
    let shifted = series(h(2).add(&Hyperreal::dt())).exp().unwrap();
    assert_eq!(shown(&shifted, 0..2), ["e^2", "e^2"]);
    assert_eq!(shifted.render("dt"), "e^2 + e^2·dt + O(dt^2)");
    let fractional = root_of(Hyperreal::dt(), 2).exp().unwrap();
    assert_eq!(fractional.ramification(), 2);
    assert_eq!(shown(&fractional, 0..5), ["1", "1", "1/2", "1/6", "1/24"]);
    assert_eq!(fractional.render("dt"), "1 + √dt + O(dt)");
    let cancelled = shifted.mul(&dt_germ().neg().exp().unwrap());
    assert_eq!(shown(&cancelled, 0..4), ["e^2", "0", "0", "0"], "e^(2 + dt)·e^(−dt) = e² at every order");
}

/// The exponential of an infinite value lies beyond every power of `dt`, above them all or below.
#[test]
fn an_infinite_value_has_no_exponential_germ() {
    let reciprocal = Hyperreal::dt().recip().unwrap();
    assert_eq!(series(reciprocal.clone()).exp().err(), Some(GermError::BeyondEveryPower));
    assert_eq!(series(reciprocal.neg()).exp().err(), Some(GermError::BeyondEveryPower), "e^(−1/dt) lies below every power");
    assert_eq!(Germ::unlimited(1).exp().err(), Some(GermError::BeyondEveryPower));
    assert_eq!(Germ::unlimited(-1).exp().err(), Some(GermError::BeyondEveryPower));
    assert_eq!(
        GermError::BeyondEveryPower.to_string(),
        "the exponential of this value is beyond every power of dt"
    );
}

#[test]
fn the_logarithm_of_an_appreciable_positive_germ_is_its_series() {
    let l = series(h(1).add(&Hyperreal::dt())).ln().unwrap();
    assert_eq!(shown(&l, 0..5), ["0", "1", "-1/2", "1/3", "-1/4"]);
    assert_eq!(l.render("dt"), "dt - (1/2)·dt^2 + O(dt^3)");
    let back = dt_germ().exp().unwrap().ln().unwrap();
    assert_eq!(shown(&back, 0..6), ["0", "1", "0", "0", "0", "0"], "ln(exp(dt)) = dt order by order");
    let round = series(h(2).add(&Hyperreal::dt())).ln().unwrap().exp().unwrap();
    assert_eq!(shown(&round, 0..4), ["2", "1", "0", "0"], "exp(ln(2 + dt)) = 2 + dt order by order");
    let flow = euler(1).ln().unwrap();
    assert_eq!(shown(&flow, 0..4), ["1", "-1/2", "1/3", "-1/4"], "ln((1 + dt)^(1/dt)) = ln(1 + dt)/dt");
    let fractional = root_of(h(1).add(&Hyperreal::dt()), 2).add(&root_of(Hyperreal::dt(), 2)).ln().unwrap();
    assert_eq!(fractional.ramification(), 2);
    assert_eq!(shown(&fractional, 0..3), ["0", "1", "0"], "ln(√(1 + dt) + √dt) = √dt − … with no dt term");
}

/// The logarithm of a positive infinitesimal carries `ln(dt)`, of an infinite value `ln(1/dt)`; a
/// negative value has none.
#[test]
fn the_logarithm_of_what_is_not_appreciable_and_positive_is_refused() {
    let reciprocal = Hyperreal::dt().recip().unwrap();
    assert_eq!(dt_germ().ln().err(), Some(GermError::CarriesLog { infinite: false }));
    assert_eq!(root_of(Hyperreal::dt(), 2).ln().err(), Some(GermError::CarriesLog { infinite: false }));
    assert_eq!(series(reciprocal.clone()).ln().err(), Some(GermError::CarriesLog { infinite: true }));
    assert_eq!(series(reciprocal.neg()).ln().err(), Some(GermError::Nonpositive));
    assert_eq!(series(h(-1).sub(&Hyperreal::dt())).ln().err(), Some(GermError::Nonpositive));
    assert_eq!(series(Hyperreal::dt().neg()).ln().err(), Some(GermError::Nonpositive));
    assert_eq!(Germ::unlimited(1).ln().err(), Some(GermError::CarriesLog { infinite: true }));
    assert_eq!(Germ::unlimited(-1).ln().err(), Some(GermError::Nonpositive));
    assert_eq!(GermError::Nonpositive.to_string(), "logarithm of a nonpositive number");
    assert_eq!(GermError::CarriesLog { infinite: false }.to_string(), "the logarithm of this value carries ln(dt)");
    assert_eq!(GermError::CarriesLog { infinite: true }.to_string(), "the logarithm of this value carries ln(1/dt)");
}

/// A root refines a germ's grid by as little as makes its index divide the leading order.
#[test]
fn a_root_of_a_germ_refines_its_grid_until_the_index_divides_its_order() {
    let half = dt_germ().root(2).unwrap();
    assert_eq!((half.ramification(), half.floor()), (2, 1));
    assert_eq!(half.render("dt"), "√dt + O(dt)");
    let three_halves = series(dt_pow(3)).root(2).unwrap();
    assert_eq!((three_halves.ramification(), three_halves.floor()), (2, 3));
    assert_eq!(three_halves.render("dt"), "dt^(3/2) + O(dt^2)");
    let fourth = series(dt_pow(2)).root(4).unwrap();
    assert_eq!((fourth.ramification(), fourth.floor()), (2, 1), "∜(dt²) = √dt");
    let even = series(dt_pow(2)).root(2).unwrap();
    assert_eq!((even.ramification(), even.floor()), (1, 1), "√(dt²) = dt on dt's own grid");
    let appreciable = series(h(1).add(&Hyperreal::dt())).root(2).unwrap();
    assert_eq!(appreciable.ramification(), 1);
    assert_eq!(shown(&appreciable, 0..4), ["1", "1/2", "-1/8", "1/16"]);
    let cube = series(h(8).add(&Hyperreal::dt().mul(&h(8)))).root(3).unwrap();
    assert_eq!(shown(&cube, 0..3), ["2", "2/3", "-2/9"], "∛(8 + 8·dt) = 2·∛(1 + dt)");
    let negative = series(Hyperreal::dt().neg()).root(3).unwrap();
    assert_eq!(negative.render("dt"), "-∛dt + O(dt^(2/3))");
    assert_eq!(series(Hyperreal::dt().neg()).root(2).err(), Some(GermError::NegativeRoot(2)));
    assert_eq!(series(h(-1).sub(&Hyperreal::dt())).root(4).err(), Some(GermError::NegativeRoot(4)));
    assert_eq!(GermError::NegativeRoot(2).to_string(), "square root of a negative number");
    assert_eq!(GermError::NegativeRoot(4).to_string(), "an even root (4th) of a negative number");
    assert_eq!(Germ::unlimited(1).root(2).unwrap().unlimited_sign(), Some(1));
    assert_eq!(Germ::unlimited(-1).root(3).unwrap().unlimited_sign(), Some(-1));
    assert_eq!(Germ::unlimited(-1).root(2).err(), Some(GermError::NegativeRoot(2)));
    assert_eq!(dt_germ().sub(&dt_germ()).root(2).err(), Some(GermError::Flat), "a root of what may be zero is not guessed");
}

/// The root of a germ with transcendental coefficients follows them order by order, and its power is
/// the germ again.
#[test]
fn a_root_of_a_transcendental_germ_follows_its_coefficients() {
    let near_pi = Germ::constant(&ExactReal::pi()).add(&dt_germ());
    let root = near_pi.root(2).unwrap();
    let p = std::f64::consts::PI;
    assert!((dec(&root.coefficient(0).unwrap()) - p.sqrt()).abs() < 1e-12);
    assert!((dec(&root.coefficient(1).unwrap()) - 1.0 / (2.0 * p.sqrt())).abs() < 1e-12);
    assert!((dec(&root.coefficient(2).unwrap()) + 1.0 / (8.0 * p.powf(1.5))).abs() < 1e-12);
    let square = root.mul(&root);
    for k in 0..4 {
        let gap = square.coefficient(k).unwrap().sub(&near_pi.coefficient(k).unwrap());
        let (lo, hi) = gap.enclosure(80).unwrap();
        assert!(!lo.is_positive() && !hi.is_negative(), "dt^{k}: the root's square differs by [{lo}, {hi}]");
    }
    let e_root = euler(1).root(2).unwrap();
    assert_eq!(e_root.coefficient(0).unwrap().to_string(), "e^(1/2)");
    assert_eq!(e_root.coefficient(1).unwrap().to_string(), "-(1/4)·e^(1/2)");
}

// ── flows from fractional orders ──────────────────────────────────────────────────────────────

/// A flow whose start has fractional orders runs on the start's grid: `⌈1/dt⌉` Euler steps of
/// `x′ = x` from `√dt` leave `e·√dt·(1 − dt/2 + …)`.
#[test]
fn a_flow_from_a_root_of_dt_runs_on_the_root_s_grid() {
    let flow = Flow::new(vec![m(&[&[1]])], r(1), r(0), vec![root_of(Hyperreal::dt(), 2)]).unwrap();
    let x = Germ::flow(&flow, 0);
    assert_eq!((x.ramification(), x.floor()), (2, 1));
    assert_eq!(shown(&x, 0..4), ["0", "e", "0", "-(1/2)·e"]);
    assert_eq!(x.render("dt"), "e·√dt - (1/2)·e·dt^(3/2) + O(dt^2)");
    let drift = Flow::new(
        vec![m(&[&[0, 1], &[0, 0]])],
        r(1),
        r(0),
        vec![Germ::from_i64(1), root_of(Hyperreal::dt(), 2)],
    )
    .unwrap();
    let position = Germ::flow(&drift, 0);
    assert_eq!(position.ramification(), 2, "every component runs on the start's least common grid");
    assert_eq!(shown(&position, 0..4), ["1", "1", "0", "0"], "x′ = v, v′ = 0 from (1, √dt) ends at 1 + √dt");
}

/// Without commuting update terms a flow knows its orders below its start's lowest one plus `dt`;
/// the first it does not know is named by its fractional exponent.
#[test]
fn a_flow_whose_terms_do_not_commute_names_its_first_unknown_fractional_term() {
    let terms = vec![m(&[&[0, 1], &[-1, 0]]), m(&[&[0, 0], &[0, -1]])];
    let flow = Flow::new(terms, r(1), r(0), vec![Germ::from_i64(1), root_of(Hyperreal::dt(), 2)]).unwrap();
    let x = Germ::flow(&flow, 0);
    assert_eq!(x.coefficient(0).unwrap().to_string(), "cos(1)");
    assert_eq!(x.coefficient(1).unwrap().to_string(), "sin(1)");
    assert_eq!(x.coefficient(2).err(), Some(GermError::NotComputed { order: r(1) }));
    assert_eq!(x.coefficient(3).err(), Some(GermError::NotComputed { order: q(3, 2) }));
    assert_eq!(
        GermError::NotComputed { order: q(3, 2) }.to_string(),
        "the dt^(3/2) term of this flow is not computed exactly: its update matrices do not commute"
    );
    assert_eq!(
        GermError::StandardOnly { order: q(1, 2) }.to_string(),
        "the dt^(1/2) term of this value is not known: a nonlinear flow is known by its standard part"
    );
}

// ── encoding fractional orders and functions ──────────────────────────────────────────────────

/// Every new node is written by what built it and read back by building it again; a germ refined to a
/// finer grid is written as itself, its reader meeting the grids again.
#[test]
fn germs_of_fractional_orders_and_their_functions_round_trip() {
    use crate::exact_codec::ExactCodec;
    let flow = Flow::new(vec![m(&[&[1]])], r(1), r(0), vec![root_of(Hyperreal::dt(), 2)]).unwrap();
    let samples = vec![
        root_of(Hyperreal::dt(), 2),
        root_of(Hyperreal::dt(), 2).add(&Germ::constant(&ExactReal::pi())),
        root_of(Hyperreal::dt(), 3).mul(&euler(1)),
        dt_germ().exp().unwrap(),
        root_of(Hyperreal::dt(), 2).exp().unwrap(),
        series(h(1).add(&Hyperreal::dt())).ln().unwrap(),
        euler(1).root(2).unwrap(),
        series(dt_pow(3)).root(2).unwrap(),
        Germ::flow(&flow, 0),
    ];
    for g in samples {
        let bytes = g.to_bytes();
        let back = Germ::from_bytes(&bytes).expect("decodes");
        assert_eq!(back.to_bytes(), bytes, "canonical");
        assert_eq!((back.ramification(), back.floor()), (g.ramification(), g.floor()));
        for k in g.floor()..g.floor() + 4 {
            assert_eq!(back.coefficient(k).unwrap().to_decimal(20), g.coefficient(k).unwrap().to_decimal(20), "order {k}");
        }
    }
}

#[test]
fn a_malformed_function_of_a_germ_is_refused() {
    use crate::exact_codec::ExactCodec;
    let tagged = |tag: u8, inner: &Germ, tail: &[u8]| {
        let mut bytes = vec![tag];
        bytes.extend(inner.to_bytes());
        bytes.extend(tail);
        bytes
    };
    let infinite = series(Hyperreal::dt().recip().unwrap());
    assert!(Germ::from_bytes(&tagged(11, &infinite, &[])).is_none(), "the exponential of an infinite value");
    assert!(Germ::from_bytes(&tagged(12, &dt_germ(), &[])).is_none(), "the logarithm of an infinitesimal");
    assert!(Germ::from_bytes(&tagged(13, &dt_germ().neg(), &[2])).is_none(), "a square root of a negative value");
    assert!(Germ::from_bytes(&tagged(13, &dt_germ(), &[1])).is_none(), "a first root is never written");
    assert!(Germ::from_bytes(&tagged(13, &dt_germ(), &[0])).is_none(), "nor a zeroth");
    assert!(Germ::from_bytes(&tagged(13, &dt_germ(), &[33])).is_none(), "an index past the wire's bound");
    let cube = Germ::from_bytes(&tagged(13, &dt_germ(), &[3])).expect("a cube root");
    assert_eq!(cube.render("dt"), "∛dt + O(dt^(2/3))");
    assert!(Germ::from_bytes(&[9]).is_none(), "a refined germ is written as itself, so no tag names one");
}

// ── flows whose update lies on a root of dt ──────────────────────────────────────────────────

/// `⌈1/dt⌉` steps of `x ← (1 + 2·dt + dt^(3/2))·x` from 1: `N·log A = 2 + √dt − 2·dt − …`, so the state
/// is `e²·(1 + √dt − (3/2)·dt + …)`.
#[test]
fn a_flow_whose_update_lies_on_a_root_of_dt_runs_on_that_root_s_grid() {
    let flow = Flow::on_grid(2, vec![m(&[&[0]]), m(&[&[2]]), m(&[&[1]])], r(1), r(0), vec![Germ::from_i64(1)]).unwrap();
    let x = Germ::flow(&flow, 0);
    assert_eq!((x.ramification(), x.floor()), (2, 0));
    assert_eq!(shown(&x, 0..3), ["e^2", "e^2", "-(3/2)·e^2"]);
    assert_eq!(x.render("dt"), "e^2 + e^2·√dt + O(dt)");
    let from_root = Flow::on_grid(2, vec![m(&[&[0]]), m(&[&[1]])], r(1), r(0), vec![root_of(Hyperreal::dt(), 3)]).unwrap();
    let y = Germ::flow(&from_root, 0);
    assert_eq!((y.ramification(), y.floor()), (6, 2), "an update on halves and a start on thirds meet on sixths");
    assert_eq!(shown(&y, 0..5), ["0", "0", "e", "0", "0"]);
}

/// A step of a flow has no term between `dt⁰` and `dt¹`, and has a term of `dt`.
#[test]
fn an_update_with_a_term_below_dt_or_none_of_dt_is_no_flow() {
    let one = || vec![Germ::from_i64(1)];
    assert!(Flow::on_grid(2, vec![m(&[&[1]]), m(&[&[1]])], r(1), r(0), one()).is_none(), "a √dt term");
    assert!(Flow::on_grid(2, vec![m(&[&[0]])], r(1), r(0), one()).is_none(), "no term of dt");
    assert!(Flow::on_grid(0, vec![m(&[&[1]])], r(1), r(0), one()).is_none(), "no grid");
    assert!(Flow::on_grid(1, vec![m(&[&[1]])], r(1), r(0), one()).is_some());
}

/// Without commuting terms a flow knows every order below its exponent's first nonzero `C_j`: with
/// `A_2 = A_1²/2` the exponent has no `dt` term, so the start's `dt` term flows by `e^{A_1}` alone.
#[test]
fn a_flow_whose_terms_do_not_commute_knows_every_order_below_its_first_exponent_term() {
    let terms = vec![m(&[&[0, 1], &[0, 0]]), m(&[&[0, 0], &[0, 0]]), m(&[&[0, 0], &[1, 0]])];
    let flow = Flow::new(terms, r(1), r(0), vec![Germ::from_i64(1), dt_germ()]).unwrap();
    let x = Germ::flow(&flow, 0);
    let v = Germ::flow(&flow, 1);
    assert_eq!(shown(&x, 0..2), ["1", "1"]);
    assert_eq!(shown(&v, 0..2), ["0", "1"]);
    assert_eq!(x.coefficient(2).err(), Some(GermError::NotComputed { order: r(2) }));
}

#[test]
fn a_flow_on_a_root_of_dt_round_trips_and_a_malformed_one_is_refused() {
    use crate::exact_codec::ExactCodec;
    let flow = Flow::on_grid(2, vec![m(&[&[0]]), m(&[&[2]]), m(&[&[1]])], r(1), r(0), vec![Germ::from_i64(1)]).unwrap();
    let g = Germ::flow(&flow, 0);
    let bytes = g.to_bytes();
    assert_eq!(bytes[0], 14);
    let back = Germ::from_bytes(&bytes).expect("decodes");
    assert_eq!(back.to_bytes(), bytes, "canonical");
    assert_eq!(shown(&back, 0..3), shown(&g, 0..3));
    let plain = Germ::flow(&Flow::new(vec![m(&[&[1]])], r(1), r(0), vec![Germ::from_i64(1)]).unwrap(), 0);
    assert_eq!(plain.to_bytes()[0], 5, "a flow on dt's own grid is written as it always was");
    let mut one = bytes.clone();
    one[1] = 1;
    assert!(Germ::from_bytes(&one).is_none(), "a grid of one is written as 5");
    let written = |below_dt: i64| {
        use crate::exact_codec::write_len;
        let mut b = vec![14];
        write_len(2, &mut b);
        write_len(1, &mut b);
        write_len(2, &mut b);
        r(below_dt).write(&mut b);
        r(2).write(&mut b);
        r(1).write(&mut b);
        r(0).write(&mut b);
        Germ::from_i64(1).write(&mut b);
        0u64.write(&mut b);
        b
    };
    assert!(Germ::from_bytes(&written(0)).is_some());
    assert!(Germ::from_bytes(&written(1)).is_none(), "an update with a √dt term is no flow");
}

/// A sign looks through the same orders of `dt` whatever the grid: `dt^(81/2)` lies past 64 orders of
/// `√dt` and within 64 of `dt`.
#[test]
fn a_sign_looks_through_the_same_orders_of_dt_on_every_grid() {
    let deep = root_of(Hyperreal::dt(), 2).mul(&series(dt_pow(40)));
    assert_eq!(deep.ramification(), 2);
    assert_eq!(deep.signum(), Ok(1));
    assert_eq!(deep.leading().unwrap().0, 81);
}

/// A series whose polynomial lies on `dt`'s own grid can follow a branch on a finer one: `√(dt + dt²)` is
/// a root of `y² − dt − dt²`, and its branch is a series in `√dt`.
#[test]
fn a_series_is_read_on_its_branch_s_grid_not_its_polynomial_s() {
    let mixed = root_of(Hyperreal::dt().add(&dt_pow(2)), 2);
    assert_eq!((mixed.ramification(), mixed.floor()), (2, 1));
    let near_pi = mixed.add(&Germ::constant(&ExactReal::pi()));
    assert_eq!(near_pi.render("dt"), "π + √dt + O(dt)");
}
