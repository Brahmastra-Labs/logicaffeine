//! The derived `Quot_rec` holds no primitive identity, its registered type is the prelude's fixed
//! statement, and it belongs to the family's revocation set with everything its body reads.
use super::*;
use crate::prelude::StandardLibrary;

fn standard() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}

#[test]
fn the_derived_eliminator_holds_no_primitive_identity() {
    let ctx = standard();
    assert_eq!(
        prelude_kind(&ctx, "Quot_rec"),
        None,
        "a definition is never a primitive identity"
    );
    assert_eq!(
        prelude_kind(&ctx, "Quot_sound"),
        None,
        "an axiom is never a primitive identity"
    );
    for (name, kind) in [
        ("Quot", PreludeKind::Type),
        ("Quot_mk", PreludeKind::Constructor),
        ("Quot_lift", PreludeKind::Lift),
        ("Quot_ind", PreludeKind::Induction),
    ] {
        assert_eq!(
            prelude_kind(&ctx, name),
            Some(kind),
            "`{name}` keeps its identity"
        );
    }
    let certificate = ctx
        .prelude_quotient
        .as_ref()
        .expect("the family is certified");
    assert_eq!(
        certificate.names().collect::<Vec<_>>(),
        ["Quot", "Quot_ind", "Quot_lift", "Quot_mk", "Quot_sound"],
        "the certificate attests the headers registration installed; `Quot_rec` has a body instead"
    );
}

#[test]
fn the_registered_type_is_the_fixed_statement_and_the_body_the_derivation() {
    let ctx = standard();
    assert_eq!(
        ctx.get_global("Quot_rec"),
        Some(&prelude::recursor_statement())
    );
    let (derived_type, derived) = crate::recursor::derive_quot_rec(&ctx).expect("derives");
    assert_eq!(ctx.get_definition_body("Quot_rec"), Some(&derived));
    let statement = prelude::recursor_statement();
    assert!(
        crate::is_subtype(&ctx, &derived_type, &statement)
            && crate::is_subtype(&ctx, &statement, &derived_type)
    );
}

/// `logos-coc-6` is the prelude that DEFINES `Quot_rec`; `logos-coc-5` assumed it and is retired.
/// A constant that eliminates through `Quot_rec`, captured now, replays under the current tag with
/// both kernels agreeing and rests on `Quot_sound` alone; the same certificate relabelled as the
/// retired tag is refused at rebuild and at read.
#[cfg(feature = "serde")]
#[test]
fn the_prelude_that_defines_the_eliminator_is_current_and_the_one_that_assumed_it_is_retired() {
    use crate::certificate::{
        check_prelude_version, read_certificate, CaptureRequest, ConversionDiscipline,
        EnvironmentBase, EnvironmentCertificate, EnvironmentLimits, PRELUDE_VERSION,
        RETIRED_PRELUDE_VERSIONS,
    };
    assert_eq!(PRELUDE_VERSION, "logos-coc-6");
    assert!(
        RETIRED_PRELUDE_VERSIONS.contains(&"logos-coc-5"),
        "`logos-coc-5` assumed `Quot_rec`"
    );
    assert!(check_prelude_version("logos-coc-5").is_err());
    assert!(check_prelude_version("logos-coc-6").is_ok());

    let g = |n: &str| Term::Global(n.into());
    let var = |n: &str| Term::Var(n.into());
    let apply = |f: Term, args: &[Term]| {
        args.iter()
            .cloned()
            .fold(f, |f, a| Term::App(Box::new(f), Box::new(a)))
    };
    let lam = |p: &str, ty: Term, body: Term| Term::Lambda {
        param: p.into(),
        param_type: Box::new(ty),
        body: Box::new(body),
    };
    let relation = lam(
        "x",
        g("Bool"),
        lam(
            "y",
            g("Bool"),
            apply(g("Eq"), &[g("Bool"), var("x"), var("y")]),
        ),
    );
    let quotient = apply(g("Quot"), &[g("Bool"), relation.clone()]);
    let eliminator = apply(
        g("Quot_rec"),
        &[
            g("Bool"),
            relation,
            lam("q", quotient, g("Bool")),
            lam("b", g("Bool"), var("b")),
        ],
    );
    let mut ctx = standard();
    let ty = crate::infer_type(&ctx, &eliminator).expect("the partial elimination is typed");
    ctx.declare("quotient.rec.versioned", ty, eliminator)
        .expect("declared through the checked gateway");

    let proofs = std::collections::BTreeMap::new();
    let current = EnvironmentCertificate::capture(&CaptureRequest {
        context: &ctx,
        base: EnvironmentBase::StandardLibrary,
        conversion: ConversionDiscipline::Cumulative,
        target: "quotient.rec.versioned",
        opaque_proofs: &proofs,
    })
    .expect("a constant over the defined eliminator captures");
    assert_eq!(current.prelude_version, "logos-coc-6");
    let rebuilt = current
        .rebuild(&EnvironmentLimits::default())
        .expect("the current tag replays");
    assert!(
        rebuilt.independently_agreed(),
        "both kernels corroborate the replay"
    );
    assert_eq!(
        rebuilt.axioms.axioms,
        ["Quot_sound".to_string()].into_iter().collect(),
        "through the defined eliminator the replay rests on `Quot_sound` alone"
    );
    assert!(rebuilt.axioms.unresolved.is_empty(), "{:?}", rebuilt.axioms);

    let mut retired = current.clone();
    retired.prelude_version = "logos-coc-5".into();
    assert!(
        retired.rebuild(&EnvironmentLimits::default()).is_err(),
        "the retired tag is refused at rebuild"
    );
    assert!(
        read_certificate(&serde_json::to_string(&retired).unwrap()).is_err(),
        "…and at read"
    );
}

#[test]
fn displacing_the_eliminator_or_anything_its_body_reads_revokes_the_family() {
    for replaced in [
        "Quot_rec",
        "PSigma",
        "PSigma_mk",
        "PSigma_fst",
        "PSigma_snd",
        "Eq_rec_dep",
    ] {
        let mut ctx = standard();
        let certificate = ctx
            .prelude_quotient
            .as_ref()
            .expect("the family is certified");
        assert!(
            certificate.dependencies.contains(replaced),
            "`{replaced}` is in the revocation set"
        );
        ctx.permit_meaning_changes_for_cache_tests();
        ctx.add_declaration(replaced, Term::Sort(Universe::Prop));
        assert_eq!(
            ctx.get_global(replaced),
            Some(&Term::Sort(Universe::Prop)),
            "the fixture mutation took effect"
        );
        assert!(
            ctx.prelude_quotient.is_none(),
            "`{replaced}` displaced: the family's certificate is revoked"
        );
        for primitive in ["Quot", "Quot_mk", "Quot_lift", "Quot_ind"] {
            assert_eq!(
                prelude_kind(&ctx, primitive),
                None,
                "`{replaced}` displaced: `{primitive}` lost its identity"
            );
        }
    }
}
