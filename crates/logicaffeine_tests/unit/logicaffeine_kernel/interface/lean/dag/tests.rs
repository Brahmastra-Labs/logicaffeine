use super::*;
#[test]
fn closed_chunks_with_one_source_id_keep_distinct_substitution_values() {
    let mut importer = Importer::new(false);
    importer
        .exprs
        .insert(0, serde_json::json!({"ie":0,"bvar":0}));
    let plan = Plan::new(&importer, 0, 1).unwrap();
    let mut sharing = Sharing::new(plan, &HashSet::new());
    let mut ctx = Context::new();
    crate::prelude::StandardLibrary::register(&mut ctx);
    let mut report = ImportReport::default();
    let mut budget = 100_000;
    let zero = Term::Global("Zero".into());
    let one = apply(Term::Global("Succ".into()), zero.clone());
    // The same source bvar has been substituted under two checked lets.
    // Once closed, its source ID still cannot identify its actual value.
    let (first, first_cut) = sharing
        .cut_closed(&mut ctx, &mut report, 0, zero.clone(), &mut budget)
        .unwrap();
    let (second, second_cut) = sharing
        .cut_closed(&mut ctx, &mut report, 0, one.clone(), &mut budget)
        .unwrap();
    assert!(first_cut && second_cut);
    assert_eq!(report.internal_helpers.len(), 2);
    assert_ne!(
        report.internal_helpers[0].name,
        report.internal_helpers[1].name
    );
    assert!(crate::defeq_for_test(&ctx, &first, &zero));
    assert!(crate::defeq_for_test(&ctx, &second, &one));
    assert!(!crate::defeq_for_test(&ctx, &first, &second));
    assert!(!crate::defeq_for_test(&ctx, &first, &one));
    assert!(!crate::defeq_for_test(&ctx, &second, &zero));
}

#[test]
fn an_exhausted_allowance_names_what_every_category_consumed() {
    let mut importer = Importer::new(false);
    importer
        .exprs
        .insert(0, serde_json::json!({"ie":0,"bvar":0}));
    let plan = Plan::new(&importer, 0, 1).unwrap();
    let mut sharing = Sharing::new(plan, &HashSet::new());
    let value = apply(Term::Global("Succ".into()), Term::Global("Zero".into()));
    let mut budget = 5;
    sharing.copy_checked_value(&value, &mut budget).unwrap();
    let Err(Verdict::Decline(reason)) = sharing.copy_checked_value(&value, &mut budget) else {
        panic!("a second copy exceeds the remaining allowance");
    };
    assert_eq!(reason.code(), DiagnosticCode::ResourceLimit);
    assert!(
        reason.contains("requested 3 for checked_value_copy, remaining 2"),
        "{}",
        reason.message()
    );
    assert!(
        reason.contains("checked_value_copy=3"),
        "the successful copy is on the ledger: {}",
        reason.message()
    );
    assert_eq!(
        sharing.spent(Materialized::CheckedValueCopy),
        3,
        "a refused charge is not recorded as spent"
    );
    assert_eq!(budget, 2, "a refused charge neither consumes nor refunds");
}

#[test]
fn original_app_lam_is_never_rejected_and_its_decline_is_spent_on_captured_telescopes() {
    let input = include_str!("../../../../../tests/fixtures/lean_arena/app-lam-original.ndjson");
    match check_import(input, false) {
        Ok(_) => {}
        Err(Verdict::Reject(reason)) => panic!("app-lam is valid Lean: {reason:?}"),
        Err(Verdict::Decline(reason)) => {
            assert_eq!(
                reason.code(),
                DiagnosticCode::ResourceLimit,
                "{}",
                reason.message()
            );
            // The import runs on its own sized thread; the ledger travels in the reason.
            let spent = reason
                .message()
                .split("; spent ")
                .nth(1)
                .expect("the decline names its ledger");
            let entry = |category: Materialized| {
                spent
                    .split(' ')
                    .find_map(|pair| {
                        pair.strip_prefix(category.name())
                            .and_then(|rest| rest.strip_prefix('='))
                    })
                    .and_then(|n| n.parse::<usize>().ok())
                    .expect("every category is on the ledger")
            };
            let ledger = Materialized::ALL.map(entry);
            // A shared lambda at nesting level k captures its k-1 enclosing Nat binders.
            // Lambda-lifted sharing pays for that telescope when it is captured, when the
            // helper abstracts over it and when a cached helper is applied to it again:
            // three charges linear in k per level, so quadratic in the nesting depth.
            let telescope = ledger[Materialized::CaptureTelescope as usize]
                + ledger[Materialized::HelperAbstraction as usize]
                + ledger[Materialized::CachedApplication as usize];
            let total: usize = ledger.iter().sum();
            assert!(
                telescope * 10 >= total * 9,
                "telescope-proportional charges dominate: {ledger:?}"
            );
            assert!(
                total * 2 >= 100_000,
                "the ledger, not source visits, spends the allowance: {ledger:?}"
            );
        }
    }
}

#[test]
fn checked_let_value_copies_are_charged_before_cloning() {
    let mut importer = Importer::new(false);
    importer
        .exprs
        .insert(0, serde_json::json!({"ie":0,"bvar":0}));
    let plan = Plan::new(&importer, 0, 1).unwrap();
    let mut sharing = Sharing::new(plan, &HashSet::new());
    let value = apply(Term::Global("Succ".into()), Term::Global("Zero".into()));
    let mut insufficient = 2;
    assert!(matches!(
        sharing.copy_checked_value(&value, &mut insufficient),
        Err(Verdict::Decline(_))
    ));
    let mut exact = 3;
    assert_eq!(
        sharing.copy_checked_value(&value, &mut exact).unwrap(),
        value
    );
    assert_eq!(exact, 0);
}

#[test]
fn capture_key_distinguishes_original_free_index_remapping() {
    let mut importer = Importer::new(false);
    importer
        .exprs
        .insert(0, serde_json::json!({"ie":0,"bvar":2}));
    importer
        .exprs
        .insert(1, serde_json::json!({"ie":1,"bvar":0}));
    importer
        .exprs
        .insert(2, serde_json::json!({"ie":2,"app":{"fn":0,"arg":1}}));
    let plan = Plan::new(&importer, 2, 4).unwrap();
    let mut sharing = Sharing::new(plan, &HashSet::new());
    let a = Term::Global("A".into());
    let z = apply(
        apply(Term::Global("P".into()), Term::Var("x".into())),
        Term::Var("y".into()),
    );
    let first = vec![
        ("x".into(), a.clone(), None),
        ("y".into(), a.clone(), None),
        ("z".into(), z.clone(), None),
    ];
    let second = vec![
        ("x".into(), a.clone(), None),
        ("y".into(), a.clone(), None),
        ("unused".into(), a, None),
        ("z".into(), z, None),
    ];
    let first = sharing.capture(2, &first).unwrap();
    let second = sharing.capture(2, &second).unwrap();
    assert!(first.key!=second.key,
        "source bvar2 denotes x in one scope and y in the other despite identical dependency-closed telescopes");
}
#[test]
fn projection_alias_fallback_retains_checked_transaction_helpers() {
    let mut importer = Importer::new(false);
    importer
        .ctx
        .add_structure("Box", &[], &[("field", Term::Sort(Universe::Prop))]);
    assert!(importer.ctx.get_global("Box").is_some());
    importer.names.insert(1, "Box".into());
    importer
        .exprs
        .insert(0, serde_json::json!({"ie":0,"bvar":0}));
    importer.exprs.insert(
        1,
        serde_json::json!({"ie":1,"proj":{"typeName":1,"idx":0,"struct":0}}),
    );
    let mut checked = importer.ctx.clone();
    checked
        .declare(
            "arena.internal.dag.seed",
            Term::Sort(Universe::Type(0)),
            Term::Global("Box".into()),
        )
        .unwrap();
    let mut bound = vec![
        (
            "S".into(),
            Term::Sort(Universe::Type(0)),
            Some(Term::Global("arena.internal.dag.seed".into())),
        ),
        ("value".into(), Term::Var("S".into()), None),
    ];
    let result = importer.term_core(
        &serde_json::json!(1),
        &mut bound,
        &HashSet::new(),
        &mut 100_000,
        Some(&mut checked),
        &mut ImportReport::default(),
        &mut None,
    );
    assert!(
        result.is_ok(),
        "projection fallback must retain checked helper definitions: {result:?}"
    );
}
#[test]
fn generated_capture_telescope_is_charged_before_materialization() {
    let mut importer = Importer::new(false);
    importer
        .exprs
        .insert(0, serde_json::json!({"ie":0,"bvar":1}));
    importer
        .exprs
        .insert(1, serde_json::json!({"ie":1,"bvar":0}));
    let mut root = 1;
    for i in 0..8 {
        let partial = 2 + i * 2;
        importer.exprs.insert(
            partial,
            serde_json::json!({"ie":partial,"app":{"fn":0,"arg":root}}),
        );
        root = partial + 1;
        importer.exprs.insert(
            root,
            serde_json::json!({"ie":root,"app":{"fn":partial,"arg":root-2}}),
        );
    }
    let mut a = Term::Sort(Universe::Prop);
    for i in 0..40 {
        a = Term::Pi {
            param: format!("p{i}"),
            param_type: Box::new(Term::Sort(Universe::Prop)),
            body_type: Box::new(a),
        };
    }
    let function = Term::Pi {
        param: "left".into(),
        param_type: Box::new(a.clone()),
        body_type: Box::new(Term::Pi {
            param: "right".into(),
            param_type: Box::new(a.clone()),
            body_type: Box::new(a.clone()),
        }),
    };
    let mut bound = vec![("f".into(), function, None), ("x".into(), a, None)];
    let plan = Plan::new(&importer, root, bound.len()).unwrap();
    let mut sharing = Some(Sharing::new(plan, &HashSet::new()));
    let result = importer.term_core(
        &serde_json::json!(root),
        &mut bound,
        &HashSet::new(),
        &mut 128,
        Some(&mut importer.ctx.clone()),
        &mut ImportReport::default(),
        &mut sharing,
    );
    assert!(
        matches!(result,Err(Verdict::Decline(ref reason)) if reason.contains("materialization")),
        "source visits alone do not pay for cloned captured types: {result:?}"
    );
}

#[test]
fn checked_helpers_are_delta_equal_to_plain_open_source_decoding() {
    let mut importer = Importer::new(false);
    importer
        .exprs
        .insert(0, serde_json::json!({"ie":0,"bvar":1}));
    importer
        .exprs
        .insert(1, serde_json::json!({"ie":1,"bvar":0}));
    let mut root = 1;
    for i in 0..8 {
        let partial = 2 + i * 2;
        importer.exprs.insert(
            partial,
            serde_json::json!({"ie":partial,"app":{"fn":0,"arg":root}}),
        );
        root = partial + 1;
        importer.exprs.insert(
            root,
            serde_json::json!({"ie":root,"app":{"fn":partial,"arg":root-2}}),
        );
    }
    let p = Term::Sort(Universe::Prop);
    let function = Term::Pi {
        param: "a".into(),
        param_type: Box::new(p.clone()),
        body_type: Box::new(Term::Pi {
            param: "b".into(),
            param_type: Box::new(p.clone()),
            body_type: Box::new(p.clone()),
        }),
    };
    let bound = vec![("f".into(), function, None), ("x".into(), p, None)];
    let plain = importer
        .term(
            &serde_json::json!(root),
            &mut bound.clone(),
            &HashSet::new(),
            &mut 100_000,
        )
        .unwrap();
    let mut checked = importer.ctx.clone();
    let mut report = ImportReport::default();
    let plan = Plan::new(&importer, root, bound.len()).unwrap();
    let mut sharing = Some(Sharing::new(plan, &HashSet::new()));
    let compact = importer
        .term_core(
            &serde_json::json!(root),
            &mut bound.clone(),
            &HashSet::new(),
            &mut 100_000,
            Some(&mut checked),
            &mut report,
            &mut sharing,
        )
        .unwrap();
    assert!(!report.internal_helpers.is_empty());
    let close = |mut body: Term| {
        for (name, ty, _) in bound.iter().rev() {
            body = Term::Lambda {
                param: name.clone(),
                param_type: Box::new(ty.clone()),
                body: Box::new(body),
            };
        }
        body
    };
    let plain = close(plain);
    let compact = close(compact);
    assert!(
        definitionally_equal(&checked, &plain, &compact),
        "helper delta reduction must preserve source body"
    );
    let original_type = lean_type(&checked, &plain).unwrap();
    let independently_checked = crate::recheck(&checked, &compact).unwrap();
    exact(&checked, &original_type, &independently_checked).unwrap();
}

#[test]
fn dependent_capture_remapping_preserves_well_typed_source_meaning() {
    let mut importer = Importer::new(false);
    let prop = Term::Sort(Universe::Prop);
    let pi = |n: &str, a: Term, b: Term| Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    };
    let lam = |n: &str, a: Term, b: Term| Term::Lambda {
        param: n.into(),
        param_type: Box::new(a),
        body: Box::new(b),
    };
    importer
        .ctx
        .declare(
            "P",
            pi(
                "x",
                prop.clone(),
                pi("y", prop.clone(), Term::Sort(Universe::Type(1))),
            ),
            lam(
                "x",
                prop.clone(),
                lam("y", prop.clone(), Term::Sort(Universe::Type(0))),
            ),
        )
        .unwrap();
    importer
        .exprs
        .insert(0, serde_json::json!({"ie":0,"bvar":0}));
    importer
        .exprs
        .insert(1, serde_json::json!({"ie":1,"bvar":3}));
    importer.exprs.insert(
        2,
        serde_json::json!({"ie":2,"lam":{"name":0,"type":0,"body":1,"binderInfo":"default"}}),
    );
    let z = apply(
        apply(Term::Global("P".into()), Term::Var("x".into())),
        Term::Var("y".into()),
    );
    let first = vec![
        ("x".into(), prop.clone(), None),
        ("y".into(), prop.clone(), None),
        ("z".into(), z.clone(), None),
    ];
    let second = vec![
        ("x".into(), prop.clone(), None),
        ("y".into(), prop.clone(), None),
        ("unused".into(), prop, None),
        ("z".into(), z, None),
    ];
    let plan = Plan::new(&importer, 2, 4).unwrap();
    let mut sharing = Sharing::new(plan, &HashSet::new());
    let mut checked = importer.ctx.clone();
    let mut report = ImportReport::default();
    for bound in [first, second] {
        let original = importer
            .term(
                &serde_json::json!(2),
                &mut bound.clone(),
                &HashSet::new(),
                &mut 100_000,
            )
            .unwrap();
        let capture = sharing.capture_budgeted(2, &bound, &mut 100_000).unwrap();
        let compact = match sharing.cached(&capture, &mut 100_000).unwrap() {
            Some(cached) => cached,
            None => sharing
                .finish(
                    &mut checked,
                    &mut report,
                    capture,
                    original.clone(),
                    &mut 100_000,
                )
                .unwrap(),
        };
        let close = |mut body: Term| {
            for (name, ty, _) in bound.iter().rev() {
                body = lam(name, ty.clone(), body);
            }
            body
        };
        let original = close(original);
        let compact = close(compact);
        let original_type = lean_type(&checked, &original).unwrap();
        exact(
            &checked,
            &original_type,
            &crate::recheck(&checked, &compact).unwrap(),
        )
        .unwrap();
        assert!(
            definitionally_equal(&checked, &original, &compact),
            "the same source bvar must denote its own captured local in each scope"
        );
    }
    assert_eq!(report.internal_helpers.len(), 2);
}

/// A let's uses are counted in its body's EXPANDED tree, saturated at two: a child the DAG reaches
/// twice doubles every occurrence below it, an inner binder shifts the index, and an inner let's
/// value sees the outer variable at the outer index while its body sees it one binder further out.
#[test]
fn let_occurrences_count_the_expanded_tree_to_two() {
    use crate::term_store::Occurrences::{Many, Once, Zero};
    let rows = [
        (0, serde_json::json!({"sort":0})),
        (1, serde_json::json!({"bvar":0})),
        (2, serde_json::json!({"bvar":1})),
        (3, serde_json::json!({"const":{"name":0,"us":[]}})),
        (4, serde_json::json!({"app":{"fn":3,"arg":1}})),
        (5, serde_json::json!({"app":{"fn":4,"arg":1}})),
        (6, serde_json::json!({"app":{"fn":3,"arg":4}})),
        (7, serde_json::json!({"app":{"fn":6,"arg":4}})),
        (
            8,
            serde_json::json!({"lam":{"name":0,"type":0,"body":2,"binderInfo":"default"}}),
        ),
        (
            9,
            serde_json::json!({"lam":{"name":0,"type":0,"body":1,"binderInfo":"default"}}),
        ),
        (
            10,
            serde_json::json!({"letE":{"name":0,"type":0,"value":1,"body":2,"nondep":false}}),
        ),
        (
            11,
            serde_json::json!({"letE":{"name":0,"type":0,"value":3,"body":1,"nondep":false}}),
        ),
        (
            12,
            serde_json::json!({"lam":{"name":0,"type":1,"body":3,"binderInfo":"default"}}),
        ),
    ];
    let mut importer = Importer::new(false);
    for (id, row) in rows {
        let mut row = row;
        row["ie"] = serde_json::json!(id);
        importer.exprs.insert(id, row);
    }
    let count = |body: usize| Plan::new(&importer, body, 2).unwrap().let_occurrences(body);
    assert_eq!(count(3), Zero, "a constant mentions no variable");
    assert_eq!(count(1), Once, "the variable itself");
    assert_eq!(count(4), Once, "one application argument");
    assert_eq!(count(5), Many, "both arguments of one spine");
    assert_eq!(
        count(7),
        Many,
        "one child the DAG reaches twice doubles its occurrence"
    );
    assert_eq!(
        count(8),
        Once,
        "bvar 1 under one binder is the let's variable"
    );
    assert_eq!(
        count(9),
        Zero,
        "bvar 0 under one binder is that binder's own"
    );
    assert_eq!(
        count(10),
        Many,
        "an inner let's value (bvar 0) and its body (bvar 1) both reach the variable"
    );
    assert_eq!(
        count(11),
        Zero,
        "an inner let's body bvar 0 is the inner let's own variable"
    );
    assert_eq!(
        count(12),
        Once,
        "a binder's domain sees the variable at the outer index"
    );
}
