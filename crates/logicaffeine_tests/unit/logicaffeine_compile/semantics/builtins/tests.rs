use super::*;

/// Every temporal extractor, on a Moment AND a Date, plus every error path — the interpreter
/// dispatch shared by the tree-walker and the bytecode VM. If any accessor silently returns the
/// wrong number or fails to reject a bad input, this fails.
#[test]
fn temporal_extractors_cover_every_component_and_reject_bad_inputs() {
    use logicaffeine_base::temporal;
    let call = |id, v: &RuntimeValue| call_builtin(id, vec![v.clone()]);
    let int = |id, v: &RuntimeValue| match call(id, v) {
        Ok(RuntimeValue::Int(n)) => n,
        other => panic!("{id:?} on {:?} -> {other:?}", v.type_name()),
    };

    // 2024-03-10T07:30:45Z — a Sunday (ISO weekday 7), ISO week 10, Q1.
    let m = RuntimeValue::Moment(temporal::parse_rfc3339("2024-03-10T07:30:45Z").unwrap());
    assert_eq!(int(BuiltinId::YearOf, &m), 2024);
    assert_eq!(int(BuiltinId::MonthOf, &m), 3);
    assert_eq!(int(BuiltinId::DayOf, &m), 10);
    assert_eq!(int(BuiltinId::WeekdayOf, &m), 7);
    assert_eq!(int(BuiltinId::HourOf, &m), 7);
    assert_eq!(int(BuiltinId::MinuteOf, &m), 30);
    assert_eq!(int(BuiltinId::SecondOf, &m), 45);
    assert_eq!(int(BuiltinId::WeekOf, &m), 10);
    assert_eq!(int(BuiltinId::QuarterOf, &m), 1);
    // date_of / time_of project onto the right runtime types.
    assert!(matches!(
        call(BuiltinId::DateOf, &m),
        Ok(RuntimeValue::Date(_))
    ));
    assert!(matches!(
        call(BuiltinId::TimeOf, &m),
        Ok(RuntimeValue::Time(_))
    ));

    // The calendar accessors work identically on a bare Date...
    let days = (temporal::parse_rfc3339("2024-03-10T07:30:45Z").unwrap())
        .div_euclid(temporal::NANOS_PER_DAY) as i32;
    let d = RuntimeValue::Date(days);
    assert_eq!(int(BuiltinId::YearOf, &d), 2024);
    assert_eq!(int(BuiltinId::MonthOf, &d), 3);
    assert_eq!(int(BuiltinId::DayOf, &d), 10);
    assert_eq!(int(BuiltinId::WeekdayOf, &d), 7);
    assert_eq!(int(BuiltinId::WeekOf, &d), 10);
    assert_eq!(int(BuiltinId::QuarterOf, &d), 1);
    assert!(matches!(call(BuiltinId::DateOf, &d), Ok(RuntimeValue::Date(x)) if x == days));

    // ...but a Date has NO time-of-day: clock accessors must ERROR, not silently return 0.
    for id in [
        BuiltinId::HourOf,
        BuiltinId::MinuteOf,
        BuiltinId::SecondOf,
        BuiltinId::TimeOf,
    ] {
        assert!(
            call(id, &d).unwrap_err().contains("no time-of-day"),
            "{id:?} should reject a Date"
        );
    }

    // Non-temporal inputs are rejected with a typed error — never a bogus number.
    let bogus = RuntimeValue::Int(5);
    for id in [
        BuiltinId::YearOf,
        BuiltinId::WeekOf,
        BuiltinId::QuarterOf,
        BuiltinId::DateOf,
        BuiltinId::TimeOf,
    ] {
        assert!(
            call(id, &bogus).is_err(),
            "{id:?} should reject a non-temporal value"
        );
    }

    // seconds_between requires two Moments, in either position.
    assert!(call_builtin(BuiltinId::SecondsBetween, vec![m.clone(), bogus.clone()]).is_err());
    assert!(call_builtin(BuiltinId::SecondsBetween, vec![bogus.clone(), m.clone()]).is_err());

    // Pre-epoch (negative) Moments decompose by FLOOR division — no negative hour/second.
    let pre = RuntimeValue::Moment(temporal::parse_rfc3339("1969-12-31T23:59:58Z").unwrap());
    assert_eq!(int(BuiltinId::YearOf, &pre), 1969);
    assert_eq!(int(BuiltinId::MonthOf, &pre), 12);
    assert_eq!(int(BuiltinId::DayOf, &pre), 31);
    assert_eq!(int(BuiltinId::HourOf, &pre), 23);
    assert_eq!(int(BuiltinId::MinuteOf, &pre), 59);
    assert_eq!(int(BuiltinId::SecondOf, &pre), 58);
}

/// `date_of` is the inverse-projection partner of the component accessors: extracting the date
/// then re-reading its components must agree with reading them off the Moment directly.
#[test]
fn date_of_then_components_agree_with_the_moment() {
    use logicaffeine_base::temporal;
    for ts in [
        "2024-03-10T07:30:45Z",
        "2000-02-29T00:00:00Z",
        "1969-12-31T23:59:58Z",
    ] {
        let m = RuntimeValue::Moment(temporal::parse_rfc3339(ts).unwrap());
        let d = call_builtin(BuiltinId::DateOf, vec![m.clone()]).unwrap();
        for id in [
            BuiltinId::YearOf,
            BuiltinId::MonthOf,
            BuiltinId::DayOf,
            BuiltinId::WeekdayOf,
            BuiltinId::WeekOf,
            BuiltinId::QuarterOf,
        ] {
            let from_moment = call_builtin(id, vec![m.clone()]).unwrap();
            let from_date = call_builtin(id, vec![d.clone()]).unwrap();
            assert_eq!(from_moment, from_date, "{id:?} disagrees for {ts}");
        }
    }
}

#[test]
fn run_accepted_validates_then_runs_a_shipped_computation() {
    use crate::concurrency::marshal::GenExpr;
    use crate::interpreter::ClosureValue;
    use std::collections::HashMap;
    use std::rc::Rc;
    // A shipped `3·x + 1` computation (what a peer ships via `Send computed`).
    let mk = || {
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
    };
    // In-bounds (5 ∈ [0,1000]) → 3·5 + 1 = 16, run in the sandbox.
    match call_builtin(
        BuiltinId::RunAccepted,
        vec![
            mk(),
            RuntimeValue::Int(5),
            RuntimeValue::Int(0),
            RuntimeValue::Int(1000),
        ],
    ) {
        Ok(RuntimeValue::Int(n)) => assert_eq!(n, 16),
        other => panic!("in-bounds run_accepted should return Int(16), got {other:?}"),
    }
    // Out-of-bounds (5000 ∉ [0,1000]) → refused, not clamped.
    assert!(
        call_builtin(
            BuiltinId::RunAccepted,
            vec![
                mk(),
                RuntimeValue::Int(5000),
                RuntimeValue::Int(0),
                RuntimeValue::Int(1000)
            ],
        )
        .is_err(),
        "an out-of-range argument must be refused at the contract"
    );
}

#[test]
fn arity_messages_match_treewalker() {
    assert_eq!(
        check_arity(BuiltinId::Length, 2).unwrap_err(),
        "length() takes exactly 1 argument"
    );
    assert_eq!(
        check_arity(BuiltinId::Min, 1).unwrap_err(),
        "min() takes exactly 2 arguments"
    );
    assert!(check_arity(BuiltinId::Format, 0).is_ok());
    assert!(check_arity(BuiltinId::Format, 5).is_ok());
}

#[test]
fn parse_and_chr_messages() {
    let e = call_builtin(
        BuiltinId::ParseInt,
        vec![RuntimeValue::Text(Rc::new("zz".to_string()))],
    )
    .unwrap_err();
    assert_eq!(e, "Cannot parse 'zz' as Int");
    let e = call_builtin(BuiltinId::Chr, vec![RuntimeValue::Int(-1)]).unwrap_err();
    assert_eq!(e, "Invalid character code: -1");
    let r = call_builtin(BuiltinId::Chr, vec![RuntimeValue::Int(65)]).unwrap();
    assert!(matches!(&r, RuntimeValue::Text(s) if **s == "A"));
}

#[test]
fn numeric_builtins_coerce_like_treewalker() {
    assert!(matches!(
        call_builtin(BuiltinId::Sqrt, vec![RuntimeValue::Int(9)]).unwrap(),
        RuntimeValue::Float(f) if f == 3.0
    ));
    assert!(matches!(
        call_builtin(BuiltinId::Min, vec![RuntimeValue::Int(3), RuntimeValue::Float(2.5)]).unwrap(),
        RuntimeValue::Float(f) if f == 2.5
    ));
    assert!(matches!(
        call_builtin(BuiltinId::Floor, vec![RuntimeValue::Float(2.9)]).unwrap(),
        RuntimeValue::Int(2)
    ));
    assert!(matches!(
        call_builtin(
            BuiltinId::Pow,
            vec![RuntimeValue::Int(2), RuntimeValue::Int(10)]
        )
        .unwrap(),
        RuntimeValue::Int(1024)
    ));
    // Negative Int exponent goes to float.
    assert!(matches!(
        call_builtin(BuiltinId::Pow, vec![RuntimeValue::Int(2), RuntimeValue::Int(-1)]).unwrap(),
        RuntimeValue::Float(f) if f == 0.5
    ));
}

#[test]
fn decimal_builtin_constructs_exact_money() {
    // decimal("19.99") is an exact, scale-preserving Decimal — the non-breaking entry
    // into the money tower.
    let d = call_builtin(
        BuiltinId::Decimal,
        vec![RuntimeValue::Text(Rc::new("19.99".into()))],
    )
    .unwrap();
    assert!(matches!(&d, RuntimeValue::Decimal(_)));
    assert_eq!(d.to_display_string(), "19.99");
    // An Int widens to a scale-0 Decimal.
    let i = call_builtin(BuiltinId::Decimal, vec![RuntimeValue::Int(5)]).unwrap();
    assert!(matches!(&i, RuntimeValue::Decimal(_)));
    assert_eq!(i.to_display_string(), "5");
    // Garbage text is a clean error, never a panic.
    let e = call_builtin(
        BuiltinId::Decimal,
        vec![RuntimeValue::Text(Rc::new("zz".into()))],
    )
    .unwrap_err();
    assert_eq!(e, "Cannot parse 'zz' as Decimal");
    // Arity is exactly one.
    assert_eq!(
        check_arity(BuiltinId::Decimal, 2).unwrap_err(),
        "decimal() takes exactly 1 argument"
    );
}

#[test]
fn complex_builtin_constructs_exact_complex_numbers() {
    // complex(0, 1) is the imaginary unit i.
    let i = call_builtin(
        BuiltinId::Complex,
        vec![RuntimeValue::Int(0), RuntimeValue::Int(1)],
    )
    .unwrap();
    assert!(matches!(&i, RuntimeValue::Complex(_)));
    assert_eq!(i.to_display_string(), "i");
    // complex(3, 4) → 3+4i; complex(0, -1) → -i.
    let z = call_builtin(
        BuiltinId::Complex,
        vec![RuntimeValue::Int(3), RuntimeValue::Int(4)],
    )
    .unwrap();
    assert_eq!(z.to_display_string(), "3+4i");
    let neg_i = call_builtin(
        BuiltinId::Complex,
        vec![RuntimeValue::Int(0), RuntimeValue::Int(-1)],
    )
    .unwrap();
    assert_eq!(neg_i.to_display_string(), "-i");
    let _ = i; // the imaginary unit; its i·i = −1 is covered in the arith tests.
               // A Float in EITHER part PROMOTES to a floating-point complex (ComplexF) — the same
               // `Rational` meets `Float` promotion, carried onto the complex plane. complex(1.0, 1) = 1+i.
    let cf = call_builtin(
        BuiltinId::Complex,
        vec![RuntimeValue::Float(1.0), RuntimeValue::Int(1)],
    )
    .unwrap();
    assert!(matches!(&cf, RuntimeValue::ComplexF(_)));
    assert_eq!(cf.to_display_string(), "1+i");
    // Arity is exactly two.
    assert_eq!(
        check_arity(BuiltinId::Complex, 1).unwrap_err(),
        "complex() takes exactly 2 arguments"
    );
}

#[test]
fn decimal_and_complex_pass_through_the_numeric_builtins() {
    let d = |s: &str| RuntimeValue::Decimal(Rc::new(Decimal::parse(s).unwrap()));
    // abs preserves Decimal + scale; floor/ceil/round give the exact Int; min/max value-based.
    assert_eq!(
        call_builtin(BuiltinId::Abs, vec![d("-0.05")])
            .unwrap()
            .to_display_string(),
        "0.05"
    );
    assert_eq!(
        call_builtin(BuiltinId::Floor, vec![d("19.99")])
            .unwrap()
            .to_display_string(),
        "19"
    );
    assert_eq!(
        call_builtin(BuiltinId::Ceil, vec![d("19.01")])
            .unwrap()
            .to_display_string(),
        "20"
    );
    assert_eq!(
        call_builtin(BuiltinId::Round, vec![d("2.5")])
            .unwrap()
            .to_display_string(),
        "3"
    );
    assert_eq!(
        call_builtin(BuiltinId::Round, vec![d("-19.99")])
            .unwrap()
            .to_display_string(),
        "-20"
    );
    assert_eq!(
        call_builtin(BuiltinId::Min, vec![d("0.10"), d("0.2")])
            .unwrap()
            .to_display_string(),
        "0.10"
    );
    assert_eq!(
        call_builtin(BuiltinId::Max, vec![d("0.10"), d("0.2")])
            .unwrap()
            .to_display_string(),
        "0.2"
    );
    // |3+4i| = 5 (the modulus, a Float view of a generally-irrational magnitude).
    let z = RuntimeValue::Complex(Rc::new(logicaffeine_base::Complex::new(
        logicaffeine_base::Rational::from_i64(3),
        logicaffeine_base::Rational::from_i64(4),
    )));
    match call_builtin(BuiltinId::Abs, vec![z]).unwrap() {
        RuntimeValue::Float(f) => assert!((f - 5.0).abs() < 1e-12),
        other => panic!("expected a Float magnitude, got {other:?}"),
    }
}

#[test]
fn modular_builtin_constructs_and_exponentiates_in_the_ring() {
    // modular(10, 7) reduces to 3 (mod 7).
    let x = call_builtin(
        BuiltinId::Modular,
        vec![RuntimeValue::Int(10), RuntimeValue::Int(7)],
    )
    .unwrap();
    assert!(matches!(&x, RuntimeValue::Modular(_)));
    assert_eq!(x.to_display_string(), "3 (mod 7)");
    // pow(modular(3,5), 4) = 81 ≡ 1 (mod 5) — fast modular exponentiation via the builtin.
    let base = call_builtin(
        BuiltinId::Modular,
        vec![RuntimeValue::Int(3), RuntimeValue::Int(5)],
    )
    .unwrap();
    let p = call_builtin(BuiltinId::Pow, vec![base, RuntimeValue::Int(4)]).unwrap();
    assert_eq!(p.to_display_string(), "1 (mod 5)");
    // A non-positive modulus is refused; arity is exactly two.
    assert!(call_builtin(
        BuiltinId::Modular,
        vec![RuntimeValue::Int(3), RuntimeValue::Int(0)]
    )
    .is_err());
    assert_eq!(
        check_arity(BuiltinId::Modular, 1).unwrap_err(),
        "modular() takes exactly 2 arguments"
    );
}

#[test]
fn quantity_builtins_construct_convert_and_compute_exactly() {
    use crate::semantics::arith::{add, divide, multiply, subtract};
    let q = |v: i64, u: &str| {
        call_builtin(
            BuiltinId::Quantity,
            vec![RuntimeValue::Int(v), RuntimeValue::Text(Rc::new(u.into()))],
        )
        .unwrap()
    };
    let conv = |x: RuntimeValue, u: &str| {
        call_builtin(
            BuiltinId::Convert,
            vec![x, RuntimeValue::Text(Rc::new(u.into()))],
        )
    };
    // Construction carries the display unit.
    assert!(matches!(&q(2, "inch"), RuntimeValue::Quantity(_)));
    assert_eq!(q(2, "inch").to_display_string(), "2 in");
    assert_eq!(q(20, "celsius").to_display_string(), "20 °C");
    // THE GOLDEN: 2 inches + 5 centimeters, in feet, is EXACTLY 42/127 ft — no float.
    let sum = add(q(2, "inch"), q(5, "centimeter")).unwrap();
    assert_eq!(conv(sum, "foot").unwrap().to_display_string(), "42/127 ft");
    // Same-dimension subtraction keeps the left operand's unit; the magnitude is exact.
    assert_eq!(
        subtract(q(1, "meter"), q(50, "centimeter"))
            .unwrap()
            .to_display_string(),
        "1/2 m"
    );
    // Scaling by a dimensionless number preserves the unit (× and ÷).
    assert_eq!(
        multiply(q(2, "inch"), RuntimeValue::Int(3))
            .unwrap()
            .to_display_string(),
        "6 in"
    );
    assert_eq!(
        multiply(RuntimeValue::Int(3), q(2, "inch"))
            .unwrap()
            .to_display_string(),
        "6 in"
    );
    assert_eq!(
        divide(q(6, "inch"), RuntimeValue::Int(2))
            .unwrap()
            .to_display_string(),
        "3 in"
    );
    // Length × Length = Area, shown in dimension form until a named compound unit exists.
    assert_eq!(
        multiply(q(3, "meter"), q(4, "meter"))
            .unwrap()
            .to_display_string(),
        "12 L^2"
    );
    // Length ÷ Time = Speed.
    assert_eq!(
        divide(q(100, "meter"), q(10, "second"))
            .unwrap()
            .to_display_string(),
        "10 L·T^-1"
    );
    // Quantity equality is PHYSICAL: 100 cm == 1 m (display unit is presentation only).
    assert_eq!(q(100, "centimeter"), q(1, "meter"));
    // Dimension mismatch on + is a clean typed error (Length + Mass), never a silent coercion.
    assert!(add(q(1, "meter"), q(1, "kilogram")).is_err());
    // Converting across dimensions is the forbidden cast.
    assert!(conv(q(1, "meter"), "kilogram").is_err());
    // Unknown unit names and Float magnitudes are clean errors (exactness preserved).
    assert!(call_builtin(
        BuiltinId::Quantity,
        vec![
            RuntimeValue::Int(1),
            RuntimeValue::Text(Rc::new("zorgle".into()))
        ]
    )
    .is_err());
    assert!(call_builtin(
        BuiltinId::Quantity,
        vec![
            RuntimeValue::Float(1.5),
            RuntimeValue::Text(Rc::new("meter".into()))
        ]
    )
    .is_err());
    // Arity is exactly two for both builtins.
    assert_eq!(
        check_arity(BuiltinId::Quantity, 1).unwrap_err(),
        "quantity() takes exactly 2 arguments"
    );
    assert_eq!(
        check_arity(BuiltinId::Convert, 3).unwrap_err(),
        "convert() takes exactly 2 arguments"
    );
}

#[test]
fn abs_and_pow_are_exact_promoting_past_i64() {
    // Arrange / Act / Assert: |i64::MIN| = 2^63 has no i64 representation, so abs
    // promotes to the EXACT BigInt rather than wrapping back to i64::MIN.
    let abs_min = call_builtin(BuiltinId::Abs, vec![RuntimeValue::Int(i64::MIN)]).unwrap();
    assert_eq!(abs_min.to_display_string(), "9223372036854775808");
    assert_eq!(
        abs_min.type_name(),
        "Int",
        "a BigInt is still an integer type"
    );

    // 2^63 overflows i64 → exact BigInt (not the wrapped i64::MIN).
    let two_pow_63 = call_builtin(
        BuiltinId::Pow,
        vec![RuntimeValue::Int(2), RuntimeValue::Int(63)],
    )
    .unwrap();
    assert_eq!(two_pow_63.to_display_string(), "9223372036854775808");

    // A far-larger power stays exact (2^100 = 31 digits).
    let two_pow_100 = call_builtin(
        BuiltinId::Pow,
        vec![RuntimeValue::Int(2), RuntimeValue::Int(100)],
    )
    .unwrap();
    assert_eq!(
        two_pow_100.to_display_string(),
        "1267650600228229401496703205376"
    );

    // In-range results stay narrow `Int` (downsizing is automatic).
    assert!(matches!(
        call_builtin(
            BuiltinId::Pow,
            vec![RuntimeValue::Int(3), RuntimeValue::Int(4)]
        )
        .unwrap(),
        RuntimeValue::Int(81)
    ));
    // abs of an ordinary negative is unchanged.
    assert!(matches!(
        call_builtin(BuiltinId::Abs, vec![RuntimeValue::Int(-5)]).unwrap(),
        RuntimeValue::Int(5)
    ));
}

#[test]
fn copy_is_deep() {
    use std::cell::RefCell;
    let inner = std::rc::Rc::new(RefCell::new(crate::interpreter::ListRepr::from_values(
        vec![RuntimeValue::Int(1)],
    )));
    let original = RuntimeValue::List(inner.clone());
    let copied = call_builtin(BuiltinId::Copy, vec![original]).unwrap();
    if let RuntimeValue::List(copied_items) = &copied {
        inner.borrow_mut().push(RuntimeValue::Int(2));
        assert_eq!(
            copied_items.borrow().len(),
            1,
            "copy must not share the allocation"
        );
    } else {
        panic!("copy changed the type");
    }
}

/// Every builtin of the exact extension is called by the name base gives it, and that name
/// resolves to the builtin the interpreter hands to `logicaffeine_base::extension::call`.
#[test]
fn every_extension_builtin_resolves_by_its_name_to_itself() {
    for b in logicaffeine_base::extension::Builtin::ALL {
        let id = builtin_from_name(b.name())
            .unwrap_or_else(|| panic!("`{}` is not a builtin name", b.name()));
        assert_eq!(id.extension(), Some(b), "`{}`", b.name());
    }
}
