use logicaffeine_kernel::elaborate::ParamKind::{
    self, Explicit, Implicit, Instance, StrictImplicit,
};
use logicaffeine_kernel::interface::lean as adapter;
use logicaffeine_kernel::{
    defeq_for_test, infer_type, normalize_checked, recheck, Literal, Term, Universe,
};

fn name(short: &str) -> String {
    format!("/s:\"T00_005\"/s:\"{short}\"")
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn nat(n: u64) -> Term {
    Term::Lit(Literal::Nat(logicaffeine_base::BigInt::from_u64(n)))
}
fn boolean(b: bool) -> Term {
    Term::Global(format!(
        "/s:\"Bool\"/s:\"{}\"",
        if b { "true" } else { "false" }
    ))
}

fn check_original_binder(selected: Option<&str>) {
    let (ctx, report) = adapter::import_audited(include_str!(
        "fixtures/lean_conformance/cases_lean414/T00-005.structural.ndjson"
    ))
    .unwrap();
    let expected: [(&str, &[ParamKind]); 10] = [
        ("idNat", &[Explicit]),
        ("pickMiddle", &[Explicit, Explicit, Explicit]),
        ("mixedTelescope", &[Explicit, Explicit]),
        ("implicitId", &[Implicit, Explicit]),
        ("strictImplicitId", &[StrictImplicit, Explicit]),
        ("instanceDefault", &[Implicit, Instance]),
        ("mixedKinds", &[Implicit, Instance, Explicit, Explicit]),
        ("applyToPolyFn", &[Explicit]),
        ("anonymousImplicit", &[]),
        ("domainFromExpectedType", &[Explicit, Explicit]),
    ];
    assert!(
        selected.is_none_or(|short| [
            "anonymousImplicit",
            "applyToPolyFn",
            "domainFromExpectedType",
            "idNat",
            "implicitId",
            "instanceDefault",
            "mixedKinds",
            "mixedTelescope",
            "pickMiddle",
            "strictImplicitId"
        ]
        .contains(&short)),
        "unknown source declaration"
    );
    let authored: Vec<_> = report
        .declarations
        .iter()
        .filter(|d| d.name.starts_with(r#"/s:"T00_005"/"#))
        .collect();
    assert_eq!(authored.len(), expected.len());
    for (short, kinds) in expected {
        if selected.is_some_and(|chosen| chosen != short) {
            continue;
        }
        let n = name(short);
        let audit = authored.iter().find(|d| d.name == n).unwrap();
        assert!(audit.axioms.is_empty(), "{short}: {:?}", audit.axioms);
        assert_eq!(ctx.binder_kinds(&n), Some(kinds), "{short}");
        let body = ctx.get_definition_body(&n).unwrap();
        let ty = ctx.get_definition_type(&n).unwrap();
        assert!(
            defeq_for_test(&ctx, &infer_type(&ctx, body).unwrap(), ty),
            "{short}"
        );
        assert!(
            defeq_for_test(&ctx, &recheck(&ctx, body).unwrap(), ty),
            "{short}"
        );
        let mut remaining = ty;
        for _ in kinds {
            let Term::Pi {
                param_type,
                body_type,
                ..
            } = remaining
            else {
                panic!("short telescope: {short}")
            };
            assert!(!matches!(param_type.as_ref(), Term::Hole));
            remaining = body_type;
        }
        assert!(
            !matches!(remaining, Term::Pi { .. }),
            "long telescope: {short}"
        );
    }
    let inhabitant = app(
        app(
            Term::Const {
                name: r#"/s:"Inhabited"/s:"mk""#.into(),
                levels: vec![Universe::Type(0)],
            },
            Term::Global("Nat".into()),
        ),
        nat(6),
    );
    for (short, args, result) in [
        ("idNat", vec![nat(2)], 2),
        ("pickMiddle", vec![nat(2), nat(3), nat(5)], 3),
        ("mixedTelescope", vec![nat(7), boolean(true)], 7),
        ("mixedTelescope", vec![nat(7), boolean(false)], 0),
        ("implicitId", vec![Term::Global("Nat".into()), nat(9)], 9),
        (
            "strictImplicitId",
            vec![Term::Global("Nat".into()), nat(9)],
            9,
        ),
        (
            "instanceDefault",
            vec![Term::Global("Nat".into()), inhabitant.clone()],
            6,
        ),
        (
            "mixedKinds",
            vec![
                Term::Global("Nat".into()),
                inhabitant.clone(),
                nat(5),
                boolean(true),
            ],
            5,
        ),
        (
            "mixedKinds",
            vec![
                Term::Global("Nat".into()),
                inhabitant,
                nat(5),
                boolean(false),
            ],
            6,
        ),
        ("applyToPolyFn", vec![Term::Global(name("implicitId"))], 3),
        ("anonymousImplicit", vec![], 3),
        ("domainFromExpectedType", vec![nat(7), boolean(false)], 7),
    ] {
        if selected.is_some_and(|chosen| chosen != short) {
            continue;
        }
        let call = args.into_iter().fold(Term::Global(name(short)), app);
        assert!(
            infer_type(&ctx, &call).is_ok(),
            "ill-typed observation: {short}"
        );
        assert!(
            defeq_for_test(&ctx, &normalize_checked(&ctx, &call).unwrap(), &nat(result)),
            "{short}"
        );
    }
}

#[test]
fn original_t00_005_structural_export_preserves_all_ten_named_telescopes_and_their_computations() {
    check_original_binder(None);
}

#[test]
fn original_anonymous_implicit() {
    check_original_binder(Some("anonymousImplicit"));
}

#[test]
fn original_apply_to_poly_fn() {
    check_original_binder(Some("applyToPolyFn"));
}

#[test]
fn original_domain_from_expected_type() {
    check_original_binder(Some("domainFromExpectedType"));
}

#[test]
fn original_id_nat() {
    check_original_binder(Some("idNat"));
}

#[test]
fn original_implicit_id() {
    check_original_binder(Some("implicitId"));
}

#[test]
fn original_instance_default() {
    check_original_binder(Some("instanceDefault"));
}

#[test]
fn original_mixed_kinds() {
    check_original_binder(Some("mixedKinds"));
}

#[test]
fn original_mixed_telescope() {
    check_original_binder(Some("mixedTelescope"));
}

#[test]
fn original_pick_middle() {
    check_original_binder(Some("pickMiddle"));
}

#[test]
fn original_strict_implicit_id() {
    check_original_binder(Some("strictImplicitId"));
}
