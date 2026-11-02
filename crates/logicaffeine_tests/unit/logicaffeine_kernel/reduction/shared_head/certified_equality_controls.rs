//! Equality-specific controls for certified natural observers.
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
fn lam(n: &str, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(g("Nat")),
        body: Box::new(b),
    }
}
fn pi(n: &str, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(g("Nat")),
        body_type: Box::new(b),
    }
}
fn lit(n: u64) -> Term {
    Term::Lit(Literal::Nat(crate::BigInt::from_u64(n)))
}
fn call(a: Term, b: Term) -> Term {
    app(app(g("unfamiliar.compare"), a), b)
}
fn branch(n: &str, zero: Term, succ: Term) -> Term {
    Term::Match {
        discriminant: Box::new(v(n)),
        motive: Box::new(lam("_", g("Bit"))),
        cases: vec![zero, succ],
    }
}
fn body(recursive: bool) -> Term {
    let next = if recursive {
        app(app(v("walk"), v("pm")), v("pn"))
    } else {
        g("Bit.yes")
    };
    Term::Fix {
        name: "walk".into(),
        body: Box::new(lam(
            "m",
            lam(
                "n",
                branch(
                    "m",
                    branch("n", g("Bit.yes"), lam("pn", g("Bit.no"))),
                    lam("pm", branch("n", g("Bit.no"), lam("pn", next))),
                ),
            ),
        )),
    }
}
fn context(recursive: bool) -> Context {
    let mut c = Context::new();
    c.add_inductive("Nat", Term::Sort(crate::Universe::Type(0)));
    c.add_constructor("Zero", "Nat", g("Nat"));
    c.add_constructor("Succ", "Nat", pi("n", g("Nat")));
    c.add_inductive("Bit", Term::Sort(crate::Universe::Type(0)));
    c.add_constructor("Bit.yes", "Bit", g("Bit"));
    c.add_constructor("Bit.no", "Bit", g("Bit"));
    c.declare(
        "unfamiliar.compare",
        pi("m", pi("n", g("Bit"))),
        body(recursive),
    )
    .unwrap();
    c
}
#[test]
fn huge_natural_equality_is_bounded_and_independent_of_names() {
    let c = context(true);
    for (a, b, equal) in [
        (0, 0, true),
        (0, u64::MAX, false),
        (u64::MAX, 0, false),
        (u64::MAX, u64::MAX, true),
        (u64::MAX, u64::MAX - 1, false),
    ] {
        let term = call(lit(a), lit(b));
        assert!(
            constructor_equivalent(
                &c,
                &term,
                &g(if equal { "Bit.yes" } else { "Bit.no" }),
                1000
            ),
            "{a}, {b}"
        );
        assert!(!constructor_equivalent(
            &c,
            &term,
            &g(if equal { "Bit.no" } else { "Bit.yes" }),
            1000
        ));
    }
    let huge = Term::Lit(Literal::Nat(
        crate::BigInt::parse_decimal("340282366920938463463374607431768211456").unwrap(),
    ));
    assert!(constructor_equivalent(
        &c,
        &call(huge.clone(), huge),
        &g("Bit.yes"),
        1000
    ));
}
#[test]
fn equality_recurrence_must_hold_beyond_its_base_cases() {
    let c = context(false);
    let term = call(lit(u64::MAX), lit(u64::MAX - 1));
    assert!(!constructor_equivalent(&c, &term, &g("Bit.no"), 1000));
    assert!(constructor_equivalent(&c, &term, &g("Bit.yes"), 1000));
}
#[test]
fn equality_observer_does_not_invent_unknown_or_negative_operands() {
    let mut c = context(true);
    c.add_declaration("unknown", g("Nat"));
    for term in [
        call(g("unknown"), g("unknown")),
        call(lit(0), g("unknown")),
        call(Term::Lit(Literal::Nat(crate::BigInt::from_i64(-1))), lit(0)),
    ] {
        assert!(!constructor_equivalent(&c, &term, &g("Bit.yes"), 1000));
        assert!(!constructor_equivalent(&c, &term, &g("Bit.no"), 1000));
    }
}
#[test]
fn equality_certificates_do_not_survive_a_changed_definition() {
    let c = context(true);
    let term = call(lit(u64::MAX), lit(u64::MAX));
    assert!(constructor_equivalent(&c, &term, &g("Bit.yes"), 1000));
    let mut changed = c.clone();
    changed
        .declare(
            "unfamiliar.compare",
            pi("m", pi("n", g("Bit"))),
            lam("m", lam("n", g("Bit.no"))),
        )
        .unwrap();
    assert!(constructor_equivalent(&changed, &term, &g("Bit.no"), 1000));
    assert!(!constructor_equivalent(
        &changed,
        &term,
        &g("Bit.yes"),
        1000
    ));
}

#[test]
fn equality_acceleration_agrees_with_ordinary_reduction_on_small_inputs() {
    let c = context(true);
    let ordinary = c.order_recurrence_context();
    for a in 0..6 {
        for b in 0..6 {
            let term = call(lit(a), lit(b));
            let result = crate::normalize_checked(&ordinary, &term).unwrap();
            assert_eq!(result, g(if a == b { "Bit.yes" } else { "Bit.no" }));
            assert!(constructor_equivalent(&c, &term, &result, 1000));
        }
    }
}
#[test]
fn equality_admission_refuses_a_pending_dependency_even_if_unused() {
    let mut c = context(true);
    c.add_definition("pending".into(), g("Nat"), g("missing"));
    let term = Term::Let {
        name: "unused".into(),
        ty: Box::new(g("Nat")),
        value: Box::new(g("pending")),
        body: Box::new(body(true)),
    };
    c.add_definition(
        "unfamiliar.compare".into(),
        pi("m", pi("n", g("Bit"))),
        term,
    );
    let mut reducer = HeadReducer::new(&c, 1000);
    let head = reducer.store.intern(&g("unfamiliar.compare"));
    assert!(reducer.certified_equality(head).is_none());
}

#[test]
fn unrelated_application_heads_do_not_exhaust_equality_certification() {
    let c = context(true);
    let mut r = HeadReducer::new(&c, 100_000);
    let allowance = r.equality_attempts;
    for n in 0..128 {
        let head = r.store.intern(&app(g("unfamiliar.compare"), lit(n)));
        assert!(r.certified_equality(head).is_none());
    }
    assert_eq!(
        r.equality_attempts, allowance,
        "non-candidates must not consume recurrence probes"
    );
    let head = r.store.intern(&g("unfamiliar.compare"));
    assert!(r.certified_equality(head).is_some());
}
#[test]
fn unrelated_declared_signatures_do_not_exhaust_equality_certification() {
    let mut c = context(true);
    for i in 0..128 {
        c.declare(
            &format!("noncandidate.{i}"),
            pi("x", g("Nat")),
            lam("x", v("x")),
        )
        .unwrap();
    }
    let mut r = HeadReducer::new(&c, 100_000);
    let allowance = r.equality_attempts;
    for i in 0..128 {
        let head = r.store.intern(&g(&format!("noncandidate.{i}")));
        assert!(r.certified_equality(head).is_none());
    }
    assert_eq!(r.equality_attempts, allowance);
    let head = r.store.intern(&g("unfamiliar.compare"));
    assert!(r.certified_equality(head).is_some());
}

#[test]
fn equality_admitted_failures_still_spend_once_and_exhaustion_does_not_reset() {
    let mut c = context(false);
    c.declare("valid.compare", pi("m", pi("n", g("Bit"))), body(true))
        .unwrap();
    let mut r = HeadReducer::new(&c, 100_000);
    r.equality_attempts = 1;
    let bad = r.store.intern(&g("unfamiliar.compare"));
    let good = r.store.intern(&g("valid.compare"));
    assert!(r.certified_equality(bad).is_none());
    assert_eq!(r.equality_attempts, 0);
    assert!(r.certified_equality(bad).is_none());
    assert_eq!(r.equality_attempts, 0);
    assert!(r.certified_equality(good).is_none());
    assert_eq!(r.equality_attempts, 0);
    let mut fresh = HeadReducer::new(&c, 100_000);
    let good = fresh.store.intern(&g("valid.compare"));
    let certificate = fresh
        .certified_equality(good)
        .expect("all four equations hold");
    fresh.equality_attempts = 0;
    assert_eq!(fresh.certified_equality(good), Some(certificate));
}
