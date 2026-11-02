use super::*;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
#[test]
fn explicit_polymorphic_successor_uses_its_declared_foreign_codomain() {
    let mut c = Context::new();
    c.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    c.add_constructor("Zero", "Nat", g("Nat"));
    c.add_constructor(
        "Succ",
        "Nat",
        Term::Pi {
            param: "n".into(),
            param_type: Box::new(g("Nat")),
            body_type: Box::new(g("Nat")),
        },
    );
    c.add_inductive("Bool", Term::Sort(Universe::Type(0)));
    c.add_constructor("false", "Bool", g("Bool"));
    c.add_constructor("true", "Bool", g("Bool"));
    c.declare_universe_poly(
        "Succ",
        vec!["u".into()],
        Term::Pi {
            param: "n".into(),
            param_type: Box::new(g("Nat")),
            body_type: Box::new(g("Bool")),
        },
        Term::Lambda {
            param: "n".into(),
            param_type: Box::new(g("Nat")),
            body: Box::new(g("false")),
        },
    )
    .unwrap();
    assert!(c.get_universe_poly("Succ").is_some());
    let explicit = app(
        Term::Const {
            name: "Succ".into(),
            levels: vec![Universe::Type(0)],
        },
        g("Zero"),
    );
    assert_eq!(infer_type(&c, &explicit).unwrap(), g("Bool"));
    assert_eq!(crate::recheck(&c, &explicit).unwrap(), g("Bool"));
    assert!(
        matches!(infer_type(&c,&app(g("Succ"),g("Zero"))),Err(KernelError::UnboundVariable(name)) if name=="Succ")
    );
}

/// `Nat`, `Zero`, `Succ` and `Bool` as the standard library declares them.
fn peano_and_bool() -> Context {
    let mut c = Context::new();
    c.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    c.add_constructor("Zero", "Nat", g("Nat"));
    c.add_constructor(
        "Succ",
        "Nat",
        Term::Pi {
            param: "n".into(),
            param_type: Box::new(g("Nat")),
            body_type: Box::new(g("Nat")),
        },
    );
    c.add_inductive("Bool", Term::Sort(Universe::Type(0)));
    c.add_constructor("false", "Bool", g("Bool"));
    c.add_constructor("true", "Bool", g("Bool"));
    c
}

#[test]
fn a_successor_declared_again_is_no_successor_to_the_literal_bridge() {
    let one = Term::Lit(crate::term::Literal::Nat(crate::BigInt::from_i64(1)));
    let succ_zero = app(g("Succ"), g("Zero"));
    let canonical = peano_and_bool();
    assert!(
        def_eq(&canonical, &one, &succ_zero),
        "`1 ≡ Succ Zero` while `Succ` is `Nat`'s successor"
    );
    let mut c = peano_and_bool();
    c.declare(
        "Succ",
        Term::Pi {
            param: "n".into(),
            param_type: Box::new(g("Nat")),
            body_type: Box::new(g("Bool")),
        },
        Term::Lambda {
            param: "n".into(),
            param_type: Box::new(g("Nat")),
            body: Box::new(g("false")),
        },
    )
    .expect("`Succ` declared again");
    assert!(
        !def_eq(&c, &one, &succ_zero),
        "`Succ Zero` is `false` once `Succ` is a definition into `Bool`, and no literal"
    );
}

#[test]
fn a_literal_reads_as_succ_only_while_succ_is_the_successor() {
    let lit = |n: i64| Term::Lit(crate::term::Literal::Nat(crate::BigInt::from_i64(n)));
    let mut c = peano_and_bool();
    // A second constructor first, so `Nat` is no unit type whose elements are all equal once
    // `Succ` is assumed in place of its constructor.
    c.add_constructor(
        "Other",
        "Nat",
        Term::Pi {
            param: "n".into(),
            param_type: Box::new(g("Nat")),
            body_type: Box::new(g("Nat")),
        },
    );
    c.add_declaration(
        "Succ",
        Term::Pi {
            param: "n".into(),
            param_type: Box::new(g("Nat")),
            body_type: Box::new(g("Nat")),
        },
    );
    assert!(
        c.refused_declarations().is_empty(),
        "{:?}",
        c.refused_declarations()
    );
    assert!(
        !def_eq(&c, &lit(1), &app(g("Succ"), g("Zero"))),
        "an assumed `Succ` is no successor"
    );
    assert!(
        def_eq(&c, &lit(0), &g("Zero")),
        "`0` is `Zero` while `Zero` is `Nat`'s"
    );
    assert!(
        constructor_head(&c, &lit(1)).is_none(),
        "the index unifier reads no constructor into `1`"
    );
    assert!(constructor_head(&c, &lit(0)).is_some());
}

#[test]
fn a_literal_match_is_stuck_where_succ_is_not_the_successor() {
    let lit = |n: i64| Term::Lit(crate::term::Literal::Nat(crate::BigInt::from_i64(n)));
    let lam = |p: &str, a: Term, b: Term| Term::Lambda {
        param: p.into(),
        param_type: Box::new(a),
        body: Box::new(b),
    };
    let mut c = Context::new();
    c.add_inductive("Nat", Term::Sort(Universe::Type(0)));
    c.add_constructor("Zero", "Nat", g("Nat"));
    c.add_constructor(
        "Succ",
        "Nat",
        Term::Pi {
            param: "a".into(),
            param_type: Box::new(g("Nat")),
            body_type: Box::new(Term::Pi {
                param: "b".into(),
                param_type: Box::new(g("Nat")),
                body_type: Box::new(g("Nat")),
            }),
        },
    );
    c.add_inductive("Bool", Term::Sort(Universe::Type(0)));
    c.add_constructor("false", "Bool", g("Bool"));
    c.add_constructor("true", "Bool", g("Bool"));
    let on = |n: i64| Term::Match {
        discriminant: Box::new(lit(n)),
        motive: Box::new(lam("n", g("Nat"), g("Bool"))),
        cases: vec![
            g("true"),
            lam("a", g("Nat"), lam("b", g("Nat"), g("false"))),
        ],
    };
    assert!(
        matches!(crate::normalize(&c, &on(1)), Term::Match { .. }),
        "`match 1` is stuck: {}",
        crate::normalize(&c, &on(1))
    );
    assert_eq!(crate::normalize(&c, &on(0)), g("true"));
    assert!(
        constructor_head(&c, &lit(1)).is_none(),
        "the index unifier reads no one-field `Succ` into `1`"
    );
}

/// The readings are the constructors', not the family's exact shape: with `Other` beside `Zero`
/// and `Succ`, `1` is still `Succ 0`, by conversion and by the match on it.
#[test]
fn a_literal_reads_as_succ_while_succ_is_a_constructor_of_nat() {
    let lit = |n: i64| Term::Lit(crate::term::Literal::Nat(crate::BigInt::from_i64(n)));
    let lam = |p: &str, a: Term, b: Term| Term::Lambda {
        param: p.into(),
        param_type: Box::new(a),
        body: Box::new(b),
    };
    let mut c = peano_and_bool();
    c.add_constructor(
        "Other",
        "Nat",
        Term::Pi {
            param: "n".into(),
            param_type: Box::new(g("Nat")),
            body_type: Box::new(g("Nat")),
        },
    );
    assert!(
        c.refused_declarations().is_empty(),
        "{:?}",
        c.refused_declarations()
    );
    assert!(def_eq(&c, &lit(1), &app(g("Succ"), g("Zero"))));
    let on_one = Term::Match {
        discriminant: Box::new(lit(1)),
        motive: Box::new(lam("n", g("Nat"), g("Bool"))),
        cases: vec![
            g("false"),
            lam("p", g("Nat"), g("true")),
            lam("p", g("Nat"), g("false")),
        ],
    };
    assert_eq!(crate::normalize(&c, &on_one), g("true"));
}
