use super::*;
use crate::prelude::StandardLibrary;
use crate::{Reducibility, Universe};
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn lam(n: &str, a: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(a),
        body: Box::new(b),
    }
}
fn sort() -> Term {
    Term::Sort(Universe::Type(0))
}
fn setup() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c.declare(
        "CorePi.Family",
        pi("n", g("Nat"), sort()),
        lam("n", g("Nat"), g("Nat")),
    )
    .unwrap();
    let f = app(g("CorePi.Family"), v("n"));
    c.declare(
        "CorePi.keep",
        pi("n", g("Nat"), pi("x", f.clone(), g("Nat"))),
        lam("n", g("Nat"), lam("x", f, v("n"))),
    )
    .unwrap();
    assert!(c.pending_obligations().is_empty());
    c
}
#[test]
fn core_application_preserves_unused_dependent_pi_suffix() {
    let c = setup();
    let input = app(g("CorePi.keep"), g("Zero"));
    let (term, ty) = elaborate(&c, &mut MetaCtx::new(), &input, None).unwrap();
    assert_eq!(term, input);
    let expected = pi("x", app(g("CorePi.Family"), g("Zero")), g("Nat"));
    assert_eq!(
        ty, expected,
        "core App needs only the outer Pi, not a normalized suffix"
    );
    assert!(crate::is_subtype(
        &c,
        &crate::infer_type(&c, &term).unwrap(),
        &ty
    ));
    assert!(crate::is_subtype(
        &c,
        &crate::recheck(&c, &term).unwrap(),
        &ty
    ));
}
#[test]
fn nested_core_application_checks_argument_and_result_in_both_kernels() {
    let c = setup();
    let input = app(app(g("CorePi.keep"), g("Zero")), g("Zero"));
    let (term, ty) = elaborate(&c, &mut MetaCtx::new(), &input, Some(&g("Nat"))).unwrap();
    assert_eq!(term, input);
    assert_eq!(ty, g("Nat"));
    assert_eq!(crate::infer_type(&c, &term).unwrap(), g("Nat"));
    assert_eq!(crate::recheck(&c, &term).unwrap(), g("Nat"));
}
#[test]
fn nested_core_application_still_rejects_wrong_argument() {
    let c = setup();
    let input = app(app(g("CorePi.keep"), g("Zero")), g("Nat"));
    assert!(elaborate(&c, &mut MetaCtx::new(), &input, None).is_err());
    assert!(crate::infer_type(&c, &input).is_err());
    assert!(crate::recheck(&c, &input).is_err());
}
#[test]
fn core_lambda_preserves_and_alpha_renames_expected_dependent_codomain() {
    let c = setup();
    let expected = pi("formal", g("Nat"), app(g("CorePi.Family"), v("formal")));
    let input = lam("actual", g("Nat"), Term::Hole);
    let mut m = MetaCtx::new();
    let (term, ty) = elaborate(&c, &mut m, &input, Some(&expected)).unwrap();
    assert_eq!(
        ty,
        pi("actual", g("Nat"), app(g("CorePi.Family"), v("actual")))
    );
    // This API returns a pending elaboration, not a certified term: the hole
    // remains an unsolved meta while its expected type is preserved exactly.
    let Term::Lambda { body, .. } = term else {
        panic!("lambda expected")
    };
    let Term::Var(name) = *body else {
        panic!("pending meta expected")
    };
    assert!(is_meta(&name));
    assert!(m.solution(&name).is_none());
}
#[test]
fn surface_lambda_infers_structured_domain_and_checks_actual_value() {
    let c = setup();
    let domain = app(g("CorePi.Family"), g("Zero"));
    let expected = pi("formal", domain.clone(), g("Nat"));
    let input = lam("actual", Term::Hole, g("Zero"));
    let term = surface_elaborate_against(&c, &input, Some(&expected)).unwrap();
    assert_eq!(term, lam("actual", domain, g("Zero")));
    assert!(crate::is_subtype(
        &c,
        &crate::infer_type(&c, &term).unwrap(),
        &expected
    ));
    assert!(crate::is_subtype(
        &c,
        &crate::recheck(&c, &term).unwrap(),
        &expected
    ));
}
#[test]
fn core_application_alias_keeps_elaborator_transparency() {
    let mut c = setup();
    c.declare("CorePi.Alias", sort(), pi("x", g("Nat"), g("Nat")))
        .unwrap();
    c.add_declaration("CorePi.opaque_function", g("CorePi.Alias"));
    let input = app(g("CorePi.opaque_function"), g("Zero"));
    assert!(elaborate(&c, &mut MetaCtx::new(), &input, None).is_ok());
    c.set_reducibility("CorePi.Alias", Reducibility::Irreducible);
    assert!(matches!(
        elaborate(&c, &mut MetaCtx::new(), &input, None),
        Err(KernelError::NotAFunction(_))
    ));
}
#[test]
fn active_simp_preserves_existing_core_application_normalization() {
    let mut c = setup();
    c.declare(
        "CorePi.id",
        pi("n", g("Nat"), g("Nat")),
        lam("n", g("Nat"), v("n")),
    )
    .unwrap();
    let equality = app(
        app(app(g("Eq"), g("Nat")), app(g("CorePi.id"), v("n"))),
        v("n"),
    );
    c.declare(
        "CorePi.id.rule",
        pi("n", g("Nat"), equality),
        lam("n", g("Nat"), app(app(g("refl"), g("Nat")), v("n"))),
    )
    .unwrap();
    c.add_simp_lemma("CorePi.id.rule").unwrap();
    assert!(!c.simp_rules().is_empty());
    let (_, ty) = elaborate(
        &c,
        &mut MetaCtx::new(),
        &app(g("CorePi.keep"), g("Zero")),
        None,
    )
    .unwrap();
    assert_eq!(ty, pi("x", g("Nat"), g("Nat")));
}
