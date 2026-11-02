use super::*;
use crate::interpreter::ClosureValue;

fn args(kind: &str, base: i64, name: &str, f: RuntimeValue) -> Vec<RuntimeValue> {
    vec![
        RuntimeValue::Text(Rc::new(kind.to_string())),
        RuntimeValue::Int(base),
        RuntimeValue::Text(Rc::new(name.to_string())),
        f,
    ]
}

/// A stand-in closure value: the evaluator below answers by the argument alone.
fn closure() -> RuntimeValue {
    RuntimeValue::Function(Box::new(ClosureValue {
        body_index: 0,
        captured_env: Default::default(),
        param_names: Vec::new(),
        generated: None,
    }))
}

fn third(_: &RuntimeValue, n: i64) -> Result<RuntimeValue, String> {
    Ok(RuntimeValue::Int(if n == 0 { 0 } else { 1 }))
}

#[test]
fn a_real_defined_inside_a_run_reads_its_function_through_the_runs_evaluator() {
    let run = open_run(Rc::new(third));
    let r = run.define(args("digits", 3, "third", closure())).unwrap();
    let RuntimeValue::ExactReal(x) = &r else { panic!("{r:?}") };
    assert_eq!(x.to_string(), "0.500000000000000000000000000000");
    assert_eq!(x.describe(), "the real number whose digits in base 3 are third");
}

#[test]
fn a_real_defined_outside_a_run_is_refused() {
    let err = outside_a_run(args("digits", 3, "third", closure())).unwrap_err();
    assert_eq!(err, "third defines a real number outside a running program");
    let called = crate::semantics::builtins::call_builtin(
        crate::semantics::builtins::BuiltinId::DefinedReal,
        args("digits", 3, "third", closure()),
    );
    assert_eq!(called.unwrap_err(), err, "a builtin call with no engine behind it is outside every run");
}

#[test]
fn a_real_read_after_its_run_ended_refuses_naming_its_function() {
    let r = {
        let run = open_run(Rc::new(third));
        run.define(args("approximations", 2, "late", closure())).unwrap()
    };
    let RuntimeValue::ExactReal(x) = &r else { panic!("{r:?}") };
    let err = x.compare(&logicaffeine_base::exact_real::ExactReal::from_i64(5), 64).unwrap_err();
    assert_eq!(err.to_string(), "late is read after the run of the program that defined its real number");
    let _other = open_run(Rc::new(third));
    assert!(x.compare(&logicaffeine_base::exact_real::ExactReal::from_i64(5), 64).is_err(), "another run is not its run");
}

/// Runs alive at once on one thread — the program's and each task's, interleaved by the scheduler — each read
/// their own reals, and close in any order.
#[test]
fn runs_alive_at_once_read_their_own_reals_and_close_in_any_order() {
    let first = open_run(Rc::new(third));
    let r = first.define(args("digits", 3, "third", closure())).unwrap();
    let second = open_run(Rc::new(|_: &RuntimeValue, n| Ok(RuntimeValue::Int(if n == 1 { 2 } else { 0 }))));
    let s = second.define(args("digits", 3, "twoThirds", closure())).unwrap();
    let third_run = open_run(Rc::new(|_: &RuntimeValue, _| Ok(RuntimeValue::Int(0))));
    let (RuntimeValue::ExactReal(x), RuntimeValue::ExactReal(y)) = (&r, &s) else { panic!("{r:?} {s:?}") };
    assert_eq!(x.to_decimal(5), "0.50000");
    assert_eq!(y.to_decimal(5), "0.66667");
    drop(first);
    let late = x.compare(&beside_a_half(), 64).unwrap_err();
    assert_eq!(late.to_string(), "third is read after the run of the program that defined its real number");
    assert_eq!(y.to_decimal(8), "0.66666667", "the second run outlives the first");
    drop(third_run);
    drop(second);
}

/// A run's evaluator may hold an engine with runs of its own: closing the run closes them, and nothing is read
/// from the table while it changes.
#[test]
fn a_run_whose_evaluator_holds_another_run_closes_both() {
    let inner = open_run(Rc::new(third));
    let s = inner.define(args("digits", 3, "third", closure())).unwrap();
    let held = RefCell::new(Some(inner));
    let outer = open_run(Rc::new(move |_: &RuntimeValue, _| {
        let _ = &held;
        Ok(RuntimeValue::Int(1))
    }));
    let RuntimeValue::ExactReal(y) = &s else { panic!("{s:?}") };
    assert_eq!(y.to_decimal(5), "0.50000");
    drop(outer);
    assert!(y.compare(&beside_a_half(), 64).is_err(), "a closed run's real reads no digit it lacks");
}

/// A number within `10^-15` of a half: telling a defined half from it asks for digits a short print never read.
fn beside_a_half() -> logicaffeine_base::exact_real::ExactReal {
    let r = logicaffeine_base::Rational::parse("499999999999999/1000000000000000").expect("a ratio");
    logicaffeine_base::exact_real::ExactReal::from_rational(r)
}

#[test]
fn an_answer_that_is_no_rational_is_refused_in_the_reals_words() {
    let run = open_run(Rc::new(|_: &RuntimeValue, _| Ok(RuntimeValue::Float(0.5))));
    let r = run.define(args("approximations", 2, "halves", closure())).unwrap();
    let RuntimeValue::ExactReal(x) = &r else { panic!("{r:?}") };
    let err = x.compare(&logicaffeine_base::exact_real::ExactReal::from_i64(0), 64).unwrap_err().to_string();
    assert!(err.starts_with("halves(") && err.ends_with("is Float, and a real number is defined by whole or rational answers"), "{err}");
}

#[test]
fn a_definition_names_its_kind_and_a_base_from_two() {
    assert_eq!(Definition::named("digits", 10), Ok(Definition::Digits { base: 10 }));
    assert_eq!(Definition::named("approximations", 2), Ok(Definition::Approximations));
    assert_eq!(Definition::named("digits", 1).unwrap_err(), "a real number's digits are in a base from 2 up, got 1");
    assert!(Definition::named("guesses", 2).is_err());
    let run = open_run(Rc::new(third));
    assert_eq!(
        run.define(args("digits", 3, "third", RuntimeValue::Int(4))).unwrap_err(),
        "third defines a real number as a function, got Int"
    );
}
