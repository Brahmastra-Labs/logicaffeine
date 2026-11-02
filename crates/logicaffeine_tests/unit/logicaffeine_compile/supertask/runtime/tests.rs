use super::*;
use std::cell::RefCell;
use std::rc::Rc;

use crate::interpreter::ListRepr;
use crate::ast::stmt::BinaryOpKind;
use crate::semantics::arith::binary_op;
use crate::semantics::exact::{generator_of, new_infinitesimal};
use logicaffeine_base::supertask::check_annihilator;
use logicaffeine_base::Rational;

fn int(n: i64) -> RuntimeValue {
    RuntimeValue::Int(n)
}

fn rat(n: i64, d: i64) -> RuntimeValue {
    RuntimeValue::from_rational(Rational::from_ratio_i64(n, d).unwrap())
}

fn list(values: Vec<RuntimeValue>) -> RuntimeValue {
    RuntimeValue::List(Rc::new(RefCell::new(ListRepr::from_values(values))))
}

fn matrix(rows: Vec<Vec<RuntimeValue>>) -> RuntimeValue {
    list(rows.into_iter().map(list).collect())
}

fn items(v: &RuntimeValue) -> Vec<RuntimeValue> {
    match v {
        RuntimeValue::List(l) => l.borrow().to_values(),
        other => panic!("not a list: {other:?}"),
    }
}

fn show(v: &RuntimeValue) -> String {
    v.to_display_string()
}

fn shown(v: &RuntimeValue) -> Vec<String> {
    items(v).iter().map(show).collect()
}

fn dt() -> RuntimeValue {
    new_infinitesimal("dt", 1, &[0]).unwrap()
}

fn mul(a: &RuntimeValue, b: &RuntimeValue) -> RuntimeValue {
    binary_op(BinaryOpKind::Multiply, a.clone(), b.clone()).unwrap()
}

fn neg(a: &RuntimeValue) -> RuntimeValue {
    binary_op(BinaryOpKind::Subtract, int(0), a.clone()).unwrap()
}

fn h() -> RuntimeValue {
    generator_of(&dt()).unwrap()
}

// ── trip counts ───────────────────────────────────────────────────────────────────────────────

#[test]
fn a_clock_stepped_by_the_infinitesimal_runs_an_infinite_number_of_times() {
    let d = dt();
    assert_eq!(show(&trip_count(&int(0), &d, &int(1), "<").unwrap()), "1/dt");
    assert_eq!(show(&trip_count(&int(0), &d, &int(1), "<=").unwrap()), "1/dt + 1");
    assert_eq!(show(&trip_count(&int(0), &d, &int(5), "<").unwrap()), "5/dt");
    assert_eq!(
        show(&trip_count(&int(0), &mul(&int(2), &d), &int(1), "<").unwrap()),
        "1/(2·dt) when ⌈1/dt⌉ is even, 1/(2·dt) + 1/2 when it is odd"
    );
    assert_eq!(
        show(&trip_count(&rat(1, 2), &d, &int(1), "<").unwrap()),
        "1/(2·dt) when ⌈1/dt⌉ is even, 1/(2·dt) + 1/2 when it is odd"
    );
    assert_eq!(show(&trip_count(&int(5), &neg(&d), &int(0), ">").unwrap()), "5/dt");
    assert_eq!(show(&trip_count(&int(5), &neg(&d), &int(0), ">=").unwrap()), "5/dt + 1");
    assert_eq!(show(&trip_count(&d, &d, &int(1), "<").unwrap()), "1/dt - 1", "a clock that starts one step in");
}

#[test]
fn a_standard_step_counts_a_standard_number_of_steps() {
    let n = |t0: RuntimeValue, s: RuntimeValue, b: RuntimeValue, rel: &str| show(&trip_count(&t0, &s, &b, rel).unwrap());
    assert_eq!(n(int(0), int(1), int(10), "<"), "10");
    assert_eq!(n(int(0), int(1), int(10), "<="), "11");
    assert_eq!(n(int(0), int(3), int(10), "<"), "4");
    assert_eq!(n(int(0), rat(1, 2), int(1), "<"), "2");
    assert_eq!(n(int(10), int(-1), int(0), ">"), "10");
    assert_eq!(n(int(0), int(1), int(0), "<"), "0", "a guard false at the start runs no step");
    assert_eq!(n(int(5), int(1), int(3), "<"), "0");
    assert_eq!(n(int(5), int(-1), int(9), ">"), "0");
}

#[test]
fn a_clock_that_never_reaches_its_bound_is_refused() {
    let d = dt();
    let err = trip_count(&int(0), &neg(&d), &int(1), "<").unwrap_err();
    assert!(err.contains("never reaches its bound"), "{err}");
    assert_eq!(show(&trip_count(&int(0), &int(0), &int(1), "<").unwrap()), "-1", "a standard step that never arrives is left to the loop as written");
    assert_eq!(show(&trip_count(&int(0), &int(-1), &int(1), "<").unwrap()), "-1");
    assert!(trip_count(&int(5), &d, &int(0), ">").unwrap_err().contains("never reaches its bound"));
    assert!(trip_count(&int(0), &int(1), &int(10), "=").unwrap_err().contains("relation"));
}

// ── affine powers ─────────────────────────────────────────────────────────────────────────────

#[test]
fn the_clock_and_counter_of_remark_7() {
    let d = dt();
    let a = matrix(vec![
        vec![int(1), int(0), d.clone()],
        vec![int(0), int(1), int(1)],
        vec![int(0), int(0), int(1)],
    ]);
    let out = affine_power(&a, &list(vec![int(0), int(0), int(1)]), &h()).unwrap();
    assert_eq!(shown(&out), ["1", "1/dt", "1"]);
}

#[test]
fn the_lamp_alternates_with_the_parity_of_the_step_count() {
    let a = matrix(vec![vec![int(-1), int(1)], vec![int(0), int(1)]]);
    let out = affine_power(&a, &list(vec![int(0), int(1)]), &h()).unwrap();
    assert_eq!(shown(&out), ["0 when ⌈1/dt⌉ is even, 1 when it is odd", "1"]);
    let even = generator_of(&new_infinitesimal("dt", 2, &[0]).unwrap()).unwrap();
    assert_eq!(shown(&affine_power(&a, &list(vec![int(0), int(1)]), &even).unwrap()), ["0", "1"]);
    let odd = generator_of(&new_infinitesimal("dt", 2, &[1]).unwrap()).unwrap();
    assert_eq!(shown(&affine_power(&a, &list(vec![int(0), int(1)]), &odd).unwrap()), ["1", "1"]);
}

#[test]
fn a_finite_power_is_the_plain_product() {
    let a = matrix(vec![vec![int(1), int(1)], vec![int(0), int(1)]]);
    assert_eq!(shown(&affine_power(&a, &list(vec![int(0), int(1)]), &int(10)).unwrap()), ["10", "1"]);
    let g = matrix(vec![vec![int(2)]]);
    assert_eq!(shown(&affine_power(&g, &list(vec![int(3)]), &int(5)).unwrap()), ["96"]);
    assert_eq!(shown(&affine_power(&g, &list(vec![int(3)]), &int(0)).unwrap()), ["3"]);
}

#[test]
fn a_running_sum_of_a_counter_is_a_polynomial_in_the_count() {
    let a = matrix(vec![
        vec![int(1), int(0), int(1)],
        vec![int(1), int(1), int(0)],
        vec![int(0), int(0), int(1)],
    ]);
    let out = affine_power(&a, &list(vec![int(0), int(0), int(1)]), &h()).unwrap();
    assert_eq!(shown(&out), ["1/dt", "1/(2·dt^2) - 1/(2·dt)", "1"], "k = H and s = H(H − 1)/2");
    assert!(matches!(items(&out)[1], RuntimeValue::Hypernatural(_)));
}

#[test]
fn a_quarter_turn_cycles_with_period_four() {
    let a = matrix(vec![vec![int(0), int(-1)], vec![int(1), int(0)]]);
    let out = affine_power(&a, &list(vec![int(1), int(0)]), &h()).unwrap();
    assert_eq!(
        shown(&out),
        [
            "1 when ⌈1/dt⌉ ≡ 0 (mod 4), 0 when ⌈1/dt⌉ ≡ 1 (mod 4), -1 when ⌈1/dt⌉ ≡ 2 (mod 4), 0 when ⌈1/dt⌉ ≡ 3 (mod 4)",
            "0 when ⌈1/dt⌉ ≡ 0 (mod 4), 1 when ⌈1/dt⌉ ≡ 1 (mod 4), 0 when ⌈1/dt⌉ ≡ 2 (mod 4), -1 when ⌈1/dt⌉ ≡ 3 (mod 4)"
        ]
    );
}

#[test]
fn a_nilpotent_update_empties_the_state() {
    let a = matrix(vec![vec![int(0), int(1)], vec![int(0), int(0)]]);
    assert_eq!(shown(&affine_power(&a, &list(vec![int(5), int(7)]), &h()).unwrap()), ["0", "0"]);
}

#[test]
fn an_update_that_grows_or_shrinks_geometrically_is_refused_with_its_reason() {
    let err = affine_power(&matrix(vec![vec![int(2)]]), &list(vec![int(1)]), &h()).unwrap_err();
    assert!(err.contains("not a root of unity"), "{err}");
    let err = affine_power(&matrix(vec![vec![rat(1, 2)]]), &list(vec![int(1)]), &h()).unwrap_err();
    assert!(err.contains("not a root of unity"), "{err}");
}

#[test]
fn an_update_one_infinitesimal_step_from_the_identity_flows() {
    let one_plus_dt = binary_op(BinaryOpKind::Add, int(1), dt()).unwrap();
    let end = affine_power(&matrix(vec![vec![one_plus_dt]]), &list(vec![int(1)]), &h()).unwrap();
    let RuntimeValue::List(items) = end else {
        panic!("a state list");
    };
    let x = items.borrow().to_values()[0].clone();
    let standard = crate::semantics::exact::standard_part(&x).unwrap();
    assert_eq!(standard.to_display_string(), "e", "(1 + dt)^(1/dt) is infinitely close to e");
}

#[test]
fn a_shape_mismatch_is_refused() {
    let a = matrix(vec![vec![int(1), int(0)], vec![int(0), int(1)]]);
    assert!(affine_power(&a, &list(vec![int(1), int(2), int(3)]), &h()).is_err());
    let ragged = matrix(vec![vec![int(1), int(0)], vec![int(0)]]);
    assert!(affine_power(&ragged, &list(vec![int(1), int(2)]), &h()).is_err());
    assert!(affine_power(&a, &list(vec![int(1), int(2)]), &int(-1)).is_err());
}

// ── cycles ────────────────────────────────────────────────────────────────────────────────────

#[test]
fn a_recorded_cycle_answers_any_step() {
    let states = list(vec![list(vec![int(0)]), list(vec![int(1)])]);
    let at = |n: &RuntimeValue| shown(&cycle_at(&states, &list(vec![int(0)]), n).unwrap());
    assert_eq!(at(&h()), ["0 when ⌈1/dt⌉ is even, 1 when it is odd"]);
    assert_eq!(at(&int(5)), ["1"]);
    assert_eq!(at(&int(4)), ["0"]);
    let tail = list(vec![list(vec![int(7)]), list(vec![int(3)]), list(vec![int(5)])]);
    let at = |n: &RuntimeValue| shown(&cycle_at(&tail, &list(vec![int(3)]), n).unwrap());
    assert_eq!(at(&int(0)), ["7"]);
    assert_eq!(at(&int(1)), ["3"]);
    assert_eq!(at(&int(2)), ["5"]);
    assert_eq!(at(&int(3)), ["3"]);
    assert_eq!(at(&h()), ["5 when ⌈1/dt⌉ is even, 3 when it is odd"]);
}

#[test]
fn a_repeated_state_missing_from_the_record_is_refused() {
    let states = list(vec![list(vec![int(0)])]);
    assert!(cycle_at(&states, &list(vec![int(9)]), &h()).is_err());
}

// ── search candidates ─────────────────────────────────────────────────────────────────────────

fn candidates(atoms: Vec<(RuntimeValue, RuntimeValue)>, moduli: Vec<RuntimeValue>) -> Vec<i64> {
    let atoms = list(atoms.into_iter().map(|(a, b)| list(vec![a, b])).collect());
    items(&search_candidates(&atoms, &list(moduli)).unwrap())
        .iter()
        .map(|v| match v {
            RuntimeValue::Int(n) => *n,
            other => panic!("{other:?}"),
        })
        .collect()
}

#[test]
fn candidates_cover_every_threshold_and_every_period() {
    assert_eq!(candidates(vec![(int(2), int(-7))], vec![]), vec![0, 1, 3, 4, 5]);
    assert_eq!(candidates(vec![(int(3), int(-12))], vec![]), vec![0, 1, 4, 5]);
    assert_eq!(candidates(vec![], vec![int(2), int(3)]), vec![0, 1, 2, 3, 4, 5, 6]);
    assert_eq!(candidates(vec![(int(1), int(0))], vec![]), vec![0, 1]);
    assert_eq!(candidates(vec![(int(0), int(5))], vec![]), vec![0, 1], "a constant atom has no threshold");
    assert_eq!(candidates(vec![(rat(1, 2), int(-3))], vec![int(2)]), vec![0, 1, 2, 6, 7, 8]);
}

#[test]
fn a_search_with_inexact_or_nonstandard_coefficients_is_not_decided_here() {
    let none = |atoms: Vec<(RuntimeValue, RuntimeValue)>, moduli: Vec<RuntimeValue>| candidates(atoms, moduli).is_empty();
    assert!(none(vec![(RuntimeValue::Float(0.1), int(0))], vec![]));
    assert!(none(vec![(dt(), int(1))], vec![]));
    assert!(none(vec![], vec![int(0)]));
    assert!(none(vec![], vec![int(-2)]));
    assert!(none(vec![], vec![rat(3, 2)]));
    assert!(none(vec![], vec![int(1 << 30)]), "a period past the candidate bound");
}

#[test]
fn every_first_witness_is_among_the_candidates() {
    fn first(p: &dyn Fn(i64) -> bool, ns: impl IntoIterator<Item = i64>) -> Option<i64> {
        ns.into_iter().find(|n| p(*n))
    }
    for a in -3i64..=3 {
        for b in -12i64..=12 {
            for m in 1i64..=4 {
                for c in 0..m {
                    let cands = candidates(vec![(int(a), int(b))], vec![int(m)]);
                    let predicates: [&dyn Fn(i64) -> bool; 5] = [
                        &|n| a * n + b == 0 && n % m == c,
                        &|n| a * n + b > 0 && n % m == c,
                        &|n| a * n + b < 0 && n % m == c,
                        &|n| a * n + b >= 0 || n % m == c,
                        &|n| a * n + b != 0 && n % m != c,
                    ];
                    for (i, p) in predicates.iter().enumerate() {
                        let brute = first(*p, 0..2000);
                        let found = first(*p, cands.iter().copied());
                        assert_eq!(found, brute, "predicate {i} with a={a} b={b} m={m} c={c}: candidates {cands:?}");
                    }
                }
            }
        }
    }
}

#[test]
fn a_remainder_of_a_rational_multiple_repeats_with_the_denominator() {
    let pairs = |entries: Vec<(RuntimeValue, RuntimeValue)>| entries.into_iter().map(|(a, m)| list(vec![a, m])).collect::<Vec<_>>();
    assert_eq!(candidates(vec![], pairs(vec![(int(3), int(4))])), vec![0, 1, 2, 3, 4], "an integer multiple keeps the modulus");
    assert_eq!(candidates(vec![], pairs(vec![(rat(1, 2), int(3))])), vec![0, 1, 2, 3, 4, 5, 6], "(n/2) % 3 repeats every 6");
    assert!(candidates(vec![], pairs(vec![(dt(), int(3))])).is_empty());
    assert!(candidates(vec![], pairs(vec![(int(1), int(0))])).is_empty());
}

// ── state sizes ───────────────────────────────────────────────────────────────────────────────

#[test]
fn a_state_is_measured_by_the_bits_its_values_occupy() {
    assert_eq!(state_size(&int(0)), 64);
    assert_eq!(state_size(&RuntimeValue::Bool(true)), 1);
    let big = RuntimeValue::from_bigint(logicaffeine_base::numeric::BigInt::from_u64(1).shl_bits(1000));
    assert!(state_size(&big) >= 1001 && state_size(&big) < 1200, "{}", state_size(&big));
    let pair = list(vec![int(1), big.clone()]);
    assert_eq!(state_size(&pair), state_size(&int(1)) + state_size(&big));
    assert!(state_size(&rat(1, 3)) >= 2);
    assert!(state_size(&dt()) > 0);
    let text = RuntimeValue::Text(std::rc::Rc::new("abcd".to_string()));
    assert_eq!(state_size(&text), 32);
}

#[test]
fn rotations_of_two_periods_meet_at_their_common_period() {
    let a = matrix(vec![
        vec![int(0), int(-1), int(0), int(0)],
        vec![int(1), int(0), int(0), int(0)],
        vec![int(0), int(0), int(-1), int(-1)],
        vec![int(0), int(0), int(1), int(0)],
    ]);
    let x = list(vec![int(1), int(0), int(1), int(0)]);
    let twelve = generator_of(&new_infinitesimal("dt", 12, &[0]).unwrap()).unwrap();
    assert_eq!(shown(&affine_power(&a, &x, &twelve).unwrap()), ["1", "0", "1", "0"], "twelve steps turn both all the way round");
    let out = shown(&affine_power(&a, &x, &h()).unwrap());
    assert_eq!(out[0], "1 when ⌈1/dt⌉ ≡ 0 (mod 4), 0 when ⌈1/dt⌉ ≡ 1 (mod 4), -1 when ⌈1/dt⌉ ≡ 2 (mod 4), 0 when ⌈1/dt⌉ ≡ 3 (mod 4)");
    assert_eq!(out[2], "1 when ⌈1/dt⌉ ≡ 0 (mod 3), -1 when ⌈1/dt⌉ ≡ 1 (mod 3), 0 when ⌈1/dt⌉ ≡ 2 (mod 3)");
}

#[test]
fn a_jordan_block_at_minus_one_alternates_a_growing_count() {
    let a = matrix(vec![vec![int(-1), int(1)], vec![int(0), int(-1)]]);
    let out = affine_power(&a, &list(vec![int(0), int(1)]), &h()).unwrap();
    assert_eq!(
        shown(&out),
        ["-1/dt when ⌈1/dt⌉ is even, 1/dt when it is odd", "1 when ⌈1/dt⌉ is even, -1 when it is odd"],
        "A^N·(0, 1) = ((−1)^(N−1)·N, (−1)^N)"
    );
    for n in 0..6i64 {
        let finite = shown(&affine_power(&a, &list(vec![int(0), int(1)]), &int(n)).unwrap());
        let sign = if n % 2 == 0 { 1 } else { -1 };
        assert_eq!(finite, [(-sign * n).to_string(), sign.to_string()], "N = {n}");
    }
}

#[test]
fn a_finite_step_past_the_record_reads_the_cycle_not_the_tail() {
    let tail = list(vec![list(vec![int(7)]), list(vec![int(3)]), list(vec![int(5)])]);
    let at = |n: i64| shown(&cycle_at(&tail, &list(vec![int(3)]), &int(n)).unwrap());
    assert_eq!(at(4), ["5"], "the cycle 3, 5 starts at step 1, so step 4 is 5");
    assert_eq!(at(5), ["3"]);
    assert_eq!(at(100), ["5"]);
}

#[test]
fn a_step_that_is_not_a_number_is_not_counted() {
    let text = |t: &str| RuntimeValue::Text(std::rc::Rc::new(t.to_string()));
    assert_eq!(show(&trip_count(&text("a"), &text("a"), &text("aaaa"), "<").unwrap()), "-1");
    assert_eq!(show(&trip_count(&int(0), &RuntimeValue::Bool(true), &int(3), "<").unwrap()), "-1");
    assert_eq!(show(&trip_count(&text("a"), &int(1), &int(3), "<").unwrap()), "-1", "a clock that is not a number");
}

#[test]
fn the_closed_forms_certificate_refuses_a_structure_that_does_not_annihilate() {
    let lamp = vec![vec![int(-1), int(1)], vec![int(0), int(1)]];
    assert!(check_annihilator(&lamp, 0, 2, 1).is_ok(), "(A² − I)·… = 0 for the lamp");
    let err = check_annihilator(&lamp, 0, 1, 1).unwrap_err();
    assert!(err.contains("certificate") && err.contains("failed to check"), "{err}");
    let block = vec![vec![int(1), int(1)], vec![int(0), int(1)]];
    assert!(check_annihilator(&block, 0, 1, 2).is_ok());
    assert!(check_annihilator(&block, 0, 1, 1).is_err(), "a Jordan block needs the square");
    let nil = vec![vec![int(0), int(1)], vec![int(0), int(0)]];
    assert!(check_annihilator(&nil, 2, 1, 0).is_ok());
    assert!(check_annihilator(&nil, 1, 1, 0).is_err());
}

// ── polynomial flows ──────────────────────────────────────────────────────────────────────────

fn add(a: &RuntimeValue, b: &RuntimeValue) -> RuntimeValue {
    binary_op(BinaryOpKind::Add, a.clone(), b.clone()).unwrap()
}

/// `x ← x + x·(1 − x)·dt` as its terms: `(1 + dt)·x` and `−dt·x²`.
fn logistic_update() -> RuntimeValue {
    let d = dt();
    list(vec![list(vec![
        list(vec![add(&int(1), &d), int(1)]),
        list(vec![neg(&d), int(2)]),
    ])])
}

#[test]
fn a_finite_step_count_runs_the_polynomial_update_itself() {
    let update = list(vec![list(vec![list(vec![int(1), int(2)]), list(vec![int(1), int(0)])])]);
    let end = polynomial_flow(&update, &list(vec![int(1)]), &int(3)).unwrap();
    assert_eq!(shown(&end), vec!["26"], "x ← x² + 1 three times from 1: 2, 5, 26");
}

#[test]
fn the_logistic_euler_steps_end_on_its_sigmoid() {
    let end = polynomial_flow(&logistic_update(), &list(vec![rat(1, 2)]), &h()).unwrap();
    let x = items(&end)[0].clone();
    let standard = crate::semantics::exact::standard_part(&x).unwrap();
    assert_eq!(show(&standard), "0.731058578630004879251159241822");
    assert_eq!(show(&x), "0.731058578630004879251159241822 + O(dt)");
}

#[test]
fn a_polynomial_update_that_moves_by_a_standard_amount_is_not_a_flow() {
    let squaring = list(vec![list(vec![list(vec![int(1), int(2)])])]);
    let err = polynomial_flow(&squaring, &list(vec![int(1)]), &h()).unwrap_err();
    assert!(err.contains("standard amount"), "{err}");
    let shifted = list(vec![list(vec![list(vec![int(1), int(1)]), list(vec![int(1), int(2)])])]);
    let err = polynomial_flow(&shifted, &list(vec![int(1)]), &h()).unwrap_err();
    assert!(err.contains("standard amount"), "{err}");
    let shrinking = list(vec![list(vec![list(vec![dt(), int(1)])])]);
    let err = polynomial_flow(&shrinking, &list(vec![int(1)]), &h()).unwrap_err();
    assert!(err.contains("does not keep its state"), "{err}");
}

#[test]
fn a_scalar_flow_certified_to_blow_up_before_its_time_ends_unlimited() {
    let d = dt();
    let squaring = list(vec![list(vec![list(vec![int(1), int(1)]), list(vec![d.clone(), int(2)])])]);
    let two_units = mul(&int(2), &h());
    let end = polynomial_flow(&squaring, &list(vec![int(1)]), &two_units).unwrap();
    let x = items(&end)[0].clone();
    assert_eq!(show(&x), "unlimited");
    assert_eq!(show(&crate::semantics::exact::is_limited(&x).unwrap()), "false");
    let err = crate::semantics::exact::standard_part(&x).unwrap_err();
    assert!(err.contains("has no standard part"), "{err}");
    let half_unit = trip_count(&int(0), &dt(), &rat(1, 2), "<").unwrap();
    let before = polynomial_flow(&squaring, &list(vec![int(1)]), &half_unit).unwrap();
    let standard = crate::semantics::exact::standard_part(&items(&before)[0]).unwrap();
    assert_eq!(show(&standard), "2.00000000000000000000000000000");
}

#[test]
fn a_flow_falling_without_bound_ends_negative_unlimited() {
    let d = dt();
    let falling = list(vec![list(vec![list(vec![int(1), int(1)]), list(vec![neg(&d), int(2)])])]);
    let end = polynomial_flow(&falling, &list(vec![int(-1)]), &mul(&int(2), &h())).unwrap();
    assert_eq!(show(&items(&end)[0]), "-unlimited");
}

// ── first exits ───────────────────────────────────────────────────────────────────────────────

fn text(t: &str) -> RuntimeValue {
    RuntimeValue::Text(Rc::new(t.to_string()))
}

fn sub(a: &RuntimeValue, b: &RuntimeValue) -> RuntimeValue {
    binary_op(BinaryOpKind::Subtract, a.clone(), b.clone()).unwrap()
}

fn reciprocal(v: &RuntimeValue) -> RuntimeValue {
    binary_op(BinaryOpKind::ExactDivide, int(1), v.clone()).unwrap()
}

/// `not ((n <= x and x < n + 1) or (0 - n <= x and x < 0 - n + 1))` as the engine writes it.
fn floor_guard(x: &RuntimeValue) -> RuntimeValue {
    let atom = |rel: &str, a: RuntimeValue, b: RuntimeValue| list(vec![text(rel), a, b]);
    let below = sub(x, &int(1));
    let minus_x = sub(&int(0), x);
    list(vec![
        text("not"),
        list(vec![
            text("or"),
            list(vec![text("and"), atom("<=", int(1), minus_x.clone()), atom("<", int(-1), below.clone())]),
            list(vec![text("and"), atom("<=", int(-1), minus_x), atom("<", int(1), below)]),
        ]),
    ])
}

#[test]
fn the_interpreter_s_values_count_the_floor_loop_to_an_infinite_integer() {
    let h = generator_of(&dt()).unwrap();
    let exit = first_exit(&int(0), &int(1), &floor_guard(&reciprocal(&dt()))).unwrap();
    assert_eq!(show(&exit), show(&h));
    let half = reciprocal(&mul(&int(2), &dt()));
    let exit = first_exit(&int(0), &int(1), &floor_guard(&half)).unwrap();
    assert_eq!(show(&exit), "1/(2·dt) when ⌈1/dt⌉ is even, 1/(2·dt) - 1/2 when it is odd");
    let exit = first_exit(&int(0), &int(1), &floor_guard(&rat(7, 2))).unwrap();
    assert_eq!(show(&exit), "-1", "every number standard: the loop as written");
}

#[test]
fn the_interpreter_s_remainder_reads_a_guard_on_the_classes_it_needs() {
    let guard = list(vec![
        text("or"),
        list(vec![text("<"), int(1), sub(&int(0), &reciprocal(&dt()))]),
        list(vec![text("%"), text("!="), int(1), int(0), int(3), int(0)]),
    ]);
    let exit = first_exit(&int(0), &int(1), &guard).unwrap();
    assert_eq!(
        show(&exit),
        "1/dt when ⌈1/dt⌉ ≡ 0 (mod 3), 1/dt + 2 when ⌈1/dt⌉ ≡ 1 (mod 3), 1/dt + 1 when ⌈1/dt⌉ ≡ 2 (mod 3)"
    );
}
