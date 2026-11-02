use super::*;
fn context() -> Context {
    let mut ctx = Context::new();
    crate::prelude::StandardLibrary::register(&mut ctx);
    ctx.declare("sub", int_binop(), g("Int_sub")).unwrap();
    ctx
}
#[test]
fn exact_subtraction_attestation_rejects_every_wrapped_or_malformed_dependency() {
    for dependency in [
        "sub",
        "Int_sub",
        "Int_neg",
        "Int_negOfNat",
        "Int_add",
        "Int_subNatNat",
        "Nat_sub",
        "Nat_pred",
        "Nat_add",
    ] {
        let mut ctx = context();
        assert!(accelerates_integer_alias(&ctx, "sub"));
        let ty = ctx.get_definition_type(dependency).unwrap().clone();
        let body = ctx.get_definition_body(dependency).unwrap().clone();
        let wrapped = app(lam("wrapped", ty.clone(), v("wrapped")), body);
        super::checked_alias_attestation_controls::gate_keeps(&ctx, "sub", |c| {
            c.declare(dependency, ty.clone(), wrapped.clone())
        });
        ctx.permit_meaning_changes_for_cache_tests();
        // A checked identity wrapper is extensionally equal, but is not the
        // exact canonical body certified by this acceleration mechanism.
        ctx.declare(dependency, ty, wrapped).unwrap();
        assert!(!accelerates_integer_alias(&ctx, "sub"), "{dependency}");
    }
    let mut ctx = context();
    assert!(accelerates_integer_alias(&ctx, "sub"));
    assert!(ctx
        .declare("Int_neg", arrow(int(), int()), g("Zero"))
        .is_err());
    assert!(
        accelerates_integer_alias(&ctx, "sub"),
        "a rejected replacement must leave the checked dependency intact"
    );
}
#[test]
fn subtraction_attestation_rejects_poly_alias_and_visible_header_collision() {
    let mut ctx = context();
    assert!(accelerates_integer_alias(&ctx, "sub"));
    super::checked_alias_attestation_controls::gate_keeps(&ctx, "sub", |c| {
        c.declare_universe_poly("sub", vec![], int_binop(), g("Int_sub"))
    });
    ctx.permit_meaning_changes_for_cache_tests();
    ctx.declare_universe_poly("sub", vec![], int_binop(), g("Int_sub"))
        .unwrap();
    assert!(!accelerates_integer_alias(&ctx, "sub"));
    let mut ctx = context();
    assert!(accelerates_integer_alias(&ctx, "sub"));
    super::checked_alias_attestation_controls::gate_keeps(&ctx, "sub", |c| {
        c.add_constructor("sub", "Nat", nat());
        Ok(())
    });
    ctx.permit_meaning_changes_for_cache_tests();
    ctx.add_constructor("sub", "Nat", nat());
    assert_eq!(ctx.get_global("sub"), Some(&nat()));
    assert!(!accelerates_integer_alias(&ctx, "sub"));
}
#[test]
fn subtraction_attestation_rejects_constructor_inventory_and_parameter_changes() {
    let mut ctx = context();
    assert!(accelerates_integer_alias(&ctx, "sub"));
    super::checked_alias_attestation_controls::gate_keeps(&ctx, "sub", |c| {
        c.add_constructor("ExtraNat", "Nat", nat());
        Ok(())
    });
    ctx.permit_meaning_changes_for_cache_tests();
    ctx.add_constructor("ExtraNat", "Nat", nat());
    assert!(!accelerates_integer_alias(&ctx, "sub"));
    let mut ctx = context();
    assert!(accelerates_integer_alias(&ctx, "sub"));
    super::checked_alias_attestation_controls::gate_keeps(&ctx, "sub", |c| {
        c.set_inductive_params("Int", 1);
        Ok(())
    });
    ctx.permit_meaning_changes_for_cache_tests();
    ctx.set_inductive_params("Int", 1);
    assert!(!accelerates_integer_alias(&ctx, "sub"));
}
#[test]
fn registered_rewrites_disable_subtraction_alias_acceleration() {
    let mut ctx = context();
    assert!(accelerates_integer_alias(&ctx, "sub"));
    let equation = apps(
        g("Eq"),
        &[
            nat(),
            apps(g("Nat_add"), &[g("Zero"), g("Zero")]),
            g("Zero"),
        ],
    );
    ctx.declare(
        "subtraction_zero_add_zero",
        equation,
        apps(g("refl"), &[nat(), g("Zero")]),
    )
    .unwrap();
    ctx.add_simp_lemma("subtraction_zero_add_zero").unwrap();
    assert!(!accelerates_integer_alias(&ctx, "sub"));
}
