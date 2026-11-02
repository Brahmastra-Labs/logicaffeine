use super::*;
use crate::Universe;

fn context() -> Context {
    let mut ctx = Context::new();
    crate::prelude::StandardLibrary::register(&mut ctx);
    ctx
}
/// The stable-meaning gate refuses `change` in the full prelude, and the refused attempt
/// leaves `alias` accelerated; the caller then permits the change to test the cache itself.
pub(super) fn gate_keeps(
    ctx: &Context,
    alias: &str,
    change: impl FnOnce(&mut Context) -> crate::error::KernelResult<()>,
) {
    let kept = ctx
        .refused_meaning_change_for_test(change)
        .unwrap_or_else(|| panic!("the gate refuses the change under `{alias}`'s dependents"));
    assert!(
        accelerates_integer_alias(&kept, alias),
        "a refused change leaves `{alias}` attested"
    );
}

#[test]
fn warmed_alias_attestation_rejects_extra_or_reordered_constructors() {
    let mut ctx = context();
    assert!(accelerates_integer_alias(&ctx, "add"));
    gate_keeps(&ctx, "add", |c| {
        c.add_constructor("AnotherNat", "Nat", g("Nat"));
        Ok(())
    });
    ctx.permit_meaning_changes_for_cache_tests();
    ctx.add_constructor("AnotherNat", "Nat", g("Nat"));
    assert!(!accelerates_integer_alias(&ctx, "add"));
    let mut ctx = context();
    assert!(accelerates_integer_alias(&ctx, "mul"));
    // Under the gate the identical header and constructors are KEPT, in their order.
    let mut kept = ctx.clone();
    kept.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    kept.add_constructor("Succ", "Nat", arrow(g("Nat"), g("Nat")));
    kept.add_constructor("Zero", "Nat", g("Nat"));
    assert_eq!(
        kept.get_constructors("Nat")
            .iter()
            .map(|(n, _)| *n)
            .collect::<Vec<_>>(),
        ["Zero", "Succ"]
    );
    assert!(accelerates_integer_alias(&kept, "mul"));
    ctx.permit_meaning_changes_for_cache_tests();
    ctx.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    ctx.add_constructor("Succ", "Nat", arrow(g("Nat"), g("Nat")));
    ctx.add_constructor("Zero", "Nat", g("Nat"));
    assert!(!accelerates_integer_alias(&ctx, "mul"));
}

#[test]
fn warmed_alias_attestation_rejects_changed_inductive_parameter_splits() {
    let mut ctx = context();
    assert!(accelerates_integer_alias(&ctx, "add"));
    gate_keeps(&ctx, "add", |c| {
        c.set_inductive_params("Int", 1);
        Ok(())
    });
    ctx.permit_meaning_changes_for_cache_tests();
    ctx.set_inductive_params("Int", 1);
    assert!(!accelerates_integer_alias(&ctx, "add"));
    let mut ctx = context();
    assert!(accelerates_integer_alias(&ctx, "mul"));
    gate_keeps(&ctx, "mul", |c| {
        c.set_inductive_params("Nat", 1);
        Ok(())
    });
    ctx.permit_meaning_changes_for_cache_tests();
    ctx.set_inductive_params("Nat", 1);
    assert!(!accelerates_integer_alias(&ctx, "mul"));
}

#[test]
fn explicit_polymorphic_alias_or_dependency_is_not_a_monomorphic_shortcut() {
    let mut ctx = context();
    assert!(accelerates_integer_alias(&ctx, "add"));
    gate_keeps(&ctx, "add", |c| {
        c.declare_universe_poly("add", vec![], int_binop(), g("Int_add"))
    });
    ctx.permit_meaning_changes_for_cache_tests();
    ctx.declare_universe_poly("add", vec![], int_binop(), g("Int_add"))
        .unwrap();
    assert!(!accelerates_integer_alias(&ctx, "add"));
    let mut ctx = context();
    assert!(accelerates_integer_alias(&ctx, "mul"));
    let ty = ctx.get_definition_type("Nat_mul").unwrap().clone();
    let body = ctx.get_definition_body("Nat_mul").unwrap().clone();
    gate_keeps(&ctx, "mul", |c| {
        c.declare_universe_poly("Nat_mul", vec![], ty.clone(), body.clone())
    });
    ctx.permit_meaning_changes_for_cache_tests();
    ctx.declare_universe_poly("Nat_mul", vec![], ty, body)
        .unwrap();
    assert!(!accelerates_integer_alias(&ctx, "mul"));
}

#[test]
fn pending_dependency_never_reuses_a_previous_positive_attestation() {
    let mut ctx = context();
    assert!(accelerates_integer_alias(&ctx, "add"));
    gate_keeps(&ctx, "add", |c| {
        c.declare("Int_add", int_binop(), g("future_integer_add"))
    });
    ctx.permit_meaning_changes_for_cache_tests();
    ctx.declare("Int_add", int_binop(), g("future_integer_add"))
        .unwrap();
    assert!(!accelerates_integer_alias(&ctx, "add"));
    ctx.declare(
        "future_integer_add",
        int_binop(),
        lam("a", int(), lam("b", int(), v("a"))),
    )
    .unwrap();
    assert!(!accelerates_integer_alias(&ctx, "add"));
}

#[test]
fn registered_rewrites_force_the_existing_body_evaluation_path() {
    let mut ctx = context();
    assert!(accelerates_integer_alias(&ctx, "add"));
    let equation = apps(
        g("Eq"),
        &[
            nat(),
            apps(g("Nat_add"), &[g("Zero"), g("Zero")]),
            g("Zero"),
        ],
    );
    ctx.declare(
        "zero_add_zero",
        equation,
        apps(g("refl"), &[nat(), g("Zero")]),
    )
    .unwrap();
    ctx.add_simp_lemma("zero_add_zero").unwrap();
    assert!(!accelerates_integer_alias(&ctx, "add"));
    assert!(!accelerates_integer_alias(&ctx, "mul"));
}
