use super::*;

fn expect_bool(r: Result<RuntimeValue, String>, want: bool, label: &str) {
    match r {
        Ok(RuntimeValue::Bool(b)) => assert_eq!(b, want, "{label}"),
        other => panic!(
            "{label}: expected Bool, got {:?}",
            other.map(|v| v.type_name().to_string())
        ),
    }
}

#[test]
fn float_relational_uses_ieee_semantics() {
    use RuntimeValue::{Float, Int};
    let interner = Interner::new();
    let interp = Interpreter::new(&interner);
    let nan = f64::NAN;

    // -0.0 and +0.0 are equal under IEEE 754.
    expect_bool(
        interp.apply_binary_op(BinaryOpKind::Lt, Float(-0.0), Float(0.0)),
        false,
        "-0.0 < 0.0",
    );
    expect_bool(
        interp.apply_binary_op(BinaryOpKind::Gt, Float(0.0), Float(-0.0)),
        false,
        "0.0 > -0.0",
    );
    expect_bool(
        interp.apply_binary_op(BinaryOpKind::LtEq, Float(-0.0), Float(0.0)),
        true,
        "-0.0 <= 0.0",
    );
    expect_bool(
        interp.apply_binary_op(BinaryOpKind::GtEq, Float(-0.0), Float(0.0)),
        true,
        "-0.0 >= 0.0",
    );

    // NaN is unordered: every relational comparison is false.
    expect_bool(
        interp.apply_binary_op(BinaryOpKind::Lt, Float(nan), Float(1.0)),
        false,
        "NaN < 1",
    );
    expect_bool(
        interp.apply_binary_op(BinaryOpKind::Gt, Float(nan), Float(1.0)),
        false,
        "NaN > 1",
    );
    expect_bool(
        interp.apply_binary_op(BinaryOpKind::LtEq, Float(nan), Float(nan)),
        false,
        "NaN <= NaN",
    );
    expect_bool(
        interp.apply_binary_op(BinaryOpKind::GtEq, Float(1.0), Float(nan)),
        false,
        "1 >= NaN",
    );

    // Ordinary comparisons still work, including mixed Int/Float and pure Int.
    expect_bool(
        interp.apply_binary_op(BinaryOpKind::Lt, Float(1.5), Float(2.5)),
        true,
        "1.5 < 2.5",
    );
    expect_bool(
        interp.apply_binary_op(BinaryOpKind::Lt, Int(2), Float(2.5)),
        true,
        "2 < 2.5",
    );
    expect_bool(
        interp.apply_binary_op(BinaryOpKind::GtEq, Float(2.5), Int(2)),
        true,
        "2.5 >= 2",
    );
    expect_bool(
        interp.apply_binary_op(BinaryOpKind::Lt, Int(3), Int(5)),
        true,
        "3 < 5",
    );
    expect_bool(
        interp.apply_binary_op(BinaryOpKind::GtEq, Int(5), Int(5)),
        true,
        "5 >= 5",
    );
}
