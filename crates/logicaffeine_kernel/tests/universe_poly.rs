//! R3 — universe polymorphism end to end, locked in by TDD.
//!
//! The payoff of the level algebra: a SINGLE polymorphic definition, checked once with
//! the universe variable abstract, then instantiated at as many concrete levels as you
//! like. The decisive test is the contrast — the `Type 0` instance of the identity
//! CANNOT be applied to `Type 0` itself (`Type 0 : Type 1 ≰ Type 0`), but the `Type 1`
//! instance of the SAME definition can. That gap is exactly what universe polymorphism
//! removes, and it is two-kernel-verified.

use std::collections::HashMap;

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    double_check, infer_type, instantiate_universes, normalize, Context, DoubleCheck, Term,
    Universe,
};

fn pi(p: &str, ty: Term, body: Term) -> Term {
    Term::Pi { param: p.to_string(), param_type: Box::new(ty), body_type: Box::new(body) }
}
fn arrow(a: Term, b: Term) -> Term {
    pi("_", a, b)
}
fn const_at(name: &str, levels: Vec<Universe>) -> Term {
    Term::Const { name: name.to_string(), levels }
}

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn var(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn sort(u: Universe) -> Term {
    Term::Sort(u)
}
fn lam(p: &str, ty: Term, body: Term) -> Term {
    Term::Lambda { param: p.to_string(), param_type: Box::new(ty), body: Box::new(body) }
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn subst(pairs: &[(&str, Universe)]) -> HashMap<String, Universe> {
    pairs.iter().map(|(k, v)| (k.to_string(), v.clone())).collect()
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}

#[test]
fn universe_polymorphic_identity_instantiates_at_multiple_levels() {
    let ctx = std_ctx();
    // The polymorphic identity: λA:Sort u. λa:A. a
    let id_poly = lam("A", sort(Universe::Var("u".to_string())), lam("a", var("A"), var("a")));

    // (1) It type-checks GENERICALLY — the universe variable stays abstract.
    let poly_ty = infer_type(&ctx, &id_poly).expect("polymorphic identity type-checks generically");
    assert!(matches!(poly_ty, Term::Pi { .. }), "Π(A:Sort u). A → A, got {poly_ty}");

    // (2) ...and the independent second kernel re-derives the same type.
    assert_eq!(
        double_check(&ctx, &id_poly),
        DoubleCheck::Agreed,
        "the polymorphic identity must be two-kernel-verified, got {:?}",
        double_check(&ctx, &id_poly)
    );

    // (3) Instantiate u := Type 0 — the ordinary identity on Type-0 values.
    let id0 = instantiate_universes(&id_poly, &subst(&[("u", Universe::Type(0))]));
    assert!(infer_type(&ctx, &id0).is_ok(), "id.{{0}} type-checks");
    assert!(
        infer_type(&ctx, &app(id0.clone(), g("Nat"))).is_ok(),
        "id.{{0}} Nat : Nat → Nat (Nat is a Type-0 value)"
    );

    // (4) Instantiate u := Type 1 — the SAME definition, now applicable to `Type 0`
    // ITSELF (a Type-1 value). No second identity was written.
    let id1 = instantiate_universes(&id_poly, &subst(&[("u", Universe::Type(1))]));
    assert!(infer_type(&ctx, &id1).is_ok(), "id.{{1}} type-checks");
    assert!(
        infer_type(&ctx, &app(id1.clone(), sort(Universe::Type(0)))).is_ok(),
        "id.{{1}} (Type 0) : Type 0 → Type 0 — works on a type-of-types"
    );

    // (5) THE decisive contrast: the Type-0 instance must REJECT `Type 0` (since
    // `Type 0 : Type 1 ≰ Type 0`), proving the Type-1 instance genuinely buys something
    // the monomorphic identity cannot.
    assert!(
        infer_type(&ctx, &app(id0, sort(Universe::Type(0)))).is_err(),
        "the Type-0 identity must reject Type 0 — the limitation universe polymorphism removes"
    );
}

#[test]
fn instantiation_equals_the_hand_written_monomorphic_version() {
    // Instantiating the polymorphic identity at Type 0 yields EXACTLY the term you would
    // have written by hand — universe substitution is a faithful specialization.
    let id_poly = lam("A", sort(Universe::Var("u".to_string())), lam("a", var("A"), var("a")));
    let id0 = instantiate_universes(&id_poly, &subst(&[("u", Universe::Type(0))]));
    let hand_written = lam("A", sort(Universe::Type(0)), lam("a", var("A"), var("a")));
    assert_eq!(id0, hand_written, "id.{{0}} must equal the hand-written Type-0 identity");
}

#[test]
fn a_polymorphic_constant_function_also_checks_and_specializes() {
    // A second shape — `λ(A:Sort u)(B:Sort u)(a:A)(_:B). a` (const / K) — exercises a
    // level variable shared across two binders and a Π whose universe is `max(u,u) = u`.
    let ctx = std_ctx();
    let u = Universe::Var("u".to_string());
    let k_poly = lam(
        "A",
        sort(u.clone()),
        lam("B", sort(u.clone()), lam("a", var("A"), lam("b", var("B"), var("a")))),
    );
    assert!(infer_type(&ctx, &k_poly).is_ok(), "polymorphic K type-checks");
    assert_eq!(double_check(&ctx, &k_poly), DoubleCheck::Agreed);

    let k0 = instantiate_universes(&k_poly, &subst(&[("u", Universe::Type(0))]));
    assert!(
        infer_type(&ctx, &app(app(k0, g("Nat")), g("Bool"))).is_ok(),
        "K.{{0}} Nat Bool : Nat → Bool → Nat"
    );
}

#[test]
fn universe_polymorphic_global_is_stored_and_referenced_at_levels() {
    // Register the polymorphic identity as a GLOBAL with a universe parameter, then refer
    // to it with explicit level arguments `id.{ℓ}` — the `.{}` syntax. One stored
    // definition, reused at every level.
    let mut ctx = std_ctx();
    let u = Universe::Var("u".to_string());
    // id.{u} : Π(A:Sort u). A → A := λA:Sort u. λa:A. a
    let id_ty = pi("A", sort(u.clone()), arrow(var("A"), var("A")));
    let id_body = lam("A", sort(u.clone()), lam("a", var("A"), var("a")));
    ctx.add_universe_poly("id", vec!["u".to_string()], id_ty, id_body);

    let id0 = const_at("id", vec![Universe::Type(0)]);
    let id1 = const_at("id", vec![Universe::Type(1)]);

    // (1) The reference type-checks (the stored type instantiated at the level), and the
    // independent re-checker agrees — a universe-poly GLOBAL is two-kernel-verified.
    assert!(infer_type(&ctx, &id0).is_ok(), "id.{{0}} type-checks");
    assert_eq!(
        double_check(&ctx, &id0),
        DoubleCheck::Agreed,
        "id.{{0}} must be two-kernel-verified, got {:?}",
        double_check(&ctx, &id0)
    );

    // (2) The same stored definition works at both levels — and the decisive contrast:
    // id.{0} applies to Type-0 values but NOT to Type 0 itself; id.{1} does.
    assert!(infer_type(&ctx, &app(id0.clone(), g("Nat"))).is_ok(), "id.{{0}} Nat");
    assert!(
        infer_type(&ctx, &app(id1.clone(), sort(Universe::Type(0)))).is_ok(),
        "id.{{1}} (Type 0)"
    );
    assert!(
        infer_type(&ctx, &app(id0.clone(), sort(Universe::Type(0)))).is_err(),
        "id.{{0}} must reject Type 0"
    );

    // (3) It COMPUTES: a `Const` δ-unfolds to its instantiated body, so `id.{0} Nat Zero`
    // reduces to `Zero`.
    let applied = app(app(id0, g("Nat")), g("Zero"));
    assert_eq!(normalize(&ctx, &applied), g("Zero"), "id.{{0}} Nat Zero = Zero");

    // (4) The wrong number of level arguments is rejected.
    let bad = const_at("id", vec![Universe::Type(0), Universe::Type(1)]);
    assert!(infer_type(&ctx, &bad).is_err(), "arity mismatch on universe arguments is rejected");
}
