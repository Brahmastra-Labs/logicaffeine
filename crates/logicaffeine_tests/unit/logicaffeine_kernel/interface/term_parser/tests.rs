use super::*;

#[test]
fn test_parse_global() {
    let term = TermParser::parse("Zero").unwrap();
    assert!(matches!(term, Term::Global(ref s) if s == "Zero"));
}

#[test]
fn test_parse_sort() {
    let term = TermParser::parse("Type").unwrap();
    assert!(matches!(term, Term::Sort(Universe::Type(0))));

    let term2 = TermParser::parse("Prop").unwrap();
    assert!(matches!(term2, Term::Sort(Universe::Prop)));
}

#[test]
fn test_parse_app() {
    let term = TermParser::parse("Succ Zero").unwrap();
    if let Term::App(func, arg) = term {
        assert!(matches!(*func, Term::Global(ref s) if s == "Succ"));
        assert!(matches!(*arg, Term::Global(ref s) if s == "Zero"));
    } else {
        panic!("Expected App");
    }
}

#[test]
fn test_parse_parens() {
    let term = TermParser::parse("(Succ Zero)").unwrap();
    assert!(matches!(term, Term::App(..)));
}

#[test]
fn test_parse_arrow() {
    let term = TermParser::parse("Nat -> Nat").unwrap();
    if let Term::Pi {
        param,
        param_type,
        body_type,
    } = term
    {
        assert_eq!(param, "_");
        assert!(matches!(*param_type, Term::Global(ref s) if s == "Nat"));
        assert!(matches!(*body_type, Term::Global(ref s) if s == "Nat"));
    } else {
        panic!("Expected Pi");
    }
}

#[test]
fn test_parse_lambda() {
    let term = TermParser::parse("fun x : Nat => Succ x").unwrap();
    if let Term::Lambda { param, body, .. } = term {
        assert_eq!(param, "x");
        // Body should use Var for bound x
        if let Term::App(_, arg) = *body {
            assert!(matches!(*arg, Term::Var(ref s) if s == "x"));
        } else {
            panic!("Expected App in lambda body");
        }
    } else {
        panic!("Expected Lambda");
    }
}

fn lit(text: &str) -> Result<Literal, ParseError> {
    match TermParser::parse(text)? {
        Term::Lit(l) => Ok(l),
        other => panic!("{text:?} is not a literal: {other:?}"),
    }
}

#[test]
fn every_literal_kind_has_its_own_spelling() {
    let big = |s: &str| crate::BigInt::parse_decimal(s).unwrap();
    assert_eq!(lit("5").unwrap(), Literal::Int(5));
    assert_eq!(lit("-5").unwrap(), Literal::Int(-5));
    assert_eq!(lit("5n").unwrap(), Literal::Nat(big("5")));
    assert_eq!(lit("0n").unwrap(), Literal::Nat(big("0")));
    assert_eq!(lit("1.5f64").unwrap(), Literal::Float(1.5));
    assert_eq!(lit("-0.0f64").unwrap(), Literal::Float(-0.0));
    assert_eq!(lit("1e300f64").unwrap(), Literal::Float(1e300));
    assert_eq!(lit("2.5e-3f64").unwrap(), Literal::Float(2.5e-3));
    assert_eq!(lit("5f64").unwrap(), Literal::Float(5.0));
    assert_eq!(
        lit("0x7ff8000000000001f64").unwrap(),
        Literal::Float(f64::from_bits(0x7ff8_0000_0000_0001))
    );
    assert_eq!(lit("5s").unwrap(), Literal::Duration(5_000_000_000));
    assert_eq!(lit("-250ms").unwrap(), Literal::Duration(-250_000_000));
    assert_eq!(lit("3min").unwrap(), Literal::Duration(180_000_000_000));
    assert_eq!(lit("2h").unwrap(), Literal::Duration(7_200_000_000_000));
    assert_eq!(lit("7us").unwrap(), Literal::Duration(7_000));
    assert_eq!(
        lit("-9223372036854775808ns").unwrap(),
        Literal::Duration(i64::MIN)
    );
    assert_eq!(lit("1970-01-01").unwrap(), Literal::Date(0));
    assert_eq!(lit("1969-12-31").unwrap(), Literal::Date(-1));
    assert_eq!(lit("2000-02-29").unwrap(), Literal::Date(11_016));
    assert_eq!(lit("1970-01-01T00:00:00Z").unwrap(), Literal::Moment(0));
    assert_eq!(
        lit("1970-01-01T00:00:00.000000001Z").unwrap(),
        Literal::Moment(1)
    );
    assert_eq!(
        lit("1969-12-31T23:59:59.999999999Z").unwrap(),
        Literal::Moment(-1)
    );
    assert_eq!(
        lit("\"a\\u{1F980}b\"").unwrap(),
        Literal::Text("a\u{1F980}b".into())
    );
    assert_eq!(lit("\"\\u{0}\"").unwrap(), Literal::Text("\u{0}".into()));
}

#[test]
fn a_malformed_literal_is_refused_never_read_as_something_else() {
    for text in [
        "3.14",
        "2.5",
        "1e5",
        "2x",
        "5n3",
        "-5n",
        "0x7ff8f64",
        "0x7ff800000000000zf64",
        "2026-02-30",
        "1900-02-29",
        "2026-13-01",
        "2026-00-10",
        "2026-05-20T24:00:00Z",
        "2026-05-20T10:60:00Z",
        "2026-05-20T10:00:00",
        "2026-05-20T10:00:00.1234567890Z",
        "9223372036854775808ns",
        "1.5f64x",
        "\"\\u{110000}\"",
        "\"\\u{}\"",
        "\"\\u{d800}\"",
    ] {
        assert!(
            TermParser::parse(text).is_err(),
            "{text:?} must be refused: {:?}",
            TermParser::parse(text)
        );
    }
    // …and what was never a literal is untouched.
    assert!(
        matches!(TermParser::parse("3.x").unwrap(), Term::App(..)),
        "a projection out of a numeral"
    );
    assert_eq!(
        TermParser::parse("3.").unwrap(),
        Term::Lit(Literal::Int(3)),
        "a numeral, then a full stop"
    );
}

#[test]
fn the_calendar_conversion_is_its_own_inverse() {
    let mut day = -3_000_000i64;
    while day <= 3_000_000 {
        let (y, m, d) = civil_from_days(day);
        assert_eq!(days_from_civil(y, m, d), day, "day {day} -> {y}-{m}-{d}");
        day += 997;
    }
    for day in [i32::MIN as i64, i32::MAX as i64, -719_528, 0, -1] {
        let (y, m, d) = civil_from_days(day);
        assert_eq!(days_from_civil(y, m, d), day);
    }
}

#[test]
fn syntactic_depth_bounds_the_readers_recursion() {
    assert_eq!(syntactic_depth("x"), 0);
    assert_eq!(syntactic_depth("f (g (h x))"), 2);
    assert_eq!(syntactic_depth("A -> B -> C"), 2);
    assert_eq!(syntactic_depth("fun x => fun y => x"), 2);
    assert_eq!(syntactic_depth("(A -> B) -> C"), 2);
    assert_eq!(
        syntactic_depth("\"((((\" \u{ab}((((\u{bb}"),
        0,
        "strings and quoted names are text"
    );
    assert_eq!(
        syntactic_depth("funny letter matcher"),
        0,
        "keywords only as whole words"
    );
    let deep = "(".repeat(100) + "x" + &")".repeat(100);
    assert_eq!(syntactic_depth(&deep), 100);
    let too_deep = "(".repeat(MAX_SYNTACTIC_DEPTH + 1) + "x" + &")".repeat(MAX_SYNTACTIC_DEPTH + 1);
    assert!(
        matches!(
            TermParser::parse(&too_deep),
            Err(ParseError::Unsupported { .. })
        ),
        "a text deeper than the kernel admits is refused by name, not by exhausting the stack"
    );
}

#[test]
fn test_parse_lambda_bound_var() {
    let term = TermParser::parse("fun n : Nat => Succ n").unwrap();
    if let Term::Lambda { body, .. } = term {
        if let Term::App(_, arg) = *body {
            assert!(
                matches!(*arg, Term::Var(ref s) if s == "n"),
                "Expected Var(n), got {:?}",
                arg
            );
        } else {
            panic!("Expected App in lambda body");
        }
    } else {
        panic!("Expected Lambda");
    }
}
