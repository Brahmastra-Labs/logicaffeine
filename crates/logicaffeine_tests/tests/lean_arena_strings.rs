use logicaffeine_kernel::interface::lean as adapter;
use logicaffeine_kernel::{BigInt, Literal, Term, Universe};
use serde_json::{json, Value};

fn prefix() -> String {
    include_str!("fixtures/lean_arena/init-prelude.ndjson")
        .lines()
        .take(19542)
        .map(|s| format!("{s}\n"))
        .collect()
}
fn add_literal(input: &mut String, value: Value, annotation: &str, label: &str) {
    let rows: Vec<Value> = input
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    let name = rows
        .iter()
        .filter_map(|r| r["in"].as_u64())
        .max()
        .unwrap_or(0)
        + 1;
    let expr = rows
        .iter()
        .filter_map(|r| r["ie"].as_u64())
        .max()
        .unwrap_or(0)
        + 1;
    let ty = rows
        .iter()
        .find(|r| r["str"]["pre"] == 0 && r["str"]["str"] == annotation)
        .and_then(|r| r["in"].as_u64())
        .unwrap();
    for row in [
        json!({"in":name,"str":{"pre":0,"str":label}}),
        json!({"ie":expr,"const":{"name":ty,"us":[]}}),
        json!({"ie":expr+1,"strVal":value}),
        json!({"def":{"name":name,"all":[name],"type":expr,"value":expr+1,
            "levelParams":[],"safety":"safe","hints":"abbrev"}}),
    ] {
        input.push_str(&format!("{row}\n"));
    }
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn global(name: &str) -> Term {
    Term::Global(name.into())
}
fn expansion(s: &str) -> Term {
    let char_type = global("/s:\"Char\"");
    let list = |name: &str| Term::Const {
        name: format!("/s:\"List\"/s:\"{name}\""),
        levels: vec![Universe::Prop],
    };
    let mut out = app(list("nil"), char_type.clone());
    for c in s.chars().rev() {
        let character = app(
            global("/s:\"Char\"/s:\"ofNat\""),
            Term::Lit(Literal::Nat(BigInt::from_u64(c as u64))),
        );
        out = app(app(app(list("cons"), char_type.clone()), character), out);
    }
    app(global("/s:\"String\"/s:\"ofList\""), out)
}
#[test]
fn imports_original_first_string_literal_declaration() {
    let input = include_str!("fixtures/lean_arena/init-prelude.ndjson")
        .lines()
        .take(19545)
        .collect::<Vec<_>>()
        .join("\n");
    adapter::check_str(&input).expect("original literal-bearing declaration must check");
}
#[test]
fn unicode_literals_preserve_scalar_order_nuls_and_combining_marks() {
    let mut input = prefix();
    let values = ["", "\0Aé𝄞\u{10ffff}e\u{301}", "ab", "ba", "é", "e\u{301}"];
    for (i, value) in values.iter().enumerate() {
        add_literal(&mut input, json!(value), "String", &format!("literal{i}"));
    }
    let (ctx, _) = adapter::import_audited(&input).expect("Unicode string definitions check");
    for (i, value) in values.iter().enumerate() {
        let actual = ctx
            .get_definition_body(&format!("/s:\"literal{i}\""))
            .unwrap();
        assert_eq!(actual, &expansion(value), "Unicode value {i}");
        assert_eq!(
            logicaffeine_kernel::recheck(&ctx, actual).unwrap(),
            global("/s:\"String\"")
        );
    }
}
#[test]
fn long_string_uses_checked_dag_chunks() {
    let mut input = prefix();
    add_literal(&mut input, json!("x".repeat(260)), "String", "long_literal");
    let (ctx, report) =
        adapter::import_audited(&input).expect("long string uses conservative sharing");
    assert!(
        !report.internal_helpers.is_empty(),
        "expanded strings must enter existing chunk machinery"
    );
    let body = ctx.get_definition_body("/s:\"long_literal\"").unwrap();
    assert!(logicaffeine_kernel::term_store::term_depth(body) < 128);
    assert_eq!(
        logicaffeine_kernel::recheck(&ctx, body).unwrap(),
        global("/s:\"String\"")
    );
}
#[test]
fn string_literal_cannot_inhabit_nat() {
    let mut input = prefix();
    add_literal(&mut input, json!("num"), "Nat", "wrong_type");
    assert_eq!(adapter::check_str(&input).unwrap_err().code(), 1);
}
#[test]
fn malformed_string_payload_declines() {
    let input = "{\"meta\":{\"format\":{\"version\":\"3.1.0\"}}}\n{\"ie\":1,\"strVal\":17}\n";
    let error = adapter::check_str(input).unwrap_err();
    assert_eq!(error.code(), 2);
    assert!(format!("{error:?}").contains("malformed string literal"));
}
#[test]
fn string_expansion_budget_is_charged_before_allocation() {
    let mut input = prefix();
    add_literal(
        &mut input,
        json!("x".repeat(100_000)),
        "String",
        "oversized",
    );
    let error = adapter::check_str(&input).unwrap_err();
    assert_eq!(error.code(), 2);
    assert!(format!("{error:?}").contains("term expansion budget"));
}

fn literal_without_checked_dependencies(unsafe_char: bool) -> String {
    let mut rows = vec![
        json!({"meta":{"format":{"version":"3.1.0"}}}),
        json!({"il":1,"succ":0}),
        json!({"ie":1,"sort":1}),
        json!({"in":1,"str":{"pre":0,"str":"String"}}),
        json!({"axiom":{"name":1,"levelParams":[],"type":1,"isUnsafe":false}}),
        json!({"in":2,"str":{"pre":0,"str":"Char"}}),
    ];
    if unsafe_char {
        rows.push(json!({"axiom":{"name":2,"levelParams":[],"type":1,"isUnsafe":true}}));
    }
    rows.extend([
        json!({"in":3,"str":{"pre":0,"str":"literal"}}),
        json!({"ie":2,"const":{"name":1,"us":[]}}),
        json!({"ie":3,"strVal":""}),
        json!({"def":{"name":3,"all":[3],"type":2,"value":3,"levelParams":[],"safety":"safe","hints":"abbrev"}}),
    ]);
    rows.into_iter().map(|r| format!("{r}\n")).collect()
}
#[test]
fn empty_string_still_requires_checked_dependencies() {
    let e = adapter::check_str(&literal_without_checked_dependencies(false)).unwrap_err();
    assert_eq!(e.code(), 1, "{e:?}");
    assert!(format!("{e:?}").contains("string literal requires checked"));
}
#[test]
fn implicit_string_dependency_cannot_use_an_unsafe_character_type() {
    let e = adapter::check_str(&literal_without_checked_dependencies(true)).unwrap_err();
    assert_eq!(e.code(), 1, "{e:?}");
    assert!(format!("{e:?}").contains("string literal requires checked"));
}
#[test]
fn unpaired_json_surrogates_are_not_unicode_scalars() {
    let input = r#"{"meta":{"format":{"version":"3.1.0"}}}
{"ie":1,"strVal":"\ud800"}
"#;
    assert_eq!(adapter::check_str(input).unwrap_err().code(), 2);
}
