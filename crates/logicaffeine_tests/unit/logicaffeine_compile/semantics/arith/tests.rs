use super::*;
use crate::interpreter::StructValue;
use std::collections::HashMap;

#[test]
fn money_arithmetic_and_comparison_are_exact_and_currency_safe() {
    use crate::ast::stmt::BinaryOpKind;
    use crate::semantics::builtins::{call_builtin, BuiltinId};
    use crate::semantics::compare::compare;
    let money = |s: &str, code: &str| {
        call_builtin(
            BuiltinId::Money,
            vec![
                RuntimeValue::Decimal(Rc::new(logicaffeine_base::Decimal::parse(s).unwrap())),
                RuntimeValue::Text(Rc::new(code.to_string())),
            ],
        )
        .unwrap()
    };
    let show = |v: &RuntimeValue| v.to_display_string();

    // Construction + display at the currency's minor unit.
    assert_eq!(show(&money("19.99", "USD")), "19.99 USD");
    // Same-currency add/sub are EXACT (no float drift) and keep the currency.
    assert_eq!(
        show(&add(money("0.10", "USD"), money("0.20", "USD")).unwrap()),
        "0.30 USD"
    );
    assert_eq!(
        show(&add(money("19.99", "USD"), money("5.00", "USD")).unwrap()),
        "24.99 USD"
    );
    assert_eq!(
        show(&subtract(money("24.99", "USD"), money("5.00", "USD")).unwrap()),
        "19.99 USD"
    );
    // Cross-currency add/sub are a typed error (no common meaning).
    assert!(add(money("5.00", "USD"), money("1.00", "EUR")).is_err());
    assert!(subtract(money("5.00", "USD"), money("1.00", "EUR")).is_err());
    // Scale by a number (commutes); divide a bill by a number.
    assert_eq!(
        show(&multiply(money("19.99", "USD"), RuntimeValue::Int(3)).unwrap()),
        "59.97 USD"
    );
    assert_eq!(
        show(&multiply(RuntimeValue::Int(3), money("19.99", "USD")).unwrap()),
        "59.97 USD"
    );
    assert_eq!(
        show(&divide(money("10.00", "USD"), RuntimeValue::Int(4)).unwrap()),
        "2.50 USD"
    );
    // Same-currency ratio (30/10 = 3, narrows to Int); money × money is refused.
    assert!(matches!(
        divide(money("30.00", "USD"), money("10.00", "USD")).unwrap(),
        RuntimeValue::Int(3) | RuntimeValue::Rational(_)
    ));
    assert!(multiply(money("2.00", "USD"), money("2.00", "USD")).is_err());
    // Ordering within a currency; ordering across currencies is a typed error.
    assert_eq!(
        compare(
            BinaryOpKind::Gt,
            &money("5.00", "USD"),
            &money("1.00", "USD")
        )
        .unwrap(),
        RuntimeValue::Bool(true)
    );
    assert!(compare(
        BinaryOpKind::Lt,
        &money("5.00", "USD"),
        &money("1.00", "EUR")
    )
    .is_err());
    // `money()` refuses an inexact amount and an unknown currency.
    assert!(call_builtin(
        BuiltinId::Money,
        vec![
            RuntimeValue::Float(1.5),
            RuntimeValue::Text(Rc::new("USD".to_string()))
        ]
    )
    .is_err());
    assert!(call_builtin(
        BuiltinId::Money,
        vec![
            RuntimeValue::Int(5),
            RuntimeValue::Text(Rc::new("XYZ".to_string()))
        ]
    )
    .is_err());
}

#[test]
fn crdt_wire_int_into_nothing_takes_value() {
    let bytes = crdt_to_wire(&RuntimeValue::Int(7)).unwrap();
    assert!(matches!(
        crdt_merge_wire(RuntimeValue::Nothing, &bytes),
        RuntimeValue::Int(7)
    ));
}

#[test]
fn crdt_wire_int_merge_adds() {
    let bytes = crdt_to_wire(&RuntimeValue::Int(5)).unwrap();
    assert!(matches!(
        crdt_merge_wire(RuntimeValue::Int(3), &bytes),
        RuntimeValue::Int(8)
    ));
}

#[test]
fn crdt_wire_struct_merges_fieldwise() {
    let mut fields = HashMap::new();
    fields.insert("a".to_string(), RuntimeValue::Int(2));
    fields.insert("b".to_string(), RuntimeValue::Int(4));
    let incoming = RuntimeValue::Struct(Box::new(StructValue {
        type_name: "Counter".into(),
        fields,
    }));
    let bytes = crdt_to_wire(&incoming).unwrap();

    let mut local_fields = HashMap::new();
    local_fields.insert("a".to_string(), RuntimeValue::Int(1)); // b absent ⇒ 0
    let local = RuntimeValue::Struct(Box::new(StructValue {
        type_name: "Counter".into(),
        fields: local_fields,
    }));
    match crdt_merge_wire(local, &bytes) {
        RuntimeValue::Struct(s) => {
            assert!(
                matches!(s.fields.get("a"), Some(RuntimeValue::Int(3))),
                "1 + 2"
            );
            assert!(
                matches!(s.fields.get("b"), Some(RuntimeValue::Int(4))),
                "0 + 4"
            );
        }
        other => panic!("expected a struct, got {other:?}"),
    }
}

#[test]
fn crdt_wire_nothing_has_nothing_to_publish() {
    assert!(crdt_to_wire(&RuntimeValue::Nothing).is_none());
}

#[test]
fn crdt_wire_bool_lww_takes_incoming() {
    let bytes = crdt_to_wire(&RuntimeValue::Bool(true)).unwrap();
    assert!(matches!(
        crdt_merge_wire(RuntimeValue::Bool(false), &bytes),
        RuntimeValue::Bool(true)
    ));
}

#[test]
fn crdt_wire_text_lww() {
    let bytes = crdt_to_wire(&RuntimeValue::Text(std::rc::Rc::new("hi".into()))).unwrap();
    match crdt_merge_wire(RuntimeValue::Nothing, &bytes) {
        RuntimeValue::Text(s) => assert_eq!(&*s, "hi"),
        other => panic!("expected Text, got {other:?}"),
    }
}

#[test]
fn crdt_wire_float_roundtrips() {
    let bytes = crdt_to_wire(&RuntimeValue::Float(2.5)).unwrap();
    match crdt_merge_wire(RuntimeValue::Nothing, &bytes) {
        RuntimeValue::Float(f) => assert_eq!(f, 2.5),
        other => panic!("expected Float, got {other:?}"),
    }
}

fn set_of(ns: &[i64]) -> RuntimeValue {
    RuntimeValue::Set(std::rc::Rc::new(std::cell::RefCell::new(
        ns.iter().map(|n| RuntimeValue::Int(*n)).collect(),
    )))
}

#[test]
fn crdt_wire_set_unions() {
    let bytes = crdt_to_wire(&set_of(&[2, 3])).unwrap();
    match crdt_merge_wire(set_of(&[1, 2]), &bytes) {
        RuntimeValue::Set(items) => {
            let v = items.borrow();
            assert_eq!(v.len(), 3, "{{1,2}} ∪ {{2,3}} = {{1,2,3}}");
            for n in [1, 2, 3] {
                assert!(
                    v.iter()
                        .any(|x| matches!(x, RuntimeValue::Int(m) if *m == n)),
                    "missing {n}"
                );
            }
        }
        other => panic!("expected Set, got {other:?}"),
    }
}

#[test]
fn crdt_wire_struct_mixed_types_merge_each_by_its_rule() {
    let mut fields = HashMap::new();
    fields.insert("hits".to_string(), RuntimeValue::Int(3));
    fields.insert(
        "title".to_string(),
        RuntimeValue::Text(std::rc::Rc::new("v2".into())),
    );
    fields.insert("tags".to_string(), set_of(&[9]));
    let incoming = RuntimeValue::Struct(Box::new(StructValue {
        type_name: "Page".into(),
        fields,
    }));
    let bytes = crdt_to_wire(&incoming).unwrap();

    let mut local = HashMap::new();
    local.insert("hits".to_string(), RuntimeValue::Int(1));
    local.insert(
        "title".to_string(),
        RuntimeValue::Text(std::rc::Rc::new("v1".into())),
    );
    local.insert("tags".to_string(), set_of(&[7]));
    let local = RuntimeValue::Struct(Box::new(StructValue {
        type_name: "Page".into(),
        fields: local,
    }));

    match crdt_merge_wire(local, &bytes) {
        RuntimeValue::Struct(s) => {
            assert!(
                matches!(s.fields.get("hits"), Some(RuntimeValue::Int(4))),
                "counter 1+3"
            );
            assert!(
                matches!(s.fields.get("title"), Some(RuntimeValue::Text(t)) if &***t == "v2"),
                "LWW register"
            );
            match s.fields.get("tags") {
                Some(RuntimeValue::Set(items)) => {
                    assert_eq!(items.borrow().len(), 2, "set union {{7}} ∪ {{9}}")
                }
                other => panic!("tags not a set: {other:?}"),
            }
        }
        other => panic!("expected struct, got {other:?}"),
    }
}

#[test]
fn error_messages_are_canonical() {
    let e = add(RuntimeValue::Bool(true), RuntimeValue::Nothing).unwrap_err();
    assert_eq!(e, "Cannot add Bool and Nothing");
    let e = subtract(RuntimeValue::Bool(true), RuntimeValue::Nothing).unwrap_err();
    assert_eq!(e, "Cannot subtract Nothing from Bool");
    let e = multiply(RuntimeValue::Bool(true), RuntimeValue::Nothing).unwrap_err();
    assert_eq!(e, "Cannot multiply Bool and Nothing");
    let e = divide(RuntimeValue::Int(1), RuntimeValue::Int(0)).unwrap_err();
    assert_eq!(e, "Division by zero");
    let e = divide(RuntimeValue::Float(1.0), RuntimeValue::Float(0.0)).unwrap_err();
    assert_eq!(e, "Division by zero");
    let e = modulo(RuntimeValue::Int(1), RuntimeValue::Int(0)).unwrap_err();
    assert_eq!(e, "Modulo by zero");
    // `not` is total — logical negation of truthiness, never an error.
    let r = not_value(RuntimeValue::Nothing).unwrap();
    assert!(matches!(r, RuntimeValue::Bool(true)));
}

#[test]
fn text_add_stringifies_either_side() {
    let r = add(
        RuntimeValue::Text(Rc::new("n=".to_string())),
        RuntimeValue::Int(4),
    )
    .unwrap();
    assert!(matches!(&r, RuntimeValue::Text(s) if **s == "n=4"));
    let r = add(
        RuntimeValue::Int(4),
        RuntimeValue::Text(Rc::new("!".to_string())),
    )
    .unwrap();
    assert!(matches!(&r, RuntimeValue::Text(s) if **s == "4!"));
}

#[test]
fn date_plus_span_is_calendar_aware() {
    // 2024-01-31 (day 19753) + 1 month = 2024-02-29.
    let r = add(
        RuntimeValue::Date(19753),
        RuntimeValue::Span { months: 1, days: 0 },
    )
    .unwrap();
    assert!(matches!(r, RuntimeValue::Date(19782)));
    // And subtraction inverts the span sign.
    let r = subtract(
        RuntimeValue::Date(19782),
        RuntimeValue::Span { months: 0, days: 1 },
    )
    .unwrap();
    assert!(matches!(r, RuntimeValue::Date(19781)));
}

#[test]
fn decimal_arithmetic_stays_exact_and_promotes_correctly() {
    let d = |s: &str| RuntimeValue::Decimal(Rc::new(Decimal::parse(s).unwrap()));
    // +,-,* stay exact Decimal — money keeps its scale, no f64 drift.
    assert_eq!(
        add(d("19.99"), d("0.01")).unwrap().to_display_string(),
        "20.00"
    );
    assert_eq!(
        subtract(d("20.00"), d("0.01")).unwrap().to_display_string(),
        "19.99"
    );
    assert_eq!(
        multiply(d("1.1"), d("1.1")).unwrap().to_display_string(),
        "1.21"
    );
    assert_eq!(add(d("0.1"), d("0.2")).unwrap().to_display_string(), "0.3");
    // Decimal ∘ Int stays Decimal (a count scales money).
    assert_eq!(
        multiply(d("19.99"), RuntimeValue::Int(3))
            .unwrap()
            .to_display_string(),
        "59.97"
    );
    assert!(matches!(
        add(d("1.50"), RuntimeValue::Int(1)).unwrap(),
        RuntimeValue::Decimal(_)
    ));
    // Division promotes to the EXACT Rational tower (base-10 division need not terminate).
    let q = divide(d("1"), d("3")).unwrap();
    assert!(matches!(q, RuntimeValue::Rational(_)));
    assert_eq!(q.to_display_string(), "1/3");
    // A Rational operand promotes to exact Rational; a Float operand yields Float.
    let third = RuntimeValue::from_rational(Rational::from_ratio_i64(1, 3).unwrap());
    assert!(matches!(
        add(d("0.5"), third).unwrap(),
        RuntimeValue::Rational(_)
    ));
    assert!(matches!(
        add(d("0.5"), RuntimeValue::Float(0.25)).unwrap(),
        RuntimeValue::Float(_)
    ));
    // Cross-type ordering: 19.99 (Decimal) > 10 (Int).
    assert!(matches!(
        compare(BinaryOpKind::Gt, &d("19.99"), &RuntimeValue::Int(10)).unwrap(),
        RuntimeValue::Bool(true)
    ));
}

#[test]
fn complex_arithmetic_is_exact_closed_and_unordered() {
    let c = |re: i64, im: i64| {
        RuntimeValue::Complex(Rc::new(Complex::new(
            Rational::from_i64(re),
            Rational::from_i64(im),
        )))
    };
    let cq = |rn: i64, rd: i64, in_: i64, id: i64| {
        RuntimeValue::Complex(Rc::new(Complex::new(
            Rational::from_ratio_i64(rn, rd).unwrap(),
            Rational::from_ratio_i64(in_, id).unwrap(),
        )))
    };
    let i = c(0, 1);
    // The headline: i·i = −1, exact.
    assert_eq!(multiply(i.clone(), i.clone()).unwrap(), c(-1, 0));
    // Addition / subtraction.
    assert_eq!(add(c(2, 3), c(1, -1)).unwrap(), c(3, 2));
    assert_eq!(subtract(c(5, 2), c(1, 7)).unwrap(), c(4, -5));
    // (1+i)(1−i) = 2.
    assert_eq!(multiply(c(1, 1), c(1, -1)).unwrap(), c(2, 0));
    // (2+3i)(4+5i) = −7 + 22i.
    assert_eq!(multiply(c(2, 3), c(4, 5)).unwrap(), c(-7, 22));
    // Division stays Complex (closed field): (3+4i)/(1+2i) = (11−2i)/5; z/z = 1.
    assert_eq!(divide(c(3, 4), c(1, 2)).unwrap(), cq(11, 5, -2, 5));
    assert_eq!(divide(c(2, 3), c(2, 3)).unwrap(), c(1, 0));
    // A real embeds (re + 0i): Complex ∘ Int / BigInt / Rational stays Complex.
    assert_eq!(add(c(2, 3), RuntimeValue::Int(5)).unwrap(), c(7, 3));
    assert_eq!(multiply(i.clone(), RuntimeValue::Int(3)).unwrap(), c(0, 3)); // 3i
    let half = RuntimeValue::from_rational(Rational::from_ratio_i64(1, 2).unwrap());
    assert_eq!(add(c(1, 0), half).unwrap(), cq(3, 2, 0, 1));
    // Equality is exact and structural; reduced parts compare equal.
    assert!(matches!(
        binary_op(BinaryOpKind::Eq, c(3, 4), c(3, 4)).unwrap(),
        RuntimeValue::Bool(true)
    ));
    assert!(matches!(
        binary_op(BinaryOpKind::Eq, c(3, 4), c(3, -4)).unwrap(),
        RuntimeValue::Bool(false)
    ));
    // An inexact Float operand PROMOTES the exact Complex to a floating-point ComplexF — the
    // `Rational + Float → Float` promotion extended to the complex plane. (1+i) + 0.5 = 1.5+i;
    // 2.0 · i = 2i.
    let cf = |re: f64, im: f64| RuntimeValue::ComplexF(Rc::new(ComplexF::new(re, im)));
    assert_eq!(
        add(c(1, 1), RuntimeValue::Float(0.5)).unwrap(),
        cf(1.5, 1.0)
    );
    assert_eq!(
        multiply(RuntimeValue::Float(2.0), i.clone()).unwrap(),
        cf(0.0, 2.0)
    );
    // Division by zero is a clean error, never a panic.
    assert!(divide(c(1, 1), c(0, 0)).is_err());
    // Complex has NO total order: every relational comparison is a typed error.
    for op in [
        BinaryOpKind::Lt,
        BinaryOpKind::Gt,
        BinaryOpKind::LtEq,
        BinaryOpKind::GtEq,
    ] {
        assert!(
            compare(op, &c(1, 1), &c(2, 2)).is_err(),
            "complex is unordered under {op:?}"
        );
    }
    // Display forms.
    assert_eq!(c(3, 4).to_display_string(), "3+4i");
    assert_eq!(c(0, 1).to_display_string(), "i");
    assert_eq!(c(0, -1).to_display_string(), "-i");
    assert_eq!(c(-1, 0).to_display_string(), "-1");
}

/// The variant kind (NOT `type_name`, which folds BigInt into "Int").
fn kind(v: &RuntimeValue) -> &'static str {
    match v {
        RuntimeValue::Int(_) => "Int",
        RuntimeValue::BigInt(_) => "BigInt",
        RuntimeValue::Rational(_) => "Rational",
        RuntimeValue::Decimal(_) => "Decimal",
        RuntimeValue::Complex(_) => "Complex",
        RuntimeValue::ComplexF(_) => "ComplexF",
        RuntimeValue::Modular(_) => "Modular",
        RuntimeValue::Float(_) => "Float",
        _ => "other", // the gauntlet only uses numeric values; this keeps the return 'static
    }
}

/// THE GAUNTLET: every numeric type × every type × {+, −, ×, ÷}, asserting the promotion
/// KIND and the exact VALUE (for the exact types). The promotion precedence is: an exact
/// `Complex` meeting an inexact `Float` PROMOTES to a floating-point `ComplexF` (the
/// `Rational + Float → Float` promotion extended to the complex plane); otherwise
/// Complex > Rational > Decimal > Int, and Decimal ÷ widens to Rational. Modular combines
/// only with Modular of the same modulus.
#[test]
fn numeric_tower_cross_type_promotion_gauntlet() {
    let int = || RuntimeValue::Int(2);
    let rat = || RuntimeValue::from_rational(Rational::from_ratio_i64(1, 3).unwrap());
    let dec = || RuntimeValue::Decimal(Rc::new(Decimal::parse("0.5").unwrap()));
    let cpx = || {
        RuntimeValue::Complex(Rc::new(Complex::new(
            Rational::from_i64(2),
            Rational::from_i64(3),
        )))
    };
    let flt = || RuntimeValue::Float(2.0);

    // ---- ADD: kind + exact value where the result is an exact type ----
    let g = |a: RuntimeValue, b: RuntimeValue| add(a, b);
    assert_eq!(kind(&g(int(), int()).unwrap()), "Int");
    assert_eq!(g(int(), rat()).unwrap().to_display_string(), "7/3"); // 2 + 1/3
    assert_eq!(kind(&g(int(), rat()).unwrap()), "Rational");
    assert_eq!(g(int(), dec()).unwrap().to_display_string(), "2.5"); // 2 + 0.5
    assert_eq!(kind(&g(int(), dec()).unwrap()), "Decimal");
    assert_eq!(g(int(), cpx()).unwrap().to_display_string(), "4+3i"); // 2 + (2+3i)
    assert_eq!(kind(&g(int(), cpx()).unwrap()), "Complex");
    assert_eq!(kind(&g(int(), flt()).unwrap()), "Float");
    assert_eq!(g(rat(), dec()).unwrap().to_display_string(), "5/6"); // 1/3 + 1/2
    assert_eq!(kind(&g(rat(), dec()).unwrap()), "Rational"); // Rational beats Decimal
    assert_eq!(g(rat(), cpx()).unwrap().to_display_string(), "7/3+3i"); // 1/3 + (2+3i)
    assert_eq!(kind(&g(rat(), cpx()).unwrap()), "Complex");
    assert_eq!(kind(&g(rat(), flt()).unwrap()), "Float");
    assert_eq!(g(dec(), dec()).unwrap().to_display_string(), "1.0"); // 0.5 + 0.5
    assert_eq!(kind(&g(dec(), dec()).unwrap()), "Decimal");
    assert_eq!(g(dec(), cpx()).unwrap().to_display_string(), "5/2+3i"); // 0.5 + (2+3i)
    assert_eq!(kind(&g(dec(), cpx()).unwrap()), "Complex");
    assert_eq!(kind(&g(dec(), flt()).unwrap()), "Float");
    assert_eq!(g(cpx(), cpx()).unwrap().to_display_string(), "4+6i");
    assert_eq!(kind(&g(flt(), flt()).unwrap()), "Float");
    // The Complex/Float cell: an exact Complex meeting an inexact Float PROMOTES to a
    // floating-point ComplexF (both orders), value-exact for these representable operands.
    assert_eq!(g(cpx(), flt()).unwrap().to_display_string(), "4+3i"); // (2+3i) + 2.0
    assert_eq!(kind(&g(cpx(), flt()).unwrap()), "ComplexF");
    assert_eq!(g(flt(), cpx()).unwrap().to_display_string(), "4+3i"); // 2.0 + (2+3i)
    assert_eq!(kind(&g(flt(), cpx()).unwrap()), "ComplexF");

    // ---- Commutativity of the PROMOTION across the matrix (a∘b kind == b∘a kind) ----
    for a in [int(), rat(), dec(), cpx(), flt()] {
        for b in [int(), rat(), dec(), cpx(), flt()] {
            match (add(a.clone(), b.clone()), add(b.clone(), a.clone())) {
                (Ok(ab), Ok(ba)) => assert_eq!(
                    kind(&ab),
                    kind(&ba),
                    "promotion commutes: {} {}",
                    kind(&a),
                    kind(&b)
                ),
                (Err(_), Err(_)) => {} // both refuse (no such cell remains in the numeric tower)
                other => panic!(
                    "promotion asymmetry for {} and {}: {other:?}",
                    kind(&a),
                    kind(&b)
                ),
            }
        }
    }

    // ---- DIVIDE: Decimal widens to Rational; Complex stays Complex ----
    assert_eq!(divide(dec(), int()).unwrap().to_display_string(), "1/4"); // 0.5 / 2
    assert_eq!(kind(&divide(dec(), int()).unwrap()), "Rational");
    assert_eq!(divide(dec(), dec()).unwrap().to_display_string(), "1"); // 0.5 / 0.5 → 1 (Int)
    assert_eq!(divide(cpx(), int()).unwrap().to_display_string(), "1+3/2i"); // (2+3i)/2
    assert_eq!(kind(&divide(cpx(), int()).unwrap()), "Complex");
    assert_eq!(divide(cpx(), cpx()).unwrap().to_display_string(), "1"); // z/z
    assert_eq!(divide(rat(), int()).unwrap().to_display_string(), "1/6"); // (1/3)/2
                                                                          // Division by zero is a clean error across the exact types.
    assert!(divide(dec(), RuntimeValue::Int(0)).is_err());
    assert!(divide(cpx(), RuntimeValue::from_rational(Rational::zero())).is_err());
}

#[test]
fn modular_arithmetic_wraps_and_requires_a_shared_ring() {
    let m = |v: i64, n: i64| RuntimeValue::Modular(Rc::new(Modular::from_i64(v, n).unwrap()));
    // Add/sub/mul wrap in ℤ/nℤ.
    assert_eq!(add(m(5, 7), m(4, 7)).unwrap(), m(2, 7)); // 9 ≡ 2
    assert_eq!(subtract(m(3, 7), m(5, 7)).unwrap(), m(5, 7)); // −2 ≡ 5
    assert_eq!(multiply(m(4, 7), m(5, 7)).unwrap(), m(6, 7)); // 20 ≡ 6
                                                              // Division is by the modular inverse: 1/3 ≡ 5 (mod 7).
    assert_eq!(divide(m(1, 7), m(3, 7)).unwrap(), m(5, 7));
    // A non-invertible divisor (gcd ≠ 1) is a clean error, not a panic.
    assert!(divide(m(1, 4), m(2, 4)).is_err());
    // A modulus mismatch is refused on every op (no silent cross-ring math).
    assert!(add(m(3, 7), m(3, 5)).is_err());
    assert!(multiply(m(3, 7), m(3, 5)).is_err());
    assert!(divide(m(3, 7), m(3, 5)).is_err());
    // Mixing a Modular with a bare Int is refused (an Int has no modulus).
    assert!(add(m(3, 7), RuntimeValue::Int(5)).is_err());
    // Equality is per-ring; ℤ/nℤ is unordered so comparison is a typed error.
    assert!(matches!(
        binary_op(BinaryOpKind::Eq, m(3, 7), m(10, 7)).unwrap(),
        RuntimeValue::Bool(true)
    ));
    assert!(matches!(
        binary_op(BinaryOpKind::Eq, m(3, 7), m(3, 5)).unwrap(),
        RuntimeValue::Bool(false)
    ));
    assert!(compare(BinaryOpKind::Lt, &m(1, 7), &m(2, 7)).is_err());
    assert_eq!(m(3, 7).to_display_string(), "3 (mod 7)");
}

#[test]
fn int_arithmetic_is_exact_in_every_build_profile() {
    // The LOGOS Int spec: EXACT (promoting) i64 — identical in debug AND release.
    let r = add(RuntimeValue::Int(i64::MAX), RuntimeValue::Int(1)).unwrap();
    assert_eq!(r.to_display_string(), "9223372036854775808"); // not wrapped i64::MIN
    let r = subtract(RuntimeValue::Int(i64::MIN), RuntimeValue::Int(1)).unwrap();
    assert_eq!(r.to_display_string(), "-9223372036854775809");
    let r = multiply(RuntimeValue::Int(i64::MAX), RuntimeValue::Int(2)).unwrap();
    assert_eq!(r.to_display_string(), "18446744073709551614");
    // MIN / -1 = 2^63 (the division-overflow edge) promotes exactly; MIN % -1 = 0.
    let r = divide(RuntimeValue::Int(i64::MIN), RuntimeValue::Int(-1)).unwrap();
    assert_eq!(r.to_display_string(), "9223372036854775808");
    let r = modulo(RuntimeValue::Int(i64::MIN), RuntimeValue::Int(-1)).unwrap();
    assert!(matches!(r, RuntimeValue::Int(0)));
    // Duration is a temporal quantity, NOT part of the integer tower — it still
    // wraps (its arithmetic is modular by construction).
    let r = add(RuntimeValue::Duration(i64::MAX), RuntimeValue::Duration(1)).unwrap();
    assert!(matches!(r, RuntimeValue::Duration(i64::MIN)));
    let r = subtract(RuntimeValue::Duration(i64::MIN), RuntimeValue::Duration(1)).unwrap();
    assert!(matches!(r, RuntimeValue::Duration(i64::MAX)));
}

#[test]
fn shifts_mask_their_count_modulo_64() {
    // wrapping_shl/shr(b as u32): the count is truncated to u32 then masked
    // mod 64, so `1 << 64 == 1` and a negative count becomes (b as u32) & 63.
    let r = binary_op(
        BinaryOpKind::Shl,
        RuntimeValue::Int(1),
        RuntimeValue::Int(64),
    )
    .unwrap();
    assert!(matches!(r, RuntimeValue::Int(1)));
    let r = binary_op(
        BinaryOpKind::Shl,
        RuntimeValue::Int(1),
        RuntimeValue::Int(63),
    )
    .unwrap();
    assert!(matches!(r, RuntimeValue::Int(i64::MIN)));
    // -1 as u32 == u32::MAX; masked mod 64 → 63.
    let r = binary_op(
        BinaryOpKind::Shl,
        RuntimeValue::Int(1),
        RuntimeValue::Int(-1),
    )
    .unwrap();
    assert!(matches!(r, RuntimeValue::Int(i64::MIN)));
    let r = binary_op(
        BinaryOpKind::Shr,
        RuntimeValue::Int(i64::MIN),
        RuntimeValue::Int(63),
    )
    .unwrap();
    assert!(matches!(r, RuntimeValue::Int(-1)));
    let r = binary_op(
        BinaryOpKind::Shr,
        RuntimeValue::Int(8),
        RuntimeValue::Int(64),
    )
    .unwrap();
    assert!(matches!(r, RuntimeValue::Int(8)));
}

#[test]
fn crdt_counter_bump_wraps() {
    let r = crdt_counter_bump(RuntimeValue::Int(i64::MAX), 1, "n").unwrap();
    assert!(matches!(r, RuntimeValue::Int(i64::MIN)));
    let r = crdt_counter_bump(RuntimeValue::Nothing, 5, "n").unwrap();
    assert!(matches!(r, RuntimeValue::Int(5)));
    let e = crdt_counter_bump(RuntimeValue::Bool(true), 1, "score").unwrap_err();
    assert_eq!(e, "Field 'score' is not a counter");
}

#[test]
fn eager_and_or_are_logical_truthiness_to_bool() {
    // `and`/`or` are LOGICAL: truthiness in, Bool out — `&`/`|` are the
    // bitwise spellings (BitAnd/BitOr below).
    let r = binary_op(
        BinaryOpKind::And,
        RuntimeValue::Int(6),
        RuntimeValue::Int(3),
    )
    .unwrap();
    assert!(matches!(r, RuntimeValue::Bool(true)));
    let r = binary_op(BinaryOpKind::Or, RuntimeValue::Int(0), RuntimeValue::Int(7)).unwrap();
    assert!(matches!(r, RuntimeValue::Bool(true)));
    let r = binary_op(
        BinaryOpKind::And,
        RuntimeValue::Int(1),
        RuntimeValue::Bool(false),
    )
    .unwrap();
    assert!(matches!(r, RuntimeValue::Bool(false)));
    let r = binary_op(
        BinaryOpKind::Or,
        RuntimeValue::Bool(false),
        RuntimeValue::Bool(true),
    )
    .unwrap();
    assert!(matches!(r, RuntimeValue::Bool(true)));
    let r = binary_op(
        BinaryOpKind::BitAnd,
        RuntimeValue::Int(6),
        RuntimeValue::Int(3),
    )
    .unwrap();
    assert!(matches!(r, RuntimeValue::Int(2)));
    let r = binary_op(
        BinaryOpKind::BitOr,
        RuntimeValue::Int(6),
        RuntimeValue::Int(3),
    )
    .unwrap();
    assert!(matches!(r, RuntimeValue::Int(7)));
}
