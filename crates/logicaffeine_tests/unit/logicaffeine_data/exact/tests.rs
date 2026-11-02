use super::*;
use logicaffeine_base::hypernatural::Hypernatural;
use logicaffeine_base::hyperreal::Hyperreal;
use logicaffeine_base::limit_value::ResidueType;

fn dt() -> LogosValue {
    LogosValue::infinitesimal("dt", 1, &[0]).unwrap()
}

#[test]
fn compiled_exact_values_print_exactly_as_the_interpreter_does() {
    let root2: LogosValue = tower::from_algebraic(Algebraic::from_i64(2).sqrt().unwrap());
    assert_eq!(root2.to_string(), "√2");
    let pi: LogosValue = tower::from_exact_real(ExactReal::pi());
    assert_eq!(pi.to_string(), "π");
    let e2: LogosValue = tower::from_exact_real(ExactReal::from_i64(2).exp());
    assert_eq!(e2.to_string(), "e^2");
    assert_eq!(LogosValue::from(3).add(&dt()).unwrap().to_string(), "3 + dt");
    let h = LogosValue::infinitesimal("h", 1, &[0]).unwrap();
    assert_eq!(LogosValue::from(1).div(&h).unwrap().to_string(), "1/h");
    let d = dt();
    let Some(ExactRef::Hyper(h)) = d.exact() else {
        panic!("the infinitesimal is a hyperreal");
    };
    let natural: LogosValue =
        tower::from_natural(Hypernatural::generator(&ResidueType::any()), &h.scale);
    assert_eq!(natural.to_string(), "1/dt");
    let omega: LogosValue = tower::from_ordinal(Ordinal::omega().add(&Ordinal::one()));
    assert_eq!(omega.to_string(), "ω + 1");
    assert!(Hyperreal::dt().is_infinitesimal());
}

#[test]
fn compiled_exact_values_order_exactly() {
    let root2: LogosValue = tower::from_algebraic(Algebraic::from_i64(2).sqrt().unwrap());
    let three_halves = LogosValue::from_rational(Rational::from_ratio_i64(3, 2).unwrap());
    assert!(matches!(root2.relation(Rel::Lt, &three_halves).unwrap(), LogosValue::Bool(true)));
    let big = LogosValue::from(1_000_000);
    let omega: LogosValue = tower::from_ordinal(Ordinal::omega());
    assert!(matches!(big.relation(Rel::Lt, &omega).unwrap(), LogosValue::Bool(true)));
}

#[test]
fn an_int_held_as_a_compiled_value_keeps_its_integer_meaning() {
    let seven = LogosValue::from(7);
    assert_eq!(seven.div(&LogosValue::from(2)).unwrap().to_string(), "3");
    assert_eq!(seven.exact_div(&LogosValue::from(2)).unwrap().to_string(), "7/2");
    assert_eq!(LogosValue::from(-7).div(&LogosValue::from(2)).unwrap().to_string(), "-3");
    assert_eq!(LogosValue::from(-7).floor_div(&LogosValue::from(2)).unwrap().to_string(), "-4");
    let max = LogosValue::from(i64::MAX);
    assert_eq!(max.add(&LogosValue::from(1)).unwrap().to_string(), "9223372036854775808");
    assert_eq!(seven.div(&LogosValue::from(0)).unwrap_err(), "Division by zero");
    assert_eq!(seven.rem(&LogosValue::from(0)).unwrap_err(), "Modulo by zero");
}

#[test]
fn a_parity_that_depends_on_the_residue_refuses_a_branch() {
    let h = tower::generator_of(&dt()).unwrap();
    let lamp = h.rem(&LogosValue::from(2)).unwrap();
    assert_eq!(lamp.to_string(), "0 when ⌈1/dt⌉ is even, 1 when it is odd");
    let on = lamp.relation(Rel::Eq, &LogosValue::from(1)).unwrap();
    assert_eq!(
        on.decided_truth().unwrap_err(),
        "this condition depends on the residue of ⌈1/dt⌉: it is false when ⌈1/dt⌉ is even, true when it is odd; declare what is known about 1/dt or take a limit"
    );
    assert_eq!(on.not().unwrap().to_string(), "true when ⌈1/dt⌉ is even, false when it is odd");
}

#[test]
fn a_compiled_value_crosses_a_task() {
    fn send<T: Send + Sync>(_: &T) {}
    send(&dt());
}

/// What a compiled condition reads, `logos_truthy`, is the decided truth: a residue-dependent or a
/// provisional value stops the program with the interpreter's sentence instead of reading as true.
#[test]
fn a_compiled_condition_on_an_undecided_value_stops_with_the_interpreter_s_sentence() {
    let h = tower::generator_of(&dt()).unwrap();
    let on = h
        .rem(&LogosValue::from(2))
        .unwrap()
        .relation(Rel::Eq, &LogosValue::from(1))
        .unwrap();
    let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| crate::ops::logos_truthy(&on))).unwrap_err();
    let why = stopped
        .downcast_ref::<String>()
        .cloned()
        .unwrap_or_default();
    assert_eq!(why, on.decided_truth().unwrap_err());
    let guess = tower::provisional(LogosValue::Bool(false), 10, 0);
    let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| crate::ops::logos_truthy(&guess))).unwrap_err();
    assert_eq!(
        stopped.downcast_ref::<String>().cloned().unwrap_or_default(),
        guess.decided_truth().unwrap_err()
    );
    assert!(crate::ops::logos_truthy(&LogosValue::from(3)));
    assert!(!crate::ops::logos_truthy(&LogosValue::list(vec![])));
}

/// A compiled list of exact values reads through `LogosIndex` as the interpreter's `item` does, and a
/// bad index stops with its sentence.
#[test]
fn a_compiled_list_is_indexed_as_the_interpreter_indexes() {
    use crate::indexing::LogosIndex;
    let xs = LogosValue::list(vec![dt(), LogosValue::from(3)]);
    assert_eq!(xs.logos_get(1i64).to_string(), "dt");
    assert_eq!(xs.logos_get(-1i64).to_string(), "3");
    assert_eq!(xs.logos_get(LogosValue::from(2)).to_string(), "3");
    let stopped = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| xs.logos_get(3i64))).unwrap_err();
    assert_eq!(
        stopped.downcast_ref::<String>().cloned().unwrap_or_default(),
        crate::failure::index_out_of_bounds(3)
    );
}

#[test]
fn an_unset_exact_slot_is_nothing_as_the_interpreter_starts_one() {
    let unset = LogosValue::default();
    assert!(matches!(unset, LogosValue::Nothing));
    assert_eq!(unset.display_string(), "nothing");
}
