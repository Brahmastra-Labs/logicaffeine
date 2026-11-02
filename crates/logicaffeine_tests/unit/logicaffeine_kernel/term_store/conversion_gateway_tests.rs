use super::*;
use crate::{Context, Universe};
fn sort(n: u32) -> Term {
    Term::Sort(Universe::Type(n))
}
fn var(n: &str) -> Term {
    Term::Var(n.into())
}
fn lam(n: &str, body: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(sort(0)),
        body: Box::new(body),
    }
}
#[test]
fn shared_conversion_avoids_capture_and_rejects_the_captured_answer() {
    let c = Context::new();
    let a = Term::App(Box::new(lam("x", lam("y", var("x")))), Box::new(var("y")));
    assert!(beta_delta_equivalent(&c, &a, &lam("fresh", var("y")), 100));
    assert!(!beta_delta_equivalent(&c, &a, &lam("y", var("y")), 100));
}
#[test]
fn shared_conversion_caches_do_not_cross_contexts() {
    let mut c = Context::new();
    c.declare("A", sort(1), sort(0)).unwrap();
    let a = Term::Global("A".into());
    assert!(beta_delta_equivalent(&c, &a, &sort(0), 100));
    let mut other = Context::new();
    other
        .declare("A", sort(1), Term::Sort(Universe::Prop))
        .unwrap();
    assert!(!beta_delta_equivalent(&other, &a, &sort(0), 100));
    assert!(beta_delta_equivalent(
        &other,
        &a,
        &Term::Sort(Universe::Prop),
        100
    ));
}
#[test]
fn shared_conversion_never_certifies_holes_or_exhausted_work() {
    let c = Context::new();
    assert!(!beta_delta_equivalent(&c, &Term::Hole, &Term::Hole, 100));
    let a = Term::App(
        Box::new(lam("x", var("x"))),
        Box::new(Term::Sort(Universe::Prop)),
    );
    assert!(!beta_delta_equivalent(
        &c,
        &a,
        &Term::Sort(Universe::Prop),
        0
    ));
}
