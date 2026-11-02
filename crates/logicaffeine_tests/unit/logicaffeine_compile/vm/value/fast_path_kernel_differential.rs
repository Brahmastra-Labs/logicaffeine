//! Every Value op with an Int×Int fast path must be BIT-IDENTICAL to the
//! kernel over the full edge grid — the fast path is a transcription of
//! the locked wrapping-i64 spec, and this test is the proof. It runs under
//! both representations (the narrow inline path re-boxes a wrap past ±2^47,
//! still landing on the kernel's wrapping-i64 answer).
use super::*;

const EDGES: [i64; 12] = [
    i64::MIN,
    i64::MIN + 1,
    -4611686018427387904, // i64::MIN / 2
    -1000003,
    -2,
    -1,
    0,
    1,
    2,
    1000003,
    4611686018427387903, // i64::MAX / 2
    i64::MAX,
];

fn assert_same(
    op_name: &str,
    a: i64,
    b: i64,
    fast: Result<Value, String>,
    kernel: Result<RuntimeValue, String>,
) {
    match (fast, kernel) {
        (Ok(f), Ok(k)) => assert_eq!(
            f.into_runtime(),
            k,
            "{op_name}({a}, {b}) fast path diverged from kernel"
        ),
        (Err(f), Err(k)) => assert_eq!(f, k, "{op_name}({a}, {b}) error strings diverged"),
        (f, k) => panic!("{op_name}({a}, {b}): fast {f:?} vs kernel {k:?}"),
    }
}

#[test]
fn int_arith_matches_kernel_over_edge_grid() {
    for &a in &EDGES {
        for &b in &EDGES {
            let (va, vb) = (Value::int(a), Value::int(b));
            assert_same(
                "add",
                a,
                b,
                va.add(&vb),
                arith::add(RuntimeValue::Int(a), RuntimeValue::Int(b)),
            );
            assert_same(
                "sub",
                a,
                b,
                va.sub(&vb),
                arith::subtract(RuntimeValue::Int(a), RuntimeValue::Int(b)),
            );
            assert_same(
                "mul",
                a,
                b,
                va.mul(&vb),
                arith::multiply(RuntimeValue::Int(a), RuntimeValue::Int(b)),
            );
        }
    }
}

#[test]
fn int_comparisons_match_kernel_over_edge_grid() {
    for &a in &EDGES {
        for &b in &EDGES {
            let (va, vb) = (Value::int(a), Value::int(b));
            assert_same(
                "lt",
                a,
                b,
                va.lt(&vb),
                compare::compare(
                    BinaryOpKind::Lt,
                    &RuntimeValue::Int(a),
                    &RuntimeValue::Int(b),
                ),
            );
            assert_same(
                "gt",
                a,
                b,
                va.gt(&vb),
                compare::compare(
                    BinaryOpKind::Gt,
                    &RuntimeValue::Int(a),
                    &RuntimeValue::Int(b),
                ),
            );
            assert_same(
                "lte",
                a,
                b,
                va.lte(&vb),
                compare::compare(
                    BinaryOpKind::LtEq,
                    &RuntimeValue::Int(a),
                    &RuntimeValue::Int(b),
                ),
            );
            assert_same(
                "gte",
                a,
                b,
                va.gte(&vb),
                compare::compare(
                    BinaryOpKind::GtEq,
                    &RuntimeValue::Int(a),
                    &RuntimeValue::Int(b),
                ),
            );
            assert_eq!(
                va.eq_op(&vb).unwrap().is_truthy(),
                compare::values_equal(&RuntimeValue::Int(a), &RuntimeValue::Int(b)),
                "eq({a}, {b}) diverged"
            );
            assert_eq!(
                va.neq_op(&vb).unwrap().is_truthy(),
                !compare::values_equal(&RuntimeValue::Int(a), &RuntimeValue::Int(b)),
                "neq({a}, {b}) diverged"
            );
        }
    }
}

#[test]
fn value_width_matches_the_active_representation() {
    // The headline of WS-F: under `narrow-value` the register-file cell is
    // ONE machine word (the NaN-boxed `u64`); the default cell is the fat
    // 16-byte `RuntimeValue` newtype. This pins the size each build promises.
    let w = std::mem::size_of::<Value>();
    #[cfg(feature = "narrow-value")]
    assert_eq!(w, 8, "narrow Value must be 8 bytes (NaN-boxed u64)");
    #[cfg(not(feature = "narrow-value"))]
    assert_eq!(
        w, 16,
        "default Value must be 16 bytes (RuntimeValue newtype)"
    );
}

#[test]
fn mixed_operands_still_route_through_the_kernel() {
    // Int×Float promotion is kernel territory — the fast path must not
    // capture it.
    let sum = Value::int(2).add(&Value::float(0.5)).unwrap();
    assert_eq!(sum.into_runtime(), RuntimeValue::Float(2.5));
    assert!(Value::int(2).lt(&Value::float(2.5)).unwrap().is_truthy());
    // Int×Text: whatever the kernel says (it concatenates), bit-for-bit.
    let fast = Value::int(2).add(&Value::text("x".into()));
    let kernel = arith::add(
        RuntimeValue::Int(2),
        RuntimeValue::Text(Rc::new("x".into())),
    );
    assert_eq!(fast.map(Value::into_runtime), kernel);
    // Int×Bool arithmetic is a kernel ERROR — the fast path must not mask it.
    let fast = Value::int(2).mul(&Value::bool(true));
    let kernel = arith::multiply(RuntimeValue::Int(2), RuntimeValue::Bool(true));
    assert_eq!(fast.map(Value::into_runtime), kernel);
    assert!(matches!(Value::int(2).mul(&Value::bool(true)), Err(_)));
}
