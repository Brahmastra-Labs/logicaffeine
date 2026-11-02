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
fn lam(n: &str, ty: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(ty),
        body: Box::new(b),
    }
}
fn pi(n: &str, ty: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(ty),
        body_type: Box::new(b),
    }
}
fn call(n: &str, c: Term, a: Term, b: Term) -> Term {
    app(app(app(g(n), c), a), b)
}
fn wrapper(swapped: bool) -> Term {
    let mut ignored = g("Zero");
    for _ in 0..80 {
        ignored = app(g("Succ"), ignored);
    }
    let (a, b) = if swapped {
        (v("b"), v("a"))
    } else {
        (v("a"), v("b"))
    };
    lam(
        "c",
        g("Flag"),
        lam(
            "a",
            g("Nat"),
            lam(
                "b",
                g("Nat"),
                Term::Let {
                    name: "unused".into(),
                    ty: Box::new(g("Nat")),
                    value: Box::new(ignored),
                    body: Box::new(call("select", v("c"), a, b)),
                },
            ),
        ),
    )
}
fn selector_type() -> Term {
    pi(
        "c",
        g("Flag"),
        pi("a", g("Nat"), pi("b", g("Nat"), g("Nat"))),
    )
}
fn fixture() -> (Context, Term, Term) {
    let mut c = Context::new();
    c.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    c.add_constructor("Zero", "Nat", g("Nat"));
    c.add_constructor("Succ", "Nat", pi("n", g("Nat"), g("Nat")));
    c.add_inductive("Flag", Term::Sort(Universe::Type(0)));
    c.add_constructor("yes", "Flag", g("Flag"));
    c.add_constructor("no", "Flag", g("Flag"));
    c.with_equations_suspended(|c| {
        let mut body = v("c");
        for i in (0..96).rev() {
            let name = format!("flag_tail_{i}");
            c.declare(
                &name,
                pi("c", g("Flag"), g("Flag")),
                lam("c", g("Flag"), body),
            )
            .unwrap();
            body = app(g(&name), v("c"));
        }
        let select = lam(
            "c",
            g("Flag"),
            lam(
                "a",
                g("Nat"),
                lam(
                    "b",
                    g("Nat"),
                    Term::Match {
                        discriminant: Box::new(app(g("flag_tail_0"), v("c"))),
                        motive: Box::new(lam("unused", g("Flag"), g("Nat"))),
                        cases: vec![v("a"), v("b")],
                    },
                ),
            ),
        );
        c.declare("select", selector_type(), select).unwrap();
        c.declare("wrapper", selector_type(), wrapper(false))
            .unwrap();
    });
    let lhs = call("select", v("flag"), g("Zero"), app(g("Succ"), g("Zero")));
    let rhs = call("wrapper", v("flag"), g("Zero"), app(g("Succ"), g("Zero")));
    (c.extend("flag", g("Flag")), lhs, rhs)
}
#[test]
fn larger_alias_exposes_common_selector_before_its_decision_procedure() {
    let (c, a, b) = fixture();
    assert_eq!(infer_type(&c, &a).unwrap(), g("Nat"));
    assert_eq!(infer_type(&c, &b).unwrap(), g("Nat"));
    let mut budget = 256;
    assert!(
        bounded_conversion(&c, &a, &b, &mut budget),
        "head-aligning alias delta should avoid evaluating the common selector"
    );
    assert!(budget > 0 && budget < 256);
}
#[test]
fn head_alignment_is_symmetric_with_the_same_total_budget() {
    let (c, a, b) = fixture();
    let mut budget = 256;
    assert!(bounded_conversion(&c, &b, &a, &mut budget));
    assert!(budget > 0 && budget < 256);
}
#[test]
fn changed_branches_are_not_equal_merely_because_heads_align() {
    let (mut c, a, _) = fixture();
    c.with_equations_suspended(|c| c.declare("wrapper", selector_type(), wrapper(true)))
        .unwrap();
    let changed = call("wrapper", v("flag"), g("Zero"), app(g("Succ"), g("Zero")));
    let mut budget = 256;
    assert!(!bounded_conversion(&c, &a, &changed, &mut budget));
}
#[test]
fn head_alignment_observes_context_rebinding_and_preserves_the_original() {
    let (original, a, b) = fixture();
    let mut changed = original.clone();
    changed
        .with_equations_suspended(|c| c.declare("wrapper", selector_type(), wrapper(true)))
        .unwrap();
    let mut budget = 256;
    assert!(!bounded_conversion(&changed, &a, &b, &mut budget));
    let mut budget = 256;
    assert!(bounded_conversion(&original, &a, &b, &mut budget));
}
#[test]
fn head_alignment_does_not_replenish_exhausted_work() {
    let (c, a, b) = fixture();
    for initial in [0, 1] {
        let mut budget = initial;
        assert!(!bounded_conversion(&c, &a, &b, &mut budget));
        assert_eq!(budget, 0);
    }
}
