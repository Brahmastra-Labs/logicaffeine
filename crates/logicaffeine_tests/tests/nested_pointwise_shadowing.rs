//! Nested specialization preserves lexical scope in positive function fields.
//! The inner Nat binder shadows the outer Bool binder. The equality domain
//! refers to the inner binder, so the original telescope is well typed.
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, is_subtype, recheck, Context, NestedDecl, Term, Universe};

fn g(name: &str) -> Term {
    Term::Global(name.into())
}
fn v(name: &str) -> Term {
    Term::Var(name.into())
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
fn eq_self(name: &str) -> Term {
    app(app(app(g("Eq"), g("Nat")), v(name)), v(name))
}
fn field(outer: &str, inner: &str, codomain: Term) -> Term {
    pi(
        outer,
        g("Bool"),
        pi(inner, g("Nat"), pi("h", eq_self(inner), codomain)),
    )
}
fn context() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}

#[test]
fn a_shadowed_dependent_function_telescope_is_well_typed_before_specialization() {
    let ctx = context();
    let shadowed = field("x", "x", app(g("TList"), g("Nat")));
    let renamed = field("outer", "inner", app(g("TList"), g("Nat")));
    for ty in [&shadowed, &renamed] {
        assert!(matches!(
            infer_type(&ctx, ty).expect("the primary kernel checks the original telescope"),
            Term::Sort(_)
        ));
        assert!(matches!(
            recheck(&ctx, ty).expect("the independent kernel checks the original telescope"),
            Term::Sort(_)
        ));
    }
    assert!(is_subtype(&ctx, &shadowed, &renamed) && is_subtype(&ctx, &renamed, &shadowed));
}

fn check_nested(outer: &str, inner: &str) {
    check_nested_field(
        field(outer, inner, app(g("TList"), g("ShadowTree"))),
        field("outer", "inner", g("ShadowTree$TList")),
    );
}

fn check_nested_field(source_field: Term, expected_field: Term) {
    let mut ctx = context();
    let decl = NestedDecl {
        name: "ShadowTree".into(),
        sort: Term::Sort(Universe::Type(0)),
        constructors: vec![(
            "ShadowTree_node".into(),
            pi("children", source_field, g("ShadowTree")),
        )],
    };
    let info = ctx
        .add_nested_inductive(&decl)
        .expect("a positive function field preserves lexical binder scope");
    assert_eq!(info.siblings, vec!["ShadowTree$TList".to_string()]);
    let expected = pi("children", expected_field, g("ShadowTree"));
    let ctor = g("ShadowTree_node");
    let actual = infer_type(&ctx, &ctor).expect("the specialized constructor exists");
    assert!(
        is_subtype(&ctx, &actual, &expected) && is_subtype(&ctx, &expected, &actual),
        "the specialized telescope preserves dependent domains: {actual}"
    );
    let independent = recheck(&ctx, &ctor).expect("the independent kernel checks the constructor");
    assert!(is_subtype(&ctx, &independent, &expected) && is_subtype(&ctx, &expected, &independent));
}

#[test]
fn a_nested_function_field_preserves_a_shadowed_binders_dependent_domain() {
    check_nested("x", "x");
}

#[test]
fn the_alpha_renamed_nested_function_field_has_the_same_dependent_domain() {
    check_nested("outer", "inner");
}

#[test]
fn a_shadowing_binders_own_domain_still_refers_to_the_outer_binder() {
    // In the inner binder's domain, x still means the outer Nat. Its scope changes
    // only in the inner binder's body.
    let source = pi(
        "x",
        g("Nat"),
        pi("x", eq_self("x"), app(g("TList"), g("ShadowTree"))),
    );
    let expected = pi(
        "outer",
        g("Nat"),
        pi("proof", eq_self("outer"), g("ShadowTree$TList")),
    );
    let ctx = context();
    let original = pi(
        "x",
        g("Nat"),
        pi("x", eq_self("x"), app(g("TList"), g("Nat"))),
    );
    assert!(matches!(
        infer_type(&ctx, &original).expect("the primary kernel checks the dependent domain"),
        Term::Sort(_)
    ));
    assert!(matches!(
        recheck(&ctx, &original).expect("the independent kernel checks the dependent domain"),
        Term::Sort(_)
    ));
    check_nested_field(source, expected);
}
