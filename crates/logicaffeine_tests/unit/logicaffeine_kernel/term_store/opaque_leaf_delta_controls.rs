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
fn at(n: &str, u: Universe) -> Term {
    Term::Const {
        name: n.into(),
        levels: vec![u],
    }
}
fn op() -> Term {
    at("opaque.poly", Universe::Type(0))
}
fn context() -> Context {
    let mut c = Context::new();
    c.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    c.add_constructor("Zero", "Nat", g("Nat"));
    c.add_constructor("Succ", "Nat", pi("n", g("Nat"), g("Nat")));
    let fun = pi("n", g("Nat"), g("Nat"));
    c.add_declaration("opaque.mono", fun.clone());
    for name in ["opaque.poly", "other.poly"] {
        c.add_universe_poly_declaration(name, vec!["u".into()], fun.clone())
            .unwrap();
    }
    c.with_equations_suspended(|c| {
        c.declare("tail", fun.clone(), lam("n", g("Nat"), app(op(), v("n"))))
            .unwrap();
        c.declare(
            "left",
            fun.clone(),
            lam(
                "n",
                g("Nat"),
                Term::Let {
                    name: "continuation".into(),
                    ty: Box::new(fun.clone()),
                    value: Box::new(lam("u", g("Nat"), app(g("tail"), v("u")))),
                    body: Box::new(app(op(), app(v("continuation"), v("n")))),
                },
            ),
        )
        .unwrap();
        c.declare(
            "right",
            fun.clone(),
            lam("n", g("Nat"), app(op(), app(op(), v("n")))),
        )
        .unwrap();
        c.declare(
            "mono_left",
            fun.clone(),
            lam(
                "n",
                g("Nat"),
                app(g("opaque.mono"), app(lam("x", g("Nat"), v("x")), v("n"))),
            ),
        )
        .unwrap();
        c.declare(
            "mono_right",
            fun.clone(),
            lam("n", g("Nat"), app(g("opaque.mono"), v("n"))),
        )
        .unwrap();
    });
    c.extend("fresh", g("Nat"))
}
fn call(n: &str) -> Term {
    app(g(n), v("fresh"))
}
fn same(c: &Context, a: &Term, b: &Term) -> bool {
    beta_delta_equivalent(c, a, b, 1 << 16)
}
#[test]
fn checked_helper_let_beta_graphs_compare_with_explicit_operators_opaque() {
    let c = context();
    let a = call("left");
    let b = call("right");
    assert_eq!(crate::infer_type(&c, &a).unwrap(), g("Nat"));
    assert_eq!(crate::infer_type(&c, &b).unwrap(), g("Nat"));
    assert!(same(&c, &a, &b));
}
#[test]
fn checked_helper_graphs_preserve_declared_monomorphic_opaque_leaves() {
    let c = context();
    let a = call("mono_left");
    let b = call("mono_right");
    assert_eq!(crate::infer_type(&c, &a).unwrap(), g("Nat"));
    assert_eq!(crate::infer_type(&c, &b).unwrap(), g("Nat"));
    assert!(same(&c, &a, &b));
}
#[test]
fn opaque_leaf_names_levels_and_arguments_remain_distinct() {
    let c = context();
    let a = call("left");
    for different in [
        app(op(), app(at("other.poly", Universe::Type(0)), v("fresh"))),
        app(op(), app(at("opaque.poly", Universe::Type(1)), v("fresh"))),
        app(op(), app(op(), app(g("Succ"), v("fresh")))),
    ] {
        assert_eq!(crate::infer_type(&c, &different).unwrap(), g("Nat"));
        assert!(!same(&c, &a, &different));
    }
}
#[test]
fn helper_rebinding_and_full_kernel_transparency_stay_context_local() {
    let mut c = context();
    let a = call("left");
    let b = call("right");
    c.set_reducibility("tail", crate::context::Reducibility::Irreducible);
    assert!(same(&c, &a, &b));
    let kept = c
        .refused_meaning_change_for_test(|c| {
            c.with_equations_suspended(|c| {
                c.declare(
                    "tail",
                    pi("n", g("Nat"), g("Nat")),
                    lam("n", g("Nat"), app(g("Succ"), v("n"))),
                )
            })
        })
        .expect("the gate keeps `tail` under `left`");
    assert!(same(&kept, &a, &b));
    let mut changed = c.clone();
    changed.permit_meaning_changes_for_cache_tests();
    changed
        .with_equations_suspended(|c| {
            c.declare(
                "tail",
                pi("n", g("Nat"), g("Nat")),
                lam("n", g("Nat"), app(g("Succ"), v("n"))),
            )
        })
        .unwrap();
    assert!(!same(&changed, &a, &b));
    assert!(same(&c, &a, &b));
}
#[test]
fn open_bodies_and_pending_cycle_attempts_do_not_supply_delta_evidence() {
    let mut c = context().extend("external", g("Nat"));
    c.with_equations_suspended(|c| c.declare("open", g("Nat"), v("external")))
        .unwrap();
    assert!(c.get_definition_body("open").is_some());
    assert!(!same(&c, &g("open"), &v("external")));
    assert!(c
        .with_equations_suspended(|c| c.declare("pending_cycle", g("Nat"), g("pending_cycle")))
        .is_err());
    c.with_equations_suspended(|c| c.declare("forward", g("Nat"), g("future_value")))
        .unwrap();
    assert!(c.get_definition_body("forward").is_none());
    assert!(!same(&c, &g("forward"), &g("Zero")));
    assert!(c.get_definition_body("pending_cycle").is_none());
    assert!(crate::infer_type(&c, &g("pending_cycle")).is_err());
    assert!(!same(&c, &g("pending_cycle"), &g("Zero")));
}
#[test]
fn constant_arity_and_unknown_names_are_still_typing_errors() {
    let c = context();
    let malformed = Term::Const {
        name: "opaque.poly".into(),
        levels: vec![],
    };
    assert!(crate::infer_type(&c, &malformed).is_err());
    assert!(crate::recheck(&c, &malformed).is_err());
    assert!(crate::infer_type(&c, &g("absent")).is_err());
    assert!(crate::recheck(&c, &g("absent")).is_err());
    assert!(!same(&c, &malformed, &op()));
    assert!(!same(&c, &g("absent"), &g("another.absent")));
    // A bare Global never chooses an implicit instantiation for this poly name.
    assert!(!same(&c, &g("opaque.poly"), &op()));
}
#[test]
fn graph_probe_keeps_hole_float_and_exhaustion_admission() {
    let c = context();
    let a = call("left");
    let b = call("right");
    assert!(!beta_delta_equivalent(&c, &a, &b, 0));
    assert!(!beta_delta_equivalent(&c, &a, &b, 1));
    assert!(!same(
        &c,
        &app(g("left"), Term::Hole),
        &app(g("right"), Term::Hole)
    ));
    let nan = Term::Lit(crate::Literal::Float(f64::from_bits(0x7ff8000000000001)));
    let other = Term::Lit(crate::Literal::Float(f64::from_bits(0x7ff8000000000002)));
    assert!(!same(&c, &nan, &other));
    // Retain this probe's existing conservative non-reflexive float admission.
    assert!(!same(&c, &nan, &nan));
}
#[test]
fn lexical_capture_is_not_erased_by_opaque_graph_identity() {
    let c = context().extend("y", g("Nat"));
    let a = app(
        lam("x", g("Nat"), lam("y", g("Nat"), app(op(), v("x")))),
        v("y"),
    );
    let good = lam("fresh_binder", g("Nat"), app(op(), v("y")));
    let bad = lam("y", g("Nat"), app(op(), v("y")));
    assert!(crate::defeq_for_test(
        &c,
        &crate::infer_type(&c, &a).unwrap(),
        &crate::infer_type(&c, &good).unwrap()
    ));
    assert!(same(&c, &a, &good));
    assert!(!same(&c, &a, &bad));
}
