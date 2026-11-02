use logicaffeine_kernel::elaborate::{elaborate, surface_elaborate_against, MetaCtx};
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, normalize, Context, Literal, Term, Universe};

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn apps(f: Term, args: &[Term]) -> Term {
    args.iter().fold(f, |f, a| app(f, a.clone()))
}
fn nat(n: i64) -> Term {
    Term::Lit(Literal::Nat(logicaffeine_base::BigInt::from_i64(n)))
}
fn c(n: &str) -> Term {
    Term::Const {
        name: n.into(),
        levels: vec![Universe::Type(0)],
    }
}
fn goal(carrier: Term, n: Term) -> Term {
    apps(c("OfNat"), &[carrier, n])
}
fn dictionary(carrier: Term, n: Term, value: Term) -> Term {
    apps(c("OfNat_mk"), &[carrier, n, value])
}
fn context() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c
}
fn custom(c: &mut Context) {
    c.add_structure("Mod7", &[], &[("rep", g("Nat"))]);
    let carrier = g("Mod7");
    let value = app(g("Mod7_mk"), nat(2));
    let ty = goal(carrier.clone(), nat(9));
    let body = dictionary(carrier, nat(9), value);
    assert!(
        infer_type(c, &body).is_ok(),
        "OfNat dictionary must be available and checked"
    );
    c.add_instance(ty, body);
}

#[test]
fn prelude_ofnat_is_a_checked_dictionary_with_a_nat_default() {
    let ctx = context();
    for n in ["OfNat", "OfNat_mk", "OfNat_ofNat", "instOfNatNat"] {
        assert!(
            ctx.get_global(n).is_some(),
            "missing {n}: {:?} / {:?}",
            ctx.refused_declarations(),
            ctx.pending_obligations()
        );
    }
    assert!(
        ctx.refused_declarations().is_empty(),
        "{:?}",
        ctx.refused_declarations()
    );
    let term = apps(
        c("OfNat_ofNat"),
        &[g("Nat"), nat(8), app(g("instOfNatNat"), nat(8))],
    );
    assert_eq!(infer_type(&ctx, &term).unwrap(), g("Nat"));
    assert_eq!(normalize(&ctx, &term), nat(8));
}

#[test]
fn user_dictionary_controls_the_value_of_a_numeral() {
    let mut ctx = context();
    custom(&mut ctx);
    let term = surface_elaborate_against(&ctx, &nat(9), Some(&g("Mod7"))).unwrap();
    assert_eq!(infer_type(&ctx, &term).unwrap(), g("Mod7"));
    assert_eq!(normalize(&ctx, &term), app(g("Mod7_mk"), nat(2)));
}

#[test]
fn numeral_instances_reach_function_arguments_and_record_fields() {
    let mut ctx = context();
    custom(&mut ctx);
    ctx.add_declaration(
        "consume",
        Term::Pi {
            param: "x".into(),
            param_type: Box::new(g("Mod7")),
            body_type: Box::new(g("Mod7")),
        },
    );
    let term = surface_elaborate_against(&ctx, &app(g("consume"), nat(9)), None).unwrap();
    assert_eq!(
        normalize(&ctx, &term),
        app(g("consume"), app(g("Mod7_mk"), nat(2)))
    );
    ctx.add_structure("Envelope", &[], &[("value", g("Mod7"))]);
    let term = surface_elaborate_against(&ctx, &app(g("Envelope_mk"), nat(9)), None).unwrap();
    assert_eq!(
        normalize(&ctx, &term),
        app(g("Envelope_mk"), app(g("Mod7_mk"), nat(2)))
    );
}

#[test]
fn the_core_elaborator_uses_the_same_numeral_dictionary() {
    let mut ctx = context();
    custom(&mut ctx);
    let (term, ty) = elaborate(&ctx, &mut MetaCtx::new(), &nat(9), Some(&g("Mod7"))).unwrap();
    assert_eq!(ty, g("Mod7"));
    assert_eq!(normalize(&ctx, &term), app(g("Mod7_mk"), nat(2)));
}

#[test]
fn a_declared_nat_instance_overrides_the_default_even_when_the_literal_already_types() {
    let mut ctx = context();
    ctx.add_instance(goal(g("Nat"), nat(9)), dictionary(g("Nat"), nat(9), nat(2)));
    let term = surface_elaborate_against(&ctx, &nat(9), Some(&g("Nat"))).unwrap();
    assert_eq!(normalize(&ctx, &term), nat(2));
}

#[test]
fn an_invalid_selected_dictionary_is_refused_instead_of_falling_back_to_injection() {
    let mut ctx = context();
    ctx.add_structure("Mod7", &[], &[("rep", g("Nat"))]);
    ctx.add_instance(goal(g("Mod7"), nat(9)), g("Zero"));
    assert!(surface_elaborate_against(&ctx, &nat(9), Some(&g("Mod7"))).is_err());
}

#[test]
fn numerals_without_an_expected_type_keep_their_native_meaning() {
    let ctx = context();
    assert_eq!(
        surface_elaborate_against(&ctx, &nat(9), None).unwrap(),
        nat(9)
    );
}

#[test]
fn numeral_dictionary_projection_computation_is_checked_by_both_kernels() {
    use logicaffeine_kernel::{double_check, DoubleCheck};
    let ctx = context();
    let projected = apps(
        c("OfNat_ofNat"),
        &[g("Nat"), nat(9), dictionary(g("Nat"), nat(9), nat(2))],
    );
    let want = apps(g("Eq"), &[g("Nat"), projected, nat(2)]);
    let proof = apps(g("refl"), &[g("Nat"), nat(2)]);
    let consume = Term::Lambda {
        param: "h".into(),
        param_type: Box::new(want),
        body: Box::new(Term::Var("h".into())),
    };
    assert!(matches!(
        double_check(&ctx, &app(consume, proof)),
        DoubleCheck::Agreed
    ));
}

#[test]
fn numeral_instance_selection_happens_once_for_a_function_argument() {
    let mut ctx = context();
    for (n, value) in [(9, 2), (2, 1)] {
        ctx.add_instance(
            goal(g("Nat"), nat(n)),
            dictionary(g("Nat"), nat(n), nat(value)),
        );
    }
    ctx.add_declaration(
        "consume",
        Term::Pi {
            param: "x".into(),
            param_type: Box::new(g("Nat")),
            body_type: Box::new(g("Nat")),
        },
    );
    let term = surface_elaborate_against(&ctx, &app(g("consume"), nat(9)), None).unwrap();
    assert_eq!(normalize(&ctx, &term), app(g("consume"), nat(2)));
}

#[test]
fn a_parameterized_dictionary_is_instantiated_at_the_requested_number() {
    let mut ctx = context();
    ctx.add_structure("Wrapped", &[], &[("rep", g("Nat"))]);
    let n = Term::Var("n".into());
    let ty = Term::Pi {
        param: "n".into(),
        param_type: Box::new(g("Nat")),
        body_type: Box::new(goal(g("Wrapped"), n.clone())),
    };
    let body = Term::Lambda {
        param: "n".into(),
        param_type: Box::new(g("Nat")),
        body: Box::new(dictionary(g("Wrapped"), n.clone(), app(g("Wrapped_mk"), n))),
    };
    ctx.add_definition("allWrapped".into(), ty.clone(), body);
    assert!(ctx.get_global("allWrapped").is_some());
    ctx.add_instance(ty, g("allWrapped"));
    for i in [0, 1, 19] {
        let result = surface_elaborate_against(&ctx, &nat(i), Some(&g("Wrapped"))).unwrap();
        assert_eq!(normalize(&ctx, &result), app(g("Wrapped_mk"), nat(i)));
    }
}

#[test]
fn explicit_constant_equality_keeps_universe_instances_distinct() {
    use logicaffeine_kernel::is_subtype;
    let ctx = context();
    let at = |u| Term::Const {
        name: "OfNat".into(),
        levels: vec![u],
    };
    assert!(is_subtype(
        &ctx,
        &at(Universe::Type(0)),
        &at(Universe::Type(0))
    ));
    assert!(!is_subtype(
        &ctx,
        &at(Universe::Type(0)),
        &at(Universe::Type(1))
    ));
    assert!(!is_subtype(
        &ctx,
        &at(Universe::Type(1)),
        &at(Universe::Type(0))
    ));
}
