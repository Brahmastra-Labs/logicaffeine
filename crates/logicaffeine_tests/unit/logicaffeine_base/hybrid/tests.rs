//! The ideal run of a hybrid loop: exact roots, flights, events, Zeno tails and continuous firing,
//! on models built by hand, against values worked out on paper.

use super::*;

fn a(n: i64) -> Algebraic {
    Algebraic::from_i64(n)
}

fn q(n: i64, d: i64) -> Algebraic {
    Algebraic::from_rational(Rational::from_ratio_i64(n, d).expect("nonzero denominator"))
}

fn sqrt(n: i64) -> Algebraic {
    a(n).sqrt().expect("a nonnegative number has a square root")
}

fn up(coeffs: &[Algebraic]) -> UPoly {
    UPoly::from_coeffs(coeffs.to_vec())
}

fn exact(p: MPoly) -> Term {
    Term { poly: p, exact: true }
}

fn slot(width: usize, i: usize) -> MPoly {
    MPoly::slot(width, i)
}

fn constant(width: usize, c: Algebraic) -> MPoly {
    MPoly::constant(width, c)
}

fn num(x: Algebraic) -> Cell {
    Cell::Num { value: x, exact: true }
}

fn set(slot: usize, p: MPoly) -> Stmt {
    Stmt::Set {
        slot,
        value: Value::Poly(exact(p)),
    }
}

fn compare(rel: Rel, p: MPoly) -> Cond {
    Cond::Compare(rel, exact(p))
}

fn value(cell: &Cell) -> Algebraic {
    cell.number().cloned().expect("a numeric cell")
}

fn is_exact(cell: &Cell) -> bool {
    matches!(cell, Cell::Num { exact: true, .. })
}

/// A loop whose body is its flow, then its jumps' checks in order.
fn flow_then_jumps(rates: Vec<MPoly>, preserved: Vec<bool>, jumps: Vec<Jump>) -> Model {
    let width = rates.len();
    let mut schedule = vec![Item::Flow { rates, preserved }];
    schedule.extend((0..jumps.len()).map(Item::Check));
    Model::new(width, schedule, jumps)
}

/// A loop whose body checks its jumps, then flows.
fn jumps_then_flow(rates: Vec<MPoly>, preserved: Vec<bool>, jumps: Vec<Jump>) -> Model {
    let width = rates.len();
    let mut schedule: Vec<Item> = (0..jumps.len()).map(Item::Check).collect();
    schedule.push(Item::Flow { rates, preserved });
    Model::new(width, schedule, jumps)
}

// ── the bouncing ball: slots h, v, t, last ───────────────────────────────────────────────────

const H: usize = 0;
const V: usize = 1;
const T: usize = 2;
const LAST: usize = 3;
const W: usize = 4;

/// `h' = v`, `v' = −g`, `t' = 1`; at `h ≤ 0` falling, `h ← 0`, `v ← −c·v`, and `last` records the
/// time of every bounce whose rebound `record` lets through.
fn ball(g: Algebraic, c: Algebraic, record: Option<Cond>) -> Model {
    let mut block = vec![set(H, MPoly::zero(W)), set(V, slot(W, V).scale(&c.neg()))];
    match record {
        Some(cond) => block.push(Stmt::If {
            cond,
            then: vec![set(LAST, slot(W, T))],
            otherwise: vec![],
        }),
        None => block.push(set(LAST, slot(W, T))),
    }
    flow_then_jumps(
        vec![slot(W, V), constant(W, g.neg()), constant(W, a(1)), MPoly::zero(W)],
        vec![false, false, false, true],
        vec![Jump {
            guard: Cond::All(vec![compare(Rel::Le, slot(W, H)), compare(Rel::Lt, slot(W, V))]),
            block,
        }],
    )
}

fn not_infinitesimal(p: MPoly) -> Cond {
    Cond::Not(Box::new(Cond::Infinitesimal(exact(p))))
}

fn start(h: Algebraic, v: Algebraic) -> Vec<Cell> {
    vec![num(h), num(v), num(a(0)), num(a(0))]
}

// ── polynomials and roots ────────────────────────────────────────────────────────────────────

#[test]
fn a_polynomial_in_one_variable_evaluates_dilates_and_reports_its_lowest_term() {
    let p = up(&[a(0), a(0), a(3), a(-1)]);
    assert_eq!(p.degree(), Some(3));
    assert_eq!(p.eval(&a(2)), a(4));
    assert_eq!(p.lowest().map(|(k, c)| (k, c.clone())), Some((2, a(3))));
    assert_eq!(p.dilate(&a(2)), up(&[a(0), a(0), a(12), a(-8)]));
    assert_eq!(up(&[a(1), a(1)]).mul(&up(&[a(-1), a(1)])), up(&[a(-1), a(0), a(1)]));
    assert_eq!(up(&[a(0), a(0)]), UPoly::zero(), "trailing zeros are dropped");
    assert_eq!(UPoly::zero().as_constant(), Some(a(0)));
}

#[test]
fn the_real_roots_of_a_rational_polynomial_are_distinct_and_ascending() {
    let squared_and_simple = up(&[a(2), a(-3), a(0), a(1)]);
    assert_eq!(real_roots(&squared_and_simple), vec![a(-2), a(1)], "(x − 1)²·(x + 2)");
    assert_eq!(real_roots(&up(&[a(1), a(0), a(1)])), Vec::<Algebraic>::new());
    assert_eq!(real_roots(&up(&[a(-2), a(0), a(1)])), vec![sqrt(2).neg(), sqrt(2)]);
    assert_eq!(real_roots(&up(&[a(5)])), Vec::<Algebraic>::new());
}

#[test]
fn a_quadratic_with_irrational_coefficients_has_its_roots_by_the_formula() {
    let p = up(&[a(-2), a(0), sqrt(2)]);
    let roots = real_roots(&p);
    assert_eq!(roots.len(), 2, "√2·x² − 2 has roots ±2^(1/4)");
    for r in &roots {
        assert_eq!(r.mul(r), sqrt(2));
        assert!(p.eval(r).is_zero());
    }
    assert!(roots[0] < roots[1]);
    assert_eq!(real_roots(&up(&[sqrt(2), a(0), a(1)])), Vec::<Algebraic>::new());
    let square = up(&[a(2), sqrt(2).mul(&a(-2)), a(1)]);
    assert_eq!(real_roots(&square), vec![sqrt(2)], "(x − √2)²");
}

#[test]
fn a_cubic_with_irrational_coefficients_has_its_roots_through_the_free_norm() {
    let p = up(&[sqrt(2).neg(), a(0), a(0), a(1)]);
    let roots = real_roots(&p);
    assert_eq!(roots.len(), 1, "x³ − √2");
    assert_eq!(roots[0].pow(6), Some(a(2)));
    let r3 = sqrt(3);
    let p = up(&[a(2).mul(&r3), a(-2), r3.neg(), a(1)]);
    assert_eq!(real_roots(&p), vec![sqrt(2).neg(), sqrt(2), sqrt(3)], "(x − √3)(x² − 2)");
}

#[test]
fn the_lie_derivative_of_a_ballistic_height_is_the_velocity_then_gravity() {
    let m = ball(a(10), q(1, 2), None);
    let dh = slot(W, H).lie(&m.rates);
    assert_eq!(dh, slot(W, V));
    assert_eq!(dh.lie(&m.rates), constant(W, a(-10)));
    assert!(dh.lie(&m.rates).lie(&m.rates).is_zero());
    let series = lie_series(&m.rates).expect("a ballistic field is nilpotent");
    assert_eq!(series[H].len(), 3);
    assert_eq!(series[T].len(), 2);
    assert_eq!(series[LAST].len(), 1);
}

#[test]
fn a_field_whose_derivatives_never_vanish_is_refused_by_name() {
    let m = flow_then_jumps(vec![slot(1, 0)], vec![false], vec![]);
    let err = run(&m, vec![num(a(1))], &a(1)).unwrap_err();
    assert!(err.contains("not a polynomial in time"), "{err}");
}

// ── the bouncing ball ────────────────────────────────────────────────────────────────────────

#[test]
fn the_ball_comes_to_rest_at_its_zeno_time_and_records_only_standard_bounces() {
    let m = ball(a(10), q(1, 2), Some(not_infinitesimal(slot(W, V))));
    let end = run(&m, start(a(0), a(10)), &a(5)).expect("the ball resolves");
    assert_eq!(value(&end[H]), a(0));
    assert_eq!(value(&end[V]), a(0));
    assert_eq!(value(&end[T]), a(5));
    assert_eq!(value(&end[LAST]), a(4), "the bounces accumulate at 2·10/(10·(1 − 1/2)) = 4");
    for i in [H, V, T, LAST] {
        assert!(!is_exact(&end[i]), "slot {i} is only infinitely close to its value");
    }
}

#[test]
fn recording_every_bounce_while_the_ball_rests_tracks_the_loops_end() {
    let m = ball(a(10), q(1, 2), None);
    let end = run(&m, start(a(0), a(10)), &a(5)).expect("the ball resolves");
    assert_eq!(value(&end[LAST]), a(5), "the ball at rest bounces infinitely often until the end");
}

#[test]
fn a_rebound_known_positive_records_the_resting_bounces_too() {
    let m = ball(a(10), q(1, 2), Some(compare(Rel::Gt, slot(W, V))));
    let end = run(&m, start(a(0), a(10)), &a(5)).expect("the ball resolves");
    assert_eq!(
        value(&end[LAST]),
        a(5),
        "after v ← −c·v with v < 0 the rebound is positive even when infinitesimal"
    );
}

#[test]
fn a_loop_that_ends_mid_flight_leaves_the_flight_there() {
    let m = ball(a(10), q(1, 2), Some(not_infinitesimal(slot(W, V))));
    let end = run(&m, start(a(0), a(10)), &q(13, 4)).expect("two bounces, then a flight");
    assert_eq!(value(&end[H]), q(5, 16));
    assert_eq!(value(&end[V]), a(0));
    assert_eq!(value(&end[LAST]), a(3));
}

#[test]
fn a_bounce_at_the_loops_last_step_is_refused() {
    let m = ball(a(10), q(1, 2), None);
    let err = run(&m, start(a(0), a(10)), &a(3)).unwrap_err();
    assert!(err.contains("last step"), "{err}");
}

#[test]
fn a_ball_dropped_from_a_height_accumulates_at_its_own_zeno_time() {
    let m = ball(a(10), q(1, 2), Some(not_infinitesimal(slot(W, V))));
    let end = run(&m, start(a(5), a(0)), &a(10)).expect("the ball resolves");
    assert_eq!(value(&end[LAST]), a(3), "1 s to fall, then 2 s of bounces");
    assert_eq!(value(&end[H]), a(0));
}

#[test]
fn an_irrational_first_fall_gives_an_algebraic_zeno_time() {
    let m = ball(a(10), q(1, 2), Some(not_infinitesimal(slot(W, V))));
    let end = run(&m, start(a(1), a(0)), &a(5)).expect("the ball resolves");
    assert_eq!(value(&end[LAST]), sqrt(5).mul(&q(3, 5)), "√5/5 to fall, then 2·√5/5 of bounces");
}

#[test]
fn an_elastic_ball_bounces_periodically_and_counts_exactly() {
    let n = 3;
    let m = flow_then_jumps(
        vec![slot(W, V), constant(W, a(-10)), constant(W, a(1)), MPoly::zero(W)],
        vec![false, false, false, true],
        vec![Jump {
            guard: Cond::All(vec![compare(Rel::Le, slot(W, H)), compare(Rel::Lt, slot(W, V))]),
            block: vec![
                set(H, MPoly::zero(W)),
                set(V, slot(W, V).scale(&a(-1))),
                set(n, slot(W, n).add(&constant(W, a(1)))),
            ],
        }],
    );
    let end = run(&m, start(a(0), a(10)), &a(999_999)).expect("periodic bounces resolve");
    assert_eq!(value(&end[n]), a(499_999), "a bounce every 2 s, the last at 999,998 s");
    assert!(is_exact(&end[n]), "a count of bounces is exact");
    assert_eq!(value(&end[H]), a(5));
    assert_eq!(value(&end[V]), a(0));
    let err = run(&m, start(a(0), a(10)), &a(1_000_000)).unwrap_err();
    assert!(err.contains("last step"), "{err}");
}

#[test]
fn counting_the_bounces_of_a_zeno_ball_is_refused() {
    let mut m = ball(a(10), q(1, 2), None);
    m.jumps[0].block[2] = set(LAST, slot(W, LAST).add(&constant(W, a(1))));
    let err = run(&m, start(a(0), a(10)), &a(5)).unwrap_err();
    assert!(err.contains("grows without bound"), "{err}");
}

#[test]
fn a_thermostat_switching_between_two_jumps_runs_by_its_cycle() {
    let w = 4;
    let x = slot(w, 0);
    let r = slot(w, 1);
    let count = set(3, slot(w, 3).add(&constant(w, a(1))));
    let m = flow_then_jumps(
        vec![r.clone(), MPoly::zero(w), constant(w, a(1)), MPoly::zero(w)],
        vec![false, true, false, true],
        vec![
            Jump {
                guard: Cond::All(vec![
                    compare(Rel::Ge, x.sub(&constant(w, a(22)))),
                    compare(Rel::Gt, r.clone()),
                ]),
                block: vec![set(1, constant(w, a(-1))), count.clone()],
            },
            Jump {
                guard: Cond::All(vec![
                    compare(Rel::Le, x.sub(&constant(w, a(18)))),
                    compare(Rel::Lt, r.clone()),
                ]),
                block: vec![set(1, constant(w, a(1))), count],
            },
        ],
    );
    let cells = vec![num(a(20)), num(a(1)), num(a(0)), num(a(0))];
    let end = run(&m, cells, &a(1_000_001)).expect("the thermostat cycles");
    assert_eq!(value(&end[3]), a(250_000), "a switch every 4 s from 2 s on, the last at 999,998 s");
    assert_eq!(value(&end[1]), a(1), "the last switch, at 999,998 s, turned the heating on");
    assert_eq!(value(&end[0]), a(21), "heating for 3 s from 18");
    assert!(is_exact(&end[1]) && is_exact(&end[3]));
}

#[test]
fn a_guard_that_holds_on_after_its_jump_records_until_the_loop_ends() {
    let w = 5;
    let m = flow_then_jumps(
        vec![slot(w, 1), constant(w, a(-10)), constant(w, a(1)), MPoly::zero(w), MPoly::zero(w)],
        vec![false, false, false, true, true],
        vec![Jump {
            guard: compare(Rel::Le, slot(w, 0)),
            block: vec![
                Stmt::Set {
                    slot: 3,
                    value: Value::Fixed(Cell::Bool(true)),
                },
                set(4, slot(w, 2)),
            ],
        }],
    );
    let cells = vec![num(a(5)), num(a(0)), num(a(0)), Cell::Bool(false), num(a(0))];
    let end = run(&m, cells, &a(4)).expect("the recording jump resolves");
    assert_eq!(end[3], Cell::Bool(true));
    assert_eq!(value(&end[4]), a(4), "it fires at every step from the crossing to the end");
    assert_eq!(value(&end[0]), a(-75));
}

#[test]
fn a_guard_the_flow_only_touches_is_refused() {
    let w = 2;
    let m = flow_then_jumps(
        vec![slot(w, 1), constant(w, a(2))],
        vec![false, false],
        vec![Jump {
            guard: compare(Rel::Le, slot(w, 0)),
            block: vec![set(1, constant(w, a(5)))],
        }],
    );
    let err = run(&m, vec![num(a(1)), num(a(-2))], &a(3)).unwrap_err();
    assert!(err.contains("touches"), "h = (t − 1)² touches zero at 1: {err}");
}

#[test]
fn two_jumps_that_fire_at_once_are_refused() {
    let w = 2;
    let reach_one = || compare(Rel::Ge, slot(w, 0).sub(&constant(w, a(1))));
    let m = flow_then_jumps(
        vec![constant(w, a(1)), MPoly::zero(w)],
        vec![false, true],
        vec![
            Jump {
                guard: reach_one(),
                block: vec![set(1, constant(w, a(1)))],
            },
            Jump {
                guard: reach_one(),
                block: vec![set(1, constant(w, a(2)))],
            },
        ],
    );
    let err = run(&m, vec![num(a(0)), num(a(0))], &a(3)).unwrap_err();
    assert!(err.contains("at once"), "{err}");
}

#[test]
fn a_jump_that_leaves_the_state_in_its_guard_but_moves_it_is_refused() {
    let w = 1;
    let m = flow_then_jumps(
        vec![constant(w, a(1))],
        vec![false],
        vec![Jump {
            guard: compare(Rel::Ge, slot(w, 0).sub(&constant(w, a(1)))),
            block: vec![set(0, slot(w, 0).add(&constant(w, a(1))))],
        }],
    );
    let err = run(&m, vec![num(a(0))], &a(3)).unwrap_err();
    assert!(err.contains("moves the state"), "{err}");
}

#[test]
fn a_condition_on_values_infinitely_close_to_each_other_is_refused() {
    let mut m = ball(a(10), q(1, 2), Some(compare(Rel::Lt, slot(W, H))));
    let record = m.jumps[0].block.remove(2);
    m.jumps[0].block.insert(0, record);
    let err = run(&m, start(a(0), a(10)), &a(5)).unwrap_err();
    assert!(
        err.contains("infinitely close"),
        "the guard knows only h ≤ 0 at the bounce, so h < 0 is not decided: {err}"
    );
}

#[test]
fn a_jump_checked_before_the_flow_fires_on_the_starting_state_exactly() {
    let w = 2;
    let m = jumps_then_flow(
        vec![constant(w, a(1)), MPoly::zero(w)],
        vec![false, true],
        vec![Jump {
            guard: compare(Rel::Le, slot(w, 0)),
            block: vec![set(0, constant(w, a(5))), set(1, slot(w, 1).add(&constant(w, a(1))))],
        }],
    );
    let end = run(&m, vec![num(a(0)), num(a(0))], &a(2)).expect("one firing, then a flight");
    assert_eq!(value(&end[0]), a(7));
    assert_eq!(value(&end[1]), a(1));
    assert!(is_exact(&end[1]));
}

#[test]
fn a_jump_checked_after_the_flow_does_not_fire_on_a_state_the_flow_leaves() {
    let w = 2;
    let m = flow_then_jumps(
        vec![constant(w, a(1)), MPoly::zero(w)],
        vec![false, true],
        vec![Jump {
            guard: compare(Rel::Le, slot(w, 0)),
            block: vec![set(1, slot(w, 1).add(&constant(w, a(1))))],
        }],
    );
    let end = run(&m, vec![num(a(0)), num(a(0))], &a(2)).expect("x leaves zero before the check");
    assert_eq!(value(&end[0]), a(2));
    assert_eq!(value(&end[1]), a(0));
}

// ── the scaling certificate ──────────────────────────────────────────────────────────────────

fn ball_scaling() -> (Model, Scaling, Vec<Cell>, Vec<Cell>) {
    let m = ball(a(10), q(1, 2), Some(not_infinitesimal(slot(W, V))));
    let cycle = vec![(0usize, vec![true])];
    let slots = scaled_slots(&m, &cycle);
    assert_eq!(slots, vec![H, V], "the guard reads h and v, and h's rate reads v");
    let scaling = Scaling {
        cycle,
        slots,
        weights: vec![2, 1],
        q: 1,
        s: q(1, 2),
    };
    let rebound = |v: Algebraic, t: i64| vec![num(a(0)), Cell::Num { value: v, exact: false }, num(a(t)), num(a(t))];
    (m, scaling, rebound(a(5), 2), rebound(q(5, 2), 3))
}

#[test]
fn the_balls_scaling_is_certified() {
    let (m, scaling, before, after) = ball_scaling();
    check_scaling(&m, &scaling, &before, &after, &[a(1)], &[q(1, 2)]).expect("the ball scales by 1/2");
}

#[test]
fn a_tampered_scaling_is_rejected() {
    let (m, scaling, before, after) = ball_scaling();
    let check = |s: &Scaling, af: &[Cell], fb: &[Algebraic], fa: &[Algebraic]| check_scaling(&m, s, &before, af, fb, fa);
    let flights = (vec![a(1)], vec![q(1, 2)]);
    let mut wrong = scaling.clone();
    wrong.weights = vec![1, 1];
    assert!(check(&wrong, &after, &flights.0, &flights.1).is_err(), "h' = v needs w_h = w_v + q");
    let mut wrong = scaling.clone();
    wrong.s = q(1, 3);
    assert!(check(&wrong, &after, &flights.0, &flights.1).is_err(), "5 does not scale to 5/2 by 1/3");
    let mut wrong = scaling.clone();
    wrong.q = 2;
    assert!(check(&wrong, &after, &flights.0, &flights.1).is_err());
    let mut wrong = scaling.clone();
    wrong.slots = vec![V];
    wrong.weights = vec![1];
    assert!(check(&wrong, &after, &flights.0, &flights.1).is_err(), "the scaling must cover h");
    let mut wrong = scaling.clone();
    wrong.s = a(-1);
    assert!(check(&wrong, &after, &flights.0, &flights.1).is_err());
    let mut wrong = scaling.clone();
    wrong.cycle = vec![];
    assert!(check(&wrong, &after, &flights.0, &flights.1).is_err());
    let mut moved = after.clone();
    moved[V] = Cell::Num { value: a(3), exact: false };
    assert!(check(&scaling, &moved, &flights.0, &flights.1).is_err());
    assert!(check(&scaling, &after, &flights.0, &[a(1)]).is_err(), "the flight must scale too");
    assert!(check(&scaling, &after, &[], &[]).is_err(), "one flight per firing of the round");
}

#[test]
fn a_reset_that_is_not_homogeneous_breaks_the_certificate() {
    let (mut m, scaling, before, after) = ball_scaling();
    m.jumps[0].block[1] = set(V, slot(W, V).scale(&q(-1, 2)).add(&constant(W, a(1))));
    let err = check_scaling(&m, &scaling, &before, &after, &[a(1)], &[q(1, 2)]).unwrap_err();
    assert!(err.contains("homogeneous"), "{err}");
}

#[test]
fn a_scaling_is_found_from_two_successive_bounces() {
    let (m, scaling, before, after) = ball_scaling();
    let found = find_scaling(&m, &scaling.cycle, &scaling.slots, &before, &after, &q(1, 2)).expect("found");
    assert_eq!(found, scaling);
}

#[test]
fn a_start_held_exactly_on_the_boundary_and_moving_in_fires_at_the_first_step() {
    let w = 2;
    let m = flow_then_jumps(
        vec![constant(w, a(1)), MPoly::zero(w)],
        vec![false, true],
        vec![Jump {
            guard: Cond::All(vec![
                compare(Rel::Gt, slot(w, 0).sub(&constant(w, a(1)))),
                compare(Rel::Eq, slot(w, 1)),
            ]),
            block: vec![set(1, constant(w, a(1)))],
        }],
    );
    let end = run(&m, vec![num(a(1)), num(a(0))], &a(2)).expect("x > 1 holds after the first step");
    assert_eq!(value(&end[1]), a(1), "the jump fired");
    assert_eq!(value(&end[0]), a(3));
}

#[test]
fn a_start_known_only_to_within_an_infinitesimal_of_the_boundary_is_refused() {
    let w = 2;
    let m = flow_then_jumps(
        vec![constant(w, a(1)), MPoly::zero(w)],
        vec![false, true],
        vec![Jump {
            guard: Cond::All(vec![
                compare(Rel::Lt, slot(w, 0).sub(&constant(w, a(1)))),
                compare(Rel::Eq, slot(w, 1)),
            ]),
            block: vec![set(1, constant(w, a(1)))],
        }],
    );
    let start = vec![Cell::Num { value: a(1), exact: false }, num(a(0))];
    let err = run(&m, start, &a(2)).unwrap_err();
    assert!(err.contains("depends on which infinitesimal"), "x may start just below 1: {err}");
    let exact = run(&m, vec![num(a(1)), num(a(0))], &a(2)).expect("an exact start moves away from x < 1");
    assert_eq!(value(&exact[1]), a(0), "the jump never fired");
}

#[test]
fn a_condition_that_turns_along_the_bounces_stops_recording_where_it_turns() {
    let m = ball(a(10), q(1, 2), Some(compare(Rel::Gt, slot(W, V).sub(&constant(W, a(1))))));
    let end = run(&m, start(a(0), a(10)), &a(5)).expect("the ball resolves");
    assert_eq!(value(&end[LAST]), q(7, 2), "rebounds 5, 5/2, 5/4 exceed 1; 5/8 does not");
    let m = ball(a(10), q(1, 2), Some(compare(Rel::Gt, slot(W, V).sub(&constant(W, q(1, 1000))))));
    let end = run(&m, start(a(0), a(10)), &a(5)).expect("the ball resolves");
    assert_eq!(
        value(&end[LAST]),
        q(8191, 2048),
        "the thirteenth rebound, 5/4096, is the last above 1/1000; it lands at 4 − 1/2048"
    );
}

#[test]
fn a_jump_that_sets_a_flowing_value_to_itself_while_its_guard_holds_leaves_the_flow_alone() {
    let w = 2;
    let m = flow_then_jumps(
        vec![constant(w, a(1)), MPoly::zero(w)],
        vec![false, true],
        vec![Jump {
            guard: compare(Rel::Ge, slot(w, 0).sub(&constant(w, a(1)))),
            block: vec![set(0, slot(w, 0)), set(1, slot(w, 0))],
        }],
    );
    let end = run(&m, vec![num(a(0)), num(a(0))], &a(3)).expect("the jump changes nothing it fires on");
    assert_eq!(value(&end[0]), a(3));
    assert_eq!(value(&end[1]), a(3), "it records x at every firing up to the end");
}

#[test]
fn a_jump_that_pushes_the_state_it_holds_away_is_refused() {
    let w = 1;
    let m = flow_then_jumps(
        vec![constant(w, a(1))],
        vec![false],
        vec![Jump {
            guard: compare(Rel::Ge, slot(w, 0).sub(&constant(w, a(1)))),
            block: vec![set(0, slot(w, 0).scale(&a(2)).sub(&constant(w, a(1))))],
        }],
    );
    let err = run(&m, vec![num(a(0))], &a(3)).unwrap_err();
    assert!(err.contains("does not pull the state back"), "2x − 1 holds 1 but doubles what leaves it: {err}");
}
