use super::*;
use crate::concurrency::marshal::GenExpr;
use crate::interpreter::ClosureValue;
use std::collections::HashMap;
use std::rc::Rc;

/// A shipped pure computation `3·x + 1` over one argument — exactly the shape a peer
/// ships via `Send computed`.
fn shipped_3x_plus_1() -> RuntimeValue {
    let gen = GenExpr::Add(
        Box::new(GenExpr::Mul(
            Box::new(GenExpr::Index),
            Box::new(GenExpr::Const(3)),
        )),
        Box::new(GenExpr::Const(1)),
    );
    RuntimeValue::Function(Box::new(ClosureValue {
        body_index: usize::MAX,
        captured_env: HashMap::default(),
        param_names: vec![logicaffeine_base::Symbol::from_index(0)],
        generated: Some(Rc::new(gen)),
    }))
}

#[test]
fn in_range_argument_runs_in_the_sandbox() {
    let contract = AcceptanceContract::new(0, 1000);
    // 3·5 + 1 = 16, and 5 ∈ [0, 1000].
    assert_eq!(contract.apply(&shipped_3x_plus_1(), 5).unwrap(), 16);
    // Boundaries are inclusive.
    assert_eq!(contract.apply(&shipped_3x_plus_1(), 0).unwrap(), 1);
    assert_eq!(contract.apply(&shipped_3x_plus_1(), 1000).unwrap(), 3001);
}

#[test]
fn out_of_range_argument_is_refused_not_clamped() {
    let contract = AcceptanceContract::new(0, 1000);
    let above = contract.apply(&shipped_3x_plus_1(), 1001);
    let below = contract.apply(&shipped_3x_plus_1(), -1);
    assert!(
        above.is_err(),
        "an argument above the range must be refused"
    );
    assert!(
        below.is_err(),
        "an argument below the range must be refused"
    );
    // Refused, not clamped: the error names the offending value, and NO result is produced.
    assert!(above.unwrap_err().contains("1001"));
    assert!(below.unwrap_err().contains("refused"));
}

#[test]
fn an_ordinary_closure_is_refused_at_the_signature_check() {
    // A closure with NO `generated` body (an arena-resident interpreter closure) must
    // never run under a contract — the sandbox is the only thing a contract evaluates.
    let ordinary = RuntimeValue::Function(Box::new(ClosureValue {
        body_index: 7,
        captured_env: HashMap::default(),
        param_names: vec![logicaffeine_base::Symbol::from_index(0)],
        generated: None,
    }));
    let contract = AcceptanceContract::new(0, 1000);
    assert!(
        contract.apply(&ordinary, 5).is_err(),
        "an ordinary closure must be refused"
    );
}

#[test]
fn a_wrong_arity_computation_is_refused() {
    let gen = GenExpr::Index;
    let two_arg = RuntimeValue::Function(Box::new(ClosureValue {
        body_index: usize::MAX,
        captured_env: HashMap::default(),
        param_names: vec![
            logicaffeine_base::Symbol::from_index(0),
            logicaffeine_base::Symbol::from_index(1),
        ],
        generated: Some(Rc::new(gen)),
    }));
    let contract = AcceptanceContract::new(0, 1000);
    assert!(
        contract.apply(&two_arg, 5).is_err(),
        "a 2-argument computation must be refused"
    );
}

#[test]
fn a_non_function_value_is_refused() {
    let contract = AcceptanceContract::new(0, 1000);
    assert!(
        contract.apply(&RuntimeValue::Int(5), 5).is_err(),
        "a non-function must be refused"
    );
}

#[test]
fn a_reversed_range_accepts_nothing() {
    // lo > hi is a vacuous contract — every argument is out of range.
    let contract = AcceptanceContract::new(1000, 0);
    assert!(contract.apply(&shipped_3x_plus_1(), 5).is_err());
    assert!(contract.apply(&shipped_3x_plus_1(), 500).is_err());
}
