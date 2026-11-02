use super::*;
fn g(s: &str) -> Term {
    Term::Global(s.into())
}
fn v(s: &str) -> Term {
    Term::Var(s.into())
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
fn list(a: Term) -> Term {
    app(g("Sequence"), a)
}
fn cons(a: Term, x: Term, xs: Term) -> Term {
    app(app(app(g("Sequence.link"), a), x), xs)
}
fn program(recurse: bool) -> Term {
    let next = if recurse {
        app(app(v("walk"), v("xs")), v("pred"))
    } else {
        v("default")
    };
    let step = Term::Match {
        discriminant: Box::new(v("index")),
        motive: Box::new(lam("_", g("Nat"), v("A"))),
        cases: vec![v("head"), lam("pred", g("Nat"), next)],
    };
    let body = Term::Match {
        discriminant: Box::new(v("items")),
        motive: Box::new(lam("_", list(v("A")), v("A"))),
        cases: vec![
            v("default"),
            lam("head", v("A"), lam("xs", list(v("A")), step)),
        ],
    };
    lam(
        "A",
        Term::Sort(crate::Universe::Type(0)),
        lam(
            "default",
            v("A"),
            Term::Fix {
                name: "walk".into(),
                body: Box::new(lam("items", list(v("A")), lam("index", g("Nat"), body))),
            },
        ),
    )
}
fn context() -> Context {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    c.add_inductive(
        "Sequence",
        pi(
            "A",
            Term::Sort(crate::Universe::Type(0)),
            Term::Sort(crate::Universe::Type(0)),
        ),
    );
    c.set_inductive_params("Sequence", 1);
    c.add_constructor(
        "Sequence.end",
        "Sequence",
        pi("A", Term::Sort(crate::Universe::Type(0)), list(v("A"))),
    );
    c.add_constructor(
        "Sequence.link",
        "Sequence",
        pi(
            "A",
            Term::Sort(crate::Universe::Type(0)),
            pi("head", v("A"), pi("tail", list(v("A")), list(v("A")))),
        ),
    );
    let ty = pi(
        "A",
        Term::Sort(crate::Universe::Type(0)),
        pi(
            "default",
            v("A"),
            pi("items", list(v("A")), pi("index", g("Nat"), v("A"))),
        ),
    );
    c.declare("unrelated.lookup", ty, program(true)).unwrap();
    c
}
fn items(count: u64) -> Term {
    let mut xs = app(g("Sequence.end"), g("Nat"));
    for i in (0..count).rev() {
        xs = cons(g("Nat"), lit(i), xs);
    }
    xs
}
fn lookup(xs: Term, n: Term) -> Term {
    app(
        app(app(app(g("unrelated.lookup"), g("Nat")), lit(999)), xs),
        n,
    )
}
#[test]
fn checked_index_recurrence_walks_the_constructor_spine_without_rebuilding_recursors() {
    let c = context();
    let mut r = HeadReducerWithWork::<true>::new(&c, 200_000);
    let t = r.store.intern(&lookup(items(100), lit(99)));
    let Some(Value::Nat(n)) = r.whnf(t) else {
        panic!("lookup must finish");
    };
    assert_eq!(n, crate::BigInt::from_u64(99));
    assert_eq!(
        r.work.as_ref().unwrap().guarded_unroll,
        0,
        "certified in-bounds lookup should walk cells without rebuilding recursive closures"
    );
}
#[test]
fn lookup_boundaries_preserve_the_original_default_and_selected_values() {
    let c = context();
    for len in 0..5 {
        for index in 0..7 {
            let t = lookup(items(len), lit(index));
            let expected = lit(if index < len { index } else { 999 });
            assert!(constructor_equivalent(&c, &t, &expected, 100_000));
            assert!(crate::defeq_for_test(
                &c.order_recurrence_context(),
                &t,
                &expected
            ));
        }
    }
}

#[test]
fn matching_zero_but_false_successor_equation_does_not_certify() {
    let mut c = context();
    let ty = c.get_definition_type("unrelated.lookup").unwrap().clone();
    c.declare("unrelated.lookup", ty, program(false)).unwrap();
    let mut r = HeadReducerWithWork::<false>::new(&c, 200_000);
    let h = r.store.intern(&g("unrelated.lookup"));
    assert!(r.certified_index(h).is_none());
    assert!(constructor_equivalent(
        &c,
        &lookup(items(3), lit(1)),
        &lit(999),
        100_000
    ));
    assert!(!constructor_equivalent(
        &c,
        &lookup(items(3), lit(1)),
        &lit(1),
        100_000
    ));
}
#[test]
fn matching_signature_and_successor_law_cannot_replace_the_zero_law() {
    let mut c = context();
    let ty = c.get_definition_type("unrelated.lookup").unwrap().clone();
    let constant = lam(
        "A",
        Term::Sort(crate::Universe::Type(0)),
        lam(
            "default",
            v("A"),
            lam("items", list(v("A")), lam("index", g("Nat"), v("default"))),
        ),
    );
    c.declare("unrelated.lookup", ty, constant).unwrap();
    let mut r = HeadReducerWithWork::<false>::new(&c, 200_000);
    let h = r.store.intern(&g("unrelated.lookup"));
    assert!(r.certified_index(h).is_none());
    assert!(constructor_equivalent(
        &c,
        &lookup(items(3), lit(0)),
        &lit(999),
        100_000
    ));
    assert!(!constructor_equivalent(
        &c,
        &lookup(items(3), lit(0)),
        &lit(0),
        100_000
    ));
}
#[test]
fn lookup_certificates_are_scoped_to_the_immutable_context_and_disabled_during_probes() {
    let c = context();
    let certify = |ctx: &Context| {
        let mut r = HeadReducerWithWork::<false>::new(ctx, 200_000);
        let h = r.store.intern(&g("unrelated.lookup"));
        r.certified_index(h).is_some()
    };
    assert!(certify(&c));
    assert!(!certify(&c.order_recurrence_context()));
    let mut changed = c.clone();
    let ty = changed
        .get_definition_type("unrelated.lookup")
        .unwrap()
        .clone();
    changed
        .declare("unrelated.lookup", ty, program(false))
        .unwrap();
    assert!(!certify(&changed));
    assert!(certify(&c));
}
#[test]
fn huge_out_of_bounds_index_keeps_the_original_function_and_index() {
    let c = context();
    let huge = Term::Lit(Literal::Nat(
        crate::BigInt::parse_decimal("340282366920938463463374607431768211456").unwrap(),
    ));
    assert!(constructor_equivalent(
        &c,
        &lookup(items(3), huge),
        &lit(999),
        100_000
    ));
}
#[test]
fn indexing_observes_only_the_selected_element_and_required_spine() {
    let c = context();
    let xs = cons(
        g("Nat"),
        g("undefined.head"),
        cons(g("Nat"), lit(42), g("undefined.tail")),
    );
    assert!(constructor_equivalent(
        &c,
        &lookup(xs.clone(), lit(1)),
        &lit(42),
        100_000
    ));
    assert!(!constructor_equivalent(
        &c,
        &lookup(xs.clone(), lit(0)),
        &lit(42),
        100_000
    ));
    assert!(!constructor_equivalent(
        &c,
        &lookup(xs, lit(2)),
        &lit(42),
        100_000
    ));
}
#[test]
fn partial_application_remains_a_function_and_does_not_consume_missing_arguments() {
    let c = context();
    let partial = app(app(g("unrelated.lookup"), g("Nat")), lit(999));
    let mut r = HeadReducerWithWork::<false>::new(&c, 100_000);
    let t = r.store.intern(&partial);
    assert!(matches!(r.whnf(t),Some(Value::Fix{args,..}) if args.is_empty()));
    let t = lookup(items(3), lit(2));
    assert!(!constructor_equivalent(&c, &t, &lit(2), 0));
    assert!(constructor_equivalent(&c, &t, &lit(2), 100_000));
}

#[test]
fn repeated_indices_reuse_observed_cells_instead_of_rewalking_the_prefix() {
    let c = context();
    let mut r = HeadReducerWithWork::<true>::new(&c, 200_000);
    let xs = items(100);
    let first = r.store.intern(&lookup(xs.clone(), lit(99)));
    assert!(matches!(r.whnf(first),Some(Value::Nat(n)) if n==crate::BigInt::from_u64(99)));
    let before = r.work.as_ref().unwrap().force;
    for i in 0..99 {
        let t = r.store.intern(&lookup(xs.clone(), lit(i)));
        assert!(matches!(r.whnf(t),Some(Value::Nat(n)) if n==crate::BigInt::from_u64(i)));
    }
    let work = r.work.as_ref().unwrap().force - before;
    assert!(
        work <= 1000,
        "repeated indexing revisited cached constructor prefixes: {work} forces"
    );
}

#[test]
fn bounded_spine_retention_preserves_indices_on_both_sides_of_the_cap() {
    let c = context();
    let mut r = HeadReducerWithWork::<true>::new(&c, 200_000);
    let cons = r.store.intern(&g("Sequence.link"));
    let nil = r.store.intern(&g("Sequence.end"));
    let nat = r.store.intern(&g("Nat"));
    let parameter = r.delay(nat, &None).unwrap();
    let mut root = r
        .thunk(Thunk::Done(Value::Constructor {
            head: nil,
            args: vec![parameter.clone()],
        }))
        .unwrap();
    let length = certified_index::SPINE_LIMIT + 3;
    for i in (0..length).rev() {
        let field = r.ready_nat(crate::BigInt::from_u64(i as u64)).unwrap();
        root = r
            .thunk(Thunk::Done(Value::Constructor {
                head: cons,
                args: vec![parameter.clone(), field, root],
            }))
            .unwrap();
    }
    let symbol = r.store.symbol("supplied.items");
    let env = bind(None, symbol, root.clone());
    for index in [
        length - 1,
        certified_index::SPINE_LIMIT - 1,
        certified_index::SPINE_LIMIT,
        0,
        length - 2,
    ] {
        let t = r
            .store
            .intern(&lookup(v("supplied.items"), lit(index as u64)));
        assert!(
            matches!(r.run(Control::Eval(t,env.clone())),Some(Value::Nat(n))
            if n==crate::BigInt::from_u64(index as u64))
        );
    }
    let view = r.index_spines.get(&(cons, root.id)).unwrap();
    assert_eq!(view.borrow().fields.len(), certified_index::SPINE_LIMIT);
}
#[test]
fn evicting_spine_views_cannot_recycle_list_identity() {
    let c = context();
    let mut r = HeadReducerWithWork::<true>::new(&c, 200_000);
    for pass in 0..2 {
        for i in 0..300 {
            let i = if pass == 0 { i } else { 299 - i };
            let xs = cons(g("Nat"), lit(i), app(g("Sequence.end"), g("Nat")));
            let t = r.store.intern(&lookup(xs, lit(0)));
            assert!(matches!(r.whnf(t),Some(Value::Nat(n)) if n==crate::BigInt::from_u64(i)));
        }
    }
    assert!(r.index_spines.len() <= 256);
}
#[test]
fn a_neutral_index_still_allows_the_original_empty_list_branch() {
    let mut c = context();
    c.add_declaration("opaque.index", g("Nat"));
    let mut r = HeadReducerWithWork::<true>::new(&c, 100_000);
    let t = r.store.intern(&lookup(items(0), g("opaque.index")));
    assert!(matches!(r.whnf(t),Some(Value::Nat(n)) if n==crate::BigInt::from_u64(999)));
}
