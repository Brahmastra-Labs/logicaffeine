use logicaffeine_kernel::interface::lean as adapter;

use logicaffeine_kernel::{
    axiom_dependencies_of, defeq_for_test, infer_type, normalize_checked, recheck, Context,
    Literal, Term, Universe,
};
use serde_json::Value;

fn name(short: &str) -> String {
    short
        .split('.')
        .fold(String::from(r#"/s:"T00_003""#), |mut n, part| {
            n.push_str(&format!("/s:{part:?}"));
            n
        })
}
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn nat(n: u64) -> Term {
    Term::Lit(Literal::Nat(logicaffeine_kernel::BigInt::from_u64(n)))
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
/// What Lean 4.14 exports for the numeral `n : Nat`: `@OfNat.ofNat Nat n (instOfNatNat n)`.
fn numeral(n: u64) -> Term {
    let of_nat = Term::Const {
        name: r#"/s:"OfNat"/s:"ofNat""#.into(),
        levels: vec![Universe::Prop],
    };
    app(
        app(app(of_nat, g("Nat")), nat(n)),
        app(g(r#"/s:"instOfNatNat""#), nat(n)),
    )
}
fn import() -> (adapter::ImportedContext, adapter::ImportReport) {
    adapter::import_audited(include_str!(
        "fixtures/lean_conformance/cases_lean414/T00-003.structural.ndjson"
    ))
    .expect("the unchanged original Lean environment must import with declaration auditing")
}
fn checked(ctx: &Context, term: &Term, want: &Term) {
    assert!(defeq_for_test(ctx, &infer_type(ctx, term).unwrap(), want));
    assert!(defeq_for_test(ctx, &recheck(ctx, term).unwrap(), want));
}
fn declaration(
    ctx: &Context,
    report: &adapter::ImportReport,
    short: &str,
    kind: &str,
    ty: Term,
    body: Option<Term>,
    axioms: &[String],
) {
    let n = name(short);
    assert!(
        ctx.get_universe_poly(&n).is_none(),
        "monomorphic source declaration {n}"
    );
    checked(ctx, &g(&n), &ty);
    assert!(defeq_for_test(ctx, ctx.get_global(&n).unwrap(), &ty));
    assert_eq!(ctx.is_inductive(&n), kind == "inductive");
    assert_eq!(ctx.is_constructor(&n), kind == "ctor");
    let usage = axiom_dependencies_of(ctx, &n).unwrap();
    assert!(usage.unresolved.is_empty(), "{n}: {usage:?}");
    let actual: Vec<_> = usage.axioms.iter().cloned().collect();
    assert_eq!(actual, axioms, "exact original assumption set for {n}");
    if kind == "def" || kind == "axiom" {
        let audit = report.declarations.iter().find(|d| d.name == n).unwrap();
        assert_eq!(audit.kind, kind);
        assert_eq!(audit.axioms, axioms);
        assert_eq!(audit.value_expression.is_some(), kind == "def");
    }
    match body {
        Some(want_body) => {
            let actual = ctx
                .get_definition_body(&n)
                .expect("actual original definition body");
            assert_eq!(
                actual, &want_body,
                "preserve the original constant reference: {n}"
            );
            checked(ctx, actual, &ty);
            assert!(ctx.is_definition(&n));
        }
        None => assert!(ctx.get_definition_body(&n).is_none()),
    }
}
fn ledger(short_names: &[&str], test: &str) {
    let data: Value = serde_json::from_str(include_str!(
        "fixtures/lean_conformance/bridge/T00-003.json"
    ))
    .unwrap();
    let rows = data["declarations"].as_array().unwrap();
    assert_eq!(
        rows.len(),
        13,
        "all authored source obligations remain present"
    );
    for short in short_names {
        let lean = format!("T00_003.{short}");
        let row = rows.iter().find(|r| r["lean"] == lean).unwrap();
        assert_eq!(
            row["status"], "bridged",
            "verified original declaration {lean} needs its evidence registered"
        );
        assert_eq!(row["test"], format!("original_constant_bridge::{test}"));
        assert!(!row["logos"].as_str().unwrap().is_empty());
    }
}

#[test]
fn original_monomorphic_definitions_preserve_constant_identity_and_values() {
    let (ctx, report) = import();
    declaration(
        &ctx,
        &report,
        "base",
        "def",
        g("Nat"),
        Some(numeral(7)),
        &[],
    );
    declaration(
        &ctx,
        &report,
        "refersToBase",
        "def",
        g("Nat"),
        Some(g(&name("base"))),
        &[],
    );
    declaration(
        &ctx,
        &report,
        "FlagCount",
        "def",
        g("Nat"),
        Some(numeral(2)),
        &[],
    );
    // The exported numeral is kept, never compacted: a bare literal is a different body.
    assert_ne!(ctx.get_definition_body(&name("base")), Some(&nat(7)));
    for (short, value) in [("base", 7), ("refersToBase", 7), ("FlagCount", 2)] {
        assert!(defeq_for_test(
            &ctx,
            &normalize_checked(&ctx, &g(&name(short))).unwrap(),
            &nat(value)
        ));
    }
    ledger(
        &["base", "refersToBase", "FlagCount"],
        "original_monomorphic_definitions_preserve_constant_identity_and_values",
    );
}

#[test]
fn original_inductive_and_constructor_references_keep_distinct_checked_names() {
    let (ctx, report) = import();
    let ty = Term::Sort(Universe::Type(0));
    let flag = g(&name("Flag"));
    declaration(&ctx, &report, "Flag", "inductive", ty.clone(), None, &[]);
    for short in ["Flag.on", "Flag.off"] {
        declaration(&ctx, &report, short, "ctor", flag.clone(), None, &[]);
    }
    declaration(
        &ctx,
        &report,
        "refersToInductive",
        "def",
        ty.clone(),
        Some(g("Nat")),
        &[],
    );
    let successor_type = Term::Pi {
        param: "n".into(),
        param_type: Box::new(g("Nat")),
        body_type: Box::new(g("Nat")),
    };
    declaration(
        &ctx,
        &report,
        "refersToConstructor",
        "def",
        successor_type,
        Some(g("Succ")),
        &[],
    );
    declaration(
        &ctx,
        &report,
        "refersToFlagType",
        "def",
        ty,
        Some(flag),
        &[],
    );
    declaration(
        &ctx,
        &report,
        "refersToFlagCtor",
        "def",
        g(&name("Flag")),
        Some(g(&name("Flag.on"))),
        &[],
    );
    assert!(ctx.is_inductive("Nat"));
    assert!(ctx.is_constructor("Succ"));
    assert!(!defeq_for_test(
        &ctx,
        &g(&name("Flag.on")),
        &g(&name("Flag.off"))
    ));
    checked(
        &ctx,
        &app(g(&name("refersToConstructor")), nat(7)),
        &g("Nat"),
    );
    assert!(defeq_for_test(
        &ctx,
        &normalize_checked(&ctx, &app(g(&name("refersToConstructor")), nat(7))).unwrap(),
        &nat(8)
    ));
    for wrong in [
        app(g(&name("refersToConstructor")), g(&name("Flag.on"))),
        app(g(&name("Flag")), nat(0)),
    ] {
        assert!(infer_type(&ctx, &wrong).is_err());
        assert!(recheck(&ctx, &wrong).is_err());
    }
    ledger(
        &[
            "Flag",
            "Flag.on",
            "Flag.off",
            "refersToInductive",
            "refersToConstructor",
            "refersToFlagType",
            "refersToFlagCtor",
        ],
        "original_inductive_and_constructor_references_keep_distinct_checked_names",
    );
}

#[test]
fn original_explicit_axiom_reference_preserves_its_exact_assumption() {
    let (ctx, report) = import();
    let axiom = name("opaqueNat");
    declaration(
        &ctx,
        &report,
        "opaqueNat",
        "axiom",
        g("Nat"),
        None,
        &[axiom.clone()],
    );
    declaration(
        &ctx,
        &report,
        "refersToAxiom",
        "def",
        g("Nat"),
        Some(g(&axiom)),
        &[axiom.clone()],
    );
    assert!(!ctx.is_definition(&axiom));
    assert_eq!(
        normalize_checked(&ctx, &g(&name("refersToAxiom"))).unwrap(),
        g(&axiom)
    );
    ledger(
        &["opaqueNat", "refersToAxiom"],
        "original_explicit_axiom_reference_preserves_its_exact_assumption",
    );
}

#[test]
fn original_polymorphic_constant_instance_keeps_its_explicit_level_argument() {
    let (ctx, report) = import();
    let list = r#"/s:"List""#;
    let head = Term::Const {
        name: list.into(),
        levels: vec![Universe::Prop],
    };
    let body = app(head.clone(), g("Nat"));
    declaration(
        &ctx,
        &report,
        "monoInstanceOfPolyConst",
        "def",
        Term::Sort(Universe::Type(0)),
        Some(body.clone()),
        &[],
    );
    assert!(ctx.is_inductive(list));
    checked(&ctx, &body, &Term::Sort(Universe::Type(0)));
    for levels in [vec![], vec![Universe::Prop, Universe::Prop]] {
        let malformed = app(
            Term::Const {
                name: list.into(),
                levels,
            },
            g("Nat"),
        );
        assert!(infer_type(&ctx, &malformed).is_err());
        assert!(recheck(&ctx, &malformed).is_err());
    }
    ledger(
        &["monoInstanceOfPolyConst"],
        "original_polymorphic_constant_instance_keeps_its_explicit_level_argument",
    );
}
