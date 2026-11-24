//! Exact original T04_015.accDepth source, with independent goal and computation checks.
//! No bridge-ledger promotion and no replacement prelude combinator are part of this test.
use logicaffeine_kernel::interface::lean as adapter;

use logicaffeine_kernel::term_store::{NodeShape, TermStore};
use logicaffeine_kernel::{
    axiom_dependencies, axiom_dependencies_of, defeq_for_test, infer_type, normalize_checked,
    recheck, BigInt, Context, KernelError, Literal, ReCheckError, Term, Universe,
};
use serde_json::Value;

const INPUT: &str = include_str!("fixtures/lean_conformance/cases_lean414/T04-015.accDepth.ndjson");
const PROVENANCE: &str =
    include_str!("fixtures/lean_conformance/cases_lean414/T04-015.accDepth.provenance.json");
const ROOT: &str = r#"/s:"T04_015"/s:"accDepth""#;

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
fn prop() -> Term {
    Term::Sort(Universe::Prop)
}
fn zero() -> Term {
    Term::Lit(Literal::Nat(BigInt::from_u64(0)))
}
fn acc(a: Term, r: Term, x: Term) -> Term {
    apps(
        Term::Const {
            name: r#"/s:"Acc""#.into(),
            levels: vec![Universe::Type(0)],
        },
        vec![a, r, x],
    )
}
fn original_goal() -> Term {
    // Exact source: {α : Type} {r : α → α → Prop} {x : α} → Acc r x → Nat.
    // These are term parameters; the authored declaration has NO level parameters.
    pi(
        "alpha",
        type0(),
        pi(
            "relation",
            pi("y", v("alpha"), pi("x", v("alpha"), prop())),
            pi(
                "point",
                v("alpha"),
                pi(
                    "accessible",
                    acc(v("alpha"), v("relation"), v("point")),
                    g("Nat"),
                ),
            ),
        ),
    )
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
    infer_type(ctx, &term).expect("main checker must validate the actual goal");
    recheck(ctx, &term).expect("independent checker must validate the actual goal");
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
        "independent checker must reject as ill typed, got {independent:?}"
    );
}
fn original() -> (adapter::ImportedContext, Term) {
    assert_eq!(INPUT.len(), 16_292);
    assert_eq!(INPUT.lines().count(), 300);
    let p: Value = serde_json::from_str(PROVENANCE).unwrap();
    assert_eq!(p["case"], "T04-015");
    assert_eq!(
        p["source_sha256"],
        "21c0b33bcd73fd5a901f4c76b5fd0c2d5ebd079b5f9dde6ea2aeaa117497d877"
    );
    assert_eq!(
        p["export_sha256"],
        "261c066a18ed4c6dba61c2e8b3bf846d22d2072bbdafe0765ad17271a6c26378"
    );
    assert_eq!(p["toolchain"], "leanprover/lean4:v4.14.0");
    assert_eq!(
        p["exporter_commit"],
        "20c4a64aa1b76f7c79a6db15d089084359f8c3dc"
    );
    assert_eq!(p["expression_equality"], "structural");
    assert_eq!(p["declaration_selection"], "explicit_authored_roots");
    assert_eq!(p["declarations"], serde_json::json!(["T04_015.accDepth"]));
    assert_eq!(p["compiler_exit"], 0);
    assert_eq!(p["exporter_exit"], 0);
    assert_eq!(
        serde_json::from_str::<Value>(INPUT.lines().next().unwrap()).unwrap(),
        p["export_header"]
    );

    let (ctx, report) = adapter::import_audited_retaining_source(INPUT, ROOT)
        .expect("the unchanged original export must import");
    assert!(report.declaration_audit_requested);
    assert!(report.metadata_losses.is_empty());
    assert!(report.runtime_declarations.is_empty());
    let rows: Vec<_> = report
        .declarations
        .iter()
        .filter(|d| d.name.starts_with(r#"/s:"T04_015"/"#))
        .collect();
    assert_eq!(rows.len(), 1, "only the selected authored root is counted");
    let row = rows[0];
    assert_eq!(row.name, ROOT);
    assert_eq!(row.kind, "def");
    assert_eq!(row.type_expression, 178);
    assert_eq!(row.value_expression, Some(231));
    assert!(row.axioms.is_empty());
    let usage = axiom_dependencies_of(&ctx, ROOT).unwrap();
    assert!(usage.axioms.is_empty());
    assert!(usage.unresolved.is_empty());

    let (name, declared, body) = ctx.retained_source().expect("actual source retention");
    assert_eq!(name, ROOT);
    assert_ne!(body, &g(ROOT));
    assert_eq!(ctx.get_definition_body(ROOT), Some(body));
    assert_eq!(ctx.get_global(ROOT), Some(declared));
    let expected = original_goal();
    let mut store = TermStore::new();
    for term in [declared, body, &expected] {
        let id = store.intern(term);
        assert!(store.free_names_are_empty(id));
        assert!(!store.shapes(id).contains(NodeShape::Hole));
    }
    assert!(defeq_for_test(&ctx, declared, &expected));
    checks(&ctx, body.clone(), expected);
    let body = body.clone();
    (ctx, body)
}

// ==== An actual accessibility proof, using only imported declarations ====

// Empty := ∀ P:Prop, P. This is a proposition, not an added axiom or declaration.
fn empty_proposition() -> Term {
    pi("P", prop(), v("P"))
}
fn empty_relation() -> Term {
    lam("y", g("Nat"), lam("x", g("Nat"), empty_proposition()))
}
fn accessible_zero() -> Term {
    let r = empty_relation();
    let field = lam(
        "y",
        g("Nat"),
        lam(
            "impossible",
            apps(r.clone(), vec![v("y"), zero()]),
            app(v("impossible"), acc(g("Nat"), r.clone(), v("y"))),
        ),
    );
    apps(
        Term::Const {
            name: r#"/s:"Acc"/s:"intro""#.into(),
            levels: vec![Universe::Type(0)],
        },
        vec![g("Nat"), r, zero(), field],
    )
}

#[test]
fn original_acc_depth_retains_its_closed_exact_source_goal_without_axioms() {
    let (ctx, body) = original();
    // Wrong claimed theorem types are rejected by each checker, not compared
    // afterward through a shared main-kernel conversion helper.
    rejects(&ctx, body, g("Nat"));
}

#[test]
fn original_acc_depth_computes_zero_on_a_checked_accessibility_proof() {
    let (ctx, body) = original();
    let r = empty_relation();
    let proof = accessible_zero();
    checks(&ctx, proof.clone(), acc(g("Nat"), r.clone(), zero()));
    let usage = axiom_dependencies(&ctx, &proof);
    assert!(usage.axioms.is_empty());
    assert!(usage.unresolved.is_empty());

    let observed = apps(body.clone(), vec![g("Nat"), r.clone(), zero(), proof]);
    checks(&ctx, observed.clone(), g("Nat"));
    let reduced = normalize_checked(&ctx, &observed).expect("original Acc.rec computation");
    assert!(defeq_for_test(&ctx, &reduced, &zero()));

    // Eq is not among this minimal original export's dependencies. An abstract
    // indexed family instead demands conversion observed = 0 in BOTH kernels.
    let at_zero = app(v("Family"), zero());
    let at_result = app(v("Family"), observed.clone());
    let observation = lam(
        "Family",
        pi("n", g("Nat"), type0()),
        lam(
            "at_zero",
            at_zero.clone(),
            annotated(v("at_zero"), at_result.clone()),
        ),
    );
    let observation_goal = pi(
        "Family",
        pi("n", g("Nat"), type0()),
        pi("at_zero", at_zero, at_result),
    );
    checks(&ctx, observation, observation_goal);

    // A Nat value is not an accessibility proof; preserve the actual final argument.
    rejects(
        &ctx,
        apps(body, vec![g("Nat"), r, zero(), zero()]),
        g("Nat"),
    );
}

#[test]
fn replacing_the_original_definition_body_by_nat_is_rejected() {
    let (ctx, _) = original();
    rejects(&ctx, g("Nat"), original_goal());
    let mut rows: Vec<Value> = INPUT
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    // Source expression 95 is the actual original Nat constant, not a new ID.
    assert_eq!(
        rows.iter().find(|r| r["ie"] == 95).unwrap()["const"],
        serde_json::json!({"name": 24, "us": []})
    );
    let definition = rows.iter_mut().find(|r| r["def"]["name"] == 46).unwrap();
    assert_eq!(definition["def"]["type"], 178);
    assert_eq!(definition["def"]["value"], 231);
    definition["def"]["value"] = serde_json::json!(95);
    let malformed = rows
        .iter()
        .map(|r| serde_json::to_string(r).unwrap())
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    match adapter::import_audited(&malformed) {
        Err(adapter::Verdict::Reject(_)) => {}
        Err(other) => panic!("a mistyped body must be rejected, not merely declined: {other:?}"),
        Ok(_) => panic!("a Nat type cannot replace the original function's body"),
    }
}

#[test]
fn original_acc_depth_bridge_row_names_the_verified_original_declaration() {
    let ledger: Value = serde_json::from_str(include_str!(
        "fixtures/lean_conformance/bridge/T04-015.json"
    ))
    .unwrap();
    assert_eq!(ledger["case"], "T04-015");
    assert_eq!(ledger["lean"], "4.14.0");
    assert_eq!(ledger["lean_compiles"], true);
    let rows = ledger["declarations"].as_array().unwrap();
    let matching: Vec<_> = rows
        .iter()
        .filter(|row| row["lean"] == "T04_015.accDepth")
        .collect();
    assert_eq!(matching.len(), 1);
    let row = matching[0];
    assert_eq!(row["status"], "bridged");
    assert_eq!(row["lean_kind"], "def");
    assert_eq!(row["lean_levels"], serde_json::json!([]));
    assert_eq!(row["lean_axioms"], serde_json::json!([]));
    assert_eq!(row["logos"], ROOT);
    assert_eq!(row["test"], "crates/logicaffeine_tests/tests/original_acc_depth_bridge.rs::original_acc_depth_retains_its_closed_exact_source_goal_without_axioms");
}
