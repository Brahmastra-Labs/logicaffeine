use super::*;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn lam(n: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}
fn pi(n: &str, ty: Term, body: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(ty),
        body_type: Box::new(body),
    }
}
fn continuation(n: usize) -> Term {
    let mut body = v("x");
    for _ in 0..n {
        body = app(g("Succ"), body);
    }
    lam("x", g("Nat"), body)
}
fn fixture() -> (Context, Term, Term, Term) {
    let mut c = Context::new();
    c.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    c.add_constructor("Zero", "Nat", g("Nat"));
    c.add_constructor("Succ", "Nat", pi("n", g("Nat"), g("Nat")));
    c.add_declaration("combine", pi("x", g("Nat"), pi("y", g("Nat"), g("Nat"))));
    let unary = pi("x", g("Nat"), g("Nat"));
    let mut tree = app(v("f"), g("Zero"));
    for _ in 0..6 {
        tree = app(app(g("combine"), tree.clone()), tree);
    }
    let body = continuation(40);
    c.with_equations_suspended(|c| {
        c.declare(
            "dispatch",
            pi("f", unary.clone(), g("Nat")),
            lam("f", unary.clone(), tree),
        )
        .unwrap();
        c.declare("continuation", unary, body.clone()).unwrap();
        c.declare("wrapped", g("Nat"), app(g("dispatch"), body.clone()))
            .unwrap();
    });
    let lhs = app(g("dispatch"), g("continuation"));
    let beta = app(lam("unused", g("Nat"), app(g("dispatch"), body)), g("Zero"));
    (c, lhs, beta, g("wrapped"))
}
fn measured(c: &Context, a: &Term, b: &Term) -> usize {
    assert_eq!(infer_type(c, a).unwrap(), g("Nat"));
    assert_eq!(infer_type(c, b).unwrap(), g("Nat"));
    CONVERSION_FULL_NORMALIZATIONS.with(|count| count.set(0));
    assert!(def_eq(c, a, b));
    CONVERSION_FULL_NORMALIZATIONS.with(|count| count.get())
}
#[test]
fn beta_wrapper_aligns_before_expanding_a_shared_dispatch() {
    let (c, a, b, _) = fixture();
    assert_eq!(measured(&c,&a,&b),0,
        "beta head alignment and continuation delta suffice; full dispatch normalization is unnecessary");
}
#[test]
fn helper_wrapper_aligns_before_expanding_a_shared_dispatch() {
    let (c, a, _, b) = fixture();
    assert_eq!(
        measured(&c, &a, &b),
        0,
        "a small closed wrapper must expose congruence before expanding the shared dispatch"
    );
}
#[test]
fn bounded_comparison_uses_one_budget_across_head_alignment() {
    let (c, a, b, _) = fixture();
    let mut tiny = 1;
    assert!(!bounded_conversion(&c, &a, &b, &mut tiny));
    assert_eq!(tiny, 0);
    let mut ordinary = 256;
    assert!(bounded_conversion(&c, &a, &b, &mut ordinary));
    assert!(ordinary < 256);
}
#[test]
fn unequal_continuations_do_not_certify_congruence() {
    let (c, a, _, _) = fixture();
    let different = app(
        lam("unused", g("Nat"), app(g("dispatch"), continuation(39))),
        g("Zero"),
    );
    let mut budget = 256;
    assert!(!bounded_conversion(&c, &a, &different, &mut budget));
    assert!(!def_eq(&c, &a, &different));
}
#[test]
fn rebinding_a_continuation_changes_the_conversion_result() {
    let (original, a, b, _) = fixture();
    assert!(def_eq(&original, &a, &b));
    let mut changed = original.clone();
    changed
        .with_equations_suspended(|c| {
            c.declare(
                "continuation",
                pi("x", g("Nat"), g("Nat")),
                continuation(39),
            )
        })
        .unwrap();
    assert!(!def_eq(&changed, &a, &b));
    assert!(def_eq(&original, &a, &b));
}
#[test]
fn shared_opaque_names_do_not_erase_different_arguments_or_levels() {
    let (mut c, _, _, _) = fixture();
    c.add_declaration("opaque", pi("x", g("Nat"), g("Nat")));
    let mut budget = 256;
    assert!(!bounded_conversion(
        &c,
        &app(g("opaque"), g("Zero")),
        &app(g("opaque"), app(g("Succ"), g("Zero"))),
        &mut budget
    ));
    let at = |u| Term::Const {
        name: "opaque.universe".into(),
        levels: vec![u],
    };
    let mut budget = 256;
    assert!(!bounded_conversion(
        &c,
        &at(Universe::Type(0)),
        &at(Universe::Type(1)),
        &mut budget
    ));
}
