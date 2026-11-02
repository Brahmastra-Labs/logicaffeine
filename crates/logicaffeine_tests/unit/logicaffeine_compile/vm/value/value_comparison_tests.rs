use super::*;

#[test]
fn float_relational_is_ieee() {
    // -0.0 == 0.0 under IEEE 754.
    assert!(!Value::float(-0.0)
        .lt(&Value::float(0.0))
        .unwrap()
        .is_truthy());
    assert!(Value::float(-0.0)
        .lte(&Value::float(0.0))
        .unwrap()
        .is_truthy());
    // NaN is unordered: every relational comparison is false (never an error).
    let nan = Value::float(f64::NAN);
    assert!(!nan.lt(&Value::float(1.0)).unwrap().is_truthy());
    assert!(!nan.gt(&Value::float(1.0)).unwrap().is_truthy());
    assert!(!nan.lte(&nan).unwrap().is_truthy());
    assert!(!Value::float(1.0).gte(&nan).unwrap().is_truthy());
    // Ordinary comparisons still hold.
    assert!(Value::float(1.5)
        .lt(&Value::float(2.5))
        .unwrap()
        .is_truthy());
    assert!(Value::int(2).lt(&Value::float(2.5)).unwrap().is_truthy());
    assert!(Value::int(5).gte(&Value::int(5)).unwrap().is_truthy());
}

#[test]
fn float_equality_matches_treewalker() {
    // NaN is never equal to itself (IEEE, both engines).
    assert!(!Value::float(f64::NAN)
        .eq_op(&Value::float(f64::NAN)).unwrap()
        .is_truthy());
    assert!(Value::float(f64::NAN)
        .neq_op(&Value::float(f64::NAN)).unwrap()
        .is_truthy());
    // Exact equal floats are equal.
    assert!(Value::float(1.5).eq_op(&Value::float(1.5)).unwrap().is_truthy());
    // IEEE equality: 0.1 + 0.2 is NOT 0.3 — matching the tree-walker
    // (and the compiled backend; `is approximately` is the tolerant form).
    let sum = Value::float(0.1).add(&Value::float(0.2)).unwrap(); // 0.30000000000000004
    assert!(!sum.eq_op(&Value::float(0.3)).unwrap().is_truthy());
    assert!(sum.eq_op(&Value::float(0.30000000000000004)).unwrap().is_truthy());
    // Distinct floats are not equal.
    assert!(!Value::float(1.0).eq_op(&Value::float(2.0)).unwrap().is_truthy());
    // Other types unaffected.
    assert!(Value::int(5).eq_op(&Value::int(5)).unwrap().is_truthy());
    assert!(Value::text("hi".to_string())
        .eq_op(&Value::text("hi".to_string()))
        .unwrap()
        .is_truthy());
    assert!(!Value::int(5).eq_op(&Value::int(6)).unwrap().is_truthy());
}
