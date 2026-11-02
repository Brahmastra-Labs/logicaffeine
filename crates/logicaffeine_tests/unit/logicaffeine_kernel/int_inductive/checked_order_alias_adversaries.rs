use super::*;
use crate::Universe;
fn context() -> Context {
    let mut ctx = Context::new();
    crate::prelude::StandardLibrary::register(&mut ctx);
    ctx
}
fn order_ty() -> Term {
    pi("left", int(), pi("right", int(), g("Bool")))
}

#[test]
fn order_attestation_checks_exact_alias_and_helper_bodies() {
    let base = context();
    assert!(accelerates_integer_alias(&base, "le"));
    for dependency in ["le", "Int_le", "natLe"] {
        let mut ctx = base.clone();
        let ty = ctx.get_definition_type(dependency).unwrap().clone();
        let body = ctx.get_definition_body(dependency).unwrap().clone();
        let wrapped = app(lam("wrapped", ty.clone(), v("wrapped")), body);
        super::checked_alias_attestation_controls::gate_keeps(&ctx, "le", |c| {
            c.declare(dependency, ty.clone(), wrapped.clone())
        });
        ctx.permit_meaning_changes_for_cache_tests();
        ctx.declare(dependency, ty, wrapped).unwrap();
        assert!(!accelerates_integer_alias(&ctx, "le"), "{dependency}");
    }
    let mut ctx = base;
    assert!(ctx.declare("Int_le", order_ty(), g("Zero")).is_err());
    assert!(accelerates_integer_alias(&ctx, "le"));
}
#[test]
fn order_attestation_rejects_polymorphic_and_visible_header_collisions() {
    let base = context();
    assert!(accelerates_integer_alias(&base, "le"));
    for dependency in ["le", "Int_le", "natLe"] {
        let mut ctx = base.clone();
        let ty = ctx.get_definition_type(dependency).unwrap().clone();
        let body = ctx.get_definition_body(dependency).unwrap().clone();
        super::checked_alias_attestation_controls::gate_keeps(&ctx, "le", |c| {
            c.declare_universe_poly(dependency, vec![], ty.clone(), body.clone())
        });
        ctx.permit_meaning_changes_for_cache_tests();
        ctx.declare_universe_poly(dependency, vec![], ty, body)
            .unwrap();
        assert!(!accelerates_integer_alias(&ctx, "le"), "{dependency}");
    }
    let mut ctx = base;
    super::checked_alias_attestation_controls::gate_keeps(&ctx, "le", |c| {
        c.add_constructor("le", "Nat", nat());
        Ok(())
    });
    ctx.permit_meaning_changes_for_cache_tests();
    ctx.add_constructor("le", "Nat", nat());
    assert!(!accelerates_integer_alias(&ctx, "le"));
}
#[test]
fn order_attestation_invalidates_for_each_carrier_and_boolean_definition() {
    let base = context();
    assert!(accelerates_integer_alias(&base, "le"));
    for carrier in ["Nat", "Int", "Bool"] {
        let mut ctx = base.clone();
        let extra = format!("Extra{carrier}");
        super::checked_alias_attestation_controls::gate_keeps(&ctx, "le", |c| {
            c.add_constructor(&extra, carrier, g(carrier));
            Ok(())
        });
        ctx.permit_meaning_changes_for_cache_tests();
        ctx.add_constructor(&extra, carrier, g(carrier));
        assert!(!accelerates_integer_alias(&ctx, "le"), "{carrier}");
        let mut ctx = base.clone();
        super::checked_alias_attestation_controls::gate_keeps(&ctx, "le", |c| {
            c.set_inductive_params(carrier, 1);
            Ok(())
        });
        ctx.permit_meaning_changes_for_cache_tests();
        ctx.set_inductive_params(carrier, 1);
        assert!(
            !accelerates_integer_alias(&ctx, "le"),
            "{carrier} parameters"
        );
    }
    let mut ctx = base;
    super::checked_alias_attestation_controls::gate_keeps(&ctx, "le", |c| {
        c.declare("true", g("Bool"), g("false"))
    });
    ctx.permit_meaning_changes_for_cache_tests();
    ctx.declare("true", g("Bool"), g("false")).unwrap();
    assert!(!accelerates_integer_alias(&ctx, "le"));
}
#[test]
fn order_attestation_rejects_rewrites_and_reuses_unmodified_local_contexts() {
    let mut ctx = context();
    assert!(accelerates_integer_alias(&ctx, "le"));
    let local = ctx.extend("x", int());
    assert!(accelerates_integer_alias(&local, "le"));
    let equation = apps(
        g("Eq"),
        &[
            g("Bool"),
            apps(g("natLe"), &[g("Zero"), g("Zero")]),
            g("true"),
        ],
    );
    ctx.declare(
        "order_zero_le_zero",
        equation,
        apps(g("refl"), &[g("Bool"), g("true")]),
    )
    .unwrap();
    ctx.add_simp_lemma("order_zero_le_zero").unwrap();
    assert!(!accelerates_integer_alias(&ctx, "le"));
}
#[test]
fn order_alias_allows_only_cosmetic_binder_changes_in_its_bool_header() {
    let mut ctx = context();
    assert!(accelerates_integer_alias(&ctx, "le"));
    ctx.declare("le", order_ty(), g("Int_le")).unwrap();
    assert!(accelerates_integer_alias(&ctx, "le"));
    assert!(
        ctx.refused_declarations().is_empty(),
        "a binder rename is not a change of meaning"
    );
    ctx.add_inductive("WrongOrderHeader", Term::Sort(Universe::Type(0)));
    super::checked_alias_attestation_controls::gate_keeps(&ctx, "le", |c| {
        c.add_constructor("le", "WrongOrderHeader", g("WrongOrderHeader"));
        Ok(())
    });
    ctx.permit_meaning_changes_for_cache_tests();
    ctx.add_constructor("le", "WrongOrderHeader", g("WrongOrderHeader"));
    assert!(!accelerates_integer_alias(&ctx, "le"));
}
