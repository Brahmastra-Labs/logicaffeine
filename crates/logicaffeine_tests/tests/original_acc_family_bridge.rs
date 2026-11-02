//! Additive proposal: actual original T04_015.accIsAType, not a rebuilt definition.
//! Existing original_well_founded_bridge tests and failed assertions stay unchanged.
use logicaffeine_kernel::interface::lean as adapter;
use logicaffeine_kernel::term_store::{NodeShape, TermStore};
use logicaffeine_kernel::{
    axiom_dependencies, axiom_dependencies_of, infer_type, recheck, Context, KernelError,
    ReCheckError, Term, Universe,
};
use serde_json::Value;
const INPUT: &str = include_str!("fixtures/lean_conformance/cases_lean414/T04-015.four.ndjson");
const PROVENANCE: &str =
    include_str!("fixtures/lean_conformance/cases_lean414/T04-015.four.provenance.json");
const ROOT: &str = r#"/s:"T04_015"/s:"accIsAType""#;
// Importer::name maps the Nat prefix, retaining the structured suffix.
const NAT_LT: &str = r#"Nat/s:"lt""#;
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
fn acc(n: Term) -> Term {
    apps(
        Term::Const {
            name: r#"/s:"Acc""#.into(),
            levels: vec![Universe::Type(0)],
        },
        vec![g("Nat"), g(NAT_LT), n],
    )
}
fn annotated(value: Term, expected: Term) -> Term {
    Term::Let {
        name: "checked_original".into(),
        ty: Box::new(expected),
        value: Box::new(value),
        body: Box::new(v("checked_original")),
    }
}
fn checks(ctx: &Context, value: Term, expected: Term) {
    let term = annotated(value, expected);
    infer_type(ctx, &term).expect("main actual annotated goal");
    recheck(ctx, &term).expect("independent actual annotated goal");
}
fn rejects(ctx: &Context, term: Term) {
    let main = infer_type(ctx, &term);
    let independent = recheck(ctx, &term);
    assert!(
        matches!(main, Err(KernelError::TypeMismatch { .. })),
        "{main:?}"
    );
    assert!(
        matches!(independent, Err(ReCheckError::Ill(_))),
        "{independent:?}"
    );
}
fn source() -> (adapter::ImportedContext, Term) {
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
        serde_json::json!([
            "T04_015.accCount",
            "T04_015.accIsAType",
            "T04_015.ltWf",
            "T04_015.wfSum"
        ])
    );
    assert_eq!(p["compiler_exit"], 0);
    assert_eq!(p["exporter_exit"], 0);
    let rows: Vec<Value> = INPUT
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(rows[0], p["export_header"]);
    assert_eq!(p["records"], rows.len());
    let (ctx, report) = adapter::import_audited_retaining_source(INPUT, ROOT)
        .expect("unchanged original four-root fixture imports");
    assert!(report.declaration_audit_requested);
    assert!(report.runtime_declarations.is_empty());
    // Verify all original metadata losses rather than silently claiming lossless translation.
    let mut expected = Vec::new();
    for row in &rows {
        if let Some(m) = row.get("mdata") {
            for (key, value) in m["data"].as_object().unwrap() {
                expected.push((
                    row["ie"].as_u64().unwrap() as usize,
                    key.clone(),
                    value.as_str().unwrap().to_owned(),
                ));
            }
        }
    }
    expected.sort();
    assert_eq!(expected.len(), 14);
    let mut actual: Vec<_> = report
        .metadata_losses
        .iter()
        .map(|m| (m.expression, m.key.clone(), m.value.clone()))
        .collect();
    actual.sort();
    assert_eq!(actual, expected);
    let selected: Vec<_> = report
        .declarations
        .iter()
        .filter(|d| d.name == ROOT)
        .collect();
    assert_eq!(selected.len(), 1);
    let row = selected[0];
    assert_eq!(row.kind, "def");
    assert_eq!(row.type_expression, 833);
    assert_eq!(row.value_expression, Some(834));
    assert!(row.axioms.is_empty());
    let audit = axiom_dependencies_of(&ctx, ROOT).unwrap();
    assert!(audit.axioms.is_empty());
    assert!(audit.unresolved.is_empty());
    // Verify the public registry spelling before constructing the independent family.
    assert!(
        ctx.get_global(NAT_LT).is_some(),
        "actual importer Nat.lt identity"
    );
    let (name, declared, body) = ctx.retained_source().unwrap();
    assert_eq!(name, ROOT);
    assert_ne!(body, &g(ROOT));
    assert_eq!(ctx.get_global(ROOT), Some(declared));
    assert_eq!(ctx.get_definition_body(ROOT), Some(body));
    let goal = pi("n", g("Nat"), prop());
    let family = lam("n", g("Nat"), acc(v("n")));
    let mut store = TermStore::new();
    for term in [declared, body, &goal, &family] {
        let id = store.intern(term);
        assert!(store.free_names_are_empty(id));
        assert!(!store.shapes(id).contains(NodeShape::Hole));
        let audit = axiom_dependencies(&ctx, term);
        assert!(audit.axioms.is_empty());
        assert!(audit.unresolved.is_empty());
    }
    checks(&ctx, body.clone(), declared.clone());
    checks(&ctx, body.clone(), goal.clone());
    checks(&ctx, family, goal);
    let body = body.clone();
    (ctx, body)
}
#[test]
fn original_acc_family_retains_source_goal_and_rejects_wrong_goal() {
    let (ctx, body) = source();
    rejects(&ctx, annotated(body, pi("n", g("Nat"), g("Nat"))));
}
#[test]
fn original_acc_family_is_the_actual_indexed_accessibility_family_in_both_kernels() {
    let (ctx, body) = source();
    let actual = app(body, v("n"));
    let expected = acc(v("n"));
    // Both directions force conversion of the actual source result with Acc Nat.lt n.
    // The accessible proof is a local assumption, not a new Context axiom or hand proof.
    let forward = lam(
        "n",
        g("Nat"),
        lam(
            "accessible",
            expected.clone(),
            annotated(v("accessible"), actual.clone()),
        ),
    );
    checks(
        &ctx,
        forward,
        pi(
            "n",
            g("Nat"),
            pi("accessible", expected.clone(), actual.clone()),
        ),
    );
    let backward = lam(
        "n",
        g("Nat"),
        lam(
            "original_accessible",
            actual.clone(),
            annotated(v("original_accessible"), expected.clone()),
        ),
    );
    checks(
        &ctx,
        backward,
        pi("n", g("Nat"), pi("original_accessible", actual, expected)),
    );
}
#[test]
fn original_acc_family_rejects_changed_index_and_non_nat_argument() {
    let (ctx, body) = source();
    let shifted = app(body.clone(), app(g("Succ"), v("n")));
    let wrong_index = lam(
        "n",
        g("Nat"),
        lam(
            "accessible",
            acc(v("n")),
            annotated(v("accessible"), shifted),
        ),
    );
    rejects(&ctx, wrong_index);
    // The unchanged source domain is Nat, not an arbitrary sort or proposition.
    rejects(&ctx, app(body, prop()));
}
