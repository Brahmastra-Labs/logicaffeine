use logicaffeine_kernel::interface::lean as adapter;
use logicaffeine_kernel::term_store::{NodeShape, TermStore};
use logicaffeine_kernel::{
    defeq_for_test, infer_type, normalize_checked, recheck, BigInt, Context, Literal, Term,
    Universe,
};
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
#[test]
fn original_dependent_lets_replay_all_seven_actual_bodies_and_axiom_sets() {
    let (ctx, report) = adapter::import_audited(include_str!(
        "fixtures/lean_conformance/cases_lean414/T00-017.ndjson"
    ))
    .unwrap();
    let rows = &report.declarations;
    for (short, value, has_quotient_proof) in [
        ("chainedDependency", 2, true),
        ("finAtLet", 2, true),
        ("letOfProof", 4, true),
        ("letOfTypeDependent", 5, false),
        ("letUnderBinder", 3, true),
        ("nonDependentControl", 6, false),
        ("resultTypeDependsOnLet", 2, true),
    ] {
        let n = name("T00_017", short);
        let row = rows.iter().find(|d| d.name == n).unwrap();
        assert_eq!(row.kind, "def");
        let expected = if has_quotient_proof {
            vec![
                r#"/s:"Quot"/s:"sound""#.to_string(),
                r#"/s:"propext""#.to_string(),
            ]
        } else {
            vec![]
        };
        assert_eq!(row.axioms, expected, "{short}");
        let body = ctx.get_definition_body(&n).unwrap();
        let ty = ctx.get_definition_type(&n).unwrap();
        closed_exact(&ctx, body, ty);
        let mut observed = Term::Global(n);
        if short == "letUnderBinder" {
            observed = Term::App(
                Box::new(observed),
                Box::new(Term::Lit(Literal::Nat(BigInt::from_u64(7)))),
            );
        }
        if short == "resultTypeDependsOnLet" {
            let reduced = normalize_checked(&ctx, &observed).unwrap();
            let mut args = Vec::new();
            let mut head = &reduced;
            while let Term::App(f, a) = head {
                args.push(a.as_ref());
                head = f;
            }
            args.reverse();
            assert!(matches!(head,Term::Global(n) if n==r#"/s:"Fin"/s:"mk""#));
            assert_eq!(args.len(), 3);
            assert!(defeq_for_test(
                &ctx,
                args[0],
                &Term::Lit(Literal::Nat(BigInt::from_u64(3)))
            ));
            observed = args[1].clone();
        }
        assert!(infer_type(&ctx, &observed).is_ok(), "{short}");
        assert!(
            defeq_for_test(
                &ctx,
                &normalize_checked(&ctx, &observed).unwrap(),
                &Term::Lit(Literal::Nat(BigInt::from_u64(value)))
            ),
            "{short}"
        );
    }
}
#[test]
fn original_type_let_cannot_be_replaced_by_an_opaque_lambda_parameter() {
    let (ctx, _) = adapter::import_audited(include_str!(
        "fixtures/lean_conformance/cases_lean414/T00-017.ndjson"
    ))
    .unwrap();
    let body = ctx
        .get_definition_body(&name("T00_017", "letOfTypeDependent"))
        .unwrap();
    let Term::Let {
        name,
        ty,
        value,
        body,
    } = body
    else {
        panic!("source let must remain a let")
    };
    let wrong = Term::App(
        Box::new(Term::Lambda {
            param: name.clone(),
            param_type: ty.clone(),
            body: body.clone(),
        }),
        value.clone(),
    );
    assert!(
        infer_type(&ctx, &wrong).is_err(),
        "dependent body cannot be typed with its type alias opaque"
    );
    assert!(
        recheck(&ctx, &wrong).is_err(),
        "second kernel must reject the same changed binding graph"
    );
}
