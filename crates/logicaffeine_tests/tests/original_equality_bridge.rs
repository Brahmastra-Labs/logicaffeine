//! The six unchanged original T06-002 declarations, including opaque proof bodies.
use logicaffeine_kernel::interface::lean as adapter;
use logicaffeine_kernel::term_store::{NodeShape, TermStore};
use logicaffeine_kernel::{
    axiom_dependencies_of, defeq_for_test, infer_type, normalize_checked, recheck, BigInt, Context,
    Literal, Term, Universe,
};
use serde_json::Value;
const INPUT: &str = include_str!("fixtures/lean_conformance/cases_lean414/T06-002.ndjson");
const PROVENANCE: &str =
    include_str!("fixtures/lean_conformance/cases_lean414/T06-002.provenance.json");
const ROOTS: [&str; 6] = [
    "bigMotive",
    "jDependent",
    "symmViaRec",
    "transport",
    "transportRaw",
    "viaMpr",
];
fn name(s: &str) -> String {
    format!("/s:\"T06_002\"/s:{s:?}")
}
fn g(s: &str) -> Term {
    Term::Global(s.into())
}
fn v(s: &str) -> Term {
    Term::Var(s.into())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn apps(f: Term, xs: Vec<Term>) -> Term {
    xs.into_iter().fold(f, app)
}
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn lam(n: &str, a: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(a),
        body: Box::new(b),
    }
}
fn type0() -> Term {
    Term::Sort(Universe::Type(0))
}
fn nat(n: u64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_u64(n)))
}
fn eq(a: Term, x: Term, y: Term) -> Term {
    apps(
        Term::Const {
            name: r#"/s:"Eq""#.into(),
            levels: vec![Universe::Type(0)],
        },
        vec![a, x, y],
    )
}
fn refl(a: Term, x: Term) -> Term {
    apps(
        Term::Const {
            name: r#"/s:"Eq"/s:"refl""#.into(),
            levels: vec![Universe::Type(0)],
        },
        vec![a, x],
    )
}
fn goal(short: &str) -> Term {
    match short {
        "transport" | "transportRaw" => pi(
            "A",
            type0(),
            pi(
                "P",
                pi("x", v("A"), Term::Sort(Universe::Prop)),
                pi(
                    "a",
                    v("A"),
                    pi(
                        "b",
                        v("A"),
                        pi(
                            "h",
                            eq(v("A"), v("a"), v("b")),
                            pi("pa", app(v("P"), v("a")), app(v("P"), v("b"))),
                        ),
                    ),
                ),
            ),
        ),
        "symmViaRec" => pi(
            "A",
            type0(),
            pi(
                "a",
                v("A"),
                pi(
                    "b",
                    v("A"),
                    pi("h", eq(v("A"), v("a"), v("b")), eq(v("A"), v("b"), v("a"))),
                ),
            ),
        ),
        "viaMpr" => pi(
            "a",
            g("Nat"),
            pi(
                "b",
                g("Nat"),
                pi(
                    "h",
                    eq(g("Nat"), v("a"), v("b")),
                    pi(
                        "p",
                        eq(g("Nat"), v("b"), v("b")),
                        eq(g("Nat"), v("a"), v("a")),
                    ),
                ),
            ),
        ),
        "bigMotive" => pi(
            "a",
            g("Nat"),
            pi(
                "b",
                g("Nat"),
                pi(
                    "h",
                    eq(g("Nat"), v("a"), v("b")),
                    pi(
                        "F",
                        pi("n", g("Nat"), type0()),
                        pi("x", app(v("F"), v("a")), app(v("F"), v("b"))),
                    ),
                ),
            ),
        ),
        "jDependent" => {
            let motive = pi(
                "b",
                v("A"),
                pi("h", eq(v("A"), v("a"), v("b")), Term::Sort(Universe::Prop)),
            );
            let base = apps(v("motive"), vec![v("a"), refl(v("A"), v("a"))]);
            pi(
                "A",
                type0(),
                pi(
                    "a",
                    v("A"),
                    pi(
                        "motive",
                        motive,
                        pi(
                            "base",
                            base,
                            pi(
                                "b",
                                v("A"),
                                pi(
                                    "h",
                                    eq(v("A"), v("a"), v("b")),
                                    apps(v("motive"), vec![v("b"), v("h")]),
                                ),
                            ),
                        ),
                    ),
                ),
            )
        }
        _ => unreachable!(),
    }
}
fn check_goal(_ctx: &Context, body: &Term, expected: &Term) -> Term {
    // This application makes each checker perform its own expected-goal
    // conversion, rather than comparing both inferred types using one checker.
    app(
        lam("__original_goal", expected.clone(), nat(0)),
        body.clone(),
    )
}
fn check(short: &str, kind: &str, type_id: usize, value_id: usize) {
    assert_eq!(INPUT.len(), 24_158);
    assert_eq!(INPUT.lines().count(), 466);
    let p: Value = serde_json::from_str(PROVENANCE).unwrap();
    assert_eq!(
        p["source_sha256"],
        "57af1f3bab19363f4d4c82e19f2d50940c197da9559df418f7d40f95e4a8aeaf"
    );
    assert_eq!(
        p["export_sha256"],
        "caae696b50c275e0f9c17b0f07b27d10798a684d2e35f3e7550475f1c79d21d3"
    );
    assert_eq!(p["toolchain"], "leanprover/lean4:v4.14.0");
    assert_eq!(
        p["exporter_commit"],
        "20c4a64aa1b76f7c79a6db15d089084359f8c3dc"
    );
    assert_eq!(p["expression_equality"], "structural");
    assert_eq!(p["declaration_selection"], "all_authored");
    assert_eq!(
        p["declarations"],
        serde_json::json!(ROOTS.map(|s| format!("T06_002.{s}")))
    );
    assert_eq!(
        serde_json::from_str::<Value>(INPUT.lines().next().unwrap()).unwrap(),
        p["export_header"]
    );
    let target = name(short);
    let (ctx, report) = adapter::import_audited_retaining_source(INPUT, &target).unwrap();
    assert!(report.declaration_audit_requested);
    let mut names: Vec<_> = report
        .declarations
        .iter()
        .filter(|d| d.name.starts_with(r#"/s:"T06_002"/"#))
        .map(|d| d.name.clone())
        .collect();
    names.sort();
    assert_eq!(names, ROOTS.map(name).to_vec());
    let row = report
        .declarations
        .iter()
        .find(|d| d.name == target)
        .unwrap();
    assert_eq!(row.kind, kind);
    assert_eq!(row.type_expression, type_id);
    assert_eq!(row.value_expression, Some(value_id));
    assert!(row.axioms.is_empty());
    let dependencies = axiom_dependencies_of(&ctx, &target).unwrap();
    assert!(dependencies.axioms.is_empty());
    assert!(dependencies.unresolved.is_empty());
    assert!(report.metadata_losses.is_empty());
    assert!(report.runtime_declarations.is_empty());
    let (retained, declared, body) = ctx.retained_source().unwrap();
    assert_eq!(retained, target);
    assert_ne!(body, &g(&target));
    let expected = goal(short);
    let mut store = TermStore::new();
    for term in [body, declared, &expected] {
        let id = store.intern(term);
        assert!(store.free_names_are_empty(id));
        assert!(!store.shapes(id).contains(NodeShape::Hole));
    }
    assert!(infer_type(&ctx, &expected).is_ok());
    assert!(recheck(&ctx, &expected).is_ok());
    assert!(defeq_for_test(&ctx, declared, &expected));
    assert!(defeq_for_test(
        &ctx,
        ctx.get_global(&target).unwrap(),
        declared
    ));
    let checked = check_goal(&ctx, body, &expected);
    assert!(
        infer_type(&ctx, &checked).is_ok(),
        "{short}: main expected-goal check"
    );
    assert!(
        recheck(&ctx, &checked).is_ok(),
        "{short}: independent expected-goal check"
    );
    let wrong = check_goal(&ctx, body, &g("Nat"));
    assert!(infer_type(&ctx, &wrong).is_err());
    assert!(recheck(&ctx, &wrong).is_err());
    // Lean's kernel unfolds a theorem: its published body is exactly the checked proof.
    if kind == "thm" {
        assert_eq!(
            ctx.get_definition_body(&target),
            Some(body),
            "{short}: theorem unfolds like Lean's"
        );
    } else {
        assert_eq!(ctx.get_definition_body(&target), Some(body));
        let observed = apps(
            body.clone(),
            vec![
                nat(0),
                nat(0),
                refl(g("Nat"), nat(0)),
                lam("n", g("Nat"), g("Nat")),
                nat(42),
            ],
        );
        assert!(defeq_for_test(
            &ctx,
            &normalize_checked(&ctx, &observed).unwrap(),
            &nat(42)
        ));
        let computation = check_goal(
            &ctx,
            &refl(g("Nat"), nat(42)),
            &eq(g("Nat"), observed, nat(42)),
        );
        assert!(infer_type(&ctx, &computation).is_ok());
        assert!(recheck(&ctx, &computation).is_ok());
    }
}
#[test]
fn original_big_motive_checks_its_actual_goal_and_reflexive_computation() {
    check("bigMotive", "def", 84, 101);
}
#[test]
fn original_dependent_j_checks_its_actual_proof_dependent_goal() {
    check("jDependent", "thm", 130, 143);
}
#[test]
fn original_symmetry_via_rec_checks_its_actual_goal() {
    check("symmViaRec", "thm", 150, 167);
}
#[test]
fn original_transport_checks_its_actual_goal() {
    check("transport", "thm", 177, 195);
}
#[test]
fn original_raw_transport_checks_its_actual_goal() {
    check("transportRaw", "thm", 198, 210);
}
#[test]
fn original_mpr_checks_its_actual_goal() {
    check("viaMpr", "thm", 320, 360);
}

#[test]
fn bridge_rows_require_the_six_verified_original_declarations() {
    let ledger: Value = serde_json::from_str(include_str!(
        "fixtures/lean_conformance/bridge/T06-002.json"
    ))
    .unwrap();
    assert_eq!(ledger["case"], "T06-002");
    assert_eq!(ledger["lean"], "4.14.0");
    assert_eq!(ledger["lean_compiles"], true);
    let rows = ledger["declarations"].as_array().unwrap();
    assert_eq!(rows.len(), 6);
    for (short, test) in [
        (
            "bigMotive",
            "original_big_motive_checks_its_actual_goal_and_reflexive_computation",
        ),
        (
            "jDependent",
            "original_dependent_j_checks_its_actual_proof_dependent_goal",
        ),
        (
            "symmViaRec",
            "original_symmetry_via_rec_checks_its_actual_goal",
        ),
        ("transport", "original_transport_checks_its_actual_goal"),
        (
            "transportRaw",
            "original_raw_transport_checks_its_actual_goal",
        ),
        ("viaMpr", "original_mpr_checks_its_actual_goal"),
    ] {
        let row = rows
            .iter()
            .find(|r| r["lean"] == format!("T06_002.{short}"))
            .unwrap();
        assert_eq!(row["status"], "bridged", "{short}");
        assert_eq!(
            row["test"],
            format!("crates/logicaffeine_tests/tests/original_equality_bridge.rs::{test}")
        );
        assert_eq!(row["lean_levels"], serde_json::json!([]));
        assert_eq!(row["lean_axioms"], serde_json::json!([]));
        assert!(!row["logos"].as_str().unwrap().trim().is_empty());
    }
}
