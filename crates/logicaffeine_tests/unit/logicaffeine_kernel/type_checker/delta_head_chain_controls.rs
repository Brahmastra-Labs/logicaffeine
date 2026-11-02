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

fn chain_fixture(length: usize) -> (Context, Term, Term) {
    let (mut c, a, _) = fixture();
    let mut prior = "wrapper".to_string();
    c.with_equations_suspended(|c| {
        for i in 0..length {
            let name = format!("chain_{i}");
            let mut ignored = g("Zero");
            for _ in 0..80 {
                ignored = app(g("Succ"), ignored);
            }
            let body = lam(
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
                            body: Box::new(call(&prior, v("c"), v("a"), v("b"))),
                        },
                    ),
                ),
            );
            c.declare(&name, selector_type(), body).unwrap();
            prior = name;
        }
    });
    let rhs = call(&prior, v("flag"), g("Zero"), app(g("Succ"), g("Zero")));
    (c, a, rhs)
}
#[test]
fn several_transparent_wrappers_expose_the_common_selector() {
    let (c, a, b) = chain_fixture(2);
    assert_eq!(infer_type(&c, &b).unwrap(), g("Nat"));
    let mut budget = 256;
    assert!(bounded_conversion(&c, &a, &b, &mut budget));
    assert!(budget > 0 && budget < 256);
}
#[test]
fn several_transparent_wrappers_are_symmetric() {
    let (c, a, b) = chain_fixture(2);
    let mut budget = 256;
    assert!(bounded_conversion(&c, &b, &a, &mut budget));
    assert!(budget > 0 && budget < 256);
}
#[test]
fn chain_hint_does_not_hide_rebound_inner_branches() {
    let (original, a, b) = chain_fixture(2);
    let kept = original
        .refused_meaning_change_for_test(|c| {
            c.with_equations_suspended(|c| c.declare("wrapper", selector_type(), wrapper(true)))
        })
        .expect("the gate keeps `wrapper` under `chain_0`");
    let mut budget = 256;
    assert!(bounded_conversion(&kept, &a, &b, &mut budget));
    let mut changed = original.clone();
    changed.permit_meaning_changes_for_cache_tests();
    changed
        .with_equations_suspended(|c| c.declare("wrapper", selector_type(), wrapper(true)))
        .unwrap();
    let mut budget = 256;
    assert!(!bounded_conversion(&changed, &a, &b, &mut budget));
    let mut budget = 256;
    assert!(bounded_conversion(&original, &a, &b, &mut budget));
}
#[test]
fn chain_stops_at_a_pending_definition() {
    let (mut c, a, _) = chain_fixture(2);
    c.with_equations_suspended(|c| c.declare("pending", selector_type(), g("unknown_selector")))
        .unwrap();
    assert!(c.get_definition_body("pending").is_none());
    let b = call("pending", v("flag"), g("Zero"), app(g("Succ"), g("Zero")));
    let mut budget = 256;
    assert!(!bounded_conversion(&c, &a, &b, &mut budget));
}
#[test]
fn declaration_cycle_cannot_enter_hint_environment() {
    let (mut c, a, b) = chain_fixture(2);
    assert!(c
        .with_equations_suspended(|c| c.declare("wrapper", selector_type(), g("chain_1")))
        .is_err());
    let mut budget = 256;
    assert!(bounded_conversion(&c, &a, &b, &mut budget));
}
#[test]
fn long_chain_observation_cannot_replenish_fuel() {
    let (c, a, b) = chain_fixture(48);
    let mut budget = 256;
    assert!(!bounded_conversion(&c, &a, &b, &mut budget));
    assert_eq!(budget, 0);
}

#[test]
fn transparent_wrapper_preserves_an_opaque_function_parameter() {
    let (mut c, a, _) = fixture();
    c.with_equations_suspended(|c| {
        c.declare(
            "pass_function",
            pi("f", selector_type(), selector_type()),
            lam("f", selector_type(), v("f")),
        )
    })
    .unwrap();
    let c = c.extend("opaque_function", selector_type());
    let b = app(
        app(
            app(app(g("pass_function"), v("opaque_function")), v("flag")),
            g("Zero"),
        ),
        app(g("Succ"), g("Zero")),
    );
    assert_eq!(infer_type(&c, &b).unwrap(), g("Nat"));
    let mut budget = 256;
    assert!(!bounded_conversion(&c, &a, &b, &mut budget));
}
