//! Contract controls for the bounded native conversion bridge.
//! Kept separate from implementation; these checks grant no caller-supplied
//! authority to expose a definition or certify a proposed endpoint.
use super::*;

fn g(name: &str) -> Term {
    Term::Global(name.into())
}
fn v(name: &str) -> Term {
    Term::Var(name.into())
}
fn ty() -> Term {
    Term::Sort(Universe::Type(0))
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn pi(name: &str, domain: Term, body: Term) -> Term {
    Term::Pi {
        param: name.into(),
        param_type: Box::new(domain),
        body_type: Box::new(body),
    }
}
fn lam(name: &str, domain: Term, body: Term) -> Term {
    Term::Lambda {
        param: name.into(),
        param_type: Box::new(domain),
        body: Box::new(body),
    }
}
fn context() -> Context {
    let mut ctx = Context::new();
    ctx.add_declaration("A", ty());
    ctx.add_declaration("B", ty());
    ctx
}
fn with_identity() -> Context {
    let mut ctx = context();
    ctx.declare(
        "unrelated_wrapper_name",
        pi("x", ty(), ty()),
        lam("x", ty(), v("x")),
    )
    .unwrap();
    ctx
}
fn polymorphic_context() -> Context {
    let mut ctx = context();
    let u = Term::Sort(Universe::Var("u".into()));
    let w = Term::Sort(Universe::Var("w".into()));
    ctx.declare_universe_poly(
        "select_type",
        vec!["u".into(), "w".into()],
        pi("X", u.clone(), pi("Y", w.clone(), u.clone())),
        lam("X", u, lam("Y", w, v("X"))),
    )
    .unwrap();
    ctx
}
fn poly(levels: Vec<Universe>) -> Term {
    Term::Const {
        name: "select_type".into(),
        levels,
    }
}
fn instantiated_body(first: u32, second: u32) -> Term {
    lam(
        "X",
        Term::Sort(Universe::Type(first)),
        lam("Y", Term::Sort(Universe::Type(second)), v("X")),
    )
}

#[test]
fn expose_uses_the_checked_body_of_an_arbitrarily_named_wrapper() {
    let ctx = with_identity();
    let mut allowance = 100_000;
    let mut certificates = NativeCertificates::new(&ctx, &mut allowance).unwrap();
    let body = certificates
        .expose(&g("unrelated_wrapper_name"), &mut allowance)
        .unwrap();
    assert_eq!(body, lam("x", ty(), v("x")));
    assert!(allowance > 0 && allowance < 100_000);
}

#[test]
fn beta_equal_endpoints_have_a_real_conversion_certificate() {
    let ctx = with_identity();
    let mut certificates = NativeCertificates::new(&ctx, &mut 100_000).unwrap();
    let left = app(g("unrelated_wrapper_name"), g("A"));
    certificates
        .conversion(&left, &g("A"), &ty(), &mut 100_000)
        .unwrap();
}

#[test]
fn equal_types_do_not_certify_unequal_endpoints() {
    let ctx = context();
    let mut certificates = NativeCertificates::new(&ctx, &mut 100_000).unwrap();
    assert!(matches!(
        certificates.conversion(&g("A"), &g("B"), &ty(), &mut 100_000),
        Err(Verdict::Reject(_))
    ));
}

#[test]
fn a_correct_equation_at_the_wrong_claimed_type_is_rejected() {
    let ctx = context();
    let mut certificates = NativeCertificates::new(&ctx, &mut 100_000).unwrap();
    // A has type Type 0, not A. Equality alone cannot admit this witness.
    assert!(matches!(
        certificates.conversion(&g("A"), &g("A"), &g("A"), &mut 100_000),
        Err(Verdict::Reject(_))
    ));
}

#[test]
fn conversion_checks_an_erased_ill_typed_original_argument() {
    let ctx = context();
    let mut certificates = NativeCertificates::new(&ctx, &mut 100_000).unwrap();
    let left = app(lam("ignored", ty(), g("A")), ty());
    assert!(matches!(
        certificates.conversion(&left, &g("A"), &ty(), &mut 100_000),
        Err(Verdict::Reject(_))
    ));
}

#[test]
fn a_checked_opaque_body_is_not_exposed() {
    let mut ctx = context();
    ctx.declare_opaque_checked("sealed", vec![], ty(), g("A"))
        .unwrap();
    let mut certificates = NativeCertificates::new(&ctx, &mut 100_000).unwrap();
    assert!(certificates.expose(&g("sealed"), &mut 100_000).is_err());
}

#[test]
fn pending_forward_reference_definitions_are_not_exposed() {
    let mut ctx = context();
    ctx.declare("waiting", ty(), g("not_yet_declared")).unwrap();
    assert!(
        !ctx.pending_obligations().is_empty(),
        "test requires a genuine pending obligation"
    );
    let mut certificates = NativeCertificates::new(&ctx, &mut 100_000).unwrap();
    assert!(certificates.expose(&g("waiting"), &mut 100_000).is_err());
}

#[test]
fn explicit_levels_instantiate_the_whole_original_body_in_order() {
    let ctx = polymorphic_context();
    let mut certificates = NativeCertificates::new(&ctx, &mut 100_000).unwrap();
    let result = certificates
        .expose(
            &poly(vec![Universe::Type(0), Universe::Type(1)]),
            &mut 100_000,
        )
        .unwrap();
    assert_eq!(result, instantiated_body(0, 1));
}

#[test]
fn missing_and_wrong_arity_universe_arguments_are_not_inferred() {
    let ctx = polymorphic_context();
    let mut certificates = NativeCertificates::new(&ctx, &mut 100_000).unwrap();
    for constant in [
        g("select_type"),
        poly(vec![]),
        poly(vec![Universe::Type(0)]),
        poly(vec![
            Universe::Type(0),
            Universe::Type(1),
            Universe::Type(2),
        ]),
    ] {
        assert!(
            certificates.expose(&constant, &mut 100_000).is_err(),
            "wrong universe arity must not be repaired: {constant:?}"
        );
    }
}

#[test]
fn cached_polymorphic_exposure_keeps_the_ordered_universe_vector() {
    let ctx = polymorphic_context();
    let mut certificates = NativeCertificates::new(&ctx, &mut 100_000).unwrap();
    let a = poly(vec![Universe::Type(0), Universe::Type(1)]);
    let b = poly(vec![Universe::Type(1), Universe::Type(0)]);
    assert_eq!(
        certificates.expose(&a, &mut 100_000).unwrap(),
        instantiated_body(0, 1)
    );
    assert_eq!(
        certificates.expose(&b, &mut 100_000).unwrap(),
        instantiated_body(1, 0)
    );
    assert_eq!(
        certificates.expose(&a, &mut 100_000).unwrap(),
        instantiated_body(0, 1)
    );
}

#[test]
fn a_definition_cache_does_not_cross_context_clones() {
    let mut first = context();
    first.declare("chosen", ty(), g("A")).unwrap();
    let mut second = first.clone();
    second.declare("chosen", ty(), g("B")).unwrap();
    let mut left = NativeCertificates::new(&first, &mut 100_000).unwrap();
    let mut right = NativeCertificates::new(&second, &mut 100_000).unwrap();
    assert_eq!(left.expose(&g("chosen"), &mut 100_000).unwrap(), g("A"));
    assert_eq!(right.expose(&g("chosen"), &mut 100_000).unwrap(), g("B"));
    assert_eq!(left.expose(&g("chosen"), &mut 100_000).unwrap(), g("A"));
}

#[test]
fn exhausted_allowances_are_never_refilled_even_for_cached_evidence() {
    let ctx = with_identity();
    let mut empty = 0;
    assert!(matches!(
        NativeCertificates::new(&ctx, &mut empty),
        Err(Verdict::Decline(_))
    ));
    assert_eq!(empty, 0);
    let mut certificates = NativeCertificates::new(&ctx, &mut 100_000).unwrap();
    certificates
        .expose(&g("unrelated_wrapper_name"), &mut 100_000)
        .unwrap();
    assert!(matches!(
        certificates.expose(&g("unrelated_wrapper_name"), &mut empty),
        Err(Verdict::Decline(_))
    ));
    assert!(matches!(
        certificates.conversion(&g("A"), &g("A"), &ty(), &mut empty),
        Err(Verdict::Decline(_))
    ));
    assert!(matches!(
        certificates.normalize_ground(&app(lam("x", ty(), v("x")), g("A")), &mut empty),
        Err(Verdict::Decline(_))
    ));
    assert_eq!(empty, 0);
}

#[test]
fn body_materialization_is_admitted_before_a_tiny_allowance_can_clone_it() {
    let mut ctx = context();
    let mut body = g("A");
    for index in 0..128 {
        body = Term::Let {
            name: format!("unused_{index}"),
            ty: Box::new(ty()),
            value: Box::new(g("A")),
            body: Box::new(body),
        };
    }
    ctx.declare("large_valid_body", ty(), body).unwrap();
    let mut certificates = NativeCertificates::new(&ctx, &mut 100_000).unwrap();
    let mut allowance = 8;
    assert!(matches!(
        certificates.expose(&g("large_valid_body"), &mut allowance),
        Err(Verdict::Decline(_))
    ));
    assert!(allowance <= 8, "failed body admission must not refund fuel");
}

#[test]
fn a_closed_ground_application_returns_its_checked_canonical_nat_value() {
    let mut ctx = Context::new();
    crate::prelude::StandardLibrary::register(&mut ctx);
    let three = Term::Lit(crate::Literal::Nat(crate::BigInt::from_u64(3)));
    ctx.declare(
        "renamed_nat_identity",
        pi("n", g("Nat"), g("Nat")),
        lam("n", g("Nat"), v("n")),
    )
    .unwrap();
    let original = app(g("renamed_nat_identity"), three.clone());
    let mut allowance = 100_000;
    let mut certificates = NativeCertificates::new(&ctx, &mut allowance).unwrap();
    let result = certificates
        .normalize_ground(&original, &mut allowance)
        .unwrap();
    assert_eq!(result, three);
    assert!(allowance > 0 && allowance < 100_000);
    certificates
        .conversion(&original, &result, &g("Nat"), &mut allowance)
        .unwrap();
}

#[test]
fn nonconstant_exposure_and_open_or_ill_typed_ground_inputs_are_refused() {
    let ctx = with_identity();
    let mut certificates = NativeCertificates::new(&ctx, &mut 100_000).unwrap();
    assert!(certificates
        .expose(&app(g("unrelated_wrapper_name"), g("A")), &mut 100_000)
        .is_err());
    assert!(certificates
        .normalize_ground(&v("unbound"), &mut 100_000)
        .is_err());
    let invalid = app(lam("ignored", ty(), g("A")), ty());
    assert!(matches!(
        certificates.normalize_ground(&invalid, &mut 100_000),
        Err(Verdict::Reject(_))
    ));
}
