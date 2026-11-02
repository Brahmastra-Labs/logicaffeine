use super::*;
use crate::term::Universe;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn sort() -> Term {
    Term::Sort(Universe::Type(0))
}
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn lambda(body: Term) -> Term {
    Term::Lambda {
        param: "A".into(),
        param_type: Box::new(sort()),
        body: Box::new(body),
    }
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn context() -> Context {
    let mut ctx = Context::new();
    ctx.set_auto_auxiliaries(false);
    ctx.add_inductive("Bad", sort());
    ctx
}
fn positive(a: Term) -> Term {
    pi("field", a, g("Bad"))
}
fn refusal(ctx: &Context, term: &Term, allowance: u32) {
    assert_eq!(
        crate::type_checker::infer_sort(ctx, term).unwrap(),
        Universe::Type(0)
    );
    let delta = Delta::over(ctx);
    *delta.fuel.borrow_mut() = allowance;
    let result = check_strictly_positive(&delta, &["Bad"], "ctor", term);
    assert!(
        matches!(result, Err(KernelError::CertificationError(_))),
        "{result:?}"
    );
    assert_eq!(*delta.fuel.borrow(), 0);
    assert!(delta.inconclusive.get());
}
#[test]
fn beta_exposure_with_no_remaining_allowance_is_inconclusive() {
    let ctx = context();
    refusal(&ctx, &app(lambda(positive(v("A"))), g("Bad")), 0);
}
#[test]
fn let_exposure_with_no_remaining_allowance_is_inconclusive() {
    let ctx = context();
    let term = Term::Let {
        name: "A".into(),
        ty: Box::new(sort()),
        value: Box::new(g("Bad")),
        body: Box::new(positive(v("A"))),
    };
    refusal(&ctx, &term, 0);
}
#[test]
fn let_and_beta_exposure_share_the_same_remaining_allowance() {
    let ctx = context();
    let term = Term::Let {
        name: "B".into(),
        ty: Box::new(sort()),
        value: Box::new(g("Bad")),
        body: Box::new(app(lambda(positive(v("A"))), v("B"))),
    };
    refusal(&ctx, &term, 1);
}
#[test]
fn checked_named_alias_exposure_with_no_allowance_is_inconclusive() {
    let mut ctx = context();
    let body = lambda(positive(v("A")));
    let ty = crate::type_checker::infer_type(&ctx, &body).unwrap();
    ctx.declare("Former", ty, body).unwrap();
    assert!(ctx.get_definition_body("Former").is_some());
    refusal(&ctx, &app(g("Former"), g("Bad")), 0);
}
