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
fn wrap(t: Term) -> Term {
    app(lam("unused", g("Nat"), t), g("Zero"))
}
fn fun_ty() -> Term {
    pi("n", g("Nat"), g("Nat"))
}
fn context() -> Context {
    let mut c = Context::new();
    c.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    c.add_constructor("Zero", "Nat", g("Nat"));
    c.add_constructor("Succ", "Nat", fun_ty());
    for n in ["opaque", "other"] {
        c.add_universe_poly_declaration(n, vec![], fun_ty())
            .unwrap();
    }
    c
}
fn same(c: &Context, a: &Term, b: &Term) -> bool {
    beta_delta_equivalent(c, a, b, 256)
}
#[test]
fn checked_wrappers_retain_empty_universe_opaque_headers() {
    let mut c = context();
    c.with_equations_suspended(|c| {
        c.declare(
            "left",
            fun_ty(),
            lam(
                "x",
                g("Nat"),
                app(g("opaque"), app(lam("y", g("Nat"), v("y")), v("x"))),
            ),
        )
        .unwrap();
        c.declare(
            "right",
            fun_ty(),
            lam("x", g("Nat"), app(g("opaque"), v("x"))),
        )
        .unwrap();
    });
    assert!(c.poly_declaration("opaque").unwrap().0.is_empty());
    assert!(c.get_definition_body("opaque").is_none());
    let a = app(g("left"), g("Zero"));
    let b = app(g("right"), g("Zero"));
    for t in [&a, &b] {
        assert_eq!(crate::infer_type(&c, t).unwrap(), g("Nat"));
        assert_eq!(crate::recheck(&c, t).unwrap(), g("Nat"));
    }
    assert!(same(&c, &a, &b));
}
#[test]
fn empty_universe_type_headers_remain_exact_named_types() {
    let mut c = context();
    c.add_universe_poly_declaration("OpaqueType", vec![], Term::Sort(Universe::Type(0)))
        .unwrap();
    c.add_universe_poly_declaration("OtherType", vec![], Term::Sort(Universe::Type(0)))
        .unwrap();
    assert!(same(&c, &wrap(g("OpaqueType")), &g("OpaqueType")));
    assert!(!same(&c, &wrap(g("OpaqueType")), &g("OtherType")));
}
#[test]
fn actual_universe_parameters_still_require_explicit_arguments() {
    let mut c = context();
    c.add_universe_poly_declaration("poly", vec!["u".into()], fun_ty())
        .unwrap();
    assert!(!same(&c, &wrap(g("poly")), &g("poly")));
    let malformed = Term::Const {
        name: "poly".into(),
        levels: vec![],
    };
    assert!(crate::infer_type(&c, &malformed).is_err());
    assert!(crate::recheck(&c, &malformed).is_err());
    let at = |u| Term::Const {
        name: "poly".into(),
        levels: vec![Universe::Type(u)],
    };
    assert!(same(&c, &wrap(at(0)), &at(0)));
    assert!(!same(&c, &wrap(at(0)), &at(1)));
}
#[test]
fn explicit_empty_constants_and_global_headers_each_keep_existing_identity() {
    let c = context();
    let explicit = Term::Const {
        name: "opaque".into(),
        levels: vec![],
    };
    for head in [g("opaque"), explicit] {
        let call = app(head, g("Zero"));
        assert_eq!(crate::infer_type(&c, &call).unwrap(), g("Nat"));
        assert_eq!(crate::recheck(&c, &call).unwrap(), g("Nat"));
        assert!(same(&c, &wrap(call.clone()), &call));
    }
    let excess = Term::Const {
        name: "opaque".into(),
        levels: vec![Universe::Type(0)],
    };
    assert!(crate::infer_type(&c, &excess).is_err());
    assert!(crate::recheck(&c, &excess).is_err());
}
#[test]
fn exact_names_and_all_arguments_remain_obligations() {
    let c = context();
    let a = wrap(app(g("opaque"), g("Zero")));
    assert!(!same(&c, &a, &app(g("other"), g("Zero"))));
    assert!(!same(&c, &a, &app(g("opaque"), app(g("Succ"), g("Zero")))));
}
#[test]
fn holes_and_expanded_floats_remain_refused() {
    let mut c = context();
    assert!(!same(
        &c,
        &wrap(app(g("opaque"), Term::Hole)),
        &app(g("opaque"), Term::Hole)
    ));
    c.add_declaration("Float", Term::Sort(Universe::Type(0)));
    c.with_equations_suspended(|c| {
        c.declare("float_value", g("Float"), Term::Lit(Literal::Float(1.5)))
    })
    .unwrap();
    assert!(!same(&c, &wrap(g("float_value")), &g("float_value")));
}
#[test]
fn retained_headers_consume_actual_expansion_work_with_existing_limits() {
    let c = context();
    let a = wrap(app(g("opaque"), g("Zero")));
    let b = app(g("opaque"), g("Zero"));
    assert!(!beta_delta_equivalent(&c, &a, &b, 0));
    assert!(!beta_delta_equivalent(&c, &a, &b, 1));
    DELTA_EXPANSION_WORK.with(|counter| counter.set(Some((0, 0))));
    let answer = beta_delta_equivalent(&c, &a, &b, 64);
    let (visits, matches) = DELTA_EXPANSION_WORK.with(|counter| counter.replace(None).unwrap());
    assert!(
        visits > 0 && visits <= 64,
        "real uncached expansion visits stay bounded"
    );
    assert_eq!(matches, 0);
    assert!(answer);
}

#[test]
fn independently_registered_contexts_keep_distinct_universe_arities() {
    let empty = context();
    let mut nonempty = Context::new();
    nonempty.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    nonempty.add_constructor("Zero", "Nat", g("Nat"));
    nonempty
        .add_universe_poly_declaration("opaque", vec!["u".into()], fun_ty())
        .unwrap();
    let a = wrap(g("opaque"));
    let b = g("opaque");
    assert!(same(&empty, &a, &b));
    assert!(!same(&nonempty, &a, &b));
    assert!(same(&empty, &a, &b));
}
#[test]
fn checked_opaque_header_replacement_is_rejected_without_mutation() {
    let mut c = context();
    let before = c.clone();
    let result = c.add_universe_poly_declaration("opaque", vec!["u".into()], fun_ty());
    assert!(result.is_err());
    assert!(c.poly_declaration("opaque").unwrap().0.is_empty());
    assert!(before.poly_declaration("opaque").unwrap().0.is_empty());
    let a = wrap(g("opaque"));
    let b = g("opaque");
    assert!(same(&c, &a, &b));
    assert!(same(&before, &a, &b));
}
