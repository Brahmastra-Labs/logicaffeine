//! Original T00-019 kernel declarations, imported from the pinned Lean export.
//! This bridges five selected authored roots, not the two unexported Fin roots
//! or Lean's surface elaboration-time opacity policy.
use logicaffeine_kernel::interface::lean as adapter;

use logicaffeine_kernel::term_store::{NodeShape, TermStore};
use logicaffeine_kernel::{
    axiom_dependencies_of, defeq_for_test, infer_type, normalize_checked, recheck, BigInt, Context,
    Literal, Term, Universe,
};
use serde_json::Value;

const INPUT: &str =
    include_str!("fixtures/lean_conformance/cases_lean414/T00-019.sharing.structural.ndjson");
const PROVENANCE: &str = include_str!(
    "fixtures/lean_conformance/cases_lean414/T00-019.sharing.structural.provenance.json"
);
const ROOTS: [&str; 5] = [
    "letValueVisible",
    "nestedHaves",
    "sharedProof",
    "viaHave",
    "viaLet",
];

fn name(short: &str) -> String {
    format!("/s:\"T00_019\"/s:{short:?}")
}
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn nat(n: u64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_u64(n)))
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn eq(a: Term, b: Term) -> Term {
    app(
        app(
            app(
                Term::Const {
                    name: r#"/s:"Eq""#.into(),
                    levels: vec![Universe::Type(0)],
                },
                g("Nat"),
            ),
            a,
        ),
        b,
    )
}
fn and(a: Term, b: Term) -> Term {
    app(app(g(r#"/s:"And""#), a), b)
}

fn pin_scope() {
    assert_eq!(INPUT.len(), 45_876);
    assert_eq!(INPUT.lines().count(), 815);
    let p: Value = serde_json::from_str(PROVENANCE).unwrap();
    assert_eq!(
        p["source_sha256"],
        "f88021661bb506a2bc6813bce6c20750bc5a0167ca0240ab53a237544cea4329"
    );
    assert_eq!(
        p["ledger_sha256"],
        "4663c692079a62e7a9a839a36afff3035bdcfec5a592567f1807cdc9342c9dfd"
    );
    assert_eq!(
        p["export_sha256"],
        "6a8035b9d51753f7d1a9614fa940bd196cc137687d1cf399d8dd817e2b8674c2"
    );
    assert_eq!(p["toolchain"], "leanprover/lean4:v4.14.0");
    assert_eq!(
        p["exporter_commit"],
        "20c4a64aa1b76f7c79a6db15d089084359f8c3dc"
    );
    assert_eq!(p["expression_equality"], "structural");
    assert_eq!(p["declaration_selection"], "explicit_authored_roots");
    let expected: Vec<_> = ROOTS
        .iter()
        .map(|n| Value::String(format!("T00_019.{n}")))
        .collect();
    assert_eq!(p["declarations"].as_array().unwrap(), &expected);
    let header: Value = serde_json::from_str(INPUT.lines().next().unwrap()).unwrap();
    assert_eq!(header, p["export_header"]);
    assert_eq!(
        header["meta"]["lean"]["githash"],
        "410fab7284703f41660ca2454218dcca9b2ec896"
    );
}

fn closed_exact(ctx: &Context, body: &Term, ty: &Term, expected: &Term) {
    let mut store = TermStore::new();
    for term in [body, ty] {
        let id = store.intern(term);
        assert!(store.free_names_are_empty(id));
        assert!(!store.shapes(id).contains(NodeShape::Hole));
    }
    assert!(defeq_for_test(ctx, ty, expected), "original declared goal");
    assert!(
        defeq_for_test(ctx, &infer_type(ctx, body).unwrap(), expected),
        "main source-body check"
    );
    assert!(
        defeq_for_test(ctx, &recheck(ctx, body).unwrap(), expected),
        "independent source-body check"
    );
}

// Both checkers must reject using the actual source body at an incompatible
// (but itself well-formed) goal. No declaration or axiom is manufactured.
fn reject_as(ctx: &Context, body: &Term, wrong: Term) {
    assert!(infer_type(ctx, &wrong).is_ok());
    assert!(recheck(ctx, &wrong).is_ok());
    let demand = Term::Lambda {
        param: "__bridge_wrong_goal".into(),
        param_type: Box::new(wrong),
        body: Box::new(nat(0)),
    };
    let ill_typed = app(demand, body.clone());
    assert!(infer_type(ctx, &ill_typed).is_err());
    assert!(recheck(ctx, &ill_typed).is_err());
}

fn check(
    short: &str,
    kind: &str,
    type_id: usize,
    value_id: usize,
    expected: Term,
    value: Option<u64>,
) {
    pin_scope();
    let target = name(short);
    let (ctx, report) = adapter::import_audited_retaining_source(INPUT, &target).unwrap();
    assert!(report.declaration_audit_requested);
    let mut authored: Vec<_> = report
        .declarations
        .iter()
        .filter(|d| d.name.starts_with(r#"/s:"T00_019"/"#))
        .map(|d| d.name.clone())
        .collect();
    authored.sort();
    let expected_names: Vec<_> = ROOTS.iter().map(|n| name(n)).collect();
    assert_eq!(authored, expected_names);
    let row = report
        .declarations
        .iter()
        .find(|d| d.name == target)
        .unwrap();
    assert_eq!(row.kind, kind);
    assert_eq!(row.type_expression, type_id);
    assert_eq!(row.value_expression, Some(value_id));
    assert!(
        row.axioms.is_empty(),
        "the original five authored roots are axiom-free"
    );
    let usage = axiom_dependencies_of(&ctx, &target).unwrap();
    assert!(usage.axioms.is_empty());
    assert!(usage.unresolved.is_empty());

    let (retained, declared, body) = ctx.retained_source().expect("actual original body");
    assert_eq!(retained, target);
    assert_ne!(
        body,
        &g(&target),
        "check the proof body, not the published theorem reference"
    );
    assert!(
        defeq_for_test(&ctx, ctx.get_global(&target).unwrap(), declared),
        "actual published type"
    );
    closed_exact(&ctx, body, declared, &expected);
    if kind == "def" {
        assert_eq!(ctx.get_definition_body(&target), Some(body));
        assert!(defeq_for_test(
            &ctx,
            &normalize_checked(&ctx, body).unwrap(),
            &nat(value.unwrap())
        ));
        match short {
            "viaLet" | "letValueVisible" => assert!(matches!(body, Term::Let { .. })),
            "viaHave" => assert!(
                matches!(body, Term::App(..)),
                "retain exported letFun application"
            ),
            _ => unreachable!(),
        }
        reject_as(&ctx, body, pi("x", g("Nat"), g("Nat")));
    } else {
        // Lean's kernel unfolds a theorem: its published body is exactly the checked proof.
        assert_eq!(
            ctx.get_definition_body(&target),
            Some(body),
            "theorem publication unfolds like Lean's"
        );
        reject_as(
            &ctx,
            body,
            pi("n", g("Nat"), eq(v("n"), app(g("Succ"), v("n")))),
        );
    }

    // The export contains this exact source-only metadata, not a letFun marker.
    // The importer must report it; no invented opacity-loss claim is made.
    assert_eq!(report.metadata_losses.len(), 1);
    let loss = &report.metadata_losses[0];
    assert_eq!(
        (loss.expression, loss.key.as_str(), loss.value.as_str()),
        (400, "borrowed", "Lean.DataValue.ofBool true")
    );
}

#[test]
fn original_let_value_visible_preserves_the_dependent_proof_binding() {
    check("letValueVisible", "def", 1, 163, g("Nat"), Some(3));
}
#[test]
fn original_nested_haves_preserves_the_actual_transitivity_proof() {
    check(
        "nestedHaves",
        "thm",
        539,
        576,
        pi("n", g("Nat"), eq(v("n"), v("n"))),
        None,
    );
}
#[test]
fn original_shared_proof_preserves_all_three_conjuncts_without_new_axioms() {
    let e = eq(v("n"), v("n"));
    check(
        "sharedProof",
        "thm",
        611,
        629,
        pi("n", g("Nat"), and(e.clone(), and(e.clone(), e))),
        None,
    );
}
#[test]
fn original_via_have_preserves_the_exported_application_and_value() {
    check("viaHave", "def", 1, 641, g("Nat"), Some(4));
}
#[test]
fn original_via_let_preserves_the_original_let_and_value() {
    check("viaLet", "def", 1, 642, g("Nat"), Some(4));
}
