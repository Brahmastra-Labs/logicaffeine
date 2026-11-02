use logicaffeine_kernel::interface::lean as adapter;
use logicaffeine_kernel::term_store::{NodeShape, TermStore};
use logicaffeine_kernel::{defeq_for_test, infer_type, recheck, Term};

const INPUT: &str = include_str!("fixtures/lean_conformance/cases_lean414/T00-014.ndjson");
fn name(short: &str) -> String {
    format!("/s:\"T00_014\"/s:\"{short}\"")
}
fn closed_exact(ctx: &logicaffeine_kernel::Context, body: &Term, ty: &Term) {
    let mut store = TermStore::new();
    for term in [body, ty] {
        let id = store.intern(term);
        assert!(
            store.free_vars(id).is_empty(),
            "original artifact has free variables"
        );
        assert!(
            !store.shapes(id).contains(NodeShape::Hole),
            "original artifact contains a hole"
        );
    }
    assert!(defeq_for_test(ctx, &infer_type(ctx, body).unwrap(), ty));
    assert!(defeq_for_test(ctx, &recheck(ctx, body).unwrap(), ty));
}
#[test]
fn original_eta_in_type_uses_conversion_without_rewriting_the_source_body() {
    let (ctx, report) = adapter::import_audited(INPUT).unwrap();
    let n = name("etaInType");
    let evidence = report.declarations.iter().find(|d| d.name == n).unwrap();
    assert_eq!(evidence.kind, "def");
    assert!(evidence.axioms.is_empty(), "{:?}", evidence.axioms);
    let body = ctx.get_definition_body(&n).unwrap();
    let ty = ctx.get_definition_type(&n).unwrap();
    closed_exact(&ctx, body, ty);
    let Term::Lambda { body: second, .. } = body else {
        panic!("original F binder")
    };
    let Term::Lambda { body: third, .. } = second.as_ref() else {
        panic!("original f binder")
    };
    let Term::Lambda {
        param,
        body: result,
        ..
    } = third.as_ref()
    else {
        panic!("original x binder")
    };
    assert_eq!(
        result.as_ref(),
        &Term::Var(param.clone()),
        "eta needs no body repair"
    );
}
#[test]
fn original_function_extensionality_retains_its_actual_proof_and_exact_logical_axiom() {
    let n = name("needsFunext");
    let (ctx, report) = adapter::import_audited_retaining_source(INPUT, &n).unwrap();
    let (retained, ty, body) = ctx.retained_source().unwrap();
    assert_eq!(retained, n);
    closed_exact(&ctx, body, ty);
    let evidence = report.declarations.iter().find(|d| d.name == n).unwrap();
    assert_eq!(evidence.kind, "thm");
    assert_eq!(
        evidence.axioms,
        vec![r#"/s:"Quot"/s:"sound""#],
        "certified quotient computation rules are primitives; Lean reports only Quot.sound"
    );
}
