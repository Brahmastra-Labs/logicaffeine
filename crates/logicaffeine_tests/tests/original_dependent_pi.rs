use logicaffeine_kernel::interface::lean as adapter;
use logicaffeine_kernel::term_store::{NodeShape, TermStore};
use logicaffeine_kernel::{defeq_for_test, infer_type, recheck, Context, Term, Universe};
fn name(case: &str, short: &str) -> String {
    format!(r#"/s:"{case}"/s:"{short}""#)
}
fn closed_exact(ctx: &Context, body: &Term, ty: &Term) {
    let mut store = TermStore::new();
    for t in [body, ty] {
        let id = store.intern(t);
        assert!(store.free_vars(id).is_empty());
        assert!(!store.shapes(id).contains(NodeShape::Hole));
    }
    assert!(defeq_for_test(ctx, &infer_type(ctx, body).unwrap(), ty));
    assert!(defeq_for_test(ctx, &recheck(ctx, body).unwrap(), ty));
}
fn free_in(t: &Term, name: &str) -> bool {
    let mut store = TermStore::new();
    let id = store.intern(t);
    store.free_vars(id).contains(&name)
}
#[test]
fn original_dependent_pi_preserves_all_four_binding_graphs() {
    let (ctx, report) = adapter::import_audited(include_str!(
        "fixtures/lean_conformance/cases_lean414/T00-004.ndjson"
    ))
    .unwrap();
    let rows: Vec<_> = report
        .declarations
        .iter()
        .filter(|d| d.name.starts_with(r#"/s:"T00_004"/"#))
        .collect();
    assert_eq!(rows.len(), 4);
    for short in [
        "dependentArrow",
        "plainArrow",
        "shadowedBinder",
        "usedOnlyInLaterBinderType",
    ] {
        let n = name("T00_004", short);
        let row = rows.iter().find(|d| d.name == n).unwrap();
        assert_eq!(row.kind, "def");
        assert!(row.axioms.is_empty());
        let body = ctx.get_definition_body(&n).unwrap();
        let ty = ctx.get_definition_type(&n).unwrap();
        closed_exact(&ctx, body, ty);
        assert_eq!(ty, &Term::Sort(Universe::Type(0)));
        let Term::Pi {
            param,
            param_type,
            body_type,
        } = body
        else {
            panic!("original Pi: {short}")
        };
        assert_eq!(param_type.as_ref(), &Term::Global("Nat".into()));
        if short == "plainArrow" {
            assert_eq!(body_type.as_ref(), &Term::Global("Nat".into()));
            assert!(!free_in(body_type, param));
        } else {
            assert_ne!(param, "_");
            let Term::Pi {
                param: inner,
                param_type: domain,
                body_type: codomain,
            } = body_type.as_ref()
            else {
                panic!("original inner Pi: {short}")
            };
            assert_ne!(param, inner, "source shadowing must be freshened");
            assert!(
                free_in(domain, param),
                "dependency in the later binder type"
            );
            assert_eq!(free_in(codomain, param), short == "dependentArrow");
        }
    }
}
