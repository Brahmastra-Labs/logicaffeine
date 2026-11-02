use super::*;

#[test]
fn float_equality_is_ieee_and_nan_unequal() {
    // IEEE bit equality — the float artifact is REAL and visible
    // (`is approximately` is the tolerant spelling), identical to what
    // the compiled backend emits.
    let sum = RuntimeValue::Float(0.1 + 0.2);
    assert!(!values_equal(&sum, &RuntimeValue::Float(0.3)));
    assert!(values_equal(
        &sum,
        &RuntimeValue::Float(0.30000000000000004)
    ));
    assert!(!values_equal(
        &RuntimeValue::Float(f64::NAN),
        &RuntimeValue::Float(f64::NAN)
    ));
    // Cross-type numeric equality is EXACT: `1 == 1.0` …
    assert!(values_equal(
        &RuntimeValue::Int(1),
        &RuntimeValue::Float(1.0)
    ));
    // … but never a lossy cast: 2^53 + 1 is NOT the float 2^53.
    assert!(!values_equal(
        &RuntimeValue::Int(9_007_199_254_740_993),
        &RuntimeValue::Float(9_007_199_254_740_992.0)
    ));
}

#[test]
fn collections_compare_structurally() {
    use std::cell::RefCell;
    use std::rc::Rc;
    let list = |vals: Vec<i64>| {
        RuntimeValue::List(Rc::new(RefCell::new(
            crate::interpreter::ListRepr::from_values(
                vals.into_iter().map(RuntimeValue::Int).collect(),
            ),
        )))
    };
    // Same contents ⇒ equal (the audited `[1,2,3] == [1,2,3]` row).
    assert!(values_equal(&list(vec![1, 2, 3]), &list(vec![1, 2, 3])));
    // Different contents or length ⇒ unequal.
    assert!(!values_equal(&list(vec![1, 2, 3]), &list(vec![1, 2, 4])));
    assert!(!values_equal(&list(vec![1, 2]), &list(vec![1, 2, 3])));
}

#[test]
fn nan_relational_comparisons_are_false_not_errors() {
    let nan = RuntimeValue::Float(f64::NAN);
    for op in [
        BinaryOpKind::Lt,
        BinaryOpKind::Gt,
        BinaryOpKind::LtEq,
        BinaryOpKind::GtEq,
    ] {
        let r = compare(op, &nan, &RuntimeValue::Float(1.0)).unwrap();
        assert!(matches!(r, RuntimeValue::Bool(false)));
    }
}

#[test]
fn moment_compares_to_time_by_time_of_day() {
    let nanos_per_day = 86_400_000_000_000i64;
    // A moment at 10:00 into some day vs a time of 11:00.
    let m = RuntimeValue::Moment(3 * nanos_per_day + 10 * 3_600_000_000_000);
    let t = RuntimeValue::Time(11 * 3_600_000_000_000);
    assert!(matches!(
        compare(BinaryOpKind::Lt, &m, &t).unwrap(),
        RuntimeValue::Bool(true)
    ));
    assert!(matches!(
        compare(BinaryOpKind::Gt, &t, &m).unwrap(),
        RuntimeValue::Bool(true)
    ));
}

#[test]
fn comparison_type_error_message() {
    let e = compare(
        BinaryOpKind::Lt,
        &RuntimeValue::Bool(true),
        &RuntimeValue::Int(1),
    )
    .unwrap_err();
    assert_eq!(e, "Cannot compare Bool and Int");
}
