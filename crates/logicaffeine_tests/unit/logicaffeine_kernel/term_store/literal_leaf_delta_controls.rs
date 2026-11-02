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
fn opaque() -> Term {
    Term::Const {
        name: "literal.opaque".into(),
        levels: vec![Universe::Type(0)],
    }
}
fn context(carrier: &str) -> Context {
    let mut c = Context::new();
    for name in ["Nat", "Int", "Text", "Duration", "Date", "Moment", "Float"] {
        c.add_declaration(name, Term::Sort(Universe::Type(0)));
    }
    c.add_universe_poly_declaration(
        "literal.opaque",
        vec!["u".into()],
        pi("value", g(carrier), g(carrier)),
    )
    .unwrap();
    c
}
fn pair(carrier: &str, lit: Literal) -> Context {
    let mut c = context(carrier);
    c.with_equations_suspended(|c| {
        c.declare(
            "literal.left",
            g(carrier),
            app(
                opaque(),
                Term::Let {
                    name: "x".into(),
                    ty: Box::new(g(carrier)),
                    value: Box::new(Term::Lit(lit.clone())),
                    body: Box::new(v("x")),
                },
            ),
        )
        .unwrap();
        c.declare("literal.right", g(carrier), app(opaque(), Term::Lit(lit)))
            .unwrap();
    });
    c
}
fn same(c: &Context, a: &Term, b: &Term) -> bool {
    beta_delta_equivalent(c, a, b, 1 << 16)
}
#[test]
fn checked_nat_helper_continuations_keep_exact_literal_leaves() {
    let mut c = context("Nat");
    let fun = pi("n", g("Nat"), g("Nat"));
    c.with_equations_suspended(|c| {
        c.declare(
            "literal.tail",
            fun.clone(),
            lam("n", g("Nat"), app(opaque(), v("n"))),
        )
        .unwrap();
        c.declare(
            "literal.left",
            fun.clone(),
            lam(
                "n",
                g("Nat"),
                Term::Let {
                    name: "continuation".into(),
                    ty: Box::new(fun.clone()),
                    value: Box::new(lam("u", g("Nat"), app(g("literal.tail"), v("u")))),
                    body: Box::new(app(opaque(), app(v("continuation"), v("n")))),
                },
            ),
        )
        .unwrap();
        c.declare(
            "literal.right",
            fun.clone(),
            lam("n", g("Nat"), app(opaque(), app(opaque(), v("n")))),
        )
        .unwrap();
    });
    let a = app(g("literal.left"), nat(1));
    let b = app(g("literal.right"), nat(1));
    for term in [&a, &b] {
        assert_eq!(crate::infer_type(&c, term).unwrap(), g("Nat"));
        assert_eq!(crate::recheck(&c, term).unwrap(), g("Nat"));
    }
    assert!(same(&c, &a, &b));
}
#[test]
fn seven_non_float_literal_kinds_survive_checked_delta_and_zeta() {
    for (carrier, lit) in [
        ("Nat", Literal::Nat(BigInt::from_u64(1))),
        ("Int", Literal::Int(-7)),
        ("Int", Literal::BigInt(BigInt::from_u64(u64::MAX))),
        ("Text", Literal::Text("lambda λ; null \0; newline\n".into())),
        ("Duration", Literal::Duration(-37)),
        ("Date", Literal::Date(-1)),
        ("Moment", Literal::Moment(i64::MAX)),
    ] {
        let c = pair(carrier, lit);
        for name in ["literal.left", "literal.right"] {
            assert_eq!(crate::infer_type(&c, &g(name)).unwrap(), g(carrier));
            assert_eq!(crate::recheck(&c, &g(name)).unwrap(), g(carrier));
        }
        assert!(
            same(&c, &g("literal.left"), &g("literal.right")),
            "{carrier}"
        );
    }
}
#[test]
fn unequal_literal_values_and_distinct_carrier_tags_are_not_identified() {
    let c = context("Nat");
    let literals = [
        Literal::Nat(BigInt::from_u64(0)),
        Literal::Nat(BigInt::from_u64(1)),
        Literal::Int(1),
        Literal::Int(-1),
        Literal::BigInt(BigInt::from_u64(u64::MAX)),
        Literal::BigInt(BigInt::from_u64(u64::MAX - 1)),
        Literal::Text("1".into()),
        Literal::Text("01".into()),
        Literal::Duration(1),
        Literal::Date(1),
        Literal::Moment(1),
    ];
    for (i, a) in literals.iter().enumerate() {
        for (j, b) in literals.iter().enumerate() {
            if i != j {
                assert!(
                    !same(&c, &Term::Lit(a.clone()), &Term::Lit(b.clone())),
                    "literal indexes {i} and {j}"
                );
            }
        }
    }
}
#[test]
fn retaining_a_literal_does_not_execute_an_opaque_operator() {
    let mut c = context("Int");
    c.add_declaration(
        "literal.add",
        pi("a", g("Int"), pi("b", g("Int"), g("Int"))),
    );
    let expression = app(
        app(g("literal.add"), Term::Lit(Literal::Int(1))),
        Term::Lit(Literal::Int(1)),
    );
    assert_eq!(crate::infer_type(&c, &expression).unwrap(), g("Int"));
    assert_eq!(crate::recheck(&c, &expression).unwrap(), g("Int"));
    assert!(!same(&c, &expression, &Term::Lit(Literal::Int(2))));
}
#[test]
fn floats_introduced_by_definition_bodies_remain_outside_the_fragment() {
    for value in [
        1.5,
        0.0,
        -0.0,
        f64::INFINITY,
        f64::from_bits(0x7ff8000000000001),
        f64::from_bits(0x7ff8000000000002),
    ] {
        let c = pair("Float", Literal::Float(value));
        // The inputs contain only names; this exercises expansion admission,
        // not merely the input's eq_faithful guard.
        assert!(!same(&c, &g("literal.left"), &g("literal.right")));
    }
}
#[test]
fn literal_retention_keeps_independent_typing_obligations() {
    let c = context("Nat");
    let negative = Term::Lit(Literal::Nat(BigInt::from_i64(-1)));
    assert!(crate::infer_type(&c, &negative).is_err());
    assert!(crate::recheck(&c, &negative).is_err());
    for wrong in [
        Term::Lit(Literal::Int(1)),
        Term::Lit(Literal::Text("1".into())),
        Term::Lit(Literal::BigInt(BigInt::from_u64(u64::MAX))),
    ] {
        let application = app(opaque(), wrong);
        assert!(crate::infer_type(&c, &application).is_err());
        assert!(crate::recheck(&c, &application).is_err());
    }
}
#[test]
fn literal_graph_budget_and_hole_admission_remain_fail_closed() {
    let c = pair("Nat", Literal::Nat(BigInt::from_u64(1)));
    let a = g("literal.left");
    let b = g("literal.right");
    assert!(!beta_delta_equivalent(&c, &a, &b, 0));
    assert!(!beta_delta_equivalent(&c, &a, &b, 1));
    assert!(!same(
        &c,
        &app(opaque(), Term::Hole),
        &app(opaque(), Term::Hole)
    ));
    assert!(same(&c, &a, &b));
}
#[test]
fn literal_graphs_observe_current_context_and_full_kernel_transparency() {
    let mut c = pair("Nat", Literal::Nat(BigInt::from_u64(1)));
    c.set_reducibility("literal.left", crate::context::Reducibility::Irreducible);
    assert!(same(&c, &g("literal.left"), &g("literal.right")));
    let mut changed = c.clone();
    changed
        .with_equations_suspended(|c| c.declare("literal.left", g("Nat"), app(opaque(), nat(2))))
        .unwrap();
    assert!(!same(&changed, &g("literal.left"), &g("literal.right")));
    assert!(same(&c, &g("literal.left"), &g("literal.right")));
}
