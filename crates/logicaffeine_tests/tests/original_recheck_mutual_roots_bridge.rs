//! Two corpus roots the independent re-checker used to refuse (`recheck_refused`, 54 roots):
//! `T02_014.forestSize` (structural recursion through Lean's generated `Forest.brecOn` for a
//! mutual block) and `T03_008.Labelled.binductionOn_1` (Lean's generated `binductionOn` for a
//! nested family). Both come from unchanged per-root exports of the corpus cases, taken with the
//! pinned Lean 4.14.0 + structural lean4export. The root cause was `recheck::open_mutual`
//! substituting mutual-block projections without shifting them under the remaining binders
//! (`recheck_open_mutual_controls`; `original_mutual_brecon_bridge` is the minimal witness).
use logicaffeine_kernel::interface::lean as adapter;
use logicaffeine_kernel::{
    axiom_dependencies_of, defeq_for_test, infer_type, normalize_checked, recheck, BigInt, Context,
    Literal, Term,
};
use serde_json::Value;

const FOREST: &str =
    include_str!("fixtures/lean_conformance/cases_lean414/T02-014.forestSize.ndjson");
const FOREST_PROVENANCE: &str =
    include_str!("fixtures/lean_conformance/cases_lean414/T02-014.forestSize.provenance.json");
const LABELLED: &str =
    include_str!("fixtures/lean_conformance/cases_lean414/T03-008.binductionOn_1.ndjson");
const LABELLED_PROVENANCE: &str =
    include_str!("fixtures/lean_conformance/cases_lean414/T03-008.binductionOn_1.provenance.json");

fn n(dotted: &str) -> String {
    dotted.split('.').map(|p| format!("/s:{p:?}")).collect()
}
fn g(dotted: &str) -> Term {
    Term::Global(n(dotted))
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn apps(f: Term, xs: Vec<Term>) -> Term {
    xs.into_iter().fold(f, app)
}
fn lit(k: u64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_u64(k)))
}

fn pinned(
    provenance: &str,
    source: &str,
    source_sha: &str,
    export_sha: &str,
    root: &str,
    input: &str,
) {
    let p: Value = serde_json::from_str(provenance).unwrap();
    assert_eq!(p["source"], source);
    assert_eq!(p["source_sha256"], source_sha);
    assert_eq!(p["export_sha256"], export_sha);
    assert_eq!(p["toolchain"], "leanprover/lean4:v4.14.0");
    assert_eq!(
        p["exporter_commit"],
        "20c4a64aa1b76f7c79a6db15d089084359f8c3dc"
    );
    assert_eq!(p["expression_equality"], "structural");
    assert_eq!(p["declarations"], serde_json::json!([root]));
    assert_eq!(
        (p["compiler_exit"].as_i64(), p["exporter_exit"].as_i64()),
        (Some(0), Some(0))
    );
    assert_eq!(p["records"], input.lines().count());
}

/// The root's retained body checks against its published type in both kernels, and Lean's
/// `#print axioms` (recorded in the ledger as none) matches the audit.
fn both_kernels(ctx: &Context, root: &str) -> Term {
    let name = n(root);
    let ty = ctx
        .get_global(&name)
        .or_else(|| ctx.get_universe_poly(&name).map(|(_, t, _)| t))
        .unwrap()
        .clone();
    let body = ctx
        .get_definition_body(&name)
        .or_else(|| ctx.get_universe_poly(&name).map(|(_, _, b)| b))
        .expect("a definition keeps its body")
        .clone();
    assert!(
        defeq_for_test(ctx, &infer_type(ctx, &body).unwrap(), &ty),
        "{root}: main kernel"
    );
    assert!(
        defeq_for_test(
            ctx,
            &recheck(ctx, &body).expect("the independent kernel accepts"),
            &ty
        ),
        "{root}: independent kernel"
    );
    let usage = axiom_dependencies_of(ctx, &name).unwrap();
    assert!(
        usage.axioms.is_empty() && usage.unresolved.is_empty(),
        "{root}: {usage:?}"
    );
    body
}

#[test]
fn original_forest_size_recurses_through_leans_mutual_brecon() {
    pinned(FOREST_PROVENANCE, "crates/logicaffeine_tests/tests/fixtures/lean_conformance/cases/t02_declarations/T02-014_mutual_def.lean",
        "fef1274435ce7a3ff99880a1345f33f7faf1b13a68bc96ebe3b1510b5588bcf1",
        "4e397981b2668aea98a41bdb7c3fd9d228167b57da1e4c4c42a13064583260be", "T02_014.forestSize", FOREST);
    let (ctx, report) =
        adapter::import_audited(FOREST).expect("the audited import re-checks Forest.brecOn");
    for name in [
        "T02_014.forestSize",
        "T02_014.Forest.brecOn",
        "T02_014.Tree.brecOn",
    ] {
        if let Some(audit) = report.declarations.iter().find(|d| d.name == n(name)) {
            assert!(audit.axioms.is_empty(), "{name}: {:?}", audit.axioms);
        }
    }
    both_kernels(&ctx, "T02_014.forestSize");
    // forestSize [node nil, node nil] = 2, and not 3.
    let leaf = app(g("T02_014.Tree.node"), g("T02_014.Forest.nil"));
    let forest = apps(
        g("T02_014.Forest.cons"),
        vec![
            leaf.clone(),
            apps(
                g("T02_014.Forest.cons"),
                vec![leaf, g("T02_014.Forest.nil")],
            ),
        ],
    );
    let size = normalize_checked(&ctx, &app(g("T02_014.forestSize"), forest)).unwrap();
    assert!(defeq_for_test(&ctx, &size, &lit(2)), "{size}");
    assert!(!defeq_for_test(&ctx, &size, &lit(3)));
}

#[test]
fn original_labelled_binduction_on_is_accepted_by_both_kernels() {
    pinned(LABELLED_PROVENANCE, "crates/logicaffeine_tests/tests/fixtures/lean_conformance/cases/t03_inductives/T03-008_nested_inductive_list.lean",
        "0a910fbb844348f5407d63db313402b33abad4840543825e777505ec9541016b",
        "63038000846bd5056ffcc28cc3a69b5ccea89ad3740b2ba85064230a2732eb16",
        "T03_008.Labelled.binductionOn_1", LABELLED);
    let (ctx, _) =
        adapter::import_audited(LABELLED).expect("the audited import re-checks binductionOn_1");
    let body = both_kernels(&ctx, "T03_008.Labelled.binductionOn_1");
    // Its body at a different declared type is refused by both kernels.
    let wrong = Term::Let {
        name: "claimed".into(),
        ty: Box::new(Term::Global("Nat".into())),
        value: Box::new(body),
        body: Box::new(Term::Var("claimed".into())),
    };
    assert!(infer_type(&ctx, &wrong).is_err());
    assert!(recheck(&ctx, &wrong).is_err());
}
