use logicaffeine_kernel::interface::lean as adapter;
use logicaffeine_kernel::{defeq_for_test, infer_type, normalize_checked, recheck, Literal, Term};
use std::collections::HashSet;

fn hygiene(term: &Term, scope: &mut HashSet<String>) {
    match term {
        Term::Var(n) => assert!(
            !n.starts_with('?') && scope.contains(n),
            "unbound or metavariable-like binder {n}"
        ),
        Term::Lambda {
            param,
            param_type,
            body,
        }
        | Term::Pi {
            param,
            param_type,
            body_type: body,
        } => {
            hygiene(param_type, scope);
            assert!(
                scope.insert(param.clone()),
                "shadowing translated binder {param}"
            );
            hygiene(body, scope);
            scope.remove(param);
        }
        Term::Let {
            name,
            ty,
            value,
            body,
        } => {
            hygiene(ty, scope);
            hygiene(value, scope);
            assert!(scope.insert(name.clone()));
            hygiene(body, scope);
            scope.remove(name);
        }
        Term::App(f, a) => {
            hygiene(f, scope);
            hygiene(a, scope);
        }
        Term::Annotated { term, .. } => hygiene(term, scope),
        Term::Match {
            discriminant,
            motive,
            cases,
        } => {
            hygiene(discriminant, scope);
            hygiene(motive, scope);
            for c in cases {
                hygiene(c, scope);
            }
        }
        Term::Fix { name, body } => {
            assert!(scope.insert(name.clone()));
            hygiene(body, scope);
            scope.remove(name);
        }
        Term::MutualFix { defs, .. } => {
            for (n, _) in defs {
                assert!(scope.insert(n.clone()));
            }
            for (_, b) in defs {
                hygiene(b, scope);
            }
            for (n, _) in defs {
                scope.remove(n);
            }
        }
        Term::Hole => panic!("a translated original declaration contains a hole"),
        Term::Global(_) | Term::Const { .. } | Term::Sort(_) | Term::Lit(_) => {}
    }
}
fn name(short: &str) -> String {
    format!("/s:\"T00_001\"/s:\"{short}\"")
}
fn nat(n: u64) -> Term {
    Term::Lit(Literal::Nat(logicaffeine_base::BigInt::from_u64(n)))
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}

#[test]
fn original_t00_001_all_seven_declarations_preserve_binding_types_and_axioms() {
    let input = include_str!("fixtures/lean_conformance/cases_lean414/T00-001.ndjson");
    let (ctx, report) = adapter::import_audited(input).unwrap();
    let expected = [
        "binderNamedLikeAGlobal",
        "identityNat",
        "innerTypeMentionsOuter",
        "pickOuter",
        "shadowedInnermost",
        "shadowedOutermost",
        "usesSectionVar",
    ];
    let authored: Vec<_> = report
        .declarations
        .iter()
        .filter(|d| d.name.starts_with(r#"/s:"T00_001"/"#))
        .collect();
    assert_eq!(authored.len(), expected.len());
    for short in expected {
        let n = name(short);
        let evidence = authored.iter().find(|d| d.name == n).unwrap();
        assert!(evidence.axioms.is_empty());
        let body = ctx
            .get_definition_body(&n)
            .expect("actual translated LOGOS body");
        let ty = ctx
            .get_definition_type(&n)
            .expect("independently translated Lean type");
        hygiene(body, &mut HashSet::new());
        hygiene(ty, &mut HashSet::new());
        assert!(defeq_for_test(&ctx, &infer_type(&ctx, body).unwrap(), ty));
        assert!(defeq_for_test(&ctx, &recheck(&ctx, body).unwrap(), ty));
    }
    // These observations distinguish wrong de Bruijn directions even when a
    // captured Nat variable would leave the inferred type unchanged.
    for (f, args, result) in [
        ("identityNat", vec![2], 2),
        ("pickOuter", vec![2, 3], 2),
        ("shadowedInnermost", vec![2, 3, 5], 5),
        ("shadowedOutermost", vec![2, 3, 5], 2),
        ("usesSectionVar", vec![2], 3),
    ] {
        let term = args
            .into_iter()
            .fold(Term::Global(name(f)), |f, n| app(f, nat(n)));
        assert!(
            defeq_for_test(&ctx, &normalize_checked(&ctx, &term).unwrap(), &nat(result)),
            "{f}"
        );
    }
    let boolean = Term::Global(r#"/s:"Bool"/s:"false""#.into());
    let term = app(
        Term::Global(name("binderNamedLikeAGlobal")),
        boolean.clone(),
    );
    assert!(defeq_for_test(
        &ctx,
        &normalize_checked(&ctx, &term).unwrap(),
        &boolean
    ));
    let body = ctx
        .get_definition_body(&name("innerTypeMentionsOuter"))
        .unwrap();
    let Term::Pi {
        param, body_type, ..
    } = body
    else {
        panic!("dependent telescope missing")
    };
    let Term::Pi { param_type, .. } = body_type.as_ref() else {
        panic!("dependent second binder missing")
    };
    let Term::App(_, argument) = param_type.as_ref() else {
        panic!("Fin's index missing")
    };
    assert_eq!(argument.as_ref(), &Term::Var(param.clone()));
}

#[test]
fn retained_deep_imported_artifacts_drop_safely_on_a_small_caller_stack() {
    use serde_json::json;
    let mut rows = vec![
        json!({"meta":{"format":{"version":"3.1.0"}}}),
        json!({"in":1,"str":{"pre":0,"str":"deepArtifact"}}),
        json!({"il":1,"succ":0}),
        json!({"ie":0,"sort":1}),
        json!({"ie":1,"bvar":255}),
    ];
    let (mut body, mut ty) = (1, 0);
    for id in (2..514).step_by(2) {
        rows.push(json!({"ie":id,"lam":{"name":0,"type":0,"body":body,"binderInfo":"default"}}));
        body = id;
        rows.push(
            json!({"ie":id+1,"forallE":{"name":0,"type":0,"body":ty,"binderInfo":"default"}}),
        );
        ty = id + 1;
    }
    rows.push(json!({"def":{"name":1,"type":ty,"value":body,"all":[1],"levelParams":[],"safety":"safe","hints":"opaque"}}));
    let input: String = rows.into_iter().map(|r| r.to_string() + "\n").collect();
    std::thread::Builder::new()
        .stack_size(128 * 1024)
        .spawn(move || {
            let (ctx, report) = adapter::import_audited(&input).unwrap();
            assert_eq!(report.declarations.len(), 1);
            assert!(ctx.get_definition_body(r#"/s:"deepArtifact""#).is_some());
            drop(ctx);
        })
        .unwrap()
        .join()
        .unwrap();
}
