use z3::ast::{Bool, Dynamic, Int};
use z3::{SatResult, Solver};

use super::*;
use crate::bytecode::value::wrap64;

/// `x ∈ {6, 7}`, on a solver that may do no work at all: every incremental check comes back unknown.
fn starved() -> (Solver, Int) {
    let solver = Solver::new();
    solver.set_params(&budget(1));
    let x = Int::new_const("x");
    solver.assert(&x.gt(Int::from_i64(5)));
    solver.assert(&(&x * &x).lt(Int::from_i64(50)));
    (solver, x)
}

/// A query the incremental engine gives up on is decided afresh, and a model is read from the solver that decided.
#[test]
fn an_undecided_query_is_decided_afresh_and_its_model_read_from_the_fresh_solver() {
    let (solver, x) = starved();
    assert_eq!(solver.check_assumptions(&[]), SatResult::Unknown, "a resource limit of one unit decides nothing");
    let not_seven = x.eq(Int::from_i64(7)).not();
    let d = decide_within(&solver, &[not_seven.clone()], QUERY_RLIMIT);
    assert_eq!(d.answer, SatResult::Sat);
    assert!(d.fresh.is_some(), "the fresh solver decided");
    let model = d.by(&solver).get_model().expect("the deciding solver has the model");
    assert_eq!(model.eval(&x, true).and_then(|v| v.as_i64()), Some(6));
    let neither = [not_seven, x.eq(Int::from_i64(6)).not()];
    assert_eq!(decide_within(&solver, &neither, QUERY_RLIMIT).answer, SatResult::Unsat);
}

/// A query the incremental engine decides is not asked again.
#[test]
fn a_decided_query_is_not_asked_afresh() {
    let solver = Solver::new();
    solver.set_params(&budget(INCREMENTAL_RLIMIT));
    let x = Int::new_const("x");
    solver.assert(&x.gt(Int::from_i64(5)));
    let d = decide_within(&solver, &[x.lt(Int::from_i64(5))], QUERY_RLIMIT);
    assert_eq!((d.answer, d.fresh.is_some()), (SatResult::Unsat, false));
}

/// A wrap the solver's assertions keep in `i64` becomes its exact value; one they do not, or a term that only looks
/// like one, is left as it is.
#[test]
fn a_wrap_the_assertions_rule_out_is_replaced_by_the_exact_value() {
    let solver = Solver::new();
    solver.set_params(&budget(INCREMENTAL_RLIMIT));
    let (x, y) = (Int::new_const("x"), Int::new_const("y"));
    solver.assert(&x.ge(Int::from_i64(0)));
    solver.assert(&x.le(Int::from_i64(1000)));
    let inner = wrap64(&(&x + 1i64));
    let goal = wrap64(&(&x * &inner)).eq(Int::from_i64(42));
    let settled = discharge_wraps(&solver, &goal);
    assert_eq!(settled, (&x * &(&x + 1i64)).eq(Int::from_i64(42)), "both wraps, inner first");
    let open = wrap64(&(&y + 1i64)).eq(Int::from_i64(0));
    assert_eq!(discharge_wraps(&solver, &open), open, "nothing bounds `y`");
    let look_alike = in_i64(&(&x + 2i64)).ite(&(&x + 1i64), &x).eq(Int::from_i64(3));
    assert_eq!(discharge_wraps(&solver, &look_alike), look_alike, "its test is not of its first branch");
}

#[test]
fn a_term_is_linear_unless_it_multiplies_two_variables() {
    let (x, y) = (Int::new_const("x"), Int::new_const("y"));
    let linear = |t: &Bool| is_linear(&Dynamic::from_ast(t));
    assert!(linear(&(&(&x * 3i64) + &y).ge(Int::from_i64(1))));
    assert!(linear(&x.le(&y)));
    assert!(!linear(&(&x * &y).ge(Int::from_i64(1))));
    assert!(!linear(&(&(&x * &x) + 1i64).eq(Int::from_i64(5))));
}
