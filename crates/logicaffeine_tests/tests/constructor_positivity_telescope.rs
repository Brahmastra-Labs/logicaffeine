//! Polarity controls adjacent to constructor-telescope exposure; original16 stay frozen.
use logicaffeine_kernel::{infer_type, Context, KernelError, Term, Universe};
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn s() -> Term {
    Term::Sort(Universe::Type(0))
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
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn context() -> Context {
    let mut c = Context::new();
    c.set_auto_auxiliaries(false);
    c.add_inductive("Token", s());
    c.add_constructor("token", "Token", g("Token"));
    c.add_inductive("Bad", s());
    c
}
fn ctor_type(a: Term, negative: bool) -> Term {
    let field = if negative { pi("_", a, g("Token")) } else { a };
    pi("field", field, g("Bad"))
}
fn checked_alias(c: &mut Context, name: &str, body: Term) -> Term {
    let ty = infer_type(c, &body).unwrap();
    c.declare(name, ty, body).unwrap();
    assert!(c.get_definition_body(name).is_some());
    g(name)
}
fn wrapped(c: &mut Context, form: u8, negative: bool) -> Term {
    match form {
        0 => ctor_type(g("Bad"), negative),
        1 => checked_alias(c, "CtorType", ctor_type(g("Bad"), negative)),
        2 => {
            let f = checked_alias(c, "CtorFormer", lam("A", s(), ctor_type(v("A"), negative)));
            app(f, g("Bad"))
        }
        3 => app(lam("A", s(), ctor_type(v("A"), negative)), g("Bad")),
        4 => Term::Let {
            name: "A".into(),
            ty: Box::new(s()),
            value: Box::new(g("Bad")),
            body: Box::new(ctor_type(v("A"), negative)),
        },
        _ => unreachable!(),
    }
}
fn refused(form: u8) {
    let mut c = context();
    let ty = wrapped(&mut c, form, true);
    assert_eq!(
        infer_type(&c, &ty).unwrap(),
        s(),
        "polarity is separate from ordinary typing"
    );
    let answer = c.add_constructor_checked("bad", "Bad", ty);
    assert!(
        matches!(answer, Err(KernelError::PositivityViolation { .. })),
        "{answer:?}"
    );
    assert!(c.get_global("bad").is_none() && c.get_constructors("Bad").is_empty());
    assert!(
        c.get_global("token").is_some(),
        "refusal preserves unrelated declarations"
    );
}
fn accepted(form: u8) {
    let mut c = context();
    let ty = wrapped(&mut c, form, false);
    assert_eq!(infer_type(&c, &ty).unwrap(), s());
    c.add_constructor_checked("good", "Bad", ty)
        .expect("strictly positive recursive peer");
    assert!(infer_type(&c, &g("good")).is_ok());
}
#[test]
fn direct_negative_recursive_field_is_refused() {
    refused(0)
}
#[test]
fn checked_alias_negative_recursive_field_is_refused() {
    refused(1)
}
#[test]
fn named_applied_alias_negative_recursive_field_is_refused() {
    refused(2)
}
#[test]
fn beta_substitution_cannot_hide_a_negative_recursive_field() {
    refused(3)
}
#[test]
fn let_substitution_cannot_hide_a_negative_recursive_field() {
    refused(4)
}
#[test]
fn direct_positive_recursive_field_is_accepted() {
    accepted(0)
}
#[test]
fn checked_alias_positive_recursive_field_is_accepted() {
    accepted(1)
}
#[test]
fn named_applied_alias_positive_recursive_field_is_accepted() {
    accepted(2)
}
#[test]
fn beta_substitution_preserves_a_positive_recursive_field() {
    accepted(3)
}
#[test]
fn let_substitution_preserves_a_positive_recursive_field() {
    accepted(4)
}
#[test]
fn original_ill_typed_beta_argument_is_checked_before_erasure() {
    let mut c = context();
    let ty = app(lam("unused", s(), ctor_type(g("Bad"), false)), g("token"));
    assert!(matches!(
        infer_type(&c, &ty),
        Err(KernelError::TypeMismatch { .. })
    ));
    assert!(matches!(
        c.add_constructor_checked("erased", "Bad", ty),
        Err(KernelError::TypeMismatch { .. })
    ));
    assert!(c.get_global("erased").is_none() && c.get_constructors("Bad").is_empty());
}
