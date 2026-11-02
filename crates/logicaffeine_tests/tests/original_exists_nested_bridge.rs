//! Exact single-root export with structural equality and original metadata.
//! Actual original nested-existential body; no reconstructed replacement theorem.
use logicaffeine_kernel::interface::lean as adapter;
use logicaffeine_kernel::term_store::{NodeShape, TermStore};
use logicaffeine_kernel::{
    axiom_dependencies, axiom_dependencies_of, infer_type, normalize_checked, recheck, BigInt,
    Context, KernelError, Literal, ReCheckError, Term, Universe,
};
use serde_json::Value;
const INPUT: &str =
    include_str!("fixtures/lean_conformance/cases_lean414/T06-006.existsNested.ndjson");
const PROVENANCE: &str =
    include_str!("fixtures/lean_conformance/cases_lean414/T06-006.existsNested.provenance.json");
// Fixture SHA-256: de7a43edde78c65d3989301c4d540b2ca7eb57b6ddeafbd1eddab968a00c2bb4.
const TARGET: &str = r#"/s:"T06_006"/s:"existsNested""#;
fn g(s: &str) -> Term {
    Term::Global(s.into())
}
fn v(s: &str) -> Term {
    Term::Var(s.into())
}
fn prop() -> Term {
    Term::Sort(Universe::Prop)
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn apps(f: Term, a: Vec<Term>) -> Term {
    a.into_iter().fold(f, app)
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
fn nat(n: u64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_u64(n)))
}
fn constant(name: &str) -> Term {
    Term::Const {
        name: name.into(),
        levels: vec![Universe::Type(0)],
    }
}
fn exists(p: Term) -> Term {
    apps(constant(r#"/s:"Exists""#), vec![g("Nat"), p])
}
fn eq(a: Term, x: Term, y: Term) -> Term {
    apps(constant(r#"/s:"Eq""#), vec![a, x, y])
}
fn original_goal_with_sum(sum: u64) -> Term {
    exists(lam(
        "m",
        g("Nat"),
        exists(lam(
            "n",
            g("Nat"),
            eq(
                g("Nat"),
                apps(g(r#"Nat/s:"add""#), vec![v("m"), v("n")]),
                nat(sum),
            ),
        )),
    ))
}
fn original_goal() -> Term {
    original_goal_with_sum(5)
}
fn annotated(value: Term, ty: Term) -> Term {
    Term::Let {
        name: "source_goal".into(),
        ty: Box::new(ty),
        value: Box::new(value),
        body: Box::new(v("source_goal")),
    }
}
fn checks(c: &Context, value: Term, ty: Term) {
    let t = annotated(value, ty);
    infer_type(c, &t).expect("main actual expected-goal check");
    recheck(c, &t).expect("independent actual expected-goal check");
}
fn rejects(c: &Context, t: &Term) {
    let main = infer_type(c, t);
    let independent = recheck(c, t);
    assert!(
        matches!(main, Err(KernelError::TypeMismatch { .. })),
        "{main:?}"
    );
    assert!(
        matches!(independent, Err(ReCheckError::Ill(_))),
        "{independent:?}"
    );
}
fn load() -> (adapter::ImportedContext, Term) {
    let p: Value = serde_json::from_str(PROVENANCE).unwrap();
    assert_eq!(
        p["source_sha256"],
        "3a71d809ab8587b8f09c94420eb6fdd41ed4e416ec686be92b0ff6741dcd2b44"
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
        serde_json::json!(["T06_006.existsNested"])
    );
    assert_eq!(p["compiler_exit"], 0);
    assert_eq!(p["exporter_exit"], 0);
    assert_eq!(INPUT.len(), 38887);
    assert_eq!(
        p["export_sha256"],
        "de7a43edde78c65d3989301c4d540b2ca7eb57b6ddeafbd1eddab968a00c2bb4"
    );
    assert_eq!(p["records"], 677);
    let rows: Vec<Value> = INPUT
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(rows[0], p["export_header"]);
    assert_eq!(rows.len(), p["records"].as_u64().unwrap() as usize);
    let (ctx, report) = adapter::import_audited_retaining_source(INPUT, TARGET)
        .expect("exact original source import");
    assert!(report.declaration_audit_requested);
    assert!(report.runtime_declarations.is_empty());
    // Independent source metadata inventory: do not guess that annotations are absent.
    let mut expected = Vec::new();
    for row in &rows {
        if let Some(m) = row.get("mdata") {
            for (key, val) in m["data"].as_object().unwrap() {
                expected.push((
                    row["ie"].as_u64().unwrap() as usize,
                    key.clone(),
                    val.as_str().unwrap().to_string(),
                ));
            }
        }
    }
    expected.sort();
    let mut actual: Vec<_> = report
        .metadata_losses
        .iter()
        .map(|m| (m.expression, m.key.clone(), m.value.clone()))
        .collect();
    actual.sort();
    assert_eq!(actual, expected);
    let roots: Vec<_> = report
        .declarations
        .iter()
        .filter(|d| d.name.starts_with(r#"/s:"T06_006"/"#))
        .collect();
    assert_eq!(roots.len(), 1);
    assert_eq!(roots[0].name, TARGET);
    assert_eq!(roots[0].kind, "thm");
    assert_eq!(roots[0].value_expression, Some(529));
    assert!(roots[0].axioms.is_empty());
    let usage = axiom_dependencies_of(&ctx, TARGET).unwrap();
    assert!(usage.axioms.is_empty());
    assert!(usage.unresolved.is_empty());
    let (name, declared, body) = ctx.retained_source().unwrap();
    assert_eq!(name, TARGET);
    assert_ne!(body, &g(TARGET));
    assert_eq!(ctx.get_global(TARGET), Some(declared));
    let expected = original_goal();
    let mut store = TermStore::new();
    for t in [body, declared, &expected] {
        let id = store.intern(t);
        assert!(store.free_names_are_empty(id));
        assert!(!store.shapes(id).contains(NodeShape::Hole));
        let usage = axiom_dependencies(&ctx, t);
        assert!(usage.axioms.is_empty());
        assert!(usage.unresolved.is_empty());
    }
    checks(&ctx, body.clone(), declared.clone());
    checks(&ctx, body.clone(), expected.clone());
    let body = body.clone();
    (ctx, body)
}
fn spine(t: &Term) -> (&Term, Vec<&Term>) {
    let mut head = t;
    let mut args = Vec::new();
    while let Term::App(f, a) = head {
        args.push(a.as_ref());
        head = f.as_ref();
    }
    args.reverse();
    (head, args)
}
fn intro_args(t: &Term) -> (&Term, Vec<&Term>) {
    let (head, args) = spine(t);
    assert!(matches!(head,Term::Const{name,levels}
        if name==r#"/s:"Exists"/s:"intro""# && levels==&vec![Universe::Type(0)]));
    assert_eq!(
        args.len(),
        4,
        "all original Exists.intro arguments remain explicit"
    );
    (head, args)
}
fn assert_numeric(c: &Context, actual: &Term, wanted: u64) {
    let actual = normalize_checked(c, actual).expect("original numeric witness computes");
    let expected = normalize_checked(c, &nat(wanted)).unwrap();
    let mut store = TermStore::new();
    let a = store.intern(&actual);
    let b = store.intern(&expected);
    assert!(
        store.alpha_same(a, b),
        "actual retained constructor witness differs"
    );
}
#[test]
fn original_exists_nested_checks_source_and_independent_goal() {
    let _ = load();
}
#[test]
fn original_exists_nested_rejects_wrong_goal_in_both_kernels() {
    let (ctx, body) = load();
    rejects(&ctx, &annotated(body.clone(), g("Nat")));
    rejects(&ctx, &annotated(body, original_goal_with_sum(6)));
}
#[test]
fn original_exists_nested_preserves_both_witnesses_and_rejects_mutation() {
    let (ctx, body) = load();
    let reduced = normalize_checked(&ctx, &body).expect("actual retained original proof reduces");
    checks(&ctx, reduced.clone(), original_goal());
    let (outer, oa) = intro_args(&reduced);
    assert!(
        matches!(oa[1], Term::Lambda { .. }),
        "outer predicate is one function argument"
    );
    assert_numeric(&ctx, oa[2], 2);
    let (inner, ia) = intro_args(oa[3]);
    assert!(
        matches!(ia[1], Term::Lambda { .. }),
        "inner predicate is one function argument"
    );
    assert_numeric(&ctx, ia[2], 3);
    // Preserve the actual original predicates and proof. Only change its inner
    // numeric witness; no independently rebuilt proof can mask a wrong result.
    let mut inner_args: Vec<Term> = ia.iter().map(|x| (*x).clone()).collect();
    inner_args[2] = nat(4);
    let bad_inner = apps(inner.clone(), inner_args);
    let mut outer_args: Vec<Term> = oa.iter().map(|x| (*x).clone()).collect();
    outer_args[3] = bad_inner;
    rejects(
        &ctx,
        &annotated(apps(outer.clone(), outer_args), original_goal()),
    );
    let mut wrong_kind: Vec<Term> = oa.iter().map(|x| (*x).clone()).collect();
    wrong_kind[2] = Term::Sort(Universe::Type(0));
    rejects(
        &ctx,
        &annotated(apps(outer.clone(), wrong_kind), original_goal()),
    );
}
