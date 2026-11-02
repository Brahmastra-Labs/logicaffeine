//! Coverage for the parts of `term.rs` the universe-algebra battery never reaches: the `Literal`
//! and `Term` `Display` renderers, the `days_to_ymd` civil-date conversion, and the `bigint_dec`
//! serde. Each is pinned to EXACT output, so a flipped comparison or arithmetic op renders wrong.
use super::*;
use logicaffeine_base::BigInt;

fn g(s: &str) -> Term {
    Term::Global(s.to_string())
}
fn v(s: &str) -> Term {
    Term::Var(s.to_string())
}
fn arrow(a: Term, b: Term) -> Term {
    Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}

/// Howard Hinnant `days_from_civil` — the FORWARD direction, written independently of
/// `days_to_ymd`, so the two compose to the identity iff `days_to_ymd` is correct.
fn ymd_to_days(y: i64, m: u8, d: u8) -> i64 {
    let (m, d) = (m as i64, d as i64);
    let y = if m <= 2 { y - 1 } else { y };
    let era = (if y >= 0 { y } else { y - 399 }) / 400;
    let yoe = y - era * 400; // [0, 399]
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

#[test]
fn days_to_ymd_hits_anchors_and_is_the_exact_inverse_over_two_centuries() {
    // Hand-checked anchors: epoch, month/year boundaries, non-leap + leap February, the
    // 2000 leap-century, and pre-epoch negatives. These validate the oracle in BOTH directions.
    let anchors: &[(i64, (i64, u8, u8))] = &[
        (0, (1970, 1, 1)),
        (31, (1970, 2, 1)),
        (58, (1970, 2, 28)),
        (59, (1970, 3, 1)),
        (364, (1970, 12, 31)),
        (365, (1971, 1, 1)),
        (730, (1972, 1, 1)),
        (789, (1972, 2, 29)),
        (790, (1972, 3, 1)),
        (1096, (1973, 1, 1)),
        (-1, (1969, 12, 31)),
        (-365, (1969, 1, 1)),
        (10957, (2000, 1, 1)),
        (11016, (2000, 2, 29)),
        (11017, (2000, 3, 1)),
    ];
    for &(days, ymd) in anchors {
        assert_eq!(days_to_ymd(days), ymd, "days_to_ymd({days})");
        let (y, m, d) = ymd;
        assert_eq!(
            ymd_to_days(y, m, d),
            days,
            "oracle self-check ymd_to_days({y}-{m}-{d})"
        );
    }
    // Exhaustive inverse over ~110 years each side of the epoch (the common range).
    for days in -40000i64..40000 {
        let (y, m, d) = days_to_ymd(days);
        assert_eq!(
            ymd_to_days(y, m, d),
            days,
            "round-trip broke at day {days} → {y}-{m:02}-{d:02}"
        );
    }
    // Wide sample spanning many 400-year eras and BOTH signs of the era pivot `z = days + 719468`:
    // days below −719468 drive the `z < 0` branch (ancient, pre-~337 AD dates), and the ±2.5M-day
    // spread exercises the era arithmetic across era VALUES, not just the epoch's own era.
    for days in (-2_500_000i64..2_500_000).step_by(97) {
        let (y, m, d) = days_to_ymd(days);
        assert_eq!(
            ymd_to_days(y, m, d),
            days,
            "round-trip broke at day {days} → {y}-{m:02}-{d:02}"
        );
    }
}

#[test]
fn literal_display_renders_every_variant_exactly() {
    assert_eq!(Literal::Int(42).to_string(), "42");
    assert_eq!(Literal::Int(-7).to_string(), "-7");
    assert_eq!(Literal::Float(3.5).to_string(), "3.5");
    assert_eq!(Literal::Text("hi".to_string()).to_string(), "\"hi\"");
    assert_eq!(
        Literal::BigInt(BigInt::from_u64(u64::MAX)).to_string(),
        u64::MAX.to_string()
    );
    assert_eq!(Literal::Nat(BigInt::from_u64(5)).to_string(), "5");
    // Duration: zero (pins the sign `< 0` vs `<= 0`), each unit's EXACT boundary (pins `>=`
    // vs `>`), a just-below value, multi-unit quotients, and negatives (the sign).
    assert_eq!(Literal::Duration(0).to_string(), "0ns");
    assert_eq!(Literal::Duration(500).to_string(), "500ns");
    assert_eq!(Literal::Duration(999).to_string(), "999ns");
    assert_eq!(Literal::Duration(1_000).to_string(), "1μs"); // exact μs boundary
    assert_eq!(Literal::Duration(1_500).to_string(), "1μs");
    assert_eq!(Literal::Duration(1_000_000).to_string(), "1ms"); // exact ms boundary
    assert_eq!(Literal::Duration(2_000_000).to_string(), "2ms");
    assert_eq!(Literal::Duration(1_000_000_000).to_string(), "1s"); // exact s boundary
    assert_eq!(Literal::Duration(3_000_000_000).to_string(), "3s");
    assert_eq!(Literal::Duration(60_000_000_000).to_string(), "1min"); // exact min boundary
    assert_eq!(Literal::Duration(120_000_000_000).to_string(), "2min");
    assert_eq!(Literal::Duration(3_600_000_000_000).to_string(), "1h"); // exact h boundary
    assert_eq!(Literal::Duration(7_200_000_000_000).to_string(), "2h");
    assert_eq!(Literal::Duration(-1_000_000_000).to_string(), "-1s");
    assert_eq!(Literal::Duration(-500).to_string(), "-500ns");
    // Date + Moment: ISO-8601, driving days_to_ymd (incl. a leap day) and the time arithmetic.
    assert_eq!(Literal::Date(0).to_string(), "1970-01-01");
    assert_eq!(Literal::Date(789).to_string(), "1972-02-29");
    assert_eq!(Literal::Moment(0).to_string(), "1970-01-01T00:00:00Z");
    assert_eq!(
        Literal::Moment(7_384_000_000_000).to_string(),
        "1970-01-01T02:03:04Z"
    );
    // A moment past a day boundary exercises secs/86400 (days) and secs%86400 (time-of-day).
    assert_eq!(
        Literal::Moment(90_000_000_000_000).to_string(),
        "1970-01-02T01:00:00Z"
    );
}

#[test]
fn term_display_renders_every_constructor_exactly() {
    assert_eq!(v("x").to_string(), "x");
    assert_eq!(g("Nat").to_string(), "Nat");
    // Pi: arrow form for `_`, dependent form otherwise.
    assert_eq!(arrow(g("A"), g("B")).to_string(), "A -> B");
    assert_eq!(
        Term::Pi {
            param: "x".to_string(),
            param_type: Box::new(g("A")),
            body_type: Box::new(g("B"))
        }
        .to_string(),
        "Π(x:A). B",
    );
    assert_eq!(
        Term::Lambda {
            param: "x".to_string(),
            param_type: Box::new(g("A")),
            body: Box::new(v("x"))
        }
        .to_string(),
        "λ(x:A). x",
    );
    // App: plain, and with an arrow argument that needs inner parens.
    assert_eq!(
        Term::App(Box::new(g("f")), Box::new(g("a"))).to_string(),
        "(f a)"
    );
    assert_eq!(
        Term::App(Box::new(g("f")), Box::new(arrow(g("A"), g("B")))).to_string(),
        "(f (A -> B))",
    );
    assert_eq!(
        Term::Fix {
            name: "r".to_string(),
            body: Box::new(g("b"))
        }
        .to_string(),
        "fix r. b"
    );
    assert_eq!(
        Term::Let {
            name: "x".to_string(),
            ty: Box::new(g("A")),
            value: Box::new(g("v")),
            body: Box::new(g("b")),
        }
        .to_string(),
        "let x:A := v in b",
    );
    // Match / MutualFix: the `i > 0` separators need ≥2 cases / defs to be exercised.
    assert_eq!(
        Term::Match {
            discriminant: Box::new(g("d")),
            motive: Box::new(g("m")),
            cases: vec![g("c0"), g("c1")],
        }
        .to_string(),
        "match d return m with c0 | c1",
    );
    assert_eq!(
        Term::MutualFix {
            defs: vec![("f".to_string(), g("a")), ("h".to_string(), g("b"))],
            index: 0
        }
        .to_string(),
        "mutualfix { f := a, h := b }.0",
    );
    assert_eq!(Term::Lit(Literal::Int(5)).to_string(), "5");
    assert_eq!(Term::Hole.to_string(), "_");
}

#[cfg(feature = "serde")]
#[test]
fn bigint_and_nat_literals_serde_round_trip_via_decimal_strings() {
    // bigint_dec renders as a decimal STRING (stable across limb layout); round-trip must be
    // exact for a value beyond i64 (the whole reason the BigInt literal exists).
    let big = Literal::BigInt(BigInt::from_u64(u64::MAX));
    let json = serde_json::to_string(&big).expect("serialize BigInt literal");
    assert!(
        json.contains(&u64::MAX.to_string()),
        "serialized as a decimal string: {json}"
    );
    assert_eq!(
        serde_json::from_str::<Literal>(&json).unwrap(),
        big,
        "BigInt literal round-trips"
    );

    let nat = Literal::Nat(BigInt::parse_decimal("123456789012345678901234567890").unwrap());
    let json = serde_json::to_string(&nat).expect("serialize Nat literal");
    assert_eq!(
        serde_json::from_str::<Literal>(&json).unwrap(),
        nat,
        "Nat literal round-trips"
    );
}
