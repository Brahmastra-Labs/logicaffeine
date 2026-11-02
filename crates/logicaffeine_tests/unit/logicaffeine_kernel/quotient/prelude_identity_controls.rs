//! Quotient identity and portable accounting controls; no footprint name exemptions.
use super::*;
#[cfg(feature = "serde")]
use crate::certificate::{
    CaptureRequest, ConversionDiscipline, EnvironmentBase, EnvironmentCertificate,
    EnvironmentLimits,
};
use crate::prelude::{axiom_dependencies, axiom_dependencies_of, StandardLibrary};
#[cfg(feature = "serde")]
use std::collections::BTreeMap;
use std::collections::BTreeSet;

fn g(name: &str) -> Term {
    Term::Global(name.into())
}
fn lam(name: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: name.into(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}
fn standard() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}
fn names(items: &[&str]) -> BTreeSet<String> {
    items.iter().map(|n| n.to_string()).collect()
}
fn closed_quotient() -> Term {
    app(
        g("Quot"),
        [
            g("Bool"),
            lam("x", g("Bool"), lam("y", g("Bool"), g("True"))),
        ],
    )
}
#[cfg(feature = "serde")]
fn capture(ctx: &Context, target: &str, base: EnvironmentBase) -> EnvironmentCertificate {
    let proofs = BTreeMap::new();
    EnvironmentCertificate::capture(&CaptureRequest {
        context: ctx,
        base,
        conversion: ConversionDiscipline::Cumulative,
        target,
        opaque_proofs: &proofs,
    })
    .expect("the exact closed fixture captures")
}
#[cfg(feature = "serde")]
fn portable(certificate: &EnvironmentCertificate) -> crate::certificate::RebuiltEnvironment {
    let bytes = serde_json::to_vec(certificate).unwrap();
    let carried: EnvironmentCertificate = serde_json::from_slice(&bytes).unwrap();
    let rebuilt = carried
        .rebuild(&EnvironmentLimits::default())
        .expect("carried environment rebuilds");
    assert!(
        rebuilt.independently_agreed(),
        "both kernels must corroborate the roundtrip"
    );
    rebuilt
}

#[test]
fn registered_prelude_quotients_are_primitive_but_sound_remains_an_axiom() {
    let ctx = standard();
    for name in ["Quot", "Quot_mk", "Quot_lift", "Quot_ind"] {
        let usage = axiom_dependencies_of(&ctx, name).expect("registered primitive");
        assert!(
            usage.axioms.is_empty() && usage.unresolved.is_empty(),
            "{name}: {usage:?}"
        );
    }
    assert_eq!(
        axiom_dependencies_of(&ctx, "Quot_sound").unwrap().axioms,
        names(&["Quot_sound"])
    );
    assert_eq!(
        axiom_dependencies_of(&ctx, "funext").unwrap().axioms,
        names(&["Quot_sound"])
    );
}

#[test]
fn identical_primitive_spelling_and_signature_do_not_certify_a_raw_assumption() {
    let prelude = standard();
    // The headers are copied as ordinary assumptions in dependency order (`Eq`, then each
    // primitive under its own spelling and an impostor's), so each copy is a type over the
    // copies before it, as the assumption door requires.
    let mut raw = Context::new();
    raw.add_declaration("Eq", prelude.get_global("Eq").unwrap().clone());
    let primitives = ["Quot", "Quot_mk", "Quot_lift", "Quot_ind"];
    for primitive in primitives {
        raw.add_declaration(primitive, prelude.get_global(primitive).unwrap().clone());
    }
    for primitive in primitives {
        raw.add_declaration(
            &format!("impostor.{primitive}"),
            prelude.get_global(primitive).unwrap().clone(),
        );
    }
    assert!(
        raw.refused_declarations().is_empty(),
        "the copied headers are types: {:?}",
        raw.refused_declarations()
    );
    for primitive in primitives {
        for name in [primitive.to_string(), format!("impostor.{primitive}")] {
            assert!(raw.get_global(&name).is_some(), "fixture header is present");
            let usage = axiom_dependencies_of(&raw, &name).unwrap();
            assert!(
                usage.axioms.contains(&name),
                "a copied signature has no primitive authority: {usage:?}"
            );
        }
    }
}

#[test]
fn a_renamed_definition_tracks_its_real_primitive_and_never_hides_sound() {
    let mut ctx = standard();
    ctx.declare(
        "quotient.accounting.alias",
        Term::Sort(Universe::Type(0)),
        closed_quotient(),
    )
    .unwrap();
    let usage = axiom_dependencies_of(&ctx, "quotient.accounting.alias").unwrap();
    assert!(
        usage.axioms.is_empty() && usage.unresolved.is_empty(),
        "{usage:?}"
    );
    // Merely renaming an assumption never supplies a proof of it.
    ctx.add_declaration(
        "quotient.accounting.sound_copy",
        ctx.get_global("Quot_sound").unwrap().clone(),
    );
    assert_eq!(
        axiom_dependencies_of(&ctx, "quotient.accounting.sound_copy")
            .unwrap()
            .axioms,
        names(&["quotient.accounting.sound_copy"])
    );
    assert_eq!(
        axiom_dependencies(&ctx, &g("Quot_sound")).axioms,
        names(&["Quot_sound"])
    );
}

#[test]
fn replacement_declarations_cannot_keep_primitive_accounting_and_clones_stay_separate() {
    let original = standard();
    for name in ["Quot", "Quot_mk", "Quot_lift", "Quot_ind"] {
        let mut changed = original.clone();
        changed.permit_meaning_changes_for_cache_tests();
        changed.add_declaration(name, prop());
        assert_eq!(
            changed.get_global(name),
            Some(&prop()),
            "fixture mutation must happen"
        );
        assert_eq!(
            axiom_dependencies_of(&changed, name).unwrap().axioms,
            names(&[name])
        );
        assert!(
            axiom_dependencies_of(&original, name)
                .unwrap()
                .axioms
                .is_empty(),
            "mutating the clone must not revoke the original primitive identity"
        );
    }
}

#[cfg(feature = "serde")]
#[test]
fn standard_library_certificate_roundtrip_preserves_primitive_accounting() {
    let mut ctx = standard();
    ctx.declare(
        "quotient.accounting.roundtrip",
        Term::Sort(Universe::Type(0)),
        closed_quotient(),
    )
    .unwrap();
    let rebuilt = portable(&capture(
        &ctx,
        "quotient.accounting.roundtrip",
        EnvironmentBase::StandardLibrary,
    ));
    assert!(
        rebuilt.axioms.axioms.is_empty() && rebuilt.axioms.unresolved.is_empty(),
        "{:?}",
        rebuilt.axioms
    );
    assert_eq!(
        axiom_dependencies_of(&rebuilt.context, "Quot_sound")
            .unwrap()
            .axioms,
        names(&["Quot_sound"])
    );
}

#[cfg(feature = "serde")]
#[test]
fn bare_kernel_certificate_roundtrip_keeps_a_renamed_impostor_as_an_assumption() {
    let mut ctx = Context::new();
    ctx.add_declaration("impostor.Quot", Term::Sort(Universe::Type(0)));
    ctx.declare(
        "impostor.alias",
        Term::Sort(Universe::Type(0)),
        g("impostor.Quot"),
    )
    .unwrap();
    let rebuilt = portable(&capture(&ctx, "impostor.alias", EnvironmentBase::Kernel));
    assert_eq!(rebuilt.axioms.axioms, names(&["impostor.Quot"]));
    assert!(rebuilt.axioms.unresolved.is_empty());
}

fn relation() -> Term {
    lam(
        "x",
        g("Bool"),
        lam(
            "y",
            g("Bool"),
            app(
                g("Eq"),
                [g("Bool"), Term::Var("x".into()), Term::Var("y".into())],
            ),
        ),
    )
}
fn quotient_value() -> Term {
    app(g("Quot_mk"), [g("Bool"), relation(), g("true")])
}
fn elimination(name: &str) -> Term {
    let qtype = app(g("Quot"), [g("Bool"), relation()]);
    if name == "Quot_ind" {
        return app(
            g(name),
            [
                g("Bool"),
                relation(),
                lam("q", qtype, g("True")),
                lam("x", g("Bool"), g("I")),
                quotient_value(),
            ],
        );
    }
    let respect = lam(
        "x",
        g("Bool"),
        lam(
            "y",
            g("Bool"),
            lam(
                "h",
                app(
                    g("Eq"),
                    [g("Bool"), Term::Var("x".into()), Term::Var("y".into())],
                ),
                Term::Var("h".into()),
            ),
        ),
    );
    let motive = if name == "Quot_rec" {
        lam("q", qtype, g("Bool"))
    } else {
        g("Bool")
    };
    // For rec this probes only dispatch authority; its dependent respect term is
    // intentionally not used as a typed proof fixture. Lift is typed below.
    app(
        g(name),
        [
            g("Bool"),
            relation(),
            motive,
            lam("x", g("Bool"), Term::Var("x".into())),
            respect,
            quotient_value(),
        ],
    )
}
fn restore_as_uncertified_assumption(ctx: &mut Context, name: &str) {
    let header = ctx.get_global(name).unwrap().clone();
    ctx.permit_meaning_changes_for_cache_tests();
    ctx.add_declaration(name, prop());
    assert_eq!(
        ctx.get_global(name),
        Some(&prop()),
        "the adversarial replacement took effect"
    );
    ctx.add_declaration(name, header.clone());
    assert_eq!(
        ctx.get_global(name),
        Some(&header),
        "same signature restored without a primitive door"
    );
}

#[test]
fn actual_legacy_lift_and_induction_compute_and_lift_is_checked_by_both_kernels() {
    let ctx = standard();
    for (name, value) in [("Quot_lift", "true"), ("Quot_ind", "I")] {
        let term = elimination(name);
        assert_eq!(crate::normalize_checked(&ctx, &term).unwrap(), g(value));
        assert!(crate::infer_type(&ctx, &term).is_ok());
        assert_eq!(crate::double_check(&ctx, &term), crate::DoubleCheck::Agreed);
    }
    let term = elimination("Quot_lift");
    let goal = app(g("Eq"), [g("Bool"), term, g("true")]);
    let refl = app(g("refl"), [g("Bool"), g("true")]);
    let proof = app(lam("p", goal, Term::Var("p".into())), [refl]);
    assert_eq!(
        crate::double_check(&ctx, &proof),
        crate::DoubleCheck::Agreed
    );
}

#[test]
fn exact_restored_impostors_have_no_lift_rec_or_induction_computation() {
    for replaced in [
        "Quot",
        "Quot_mk",
        "Quot_lift",
        "Quot_rec",
        "Quot_ind",
        "Eq",
        "refl",
    ] {
        let mut ctx = standard();
        restore_as_uncertified_assumption(&mut ctx, replaced);
        for name in ["Quot_lift", "Quot_rec", "Quot_ind"] {
            let term = elimination(name);
            let reduced = crate::normalize_checked(&ctx, &term).unwrap();
            assert_ne!(reduced,g(if name=="Quot_ind" {"I"} else {"true"}),
                "{replaced} was replaced: {name} cannot retain primitive computation by spelling/signature");
        }
    }
}

#[test]
fn the_independent_checker_cannot_use_an_uncertified_lift_to_check_a_proof() {
    let mut ctx = standard();
    restore_as_uncertified_assumption(&mut ctx, "Quot_lift");
    let term = elimination("Quot_lift");
    assert!(
        crate::infer_type(&ctx, &term).is_ok(),
        "opaque lift application is still well typed"
    );
    let goal = app(g("Eq"), [g("Bool"), term, g("true")]);
    let refl = app(g("refl"), [g("Bool"), g("true")]);
    let proof = app(lam("p", goal, Term::Var("p".into())), [refl]);
    assert!(
        crate::infer_type(&ctx, &proof).is_err(),
        "main kernel cannot assume a primitive equation"
    );
    assert!(
        crate::recheck(&ctx, &proof).is_err(),
        "independent kernel cannot assume it either"
    );
}

#[cfg(feature = "serde")]
#[test]
fn standard_library_capture_rejects_a_revoked_identity_and_cannot_downgrade_the_base() {
    let mut ctx = standard();
    ctx.declare(
        "quotient.accounting.carried",
        Term::Sort(Universe::Type(0)),
        closed_quotient(),
    )
    .unwrap();
    let mut carried = capture(
        &ctx,
        "quotient.accounting.carried",
        EnvironmentBase::StandardLibrary,
    );
    carried.base = EnvironmentBase::Kernel;
    assert!(
        carried.rebuild(&EnvironmentLimits::default()).is_err(),
        "changing the base does not supply primitive identity"
    );
    restore_as_uncertified_assumption(&mut ctx, "Quot_lift");
    let proofs = BTreeMap::new();
    assert!(
        EnvironmentCertificate::capture(&CaptureRequest {
            context: &ctx,
            base: EnvironmentBase::StandardLibrary,
            conversion: ConversionDiscipline::Cumulative,
            target: "quotient.accounting.carried",
            opaque_proofs: &proofs,
        })
        .is_err(),
        "a revoked source base cannot be silently restored as certified during replay"
    );
}

#[test]
fn dependent_recursor_retains_its_sound_axiom() {
    let ctx = standard();
    let usage = axiom_dependencies_of(&ctx, "Quot_rec").unwrap();
    assert_eq!(usage.axioms, names(&["Quot_sound"]));
    assert!(usage.unresolved.is_empty());
}

#[test]
fn sound_and_transport_mutations_revoke_every_legacy_computation() {
    for replaced in ["Quot_sound", "Eq_rec_dep"] {
        let mut ctx = standard();
        restore_as_uncertified_assumption(&mut ctx, replaced);
        for name in ["Quot_lift", "Quot_rec", "Quot_ind"] {
            let term = elimination(name);
            assert_ne!(
                crate::normalize_checked(&ctx, &term).unwrap(),
                g(if name == "Quot_ind" { "I" } else { "true" }),
                "{replaced}: {name}"
            );
        }
        let term = elimination("Quot_lift");
        let goal = app(g("Eq"), [g("Bool"), term, g("true")]);
        let proof = app(
            lam("p", goal, Term::Var("p".into())),
            [app(g("refl"), [g("Bool"), g("true")])],
        );
        assert!(
            crate::infer_type(&ctx, &proof).is_err(),
            "{replaced}: primary equation refused"
        );
        assert!(
            crate::recheck(&ctx, &proof).is_err(),
            "{replaced}: independent equation refused"
        );
        assert!(axiom_dependencies_of(&ctx, "Quot")
            .unwrap()
            .axioms
            .contains("Quot"));
    }
}

#[test]
fn an_opaque_footprint_cannot_survive_revocation_through_another_family_member() {
    let mut original = standard();
    let name = "quotient.accounting.cached_type";
    original
        .declare_opaque_checked(
            name,
            vec![],
            Term::Sort(Universe::Type(0)),
            closed_quotient(),
        )
        .unwrap();
    assert!(
        original.checked_opaque_dependencies(name).is_some(),
        "fixture has cached proof evidence"
    );
    assert!(axiom_dependencies_of(&original, name)
        .unwrap()
        .axioms
        .is_empty());
    for replaced in [
        "Quot_lift",
        "Quot_rec",
        "Quot_ind",
        "Quot_sound",
        "Eq_rec_dep",
    ] {
        let mut changed = original.clone();
        restore_as_uncertified_assumption(&mut changed, replaced);
        assert!(
            changed.checked_opaque_dependencies(name).is_none(),
            "{replaced}: the cached evidence must be revoked"
        );
        assert_eq!(
            axiom_dependencies_of(&changed, name).unwrap().axioms,
            names(&[name]),
            "{replaced}: no hidden body or stale proof evidence may justify the opaque header"
        );
        assert!(original.checked_opaque_dependencies(name).is_some());
        assert!(axiom_dependencies_of(&original, name)
            .unwrap()
            .axioms
            .is_empty());
    }
}

fn dependent_rec_equation(ctx: &Context) -> Term {
    fn var(n: &str) -> Term {
        Term::Var(n.into())
    }
    fn binder(n: &str, ty: Term, body: Term) -> Term {
        Term::Pi {
            param: n.into(),
            param_type: Box::new(ty),
            body_type: Box::new(body),
        }
    }
    let qtype = app(g("Quot"), [g("Bool"), relation()]);
    // The motive is arbitrary and depends on q. It cannot reduce to a constant
    // family: the result is motive (Quot_mk Bool relation true).
    let motive_ty = binder("dep.q", qtype, Term::Sort(Universe::Type(0)));
    let mk = |a: Term| app(g("Quot_mk"), [g("Bool"), relation(), a]);
    let f_ty = binder(
        "dep.a",
        g("Bool"),
        app(var("dep.motive"), [mk(var("dep.a"))]),
    );
    let extended = ctx
        .extend("dep.motive", motive_ty.clone())
        .extend("dep.f", f_ty.clone());
    let partial = app(
        g("Quot_rec"),
        [g("Bool"), relation(), var("dep.motive"), var("dep.f")],
    );
    // Obtain the actual transport-bearing respect obligation, then independently
    // recheck the complete lambda below. No additional global axiom is installed.
    let Term::Pi {
        param_type: respect_ty,
        ..
    } = crate::infer_type(&extended, &partial).unwrap()
    else {
        panic!("Quot_rec must require its transport respect proof");
    };
    let rec = app(partial, [var("dep.h"), mk(g("true"))]);
    let value = app(var("dep.f"), [g("true")]);
    let value_ty = app(var("dep.motive"), [mk(g("true"))]);
    let goal = app(g("Eq"), [value_ty.clone(), rec, value.clone()]);
    let refl = app(g("refl"), [value_ty, value]);
    let proof = app(lam("dep.result", goal, var("dep.result")), [refl]);
    lam(
        "dep.motive",
        motive_ty,
        lam("dep.f", f_ty, lam("dep.h", *respect_ty, proof)),
    )
}

#[test]
fn true_dependent_recursor_equation_is_checked_by_both_kernels_and_revoked() {
    let ctx = standard();
    let proof = dependent_rec_equation(&ctx);
    assert!(crate::infer_type(&ctx, &proof).is_ok());
    assert!(crate::recheck(&ctx, &proof).is_ok());
    assert_eq!(
        crate::double_check(&ctx, &proof),
        crate::DoubleCheck::Agreed
    );
    for replaced in ["Quot_rec", "Quot_sound", "Eq_rec_dep"] {
        let mut changed = ctx.clone();
        restore_as_uncertified_assumption(&mut changed, replaced);
        assert!(
            crate::infer_type(&changed, &proof).is_err(),
            "{replaced}: primary dependent equation refused"
        );
        assert!(
            crate::recheck(&changed, &proof).is_err(),
            "{replaced}: independent dependent equation refused"
        );
    }
}

#[cfg(feature = "serde")]
#[test]
fn the_old_prelude_version_is_refused_and_the_new_version_replays() {
    use crate::certificate::{check_prelude_version, PRELUDE_VERSION, RETIRED_PRELUDE_VERSIONS};
    assert_ne!(
        PRELUDE_VERSION, "logos-coc-4",
        "the version moved past the axiom set that assumed the quotient formers"
    );
    assert!(RETIRED_PRELUDE_VERSIONS.contains(&"logos-coc-4"));
    assert!(check_prelude_version("logos-coc-4").is_err());
    assert!(check_prelude_version(PRELUDE_VERSION).is_ok());
    let mut ctx = standard();
    ctx.declare(
        "quotient.accounting.versioned",
        Term::Sort(Universe::Type(0)),
        closed_quotient(),
    )
    .unwrap();
    let current = capture(
        &ctx,
        "quotient.accounting.versioned",
        EnvironmentBase::StandardLibrary,
    );
    assert_eq!(current.prelude_version, PRELUDE_VERSION);
    let mut retired = current.clone();
    retired.prelude_version = "logos-coc-4".into();
    assert!(retired.rebuild(&EnvironmentLimits::default()).is_err());
    let value = serde_json::to_string(&retired).unwrap();
    assert!(crate::certificate::read_certificate(&value).is_err());
    let rebuilt = portable(&current);
    assert!(rebuilt.axioms.axioms.is_empty());
    assert_eq!(
        axiom_dependencies_of(&rebuilt.context, "Quot_rec")
            .unwrap()
            .axioms,
        names(&["Quot_sound"])
    );
}
