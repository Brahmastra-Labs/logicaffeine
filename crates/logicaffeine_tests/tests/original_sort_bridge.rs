use logicaffeine_kernel::interface::lean as adapter;
use logicaffeine_kernel::{defeq_for_test, infer_type, normalize_checked, recheck, Term, Universe};

fn name(short: &str) -> String {
    format!("/s:\"T00_002\"/s:\"{short}\"")
}
fn sort(u: Universe) -> Term {
    Term::Sort(u)
}

fn check_original_sort(selected: Option<&str>) {
    let (ctx, report) = adapter::import_audited(include_str!(
        "fixtures/lean_conformance/cases_lean414/T00-002.ndjson"
    ))
    .unwrap();
    assert!(
        selected.is_none_or(|short| [
            "appliedToProp",
            "appliedToType",
            "idTypeAtOne",
            "idTypeAtZero",
            "levelLadder",
            "propAlias",
            "quantifiesOverProps",
            "quantifiesOverTypes",
            "sortOneAlias",
            "sortTwoAlias",
            "sortZeroAlias",
            "typeAlias",
            "typeOneAlias",
            "typeZeroAlias"
        ]
        .contains(&short)),
        "unknown source declaration"
    );
    let authored: Vec<_> = report
        .declarations
        .iter()
        .filter(|d| d.name.starts_with(r#"/s:"T00_002"/"#))
        .collect();
    assert_eq!(authored.len(), 14);
    for d in &authored {
        if selected.is_some_and(|short| d.name != name(short)) {
            continue;
        }
        assert_eq!(d.kind, "def");
        assert!(d.axioms.is_empty());
        let body = ctx.get_definition_body(&d.name).unwrap();
        let ty = ctx.get_definition_type(&d.name).unwrap();
        assert!(
            defeq_for_test(&ctx, &infer_type(&ctx, body).unwrap(), ty),
            "main inferred type: {}",
            d.name
        );
        assert!(
            defeq_for_test(&ctx, &recheck(&ctx, body).unwrap(), ty),
            "independent inferred type: {}",
            d.name
        );
    }
    for (short, level, next) in [
        ("propAlias", Universe::Prop, Universe::Type(0)),
        ("sortZeroAlias", Universe::Prop, Universe::Type(0)),
        ("typeAlias", Universe::Type(0), Universe::Type(1)),
        ("typeZeroAlias", Universe::Type(0), Universe::Type(1)),
        ("sortOneAlias", Universe::Type(0), Universe::Type(1)),
        ("typeOneAlias", Universe::Type(1), Universe::Type(2)),
        ("sortTwoAlias", Universe::Type(1), Universe::Type(2)),
        ("levelLadder", Universe::Type(2), Universe::Type(3)),
    ] {
        if selected.is_some_and(|chosen| chosen != short) {
            continue;
        }
        assert_eq!(
            ctx.get_definition_body(&name(short)).unwrap(),
            &sort(level),
            "{short}"
        );
        assert_eq!(
            ctx.get_definition_type(&name(short)).unwrap(),
            &sort(next),
            "{short}"
        );
    }
    for (short, domain, result) in [
        ("quantifiesOverTypes", Universe::Type(0), Universe::Type(1)),
        ("quantifiesOverProps", Universe::Prop, Universe::Type(0)),
    ] {
        if selected.is_some_and(|chosen| chosen != short) {
            continue;
        }
        let Term::Pi {
            param_type,
            body_type,
            ..
        } = ctx.get_definition_body(&name(short)).unwrap()
        else {
            panic!("expected original Pi body: {short}")
        };
        assert_eq!(param_type.as_ref(), &sort(domain));
        assert_eq!(body_type.as_ref(), &Term::Global("Nat".into()));
        assert_eq!(
            ctx.get_definition_type(&name(short)).unwrap(),
            &sort(result)
        );
    }
    for (short, level) in [
        ("idTypeAtOne", Universe::Type(1)),
        ("idTypeAtZero", Universe::Type(0)),
    ] {
        if selected.is_some_and(|chosen| chosen != short) {
            continue;
        }
        let Term::Lambda {
            param,
            param_type,
            body,
        } = ctx.get_definition_body(&name(short)).unwrap()
        else {
            panic!("expected identity lambda: {short}")
        };
        assert_eq!(param_type.as_ref(), &sort(level.clone()));
        assert_eq!(body.as_ref(), &Term::Var(param.clone()));
        let Term::Pi {
            param_type,
            body_type,
            ..
        } = ctx.get_definition_type(&name(short)).unwrap()
        else {
            panic!("expected identity Pi: {short}")
        };
        assert_eq!(param_type.as_ref(), &sort(level.clone()));
        assert_eq!(body_type.as_ref(), &sort(level));
    }
    for (short, level, result) in [
        ("appliedToType", Universe::Type(0), Universe::Type(1)),
        ("appliedToProp", Universe::Prop, Universe::Type(0)),
    ] {
        if selected.is_some_and(|chosen| chosen != short) {
            continue;
        }
        let body = ctx.get_definition_body(&name(short)).unwrap();
        let Term::App(_, arg) = body else {
            panic!("expected source application: {short}")
        };
        assert_eq!(arg.as_ref(), &sort(level.clone()));
        assert_eq!(normalize_checked(&ctx, body).unwrap(), sort(level));
        assert_eq!(
            ctx.get_definition_type(&name(short)).unwrap(),
            &sort(result)
        );
    }
}

#[test]
fn original_t00_002_all_fourteen_declarations_preserve_exact_sort_indices() {
    check_original_sort(None);
}

#[test]
fn original_applied_to_prop() {
    check_original_sort(Some("appliedToProp"));
}

#[test]
fn original_applied_to_type() {
    check_original_sort(Some("appliedToType"));
}

#[test]
fn original_id_type_at_one() {
    check_original_sort(Some("idTypeAtOne"));
}

#[test]
fn original_id_type_at_zero() {
    check_original_sort(Some("idTypeAtZero"));
}

#[test]
fn original_level_ladder() {
    check_original_sort(Some("levelLadder"));
}

#[test]
fn original_prop_alias() {
    check_original_sort(Some("propAlias"));
}

#[test]
fn original_quantifies_over_props() {
    check_original_sort(Some("quantifiesOverProps"));
}

#[test]
fn original_quantifies_over_types() {
    check_original_sort(Some("quantifiesOverTypes"));
}

#[test]
fn original_sort_one_alias() {
    check_original_sort(Some("sortOneAlias"));
}

#[test]
fn original_sort_two_alias() {
    check_original_sort(Some("sortTwoAlias"));
}

#[test]
fn original_sort_zero_alias() {
    check_original_sort(Some("sortZeroAlias"));
}

#[test]
fn original_type_alias() {
    check_original_sort(Some("typeAlias"));
}

#[test]
fn original_type_one_alias() {
    check_original_sort(Some("typeOneAlias"));
}

#[test]
fn original_type_zero_alias() {
    check_original_sort(Some("typeZeroAlias"));
}
