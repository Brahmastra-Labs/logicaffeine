use logicaffeine_kernel::elaborate::ParamKind::{self, Explicit, Implicit, Instance};
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

#[test]
fn original_t00_005_preserves_all_ten_named_telescopes_and_their_computations() {
    let (ctx, report) = adapter::import_audited(include_str!(
        "fixtures/lean_conformance/cases_lean414/T00-005.ndjson"
    ))
    .unwrap();
    let expected: [(&str, &[ParamKind]); 10] = [
        ("idNat", &[Explicit]),
        ("pickMiddle", &[Explicit, Explicit, Explicit]),
        ("mixedTelescope", &[Explicit, Explicit]),
        ("implicitId", &[Implicit, Explicit]),
        // This export shares expressions up to alpha-equivalence, which ignores binder info: its
        // strictImplicitId type IS implicitId's node, so the export itself carries `{α}`. The
        // structural export keeps `⦃α⦄`, and original_structural_binder_bridge proves its recovery.
        ("strictImplicitId", &[Implicit, Explicit]),
        ("instanceDefault", &[Implicit, Instance]),
        ("mixedKinds", &[Implicit, Instance, Explicit, Explicit]),
        ("applyToPolyFn", &[Explicit]),
        ("anonymousImplicit", &[]),
        ("domainFromExpectedType", &[Explicit, Explicit]),
    ];
    let authored: Vec<_> = report
        .declarations
        .iter()
        .filter(|d| d.name.starts_with(r#"/s:"T00_005"/"#))
        .collect();
    assert_eq!(authored.len(), expected.len());
    for (short, kinds) in expected {
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
