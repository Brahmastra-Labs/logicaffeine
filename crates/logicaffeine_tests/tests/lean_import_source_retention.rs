use logicaffeine_kernel::interface::lean as adapter;
use logicaffeine_kernel::{defeq_for_test, infer_type, recheck, Term, Universe};

const INPUT: &str =
    include_str!("fixtures/lean_conformance/cases_lean414/T00-019.sharing.structural.ndjson");
fn name(short: &str) -> String {
    format!("/s:\"T00_019\"/s:{short:?}")
}
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn exact_shared_goal() -> Term {
    let n = Term::Var("n".into());
    let eq = app(
        app(
            app(
                Term::Const {
                    name: r#"/s:"Eq""#.into(),
                    levels: vec![Universe::Type(0)],
                },
                g("Nat"),
            ),
            n.clone(),
        ),
        n,
    );
    let and = |a: Term, b: Term| app(app(g(r#"/s:"And""#), a), b);
    Term::Pi {
        param: "n".into(),
        param_type: Box::new(g("Nat")),
        body_type: Box::new(and(eq.clone(), and(eq.clone(), eq))),
    }
}

#[test]
fn requested_original_theorem_body_is_retained_and_checked_independently() {
    let target = name("sharedProof");
    let (ctx, report) = adapter::import_audited_retaining_source(INPUT, &target).unwrap();
    let (retained_name, ty, body) = ctx.retained_source().expect("one requested checked source");
    assert_eq!(retained_name, target);
    assert_ne!(
        body,
        &g(&target),
        "capture the translated proof, never a reference to its already trusted result"
    );
    // Retention never changes publication: with or without it the theorem unfolds, as Lean's
    // kernel unfolds one, to exactly the retained checked proof.
    assert_eq!(
        ctx.get_definition_body(&target),
        Some(body),
        "the published theorem is the retained proof"
    );
    let (plain, _) = adapter::import_audited(INPUT).unwrap();
    assert_eq!(
        plain.get_definition_body(&target),
        Some(body),
        "retaining evidence does not change publication"
    );
    assert!(defeq_for_test(&ctx, ty, &exact_shared_goal()));
    assert!(defeq_for_test(
        &ctx,
        &infer_type(&ctx, body).unwrap(),
        &exact_shared_goal()
    ));
    assert!(defeq_for_test(
        &ctx,
        &recheck(&ctx, body).unwrap(),
        &exact_shared_goal()
    ));
    assert!(report
        .declarations
        .iter()
        .find(|d| d.name == target)
        .unwrap()
        .axioms
        .is_empty());
}

#[test]
fn default_audited_import_keeps_no_source_body_snapshot() {
    let (ctx, _) = adapter::import_audited(INPUT).unwrap();
    assert!(ctx.retained_source().is_none());
}

#[test]
fn requested_definition_retains_its_actual_body_and_not_a_neighbor() {
    let target = name("viaLet");
    let (ctx, _) = adapter::import_audited_retaining_source(INPUT, &target).unwrap();
    let (actual, ty, body) = ctx.retained_source().unwrap();
    assert_eq!(actual, target);
    assert_eq!(ty, ctx.get_definition_type(&target).unwrap());
    assert_eq!(body, ctx.get_definition_body(&target).unwrap());
    assert!(
        matches!(body, Term::Let { .. }),
        "the selected original viaLet, not viaHave"
    );
}

#[test]
fn absent_or_bodyless_source_cannot_return_fabricated_proof_evidence() {
    for target in [name("missing"), "Nat".into()] {
        assert!(adapter::import_audited_retaining_source(INPUT, &target).is_err());
    }
}

#[test]
fn retained_deep_source_drops_on_the_importers_arranged_stack() {
    use serde_json::json;
    let mut rows = vec![
        json!({"meta":{"format":{"version":"3.1.0"}}}),
        json!({"in":1,"str":{"pre":0,"str":"deepOriginal"}}),
        json!({"il":1,"succ":0}),
        json!({"ie":0,"sort":0}),
        json!({"ie":1,"sort":1}),
    ];
    let (mut body, mut ty) = (0, 1);
    for depth in 0..160 {
        let next = 2 + 2 * depth;
        rows.push(json!({"ie":next,"lam":{"name":0,"type":1,"body":body,"binderInfo":"default"}}));
        rows.push(
            json!({"ie":next+1,"forallE":{"name":0,"type":1,"body":ty,"binderInfo":"default"}}),
        );
        body = next;
        ty = next + 1;
    }
    rows.push(json!({"def":{"all":[1],"name":1,"levelParams":[],"type":ty,"value":body,"safety":"safe","hints":"opaque"}}));
    let input = rows
        .into_iter()
        .map(|r| r.to_string() + "\n")
        .collect::<String>();
    std::thread::Builder::new()
        .name("small-proof-owner".into())
        .stack_size(64 * 1024)
        .spawn(move || {
            let (ctx, _) =
                adapter::import_audited_retaining_source(&input, r#"/s:"deepOriginal""#).unwrap();
            let (target, _, body) = ctx.retained_source().unwrap();
            assert_eq!(target, r#"/s:"deepOriginal""#);
            assert!(matches!(body, Term::Lambda { .. }));
            drop(ctx);
        })
        .unwrap()
        .join()
        .unwrap();
}

/// Both doors that name a target — the retained source and the pipeline's closure — decline an
/// `unsafe` declaration the reader quarantined, whatever checked declarations follow it in the
/// export: it was never kernel checked, so there is nothing to retain or publish.
#[test]
fn a_quarantined_runtime_declaration_is_declined_as_a_requested_source() {
    const UNSAFE_WITNESS: &str = include_str!("fixtures/lean_semantics/LK_UnsafeRuntime.ndjson");
    let key = |dotted: &str| {
        dotted
            .split('.')
            .map(|p| format!("/s:{p:?}"))
            .collect::<String>()
    };
    for root in ["T02_008.diverge", "T02_008.fastIdImpl"] {
        let target = key(root);
        let declined = |verdict: adapter::Verdict| {
            match verdict {
            adapter::Verdict::Decline(reason) => {
                assert_eq!(reason.code(), adapter::DiagnosticCode::UnsupportedFeature, "{root}: {reason:?}");
                assert!(reason.message().contains("runtime declarations remain unchecked"), "{root}: {reason:?}");
            }
            other => panic!("{root}: a quarantined target is declined as unsupported, never rejected: {other:?}"),
        }
        };
        match adapter::import_audited_retaining_source(UNSAFE_WITNESS, &target) {
            Err(verdict) => declined(verdict),
            Ok(_) => panic!("{root}: a quarantined declaration is never retained"),
        }
        match adapter::import_retaining_closure(UNSAFE_WITNESS, &target, &[target.clone()]) {
            Err(verdict) => declined(verdict),
            Ok(_) => panic!("{root}: a quarantined declaration is never imported as a target"),
        }
    }
    let safe = key("T02_008.safeId");
    let (ctx, _) = adapter::import_audited_retaining_source(UNSAFE_WITNESS, &safe)
        .expect("the safe partner retains");
    assert_eq!(
        ctx.retained_source().map(|(name, _, _)| name),
        Some(safe.as_str())
    );
}
