//! Large arguments must stay borrowed while independent head conversion aligns wrappers.
use super::*;
fn g(n: &str) -> Db {
    Db::Global(n.into())
}
fn t(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn own(x: Db) -> DbBox {
    DbBox::owned(Box::new(x))
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
fn lambda(n: &str, a: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(a),
        body: Box::new(b),
    }
}
fn tree(depth: usize, leaf: Db) -> Db {
    if depth == 0 {
        leaf
    } else {
        app(
            app(g("join"), tree(depth - 1, leaf.clone())),
            tree(depth - 1, leaf),
        )
    }
}
fn function(leaf: Db) -> Db {
    lam(g("N"), tree(7, leaf))
}
fn context() -> Context {
    let mut c = Context::new();
    c.add_declaration("N", Term::Sort(Universe::Type(0)));
    for n in ["zero", "one"] {
        c.add_declaration(n, t("N"));
    }
    let ft = pi("x", t("N"), t("N"));
    c.add_declaration("join", pi("x", t("N"), pi("y", t("N"), t("N"))));
    c.add_declaration("observe", pi("f", ft.clone(), t("N")));
    c.declare(
        "function_identity",
        pi("f", ft.clone(), ft.clone()),
        lambda("f", ft.clone(), v("f")),
    )
    .unwrap();
    c.declare(
        "value_to_function",
        pi("a", t("N"), ft.clone()),
        lambda("a", t("N"), lambda("x", t("N"), v("a"))),
    )
    .unwrap();
    c.declare(
        "function_eta",
        pi("f", ft.clone(), ft.clone()),
        lambda(
            "f",
            ft,
            lambda("x", t("N"), Term::App(Box::new(v("f")), Box::new(v("x")))),
        ),
    )
    .unwrap();
    c
}
fn observed(x: Db) -> Db {
    app(g("observe"), x)
}
fn equal_without_whnf(c: &Context, locals: &dyn LocalTypes, a: Db, b: Db) {
    WHNF_INPUT_CLONES.with(|v| v.set(Some(0)));
    let equal = def_eq(c, locals, &observed(a), &observed(b));
    let copies = WHNF_INPUT_CLONES.with(|v| v.replace(None).unwrap());
    assert!(
        equal,
        "independent conversion must establish the actual wrapper equation"
    );
    assert_eq!(
        copies, 0,
        "head alignment must not copy large arguments into ordinary WHNF"
    );
}
#[test]
fn borrowed_head_closure_aligns_identity_with_a_large_argument() {
    let c = context();
    let f = function(Db::Var(0));
    equal_without_whnf(&c, &[], app(g("function_identity"), f.clone()), f);
}
#[test]
fn borrowed_head_closure_preserves_outer_captures_below_a_fresh_lambda() {
    let c = context();
    let value = tree(7, Db::Var(0));
    let expected = lam(g("N"), shift(&value, 1, 0));
    equal_without_whnf(
        &c,
        &vec![g("N")],
        app(g("value_to_function"), value),
        expected,
    );
}
#[test]
fn borrowed_head_closure_composes_delta_beta_and_nested_binders() {
    let c = context();
    let f = function(Db::Var(0));
    equal_without_whnf(&c, &[], app(g("function_eta"), f.clone()), f);
}
#[test]
fn borrowed_head_closure_does_not_confuse_free_and_fresh_bound_variables() {
    let c = context();
    let locals = vec![g("N")];
    let a = app(g("value_to_function"), Db::Var(0));
    let b = lam(g("N"), Db::Var(0));
    assert!(!def_eq(&c, &locals, &observed(a), &observed(b)));
}
#[test]
fn borrowed_head_closure_keeps_argument_order_and_lambda_domains() {
    let c = context();
    let a = lam(g("N"), app(app(g("join"), g("zero")), g("one")));
    let b = lam(g("N"), app(app(g("join"), g("one")), g("zero")));
    assert!(!def_eq(
        &c,
        &[],
        &observed(app(g("function_identity"), a)),
        &observed(b)
    ));
    assert!(!def_eq(
        &c,
        &[],
        &lam(g("N"), Db::Var(0)),
        &lam(Db::Sort(Universe::Type(0)), Db::Var(0))
    ));
}
#[test]
fn borrowed_head_closure_rebinding_changes_the_conversion_result() {
    let c = context();
    let f = function(g("zero"));
    let left = app(g("function_identity"), f.clone());
    assert!(def_eq(
        &c,
        &[],
        &observed(left.clone()),
        &observed(f.clone())
    ));
    let mut changed = c.clone();
    let ft = pi("x", t("N"), t("N"));
    changed
        .declare(
            "function_identity",
            pi("f", ft.clone(), ft.clone()),
            lambda("f", ft, lambda("x", t("N"), t("one"))),
        )
        .unwrap();
    assert!(!def_eq(&changed, &[], &observed(left), &observed(f)));
}
