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
fn lam(n: &str, t: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(t),
        body: Box::new(b),
    }
}
fn pi(n: &str, t: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(t),
        body_type: Box::new(b),
    }
}
fn lit(n: u64) -> Term {
    Term::Lit(Literal::Nat(crate::BigInt::from_u64(n)))
}
fn order(a: Term, b: Term) -> Term {
    app(app(g("unrelated.program"), a), b)
}
fn body(recursive: bool) -> Term {
    let call = if recursive {
        app(app(v("walk"), v("pm")), v("pn"))
    } else {
        g("Bit.no")
    };
    let inner = Term::Match {
        discriminant: Box::new(v("n")),
        motive: Box::new(lam("_", g("Nat"), g("Bit"))),
        cases: vec![g("Bit.no"), lam("pn", g("Nat"), call)],
    };
    Term::Fix {
        name: "walk".into(),
        body: Box::new(lam(
            "m",
            g("Nat"),
            lam(
                "n",
                g("Nat"),
                Term::Match {
                    discriminant: Box::new(v("m")),
                    motive: Box::new(lam("_", g("Nat"), g("Bit"))),
                    cases: vec![g("Bit.yes"), lam("pm", g("Nat"), inner)],
                },
            ),
        )),
    }
}
fn context() -> Context {
    let mut c = Context::new();
    c.add_inductive("Nat", Term::Sort(crate::Universe::Type(0)));
    c.add_constructor("Zero", "Nat", g("Nat"));
    c.add_constructor("Succ", "Nat", pi("n", g("Nat"), g("Nat")));
    c.add_inductive("Bit", Term::Sort(crate::Universe::Type(0)));
    // Reverse the familiar boolean order: the certificate must identify
    // returned constructors through conversion, never by ordinal or name.
    c.add_constructor("Bit.yes", "Bit", g("Bit"));
    c.add_constructor("Bit.no", "Bit", g("Bit"));
    c.declare(
        "unrelated.program",
        pi("m", g("Nat"), pi("n", g("Nat"), g("Bit"))),
        body(true),
    )
    .unwrap();
    c
}
#[test]
fn arbitrary_names_and_constructor_order_support_large_exact_comparisons() {
    let c = context();
    for (a, b, want) in [
        (0, 0, true),
        (0, u64::MAX, true),
        (1_114_113, 4_294_967_296, true),
        (4_294_967_296, 1_114_113, false),
        (u64::MAX, u64::MAX, true),
    ] {
        let t = order(lit(a), lit(b));
        assert!(constructor_equivalent(
            &c,
            &t,
            &g(if want { "Bit.yes" } else { "Bit.no" }),
            1000
        ));
        assert!(!constructor_equivalent(
            &c,
            &t,
            &g(if want { "Bit.no" } else { "Bit.yes" }),
            1000
        ));
    }
}
#[test]
fn shared_reducer_and_ordinary_kernel_agree_on_small_inputs() {
    let c = context();
    for a in 0..5 {
        for b in 0..5 {
            let t = order(lit(a), lit(b));
            let normal = crate::normalize_checked(&c, &t).unwrap();
            assert_eq!(normal, g(if a <= b { "Bit.yes" } else { "Bit.no" }));
            assert!(constructor_equivalent(&c, &t, &normal, 1000));
        }
    }
}
#[test]
fn recurrence_is_checked_beyond_both_base_equations() {
    let mut c = context();
    let ty = c.get_definition_type("unrelated.program").unwrap().clone();
    c.declare("unrelated.program", ty, body(false)).unwrap();
    let t = order(lit(1000000), lit(1000001));
    assert!(!constructor_equivalent(&c, &t, &g("Bit.yes"), 1000));
    assert!(constructor_equivalent(&c, &t, &g("Bit.no"), 1000));
}
#[test]
fn copied_context_mutations_do_not_inherit_certificates() {
    let c = context();
    let t = order(lit(1000000), lit(1000001));
    assert!(constructor_equivalent(&c, &t, &g("Bit.yes"), 1000));
    let mut changed = c.clone();
    let ty = changed
        .get_definition_type("unrelated.program")
        .unwrap()
        .clone();
    changed
        .declare(
            "unrelated.program",
            ty,
            lam("m", g("Nat"), lam("n", g("Nat"), g("Bit.no"))),
        )
        .unwrap();
    assert!(!constructor_equivalent(&changed, &t, &g("Bit.yes"), 1000));
    assert!(constructor_equivalent(&changed, &t, &g("Bit.no"), 1000));
    assert!(constructor_equivalent(&c, &t, &g("Bit.yes"), 1000));
}
#[test]
fn dependency_changes_invalidate_semantic_certification() {
    let mut c = context();
    let ty = c.get_definition_type("unrelated.program").unwrap().clone();
    c.declare("dependency", ty.clone(), body(true)).unwrap();
    c.declare("unrelated.program", ty.clone(), g("dependency"))
        .unwrap();
    let t = order(lit(1000000), lit(1000001));
    assert!(constructor_equivalent(&c, &t, &g("Bit.yes"), 1000));
    let kept = c
        .refused_meaning_change_for_test(|c| {
            c.declare(
                "dependency",
                ty.clone(),
                lam("m", g("Nat"), lam("n", g("Nat"), g("Bit.no"))),
            )
        })
        .expect("the gate keeps `dependency` under `unrelated.program`");
    assert!(constructor_equivalent(&kept, &t, &g("Bit.yes"), 1000));
    c.permit_meaning_changes_for_cache_tests();
    c.declare(
        "dependency",
        ty,
        lam("m", g("Nat"), lam("n", g("Nat"), g("Bit.no"))),
    )
    .unwrap();
    assert!(!constructor_equivalent(&c, &t, &g("Bit.yes"), 1000));
    assert!(constructor_equivalent(&c, &t, &g("Bit.no"), 1000));
}
#[test]
fn certification_cannot_use_itself_even_across_context_clones_and_threads() {
    let c = context().order_recurrence_context();
    let t = order(lit(1000000), lit(1000001));
    assert!(!constructor_equivalent(&c, &t, &g("Bit.yes"), 1000));
    std::thread::spawn(move || {
        assert!(!constructor_equivalent(&c.clone(), &t, &g("Bit.yes"), 1000));
    })
    .join()
    .unwrap();
}
#[test]
fn explicit_universe_work_is_bounded_before_certification() {
    let check = |annotation: Term| {
        let mut c = context();
        let ty = c.get_definition_type("unrelated.program").unwrap().clone();
        let program = Term::Let {
            name: "unused".into(),
            ty: Box::new(annotation),
            value: Box::new(Term::Sort(crate::Universe::Type(0))),
            body: Box::new(body(true)),
        };
        c.declare("unrelated.program", ty, program).unwrap();
        let mut reducer = HeadReducerWithWork::<false>::new(&c, 10_000);
        let head = reducer.store.intern(&g("unrelated.program"));
        reducer.certified_order(head).is_some()
    };
    assert!(check(Term::Sort(crate::Universe::Succ(Box::new(
        crate::Universe::Type(0)
    )))));
    let mut deep = crate::Universe::Type(0);
    for _ in 0..=crate::type_checker::MAX_TERM_DEPTH {
        deep = crate::Universe::Succ(Box::new(deep));
    }
    assert!(!check(Term::Sort(deep)));
    assert!(!check(Term::Const {
        name: "unused.annotation".into(),
        levels: vec![crate::Universe::Type(0); 4096]
    }));
}
