//! The supertask runtime run on base's own value model (`tower::tests::Val`): every algorithm here is
//! generic over [`ValueAlgebra`], so these pass without the interpreter.

use super::*;
use crate::numeric::Rational;
use crate::tower::tests::{compare, dt, even_dt, int, show, Val};
use crate::tower::{generator_of, standard_part, Rel, TowerValue};

fn list(values: Vec<Val>) -> Val {
    Val::List(values)
}

fn ints(values: &[i64]) -> Val {
    list(values.iter().map(|n| int(*n)).collect())
}

fn matrix(rows: &[&[i64]]) -> Val {
    list(rows.iter().map(|r| ints(r)).collect())
}

fn items(v: &Val) -> Vec<Val> {
    match v {
        Val::List(items) => items.clone(),
        other => panic!("not a list: {other:?}"),
    }
}

fn rat(n: i64, d: i64) -> Val {
    Val::from_rational(Rational::from_ratio_i64(n, d).unwrap())
}

fn holds(rel: Rel, a: &Val, b: &Val) -> bool {
    matches!(compare(rel, a.clone(), b.clone()).unwrap(), Val::Bool(true))
}

#[test]
fn a_clock_stepped_by_the_infinitesimal_runs_the_generator_s_count() {
    let h = generator_of(&dt()).unwrap();
    let strict = trip_count(&int(0), &dt(), &int(1), "<").unwrap();
    assert!(strict.values_equal(&h), "{}", show(&strict));
    let inclusive = trip_count(&int(0), &dt(), &int(1), "<=").unwrap();
    assert!(inclusive.values_equal(&crate::tower::tests::calc(crate::tower::Op::Add, h, int(1)).unwrap()));
}

#[test]
fn a_standard_clock_counts_its_steps_and_a_clock_that_is_not_a_number_runs_as_written() {
    assert!(trip_count(&int(0), &rat(1, 10), &int(1), "<").unwrap().values_equal(&int(10)));
    assert!(trip_count(&int(0), &int(1), &int(5), "<=").unwrap().values_equal(&int(6)));
    assert!(trip_count(&int(5), &int(1), &int(1), "<").unwrap().values_equal(&int(0)));
    assert!(trip_count(&Val::Text("t".into()), &int(1), &int(5), "<").unwrap().values_equal(&int(-1)));
    assert!(trip_count(&int(0), &int(-1), &int(5), "<").unwrap().values_equal(&int(-1)));
}

#[test]
fn an_infinitesimal_step_away_from_its_bound_is_refused() {
    let negative = crate::tower::tests::calc(crate::tower::Op::Sub, int(0), dt()).unwrap();
    let err = trip_count(&int(0), &negative, &int(1), "<").unwrap_err();
    assert_eq!(
        err,
        "the clock never reaches its bound: it starts at 0, steps by -dt and runs while it is < 1"
    );
    assert_eq!(
        trip_count(&int(0), &dt(), &int(1), "!=").unwrap_err(),
        "a clock's guard is a relation <, <=, > or >=, got `!=`"
    );
}

#[test]
fn the_lamp_s_affine_update_depends_on_the_generator_s_parity() {
    let lamp = matrix(&[&[-1, 1], &[0, 1]]);
    let end = affine_power(&lamp, &ints(&[0, 1]), &generator_of(&dt()).unwrap()).unwrap();
    let shown: Vec<String> = items(&end).iter().map(show).collect();
    assert_eq!(shown, vec!["0 when ⌈1/dt⌉ is even, 1 when it is odd", "1"]);
    let declared = affine_power(&lamp, &ints(&[0, 1]), &generator_of(&even_dt()).unwrap()).unwrap();
    assert_eq!(items(&declared).iter().map(show).collect::<Vec<_>>(), vec!["0", "1"]);
    let three = affine_power(&lamp, &ints(&[0, 1]), &int(3)).unwrap();
    assert_eq!(items(&three).iter().map(show).collect::<Vec<_>>(), vec!["1", "1"]);
}

/// A pass that adds a value depending on the generator's parity is powered on each class apart: the
/// sum of `⌈1/dt⌉` lamps is `0` on the even classes and `1/dt` on the odd ones, the clock beside it
/// reaches `1` on both, and three passes add the lamp three times.
#[test]
fn an_update_holding_a_residue_dependent_value_is_powered_on_each_class() {
    let h = generator_of(&dt()).unwrap();
    let lamp = items(&affine_power(&matrix(&[&[-1, 1], &[0, 1]]), &ints(&[0, 1]), &h).unwrap())[0].clone();
    assert_eq!(show(&lamp), "0 when ⌈1/dt⌉ is even, 1 when it is odd");
    let sum = list(vec![
        list(vec![int(1), int(0), lamp.clone()]),
        list(vec![int(0), int(1), dt()]),
        ints(&[0, 0, 1]),
    ]);
    let end = affine_power(&sum, &ints(&[0, 0, 1]), &h).unwrap();
    let shown: Vec<String> = items(&end).iter().map(show).collect();
    assert_eq!(shown, vec!["0 when ⌈1/dt⌉ is even, 1/dt when it is odd", "1", "1"]);
    let finite = affine_power(&sum, &ints(&[0, 0, 1]), &int(3)).unwrap();
    assert_eq!(show(&items(&finite)[0]), "0 when ⌈1/dt⌉ is even, 3 when it is odd", "three passes add the lamp three times");
}

/// A class's answer that holds the infinitesimal holds the program's own: `⌈1/dt⌉` lamps worth `dt²`
/// sum to `dt` on the odd classes, and that sum adds to the program's `dt` as one infinitesimal.
#[test]
fn a_class_s_answer_holding_the_infinitesimal_holds_the_program_s_own() {
    use crate::tower::tests::calc;
    use crate::tower::Op;
    let h = generator_of(&dt()).unwrap();
    let lamp = items(&affine_power(&matrix(&[&[-1, 1], &[0, 1]]), &ints(&[0, 1]), &h).unwrap())[0].clone();
    let worth = calc(Op::Mul, lamp, calc(Op::Mul, dt(), dt()).unwrap()).unwrap();
    let sum = list(vec![list(vec![int(1), worth]), ints(&[0, 1])]);
    let x = items(&affine_power(&sum, &ints(&[0, 1]), &h).unwrap())[0].clone();
    assert_eq!(show(&x), "0 when ⌈1/dt⌉ is even, dt when it is odd");
    let more = calc(Op::Add, x, dt()).unwrap();
    assert_eq!(show(&more), "dt when ⌈1/dt⌉ is even, 2·dt when it is odd");
}

#[test]
fn a_counter_s_affine_update_ends_at_the_generator() {
    let h = generator_of(&dt()).unwrap();
    let end = affine_power(&matrix(&[&[1, 1], &[0, 1]]), &ints(&[0, 1]), &h).unwrap();
    assert!(items(&end)[0].values_equal(&h), "{}", show(&end));
}

#[test]
fn the_closed_form_s_certificate_is_checked_by_multiplying_it_out() {
    let lamp = vec![vec![int(-1), int(1)], vec![int(0), int(1)]];
    assert!(check_annihilator(&lamp, 0, 2, 1).is_ok());
    let err = check_annihilator(&lamp, 0, 1, 1).unwrap_err();
    assert!(err.contains("certificate") && err.contains("failed to check"), "{err}");
}

#[test]
fn an_update_whose_eigenvalue_is_not_a_root_of_unity_has_no_closed_form() {
    let err = affine_power(&matrix(&[&[2]]), &ints(&[1]), &generator_of(&dt()).unwrap()).unwrap_err();
    assert!(err.contains("not a root of unity"), "{err}");
}

#[test]
fn a_recorded_cycle_is_read_at_any_step() {
    let states = list(vec![ints(&[0]), ints(&[1])]);
    let at_five = cycle_at(&states, &ints(&[0]), &int(5)).unwrap();
    assert_eq!(show(&at_five), "[1]");
    let at_h = cycle_at(&states, &ints(&[0]), &generator_of(&dt()).unwrap()).unwrap();
    assert_eq!(show(&at_h), "[0 when ⌈1/dt⌉ is even, 1 when it is odd]");
    assert_eq!(
        cycle_at(&states, &ints(&[7]), &int(1)).unwrap_err(),
        "the supertask engine's repeated state is not among the recorded ones"
    );
}

#[test]
fn a_search_examines_its_sign_changes_and_one_period_after_each() {
    let candidates = search_candidates(&list(vec![ints(&[1, -3])]), &list(vec![])).unwrap();
    assert_eq!(show(&candidates), "[0, 1, 3, 4]");
    let periodic = search_candidates(&list(vec![]), &ints(&[3])).unwrap();
    assert_eq!(show(&periodic), "[0, 1, 2, 3]");
    let floats = search_candidates(&list(vec![list(vec![Val::Text("a".into()), int(1)])]), &list(vec![])).unwrap();
    assert_eq!(show(&floats), "[]", "a coefficient that is not a standard exact number is not read");
}

#[test]
fn a_state_s_size_is_its_bits() {
    assert_eq!(state_size(&Val::Bool(true)), 1);
    assert_eq!(state_size(&int(7)), 64);
    assert_eq!(state_size(&Val::Text("abc".into())), 24);
    assert_eq!(state_size(&ints(&[1, 2])), 128);
}

#[test]
fn an_euler_step_of_growth_ends_infinitely_close_to_e() {
    let field = list(vec![list(vec![list(vec![int(1), int(1)]), list(vec![dt(), int(1)])])]);
    let end = polynomial_flow(&field, &ints(&[1]), &generator_of(&dt()).unwrap()).unwrap();
    let x = standard_part(&items(&end)[0]).unwrap();
    assert!(holds(Rel::Gt, &x, &rat(2718, 1000)) && holds(Rel::Lt, &x, &rat(2719, 1000)), "{}", show(&x));
    let three = polynomial_flow(&field, &ints(&[1]), &int(3)).unwrap();
    assert!(standard_part(&items(&three)[0]).unwrap().values_equal(&int(1)));
}

// ── hybrid flows ─────────────────────────────────────────────────────────────────────────────

fn text(t: &str) -> Val {
    Val::Text(t.to_string())
}

fn times(a: Val, b: Val) -> Val {
    crate::tower::tests::calc(crate::tower::Op::Mul, a, b).unwrap()
}

/// `[coefficient, e₁, …, e₄]` over the ball's slots `h, v, t, last`.
fn term(coefficient: Val, exponents: [i64; 4]) -> Val {
    let mut entry = vec![coefficient];
    entry.extend(exponents.iter().map(|e| int(*e)));
    list(entry)
}

fn poly(terms: Vec<Val>) -> Val {
    list(terms)
}

/// The paper's ball stepped by `step`: `h ← h + v·step`, `v ← v − 10·step`, `t ← t + step`; at
/// `h ≤ 0` falling, `h ← 0`, `v ← −v/2`, and `last ← t` while the rebound is not infinitesimal.
fn ball_model(step: Val) -> Val {
    let field = list(vec![
        poly(vec![term(int(1), [1, 0, 0, 0]), term(step.clone(), [0, 1, 0, 0])]),
        poly(vec![term(int(1), [0, 1, 0, 0]), term(times(int(-10), step.clone()), [0, 0, 0, 0])]),
        poly(vec![term(int(1), [0, 0, 1, 0]), term(step, [0, 0, 0, 0])]),
        poly(vec![term(int(1), [0, 0, 0, 1])]),
    ]);
    let schedule = list(vec![list(vec![text("flow"), field]), list(vec![text("check"), int(0)])]);
    let guard = list(vec![
        text("all"),
        list(vec![text("compare"), text("<="), poly(vec![term(int(1), [1, 0, 0, 0])])]),
        list(vec![text("compare"), text("<"), poly(vec![term(int(1), [0, 1, 0, 0])])]),
    ]);
    let rebound = list(vec![
        text("not"),
        list(vec![text("infinitesimal"), poly(vec![term(int(1), [0, 1, 0, 0])])]),
    ]);
    let block = list(vec![
        list(vec![text("set"), int(0), poly(vec![])]),
        list(vec![text("set"), int(1), poly(vec![term(rat(-1, 2), [0, 1, 0, 0])])]),
        list(vec![
            text("if"),
            rebound,
            list(vec![list(vec![text("set"), int(3), poly(vec![term(int(1), [0, 0, 1, 0])])])]),
            list(vec![]),
        ]),
    ]);
    list(vec![schedule, list(vec![list(vec![guard, block])])])
}

#[test]
fn a_hybrid_loop_of_infinitely_many_steps_ends_on_its_ideal_run() {
    let n = times(int(5), generator_of(&dt()).unwrap());
    let end = items(&hybrid_flow(&ball_model(dt()), &ints(&[0, 10, 0, 0]), &n).unwrap());
    let standard = |v: &Val| standard_part(v).unwrap();
    assert!(standard(&end[0]).values_equal(&int(0)));
    assert!(standard(&end[1]).values_equal(&int(0)));
    assert!(standard(&end[2]).values_equal(&int(5)));
    assert!(standard(&end[3]).values_equal(&int(4)), "{}", show(&end[3]));
    assert!(matches!(end[3], Val::Hyper(_)), "a time read at a bounce is only infinitely close to 4");
}

#[test]
fn a_hybrid_loop_of_finitely_many_steps_runs_them_as_written() {
    let end = hybrid_flow(&ball_model(rat(1, 2)), &ints(&[0, 10, 0, 0]), &int(5)).unwrap();
    let end = items(&end);
    let expected = [int(0), rat(15, 2), rat(5, 2), rat(5, 2)];
    for (got, want) in end.iter().zip(&expected) {
        assert!(got.values_equal(want), "{} is not {}", show(got), show(want));
    }
}

#[test]
fn a_malformed_hybrid_model_is_refused_by_what_is_wrong() {
    let n = generator_of(&dt()).unwrap();
    let state = ints(&[0, 10, 0, 0]);
    let err = hybrid_flow(&list(vec![int(1)]), &state, &n).unwrap_err();
    assert!(err.contains("body and its jumps"), "{err}");
    let out_of_range = list(vec![list(vec![list(vec![text("check"), int(1)])]), list(vec![])]);
    let err = hybrid_flow(&out_of_range, &state, &n).unwrap_err();
    assert!(err.contains("checks one of its 0 jumps"), "{err}");
    let unchecked = list(vec![
        list(vec![]),
        list(vec![list(vec![list(vec![text("constant"), Val::Bool(true)]), list(vec![])])]),
    ]);
    let err = hybrid_flow(&unchecked, &state, &n).unwrap_err();
    assert!(err.contains("checks each of its jumps once"), "{err}");
    let unknown = list(vec![
        list(vec![list(vec![text("check"), int(0)])]),
        list(vec![list(vec![list(vec![text("sometimes")]), list(vec![])])]),
    ]);
    let err = hybrid_flow(&unknown, &state, &n).unwrap_err();
    assert!(err.contains("no condition of kind `sometimes`"), "{err}");
}

#[test]
fn a_flow_that_moves_a_value_by_a_standard_amount_each_step_is_not_an_euler_step() {
    let field = list(vec![
        poly(vec![term(int(1), [1, 0, 0, 0]), term(int(1), [0, 1, 0, 0])]),
        poly(vec![term(int(1), [0, 1, 0, 0])]),
        poly(vec![term(int(1), [0, 0, 1, 0]), term(dt(), [0, 0, 0, 0])]),
        poly(vec![term(int(1), [0, 0, 0, 1])]),
    ]);
    let model = list(vec![
        list(vec![list(vec![text("flow"), field]), list(vec![text("check"), int(0)])]),
        list(vec![list(vec![list(vec![text("constant"), Val::Bool(false)]), list(vec![])])]),
    ]);
    let err = hybrid_flow(&model, &ints(&[0, 1, 0, 0]), &generator_of(&dt()).unwrap()).unwrap_err();
    assert!(err.contains("standard amount"), "{err}");
}

#[test]
fn values_no_polynomial_reads_pass_through_a_hybrid_loop_as_they_are() {
    let mut model = items(&ball_model(dt()));
    let mut jumps = items(&model[1]);
    let mut jump = items(&jumps[0]);
    let mut block = items(&jump[1]);
    block.push(list(vec![text("put"), int(3), text("rested")]));
    jump[1] = list(block);
    jumps[0] = list(jump);
    model[1] = list(jumps);
    let state = list(vec![int(0), int(10), int(0), text("flying")]);
    let n = times(int(5), generator_of(&dt()).unwrap());
    let end = items(&hybrid_flow(&list(model), &state, &n).unwrap());
    assert!(end[3].values_equal(&text("rested")), "the last firing put it there: {}", show(&end[3]));
}

#[test]
fn a_value_a_polynomial_reads_that_is_not_a_number_is_refused() {
    let mut model = items(&ball_model(dt()));
    let mut jumps = items(&model[1]);
    let mut jump = items(&jumps[0]);
    jump[0] = list(vec![
        text("all"),
        jump[0].clone(),
        list(vec![text("compare"), text(">"), poly(vec![term(int(1), [0, 0, 0, 1])])]),
    ]);
    jumps[0] = list(jump);
    model[1] = list(jumps);
    let n = times(int(5), generator_of(&dt()).unwrap());
    let state = list(vec![int(0), int(10), int(0), text("flying")]);
    let err = hybrid_flow(&list(model.clone()), &state, &n).unwrap_err();
    assert!(err.contains("not a number"), "{err}");
    let mut jumps = items(&model[1]);
    let mut jump = items(&jumps[0]);
    let mut block = items(&jump[1]);
    block.push(list(vec![text("put"), int(3), text("rested")]));
    jump[1] = list(block);
    jumps[0] = list(jump);
    model[1] = list(jumps);
    let err = hybrid_flow(&list(model), &ints(&[0, 10, 0, 1]), &n).unwrap_err();
    assert!(err.contains("not a number"), "{err}");
}

// ── first exits ───────────────────────────────────────────────────────────────────────────────

fn atom(relation: &str, a: Val, b: Val) -> Val {
    list(vec![text(relation), a, b])
}

fn both(f: Val, g: Val) -> Val {
    list(vec![text("and"), f, g])
}

fn either(f: Val, g: Val) -> Val {
    list(vec![text("or"), f, g])
}

fn negated(f: Val) -> Val {
    list(vec![text("not"), f])
}

fn calc(op: crate::tower::Op, a: Val, b: Val) -> Val {
    crate::tower::tests::calc(op, a, b).unwrap()
}

fn neg(v: Val) -> Val {
    calc(crate::tower::Op::Sub, int(0), v)
}

/// Imamura's Lemma 1 as the engine writes its guard over the counter `n`:
/// `not ((n <= x and x < n + 1) or (0 - n <= x and x < 0 - n + 1))`.
fn floor_guard(x: &Val) -> Val {
    let below = calc(crate::tower::Op::Sub, x.clone(), int(1));
    negated(either(
        both(atom("<=", int(1), neg(x.clone())), atom("<", int(-1), below.clone())),
        both(atom("<=", int(-1), neg(x.clone())), atom("<", int(1), below)),
    ))
}

fn reciprocal(v: Val) -> Val {
    calc(crate::tower::Op::ExactDiv, int(1), v)
}

#[test]
fn the_floor_loop_of_lemma_1_exits_at_the_floor_of_an_infinite_value() {
    let h = generator_of(&dt()).unwrap();
    let exit = first_exit(&int(0), &int(1), &floor_guard(&reciprocal(dt()))).unwrap();
    assert!(exit.values_equal(&h), "{}", show(&exit));
    let half_more = calc(crate::tower::Op::Add, reciprocal(dt()), rat(1, 2));
    let exit = first_exit(&int(0), &int(1), &floor_guard(&half_more)).unwrap();
    assert!(exit.values_equal(&h), "{}", show(&exit));
    let less = calc(crate::tower::Op::Sub, reciprocal(dt()), dt());
    let exit = first_exit(&int(0), &int(1), &floor_guard(&less)).unwrap();
    assert_eq!(show(&exit), "1/dt - 1");
    let exit = first_exit(&int(0), &int(1), &floor_guard(&neg(reciprocal(dt())))).unwrap();
    assert!(exit.values_equal(&h), "the count reaches -x for a negative x: {}", show(&exit));
}

#[test]
fn a_floor_that_depends_on_the_residue_is_read_on_each_class() {
    let half = reciprocal(calc(crate::tower::Op::Mul, int(2), dt()));
    let exit = first_exit(&int(0), &int(1), &floor_guard(&half)).unwrap();
    assert_eq!(show(&exit), "1/(2·dt) when ⌈1/dt⌉ is even, 1/(2·dt) - 1/2 when it is odd");
    let even_half = reciprocal(calc(crate::tower::Op::Mul, int(2), even_dt()));
    let exit = first_exit(&int(0), &int(1), &floor_guard(&even_half)).unwrap();
    assert_eq!(show(&exit), "1/(2·dt)");
    let third = reciprocal(calc(crate::tower::Op::Mul, int(3), dt()));
    let exit = first_exit(&int(0), &int(1), &floor_guard(&third)).unwrap();
    assert_eq!(
        show(&exit),
        "1/(3·dt) when ⌈1/dt⌉ ≡ 0 (mod 3), 1/(3·dt) - 1/3 when ⌈1/dt⌉ ≡ 1 (mod 3), 1/(3·dt) - 2/3 when ⌈1/dt⌉ ≡ 2 (mod 3)"
    );
}

#[test]
fn a_count_past_a_fraction_of_the_infinite_is_read_on_each_class() {
    // While n <= 1/(2·dt): the first natural past it is ⌊1/(2·dt)⌋ + 1, whose branches agree on
    // every comparison before it, so only the floor's own dependence tells the classes apart.
    let half = reciprocal(calc(crate::tower::Op::Mul, int(2), dt()));
    let exit = first_exit(&int(0), &int(1), &atom("<=", int(1), neg(half))).unwrap();
    assert_eq!(show(&exit), "1/(2·dt) + 1 when ⌈1/dt⌉ is even, 1/(2·dt) + 1/2 when it is odd");
}

#[test]
fn two_crossings_that_meet_on_one_class_only_are_told_apart_class_by_class() {
    // While not (n <= 1/(2·dt) and n >= (1/dt - 1)/2): the first natural in that window. The two
    // crossings' floors coincide when ⌈1/dt⌉ is odd and differ by one when it is even, so no one
    // reading of the generator orders them.
    let half = reciprocal(calc(crate::tower::Op::Mul, int(2), dt()));
    let lower = calc(crate::tower::Op::Sub, half.clone(), rat(1, 2));
    let guard = negated(both(atom("<=", int(1), neg(half)), atom(">=", int(1), neg(lower))));
    let exit = first_exit(&int(0), &int(1), &guard).unwrap();
    assert_eq!(show(&exit), "1/(2·dt) when ⌈1/dt⌉ is even, 1/(2·dt) - 1/2 when it is odd");
}

#[test]
fn a_standard_or_limited_floor_is_counted_or_left_to_the_loop() {
    for x in [rat(7, 2), rat(-7, 2), int(3), int(0)] {
        let exit = first_exit(&int(0), &int(1), &floor_guard(&x)).unwrap();
        assert!(exit.values_equal(&int(-1)), "every number standard: {}", show(&exit));
    }
    for (x, floor) in [
        (calc(crate::tower::Op::Add, int(3), dt()), 3),
        (calc(crate::tower::Op::Sub, int(3), dt()), 2),
        (neg(dt()), 1),
    ] {
        let exit = first_exit(&int(0), &int(1), &floor_guard(&x)).unwrap();
        assert!(exit.values_equal(&int(floor)), "{} for {}", show(&exit), show(&x));
    }
}

#[test]
fn a_remainder_in_the_guard_is_read_on_the_classes_it_needs() {
    // While n < 1/dt or n % 3 != 0: the first multiple of 3 at or past 1/dt.
    let guard = either(
        atom("<", int(1), neg(reciprocal(dt()))),
        list(vec![text("%"), text("!="), int(1), int(0), int(3), int(0)]),
    );
    let exit = first_exit(&int(0), &int(1), &guard).unwrap();
    assert_eq!(
        show(&exit),
        "1/dt when ⌈1/dt⌉ ≡ 0 (mod 3), 1/dt + 2 when ⌈1/dt⌉ ≡ 1 (mod 3), 1/dt + 1 when ⌈1/dt⌉ ≡ 2 (mod 3)"
    );
    let residue = list(vec![text("%"), text("!="), int(1), int(0), int(3), int(0)]);
    let exit = first_exit(&int(1), &int(1), &residue).unwrap();
    assert!(exit.values_equal(&int(-1)), "every number standard");
}

#[test]
fn a_counter_stepped_by_the_infinitesimal_exits_where_its_guard_first_fails() {
    // While t < 1 and not (t >= 1/2 and t < 1/2 + dt): the clock stops at 1/2.
    let guard = both(
        atom("<", int(1), int(-1)),
        negated(both(
            atom(">=", int(1), rat(-1, 2)),
            atom("<", int(1), calc(crate::tower::Op::Sub, rat(-1, 2), dt())),
        )),
    );
    let exit = first_exit(&int(0), &dt(), &guard).unwrap();
    let half_h = calc(crate::tower::Op::Mul, rat(1, 2), generator_of(&dt()).unwrap());
    assert_eq!(show(&exit), show(&Val::ceil(half_h).unwrap()));
    assert_eq!(show(&exit), "1/(2·dt) when ⌈1/dt⌉ is even, 1/(2·dt) + 1/2 when it is odd");
}

#[test]
fn a_guard_that_never_fails_runs_as_written_on_a_standard_step_and_is_refused_on_an_infinitesimal_one() {
    let always = atom(">=", int(0), int(0));
    let exit = first_exit(&reciprocal(dt()), &int(1), &always).unwrap();
    assert!(exit.values_equal(&int(-1)), "{}", show(&exit));
    let err = first_exit(&int(0), &dt(), &always).unwrap_err();
    assert_eq!(err, "this loop never ends: its counter starts at 0, steps by dt, and its guard holds at every step");
}

#[test]
fn a_guard_that_is_no_formula_is_refused_and_a_value_that_is_no_number_runs_as_written() {
    let err = first_exit(&int(0), &int(1), &list(vec![text("xor"), int(1), int(2), int(3)])).unwrap_err();
    assert!(err.starts_with("a loop's guard is a formula the supertask engine writes"), "{err}");
    let err = first_exit(&int(0), &int(1), &int(3)).unwrap_err();
    assert!(err.contains("Seq"), "{err}");
    let words = atom("<", text("a"), int(1));
    assert!(first_exit(&int(0), &dt(), &words).unwrap().values_equal(&int(-1)));
}
