use logicaffeine_kernel::interface::lean as adapter;
use logicaffeine_kernel::{BigInt, Literal, Term};
use serde_json::{json, Value};
fn natural(digits: &str) -> String {
    let mut rows: Vec<Value> = include_str!("fixtures/lean_arena/044_natDef.ndjson")
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    for row in &mut rows {
        if row["str"]["pre"] == 0 && row["str"]["str"] == "N" {
            row["str"]["str"] = json!("Nat");
        }
    }
    let ni = rows.iter().filter_map(|r| r["in"].as_u64()).max().unwrap() + 1;
    let ei = rows.iter().filter_map(|r| r["ie"].as_u64()).max().unwrap() + 1;
    let nat = rows
        .iter()
        .find(|r| r["str"]["pre"] == 0 && r["str"]["str"] == "Nat")
        .unwrap()["in"]
        .clone();
    rows.extend([
        json!({"in":ni,"str":{"pre":0,"str":"large_literal"}}),
        json!({"ie":ei,"const":{"name":nat,"us":[]}}),
        json!({"ie":ei+1,"natVal":digits}),
        json!({"def":{"all":[ni],"name":ni,"type":ei,"value":ei+1,"levelParams":[],"safety":"safe","hints":"abbrev"}}),
    ]);
    rows.into_iter().map(|r| format!("{r}\n")).collect()
}
#[test]
fn natural_literal_round_trips_across_machine_word_boundaries() {
    for s in [
        "18446744073709551615",
        "18446744073709551616",
        "340282366920938463463374607431768211456",
        "100000000000000000000000000000000",
        &"1234567890".repeat(50),
    ] {
        let (ctx, _) = adapter::import_audited(&natural(s)).expect("finite BigInt Nat literal");
        let value = ctx.get_definition_body("/s:\"large_literal\"").unwrap();
        let Term::Lit(Literal::Nat(n)) = value else {
            panic!("literal must stay compact: {value:?}")
        };
        assert_eq!(n.to_string(), s);
        assert_eq!(
            logicaffeine_kernel::recheck(&ctx, value).unwrap(),
            Term::Global("Nat".into())
        );
    }
}
#[test]
fn original_uint64_size_declaration_checks() {
    let input = include_str!("fixtures/lean_arena/init-prelude.ndjson")
        .lines()
        .take(20104)
        .collect::<Vec<_>>()
        .join("\n");
    let (ctx, _) = adapter::import_audited(&input).expect("original UInt64.size");
    let body = ctx.get_definition_body("/s:\"UInt64\"/s:\"size\"").unwrap();
    // Lean exports the numeral `@OfNat.ofNat Nat n (instOfNatNat n)`; the literal inside stays compact.
    let n = Term::Lit(Literal::Nat(
        BigInt::parse_decimal("18446744073709551616").unwrap(),
    ));
    let app = |f: Term, a: Term| Term::App(Box::new(f), Box::new(a));
    let of_nat = Term::Const {
        name: "/s:\"OfNat\"/s:\"ofNat\"".into(),
        levels: vec![logicaffeine_kernel::Universe::Prop],
    };
    let exported = app(
        app(app(of_nat, Term::Global("Nat".into())), n.clone()),
        app(Term::Global("/s:\"instOfNatNat\"".into()), n.clone()),
    );
    assert_eq!(body, &exported);
    let normal = logicaffeine_kernel::normalize_checked(&ctx, body).unwrap();
    assert_eq!(normal, n, "it computes to the compact literal");
}
#[test]
fn leading_zeroes_preserve_natural_value() {
    let (ctx, _) = adapter::import_audited(&natural("000018446744073709551616")).unwrap();
    assert_eq!(
        ctx.get_definition_body("/s:\"large_literal\"").unwrap(),
        &Term::Lit(Literal::Nat(
            BigInt::parse_decimal("18446744073709551616").unwrap()
        ))
    );
    adapter::check_str(&natural("000000")).unwrap();
}
#[test]
fn natural_parser_budget_is_about_input_work_not_u64_overflow() {
    let error = adapter::check_str(&natural(&"9".repeat(100_000))).unwrap_err();
    assert_eq!(error.code(), 2);
    assert!(format!("{error:?}").contains("natural literal parsing budget"));
}
#[test]
fn large_natural_parser_does_not_accept_signs_whitespace_or_non_ascii_digits() {
    for s in [
        "",
        "-18446744073709551616",
        "+18446744073709551616",
        " 18446744073709551616",
        "18446744073709551616 ",
        "１",
        "1_000",
        "1.0",
        "1e30",
    ] {
        let error = adapter::check_str(&natural(s)).unwrap_err();
        assert_eq!(error.code(), 2, "{s}: {error:?}");
        assert!(format!("{error:?}").contains("malformed natural literal"));
    }
}
