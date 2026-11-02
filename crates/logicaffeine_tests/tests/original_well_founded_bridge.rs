//! Four unchanged original T04-015 roots: source goals and separate computations.
//! Imported WellFounded is Lean's structure, not the standard-prelude abbreviation.
//! These checks do not promote bridge-ledger rows or replace original function bodies.
use logicaffeine_kernel::interface::lean as adapter;
use logicaffeine_kernel::term_store::{NodeShape, TermStore};
use logicaffeine_kernel::{
    axiom_dependencies, axiom_dependencies_of, defeq_for_test, infer_type, normalize_checked,
    recheck, BigInt, Context, KernelError, Literal, ReCheckError, Term, Universe,
};
use serde_json::Value;

const INPUT: &str = include_str!("fixtures/lean_conformance/cases_lean414/T04-015.four.ndjson");
const PROVENANCE: &str =
    include_str!("fixtures/lean_conformance/cases_lean414/T04-015.four.provenance.json");
const ROOTS: [&str; 4] = ["accCount", "accIsAType", "ltWf", "wfSum"];
fn name(s: &str) -> String {
    format!("/s:\"T04_015\"/s:{s:?}")
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
fn prop() -> Term {
    Term::Sort(Universe::Prop)
}
fn nat(n: u64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_u64(n)))
}
fn poly(s: &str) -> Term {
    Term::Const {
        name: s.into(),
        levels: vec![Universe::Type(0)],
    }
}
// The reader maps Lean's reserved `Nat` namespace onto the kernel's `Nat`, keeping the suffix.
fn lt() -> Term {
    g(r#"Nat/s:"lt""#)
}
fn acc(n: Term) -> Term {
    apps(poly(r#"/s:"Acc""#), vec![g("Nat"), lt(), n])
}
fn well_founded_type() -> Term {
    apps(poly(r#"/s:"WellFounded""#), vec![g("Nat"), lt()])
}
fn goal(short: &str) -> Term {
    match short {
        "accCount" => pi("n", g("Nat"), pi("h", acc(v("n")), g("Nat"))),
        "accIsAType" => pi("n", g("Nat"), prop()),
        "ltWf" => well_founded_type(),
        "wfSum" => pi("n", g("Nat"), g("Nat")),
        _ => unreachable!(),
    }
}
fn annotated(value: Term, expected: Term) -> Term {
    Term::Let {
        name: "original_goal".into(),
        ty: Box::new(expected),
        value: Box::new(value),
        body: Box::new(v("original_goal")),
    }
}
fn checks(ctx: &Context, value: Term, expected: Term) {
    let term = annotated(value, expected);
    infer_type(ctx, &term).expect("main checker must check the actual expected goal");
    recheck(ctx, &term).expect("independent checker must check the actual expected goal");
}
fn rejects(ctx: &Context, value: Term, expected: Term) {
    let term = annotated(value, expected);
    let main = infer_type(ctx, &term);
    assert!(
        matches!(&main, Err(KernelError::TypeMismatch { .. })),
        "main must reject a type mismatch, got {main:?}"
    );
    let independent = recheck(ctx, &term);
    assert!(
        matches!(&independent, Err(ReCheckError::Ill(_))),
        "independent must reject as ill typed, got {independent:?}"
    );
}
fn clean(ctx: &Context, term: &Term) {
    let usage = axiom_dependencies(ctx, term);
    assert!(
        usage.axioms.is_empty(),
        "unexpected source axiom: {usage:?}"
    );
    assert!(
        usage.unresolved.is_empty(),
        "unresolved source dependency: {usage:?}"
    );
}
fn original(short: &str) -> (adapter::ImportedContext, Term) {
    assert_eq!(INPUT.len(), 263_814);
    assert_eq!(INPUT.lines().count(), 4_907);
    let p: Value = serde_json::from_str(PROVENANCE).unwrap();
    assert_eq!(
        p["source_sha256"],
        "21c0b33bcd73fd5a901f4c76b5fd0c2d5ebd079b5f9dde6ea2aeaa117497d877"
    );
    assert_eq!(
        p["export_sha256"],
        "a8d5e679d0fb82e43a954f5f3edcaa96d58538fafb0344ef8c86a433c833787f"
    );
    assert_eq!(p["toolchain"], "leanprover/lean4:v4.14.0");
    assert_eq!(
        p["exporter_commit"],
        "20c4a64aa1b76f7c79a6db15d089084359f8c3dc"
    );
    assert_eq!(p["expression_equality"], "structural");
    assert_eq!(p["declaration_selection"], "explicit_authored_roots");
    assert_eq!(
        p["declarations"],
        serde_json::json!(ROOTS.map(|s| format!("T04_015.{s}")))
    );
    assert_eq!(p["compiler_exit"], 0);
    assert_eq!(p["exporter_exit"], 0);
    assert_eq!(
        serde_json::from_str::<Value>(INPUT.lines().next().unwrap()).unwrap(),
        p["export_header"]
    );

    let target = name(short);
    let (ctx, report) = adapter::import_audited_retaining_source(INPUT, &target)
        .expect("the exact original four-root source must import");
    assert!(report.declaration_audit_requested);
    assert!(report.runtime_declarations.is_empty());
    // These fourteen annotations occur in original dependencies. Preserve the
    // complete declared-loss record; do not falsely assert metadata is absent.
    let mut actual: Vec<_> = report
        .metadata_losses
        .iter()
        .map(|m| (m.expression, m.key.as_str(), m.value.as_str()))
        .collect();
    actual.sort_unstable();
    let mut expected = vec![(466, "borrowed", "Lean.DataValue.ofBool true")];
    for id in [1916, 1919, 1939, 1941, 2515, 2523, 2531, 3034, 3038, 3070] {
        expected.push((id, "_inaccessible", "Lean.DataValue.ofBool true"));
    }
    for id in [4037, 4055, 4067] {
        expected.push((id, "noImplicitLambda", "Lean.DataValue.ofBool true"));
    }
    expected.sort_unstable();
    assert_eq!(
        actual, expected,
        "every original metadata loss must remain reported"
    );

    let mut authored: Vec<_> = report
        .declarations
        .iter()
        .filter(|d| ROOTS.iter().any(|s| d.name == name(s)))
        .map(|d| d.name.clone())
        .collect();
    authored.sort();
    assert_eq!(authored, ROOTS.map(name).to_vec());
    for (s, type_id, value_id) in [
        ("accCount", 771, 832),
        ("accIsAType", 833, 834),
        ("ltWf", 4116, 4120),
        ("wfSum", 2, 4267),
    ] {
        let row = report
            .declarations
            .iter()
            .find(|d| d.name == name(s))
            .unwrap();
        assert_eq!(row.kind, "def");
        assert_eq!(row.type_expression, type_id);
        assert_eq!(row.value_expression, Some(value_id));
        assert!(row.axioms.is_empty());
    }
    let usage = axiom_dependencies_of(&ctx, &target).unwrap();
    assert!(usage.axioms.is_empty());
    assert!(usage.unresolved.is_empty());
    let (retained, declared, body) = ctx.retained_source().expect("actual source body");
    assert_eq!(retained, target);
    assert_ne!(body, &g(&target));
    assert_eq!(ctx.get_global(&target), Some(declared));
    assert_eq!(ctx.get_definition_body(&target), Some(body));
    let expected = goal(short);
    let mut store = TermStore::new();
    for term in [body, declared, &expected] {
        let id = store.intern(term);
        assert!(store.free_names_are_empty(id));
        assert!(!store.shapes(id).contains(NodeShape::Hole));
    }
    assert!(defeq_for_test(&ctx, declared, &expected));
    checks(&ctx, body.clone(), expected);
    let body = body.clone();
    (ctx, body)
}
fn source_goal(short: &str) {
    let (ctx, body) = original(short);
    rejects(&ctx, body, g("Nat"));
}
fn accessibility(wf: Term, n: Term) -> Term {
    apps(
        poly(r#"/s:"WellFounded"/s:"apply""#),
        vec![g("Nat"), lt(), wf, n],
    )
}
fn computation(ctx: &Context, value: Term, expected: u64) {
    checks(ctx, value.clone(), g("Nat"));
    clean(ctx, &value);
    let equality = apps(
        poly(r#"/s:"Eq""#),
        vec![g("Nat"), value.clone(), nat(expected)],
    );
    let reflexivity = apps(poly(r#"/s:"Eq"/s:"refl""#), vec![g("Nat"), nat(expected)]);
    checks(ctx, reflexivity, equality);
    assert!(defeq_for_test(
        ctx,
        &normalize_checked(ctx, &value).expect("original computation must normalize"),
        &nat(expected)
    ));
}

#[test]
fn original_acc_count_has_its_actual_source_goal() {
    source_goal("accCount");
}
#[test]
fn original_acc_is_a_type_has_its_actual_source_goal() {
    source_goal("accIsAType");
}
#[test]
fn original_lt_wf_has_its_actual_structural_well_founded_goal() {
    source_goal("ltWf");
}
#[test]
fn original_wf_sum_has_its_actual_source_goal() {
    source_goal("wfSum");
}

// ==== Separate computational obligations; typing alone cannot close these ====

#[test]
fn original_acc_is_a_type_exposes_the_actual_nat_accessibility_family() {
    let (ctx, body) = original("accIsAType");
    let result = app(body, v("n"));
    let witness = lam(
        "n",
        g("Nat"),
        lam(
            "accessible",
            acc(v("n")),
            annotated(v("accessible"), result.clone()),
        ),
    );
    checks(
        &ctx,
        witness,
        pi("n", g("Nat"), pi("accessible", acc(v("n")), result)),
    );
}

#[test]
fn original_lt_wf_supplies_accessibility_through_the_imported_structure() {
    let (ctx, body) = original("ltWf");
    let applied = lam("n", g("Nat"), accessibility(body, v("n")));
    clean(&ctx, &applied);
    checks(&ctx, applied, pi("n", g("Nat"), acc(v("n"))));
}

#[test]
fn original_acc_count_recurses_through_actual_nat_lt_accessibility() {
    let (ctx, body) = original("accCount");
    for n in [0, 1, 3] {
        let proof = accessibility(g(&name("ltWf")), nat(n));
        checks(&ctx, proof.clone(), acc(nat(n)));
        computation(&ctx, apps(body.clone(), vec![nat(n), proof]), n);
    }
    rejects(&ctx, apps(body, vec![nat(0), nat(0)]), g("Nat"));
}

#[test]
fn original_wf_sum_computes_zero_one_and_three() {
    let (ctx, body) = original("wfSum");
    for (n, expected) in [(0, 0), (1, 1), (3, 6)] {
        computation(&ctx, app(body.clone(), nat(n)), expected);
    }
}
