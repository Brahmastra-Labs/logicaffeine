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
fn pi(n: &str, ty: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(ty),
        body_type: Box::new(b),
    }
}
fn lam(n: &str, ty: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(ty),
        body: Box::new(b),
    }
}
fn ty() -> Term {
    pi("a", g("Nat"), pi("b", g("Nat"), g("Nat")))
}
fn helper(a: &str, b: &str, z: &str) -> Term {
    let mut tail = v(z);
    for _ in 0..24 {
        tail = app(app(g("mix"), v(b)), tail);
    }
    lam(
        a,
        g("Nat"),
        lam(
            b,
            g("Nat"),
            Term::Let {
                name: z.into(),
                ty: Box::new(g("Nat")),
                value: Box::new(v(a)),
                body: Box::new(tail),
            },
        ),
    )
}
fn context() -> Context {
    let mut c = Context::new();
    c.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    c.add_constructor("Zero", "Nat", g("Nat"));
    c.add_constructor("Succ", "Nat", pi("n", g("Nat"), g("Nat")));
    c.add_declaration("mix", ty());
    c.with_equations_suspended(|c| {
        c.declare("left", ty(), helper("a", "b", "saved")).unwrap();
        c.declare("right", ty(), helper("x", "y", "held")).unwrap();
    });
    c
}
fn call(head: Term, a: Term, b: Term) -> Term {
    app(app(head, a), b)
}
fn probe(c: &Context, a: &Term, b: &Term) -> bool {
    let mut budget = 256;
    bounded_conversion(c, a, b, &mut budget)
}
#[test]
fn renamed_closed_helpers_compare_before_repeated_specialization() {
    let c = context().extend("m", g("Nat")).extend("n", g("Nat"));
    let a = call(g("left"), v("m"), v("n"));
    let b = call(g("right"), v("m"), v("n"));
    assert_eq!(infer_type(&c, &a).unwrap(), g("Nat"));
    assert_eq!(infer_type(&c, &b).unwrap(), g("Nat"));
    crate::reduction::take_bounded_head_clone_nodes();
    let mut budget = 256;
    assert!(bounded_conversion(&c, &a, &b, &mut budget));
    let clones = crate::reduction::take_bounded_head_clone_nodes();
    assert_eq!(
        clones, 0,
        "body identity must avoid actual beta-head input copies"
    );
    assert!(budget > 0 && budget < 256);
}
#[test]
fn inline_closed_lambda_and_helper_compare_before_specialization() {
    let c = context().extend("m", g("Nat")).extend("n", g("Nat"));
    let a = call(helper("fresh_a", "fresh_b", "remembered"), v("m"), v("n"));
    let b = call(g("right"), v("m"), v("n"));
    assert_eq!(infer_type(&c, &a).unwrap(), g("Nat"));
    assert_eq!(infer_type(&c, &b).unwrap(), g("Nat"));
    crate::reduction::take_bounded_head_clone_nodes();
    assert!(probe(&c, &a, &b));
    assert_eq!(crate::reduction::take_bounded_head_clone_nodes(), 0);
}
#[test]
fn all_arguments_remain_obligations_and_erasing_fallback_remains() {
    let mut c = context();
    assert!(!probe(
        &c,
        &call(g("left"), g("Zero"), g("Zero")),
        &call(g("right"), app(g("Succ"), g("Zero")), g("Zero"))
    ));
    assert!(!probe(
        &c,
        &call(g("left"), g("Zero"), g("Zero")),
        &call(g("right"), g("Zero"), app(g("Succ"), g("Zero")))
    ));
    c.with_equations_suspended(|c| {
        c.declare(
            "erase1",
            ty(),
            lam("a", g("Nat"), lam("b", g("Nat"), g("Zero"))),
        )
        .unwrap();
        c.declare(
            "erase2",
            ty(),
            lam("x", g("Nat"), lam("y", g("Nat"), g("Zero"))),
        )
        .unwrap();
    });
    assert!(probe(
        &c,
        &call(g("erase1"), g("Zero"), g("Zero")),
        &call(g("erase2"), app(g("Succ"), g("Zero")), g("Zero"))
    ));
}
#[test]
fn current_context_rebinding_is_observed_without_cached_identity() {
    let c = context();
    let a = call(g("left"), g("Zero"), g("Zero"));
    let b = call(g("right"), g("Zero"), g("Zero"));
    assert!(probe(&c, &a, &b));
    let mut changed = c.clone();
    changed
        .with_equations_suspended(|c| {
            c.declare(
                "right",
                ty(),
                lam("x", g("Nat"), lam("y", g("Nat"), app(g("Succ"), g("Zero")))),
            )
        })
        .unwrap();
    assert!(!probe(&changed, &a, &b));
    assert!(probe(&c, &a, &b));
}
#[test]
fn capture_and_annotated_let_identity_are_not_textual_renaming() {
    let a = helper("a", "b", "saved");
    let b = helper("x", "y", "held");
    assert!(alpha_eq(&a, &b));
    // Changing a bound occurrence to a different binder changes the result.
    let captured = helper("x", "x", "held");
    assert!(!alpha_eq(&a, &captured));
    let lhs = Term::Let {
        name: "x".into(),
        ty: Box::new(g("Nat")),
        value: Box::new(g("Zero")),
        body: Box::new(v("x")),
    };
    let rhs = Term::Let {
        name: "y".into(),
        ty: Box::new(Term::Sort(Universe::Type(0))),
        value: Box::new(g("Zero")),
        body: Box::new(v("y")),
    };
    assert!(!alpha_eq(&lhs, &rhs));
    assert!(!alpha_eq(&a, &Term::Hole));
}
#[test]
fn opaque_and_pending_bodies_are_not_available_as_identity_evidence() {
    let mut c = context();
    c.add_declaration("opaque", ty());
    c.with_equations_suspended(|c| c.declare("pending", ty(), g("unknown_body")))
        .unwrap();
    assert!(c.get_definition_body("pending").is_none());
    for name in ["opaque", "pending"] {
        assert!(crate::reduction::bounded_delta_head(&c, &g(name)).is_none());
        assert!(!probe(&c, &g("left"), &g(name)));
    }
}
#[test]
fn kernel_irreducibility_policy_and_explicit_universes_are_preserved() {
    let mut c = context();
    c.set_reducibility("right", crate::context::Reducibility::Irreducible);
    assert!(probe(&c, &g("left"), &g("right")));
    let at = |u| Term::Const {
        name: "opaque.poly".into(),
        levels: vec![u],
    };
    assert!(!probe(&c, &at(Universe::Type(0)), &at(Universe::Type(1))));
    assert!(!alpha_eq(
        &Term::Sort(Universe::Prop),
        &Term::Sort(Universe::Type(0))
    ));
}
#[test]
fn zero_budget_cannot_certify_distinct_aliases() {
    let c = context();
    let mut budget = 0;
    assert!(!bounded_conversion(
        &c,
        &g("left"),
        &g("right"),
        &mut budget
    ));
    assert_eq!(budget, 0);
}
