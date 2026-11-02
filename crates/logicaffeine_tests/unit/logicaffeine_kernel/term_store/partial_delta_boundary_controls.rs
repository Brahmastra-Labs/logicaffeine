use super::*;
use crate::Context;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn pi(n: &str, t: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(t),
        body_type: Box::new(b),
    }
}
fn lam(n: &str, t: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(t),
        body: Box::new(b),
    }
}
fn nat(n: u64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_u64(n)))
}
fn apply(n: &str, flag: Term, value: Term) -> Term {
    app(app(g(n), flag), value)
}
fn function_type() -> Term {
    pi("flag", g("Flag"), pi("n", g("Nat"), g("Nat")))
}
fn match_body() -> Term {
    lam(
        "flag",
        g("Flag"),
        lam(
            "n",
            g("Nat"),
            Term::Match {
                discriminant: Box::new(v("flag")),
                motive: Box::new(lam("_", g("Flag"), g("Nat"))),
                cases: vec![v("n"), app(g("Succ"), v("n"))],
            },
        ),
    )
}
fn context() -> Context {
    let mut c = Context::new();
    c.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    c.add_constructor("Zero", "Nat", g("Nat"));
    c.add_constructor("Succ", "Nat", pi("n", g("Nat"), g("Nat")));
    c.add_inductive("Flag", Term::Sort(Universe::Type(0)));
    c.add_constructor("off", "Flag", g("Flag"));
    c.add_constructor("on", "Flag", g("Flag"));
    c.add_declaration("Float", Term::Sort(Universe::Type(0)));
    c.with_equations_suspended(|c| {
        for name in ["boundary", "other_boundary"] {
            c.declare(name, function_type(), match_body()).unwrap();
        }
        c.declare("alias.one", function_type(), g("boundary"))
            .unwrap();
        c.declare("alias.two", function_type(), g("alias.one"))
            .unwrap();
        c.declare(
            "left",
            function_type(),
            lam(
                "flag",
                g("Flag"),
                lam(
                    "n",
                    g("Nat"),
                    apply(
                        "alias.two",
                        v("flag"),
                        app(lam("x", g("Nat"), v("x")), v("n")),
                    ),
                ),
            ),
        )
        .unwrap();
        c.declare(
            "right",
            function_type(),
            lam(
                "flag",
                g("Flag"),
                lam("n", g("Nat"), apply("boundary", v("flag"), v("n"))),
            ),
        )
        .unwrap();
    });
    c
}
fn operands() -> (Term, Term) {
    (
        apply("left", g("off"), nat(1)),
        apply("right", g("off"), nat(1)),
    )
}
fn same(c: &Context, a: &Term, b: &Term) -> bool {
    beta_delta_equivalent(c, a, b, 1 << 16)
}
#[test]
fn checked_helper_graphs_stop_at_the_same_named_match_definition() {
    let c = context();
    let (a, b) = operands();
    for term in [&a, &b] {
        assert_eq!(crate::infer_type(&c, term).unwrap(), g("Nat"));
        assert_eq!(crate::recheck(&c, term).unwrap(), g("Nat"));
    }
    assert!(same(&c, &a, &b));
}
#[test]
fn fragment_preflight_avoids_actual_match_expansion_visits() {
    let c = context();
    let (a, b) = operands();
    DELTA_EXPANSION_WORK.with(|counter| counter.set(Some((0, 0))));
    let result = beta_delta_equivalent(&c, &a, &b, 128);
    let (visits, match_visits) =
        DELTA_EXPANSION_WORK.with(|counter| counter.replace(None).unwrap());
    assert_eq!(
        match_visits, 0,
        "common named Match body must remain a leaf"
    );
    assert!(
        visits <= 128,
        "actual expansion visits stay inside the existing allowance"
    );
    assert!(
        result,
        "supported aliases and beta wrappers compare within128work units"
    );
}
#[test]
fn transitive_supported_aliases_reach_an_exact_generic_boundary() {
    let mut c = context();
    c.with_equations_suspended(|c| {
        c.declare(
            "fixed",
            pi("x", g("Nat"), g("Nat")),
            Term::Fix {
                name: "self".into(),
                body: Box::new(lam("x", g("Nat"), v("x"))),
            },
        )
        .unwrap();
        c.declare("fixed.alias", pi("x", g("Nat"), g("Nat")), g("fixed"))
            .unwrap();
    });
    let a = apply(
        "alias.two",
        g("off"),
        app(lam("x", g("Nat"), v("x")), nat(1)),
    );
    let b = apply("boundary", g("off"), nat(1));
    assert!(same(&c, &a, &b));
    assert!(same(
        &c,
        &app(g("fixed.alias"), app(lam("x", g("Nat"), v("x")), nat(1))),
        &app(g("fixed"), nat(1))
    ));
}
#[test]
fn opaque_definition_names_and_every_argument_remain_obligations() {
    let c = context();
    let (a, _) = operands();
    for b in [
        apply("boundary", g("on"), nat(1)),
        apply("boundary", g("off"), nat(2)),
        apply("other_boundary", g("off"), nat(1)),
    ] {
        assert_eq!(crate::infer_type(&c, &b).unwrap(), g("Nat"));
        assert!(!same(&c, &a, &b));
    }
}
#[test]
fn boundary_lookup_is_current_context_local_with_full_kernel_transparency() {
    let mut c = context();
    let (a, b) = operands();
    c.set_reducibility("alias.one", crate::context::Reducibility::Irreducible);
    assert!(same(&c, &a, &b));
    let kept = c
        .refused_meaning_change_for_test(|c| {
            c.with_equations_suspended(|c| {
                c.declare("alias.one", function_type(), g("other_boundary"))
            })
        })
        .expect("the gate keeps `alias.one` under `alias.two`");
    assert!(same(&kept, &a, &b));
    let mut changed = c.clone();
    changed.permit_meaning_changes_for_cache_tests();
    changed
        .with_equations_suspended(|c| c.declare("alias.one", function_type(), g("other_boundary")))
        .unwrap();
    assert!(!same(&changed, &a, &b));
    assert!(same(&c, &a, &b));
}
#[test]
fn required_unfolding_remains_available_to_both_ordinary_checkers() {
    let mut c = context();
    c.with_equations_suspended(|c| {
        c.declare(
            "choose",
            pi("flag", g("Flag"), Term::Sort(Universe::Type(0))),
            lam(
                "flag",
                g("Flag"),
                Term::Match {
                    discriminant: Box::new(v("flag")),
                    motive: Box::new(lam("_", g("Flag"), Term::Sort(Universe::Type(0)))),
                    cases: vec![g("Nat"), g("Flag")],
                },
            ),
        )
    })
    .unwrap();
    let selected = app(g("choose"), g("off"));
    assert!(
        !same(&c, &selected, &g("Nat")),
        "the partial fragment does not execute Match"
    );
    assert!(crate::defeq_for_test(&c, &selected, &g("Nat")));
    let use_selected = |flag| app(lam("value", app(g("choose"), g(flag)), v("value")), nat(1));
    let valid = use_selected("off");
    let invalid = use_selected("on");
    assert!(crate::infer_type(&c, &valid).is_ok());
    assert!(crate::recheck(&c, &valid).is_ok());
    assert!(crate::infer_type(&c, &invalid).is_err());
    assert!(crate::recheck(&c, &invalid).is_err());
}
#[test]
fn float_open_hole_cycle_and_budget_failures_are_not_reclassified_as_leaves() {
    let mut c = context().extend("external", g("Nat"));
    c.with_equations_suspended(|c| {
        c.declare(
            "open",
            function_type(),
            lam(
                "flag",
                g("Flag"),
                lam(
                    "n",
                    g("Nat"),
                    Term::Match {
                        discriminant: Box::new(v("flag")),
                        motive: Box::new(lam("_", g("Flag"), g("Nat"))),
                        cases: vec![v("external"), v("n")],
                    },
                ),
            ),
        )
        .unwrap();
        c.declare(
            "float_boundary",
            pi("flag", g("Flag"), g("Float")),
            lam(
                "flag",
                g("Flag"),
                Term::Match {
                    discriminant: Box::new(v("flag")),
                    motive: Box::new(lam("_", g("Flag"), g("Float"))),
                    cases: vec![
                        Term::Lit(Literal::Float(1.5)),
                        Term::Lit(Literal::Float(2.5)),
                    ],
                },
            ),
        )
        .unwrap();
    });
    let wrap = |t: Term| app(lam("unused", g("Nat"), t), nat(1));
    let open = apply("open", g("off"), nat(1));
    assert!(c.get_definition_body("open").is_some());
    assert!(!same(&c, &wrap(open.clone()), &open));
    let float = app(g("float_boundary"), g("off"));
    assert!(!same(&c, &wrap(float.clone()), &float));
    assert!(!same(
        &c,
        &app(g("left"), Term::Hole),
        &app(g("left"), Term::Hole)
    ));
    assert!(c
        .with_equations_suspended(|c| c.declare("cycle", g("Nat"), g("cycle")))
        .is_err());
    let (a, b) = operands();
    assert!(!beta_delta_equivalent(&c, &a, &b, 0));
    assert!(!beta_delta_equivalent(&c, &a, &b, 1));
}
#[test]
fn explicit_universe_leaves_and_bare_polymorphic_globals_keep_their_boundary() {
    let mut c = context();
    c.add_universe_poly_declaration("poly.boundary", vec!["u".into()], function_type())
        .unwrap();
    let at = |u| Term::Const {
        name: "poly.boundary".into(),
        levels: vec![Universe::Type(u)],
    };
    let call = |head| app(app(head, g("off")), nat(1));
    let left = call(at(0));
    let wrapped = app(lam("unused", g("Nat"), left.clone()), nat(1));
    assert!(same(&c, &wrapped, &left));
    assert!(!same(&c, &left, &call(at(1))));
    assert!(!same(&c, &g("poly.boundary"), &at(0)));
    let malformed = Term::Const {
        name: "poly.boundary".into(),
        levels: vec![],
    };
    assert!(crate::infer_type(&c, &malformed).is_err());
    assert!(crate::recheck(&c, &malformed).is_err());
}
