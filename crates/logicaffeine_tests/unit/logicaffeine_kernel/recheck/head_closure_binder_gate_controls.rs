//! A common outer binder must not hide the same bounded function-head alignment
//! already supported for a bare argument of an opaque observation function.
use super::*;
fn g(n: &str) -> Db {
    Db::Global(n.into())
}
fn t(n: &str) -> Term {
    Term::Global(n.into())
}
fn own(t: Db) -> DbBox {
    DbBox::owned(Box::new(t))
}
fn app(f: Db, a: Db) -> Db {
    Db::App(own(f), own(a))
}
fn lam(a: Db, b: Db) -> Db {
    Db::Lam(own(a), own(b))
}
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn tree(depth: usize) -> Db {
    if depth == 0 {
        Db::Var(1)
    } else {
        app(app(g("join"), tree(depth - 1)), tree(depth - 1))
    }
}
#[test]
fn borrowed_head_closure_reaches_nested_alias_below_a_common_outer_lambda() {
    let mut c = Context::new();
    c.add_declaration("N", Term::Sort(Universe::Type(0)));
    let ft = pi("x", t("N"), t("N"));
    c.add_declaration("join", pi("a", t("N"), pi("b", t("N"), t("N"))));
    c.add_declaration("observe", pi("f", pi("outer", t("N"), ft.clone()), t("N")));
    c.declare(
        "function_identity",
        pi("f", ft.clone(), ft.clone()),
        Term::Lambda {
            param: "f".into(),
            param_type: Box::new(ft),
            body: Box::new(Term::Var("f".into())),
        },
    )
    .unwrap();
    // Var1 in the large inner function captures the common outer binder. The
    // borrowed probe must carry that capture through the identity substitution.
    let large = lam(g("N"), tree(7));
    let left = app(
        g("observe"),
        lam(g("N"), app(g("function_identity"), large.clone())),
    );
    let right = app(g("observe"), lam(g("N"), large));
    let previous = WHNF_INPUT_CLONES.with(|n| n.replace(Some(0)));
    let equal = def_eq(&c, &[], &left, &right);
    let copies = WHNF_INPUT_CLONES.with(|n| n.replace(previous).unwrap());
    assert!(
        equal,
        "the checked identity equation must retain its outer capture"
    );
    assert_eq!(
        copies, 0,
        "matching outer lambdas must not hide the borrowed function-head alignment"
    );
}
