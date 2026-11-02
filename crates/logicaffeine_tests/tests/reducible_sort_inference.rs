use logicaffeine_kernel::{infer_type, Context, Term, Universe};
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
#[test]
fn a_binders_type_is_reduced_before_testing_whether_it_is_a_sort() {
    let mut ctx = Context::new();
    ctx.declare(
        "P",
        Term::Sort(Universe::Type(0)),
        Term::Sort(Universe::Prop),
    )
    .unwrap();
    let proposition = pi("p", g("P"), pi("h", v("p"), v("p")));
    assert_eq!(
        infer_type(&ctx, &proposition).unwrap(),
        Term::Sort(Universe::Prop)
    );
}
#[test]
fn a_non_sort_alias_still_cannot_be_used_as_a_sort() {
    let mut ctx = Context::new();
    ctx.add_declaration("P", Term::Sort(Universe::Prop));
    ctx.add_declaration("p", g("P"));
    let bad = pi("h", g("p"), g("P"));
    assert!(infer_type(&ctx, &bad).is_err());
}
