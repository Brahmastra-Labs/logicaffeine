use super::*;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn lit(n: i64) -> Term {
    Term::Lit(Literal::Nat(crate::BigInt::from_i64(n)))
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn apps(f: Term, args: &[Term]) -> Term {
    args.iter().cloned().fold(f, app)
}
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn lam(n: &str, a: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(a),
        body: Box::new(b),
    }
}
fn peano(n: usize, tail: Term) -> Term {
    (0..n).fold(tail, |a, _| app(g("Succ"), a))
}
fn context() -> Context {
    let mut c = Context::new();
    c.set_auto_auxiliaries(false);
    c.add_inductive("Nat", Term::Sort(crate::Universe::Type(0)));
    c.add_constructor("Zero", "Nat", g("Nat"));
    c.add_constructor("Succ", "Nat", pi("n", g("Nat"), g("Nat")));
    c.with_equations_suspended(|c| {
        c.declare(
            "keep",
            pi("x", g("Nat"), g("Nat")),
            lam("x", g("Nat"), v("x")),
        )
    })
    .unwrap();
    c
}
fn read(c: &Context, t: &Term) -> Term {
    let (result, left) = natural_normal_form(c, t, super::super::Transparency::All, 1 << 20);
    assert!(left < 1 << 20);
    let NatReadbackOutcome::Normal(result) = result else {
        panic!("readback failed: {result:?}")
    };
    result
}
fn typed(c: &Context, t: &Term) {
    assert_eq!(crate::infer_type(c, t).unwrap(), g("Nat"));
    assert_eq!(crate::recheck(c, t).unwrap(), g("Nat"));
}

#[test]
fn literal_and_constructor_results_preserve_exact_normal_form_syntax() {
    let c = context();
    for expected in [
        lit(0),
        lit(2),
        g("Zero"),
        peano(2, g("Zero")),
        peano(1, lit(1)),
    ] {
        let term = app(g("keep"), expected.clone());
        typed(&c, &term);
        assert_eq!(read(&c, &term), expected);
    }
}

#[test]
fn repeated_identity_calls_do_not_merge_literal_and_constructor_representations() {
    let c = context();
    let mut r = HeadReducer::for_nat_readback(&c, 1 << 20);
    let values = [
        lit(0),
        g("Zero"),
        lit(2),
        peano(2, g("Zero")),
        peano(1, lit(1)),
    ];
    for expected in values
        .iter()
        .chain(values.iter().rev())
        .chain(values.iter())
    {
        let term = Term::Let {
            name: "captured".into(),
            ty: Box::new(g("Nat")),
            value: Box::new(expected.clone()),
            body: Box::new(app(g("keep"), v("captured"))),
        };
        let root = r.store.intern(&term);
        assert_eq!(
            r.readback_nat(root, 1600),
            NatReadbackOutcome::Normal(expected.clone())
        );
    }
}

#[test]
fn lexical_captures_and_shadowed_binders_keep_exact_natural_tails() {
    let c = context();
    let term = app(
        lam("x", g("Nat"), app(lam("y", g("Nat"), v("x")), lit(9))),
        peano(3, lit(2)),
    );
    typed(&c, &term);
    assert_eq!(read(&c, &term), peano(3, lit(2)));
    let term = app(
        lam("x", g("Nat"), app(lam("x", g("Nat"), v("x")), g("Zero"))),
        lit(9),
    );
    typed(&c, &term);
    assert_eq!(read(&c, &term), g("Zero"));
}

#[test]
fn readback_matches_ordinary_checked_nat_programs_without_rebuilding_them() {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    for name in [
        "Nat_add", "Nat_sub", "Nat_mul", "Nat_pow", "Nat_div", "Nat_mod",
    ] {
        for (a, b) in [(0, 0), (2, 0), (3, 2)] {
            let term = apps(g(name), &[lit(a), lit(b)]);
            typed(&c, &term);
            let expected = crate::reduction::normalize_checked(&c, &term).unwrap();
            assert_eq!(read(&c, &term), expected, "{name} {a} {b}");
        }
    }
}

#[test]
fn existing_remainder_program_observes_large_dividend_with_shared_work() {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    let term = apps(g("Nat_mod"), &[lit(255), lit(256)]);
    typed(&c, &term);
    assert_eq!(read(&c, &term), peano(255, g("Zero")));
}

#[test]
fn registered_rewrite_and_excluded_transparency_preserve_fallback() {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    let term = apps(g("Nat_add"), &[lit(2), lit(3)]);
    let excluded = |_: &str| true;
    let result = natural_normal_form(
        &c,
        &term,
        super::super::Transparency::Excluding(&excluded),
        1000,
    );
    assert_eq!(result, (NatReadbackOutcome::Unsupported, 1000));
    let statement = apps(
        g("Eq"),
        &[
            g("Nat"),
            apps(g("Nat_add"), &[g("Zero"), g("Zero")]),
            g("Zero"),
        ],
    );
    c.declare(
        "readback_refl",
        statement,
        apps(g("refl"), &[g("Nat"), g("Zero")]),
    )
    .unwrap();
    c.add_simp_lemma("readback_refl").unwrap();
    assert!(!c.simp_rules().is_empty());
    assert_eq!(
        natural_normal_form(&c, &term, super::super::Transparency::All, 1000),
        (NatReadbackOutcome::Unsupported, 1000)
    );
}

#[test]
fn canonical_metadata_and_current_definition_bodies_are_required() {
    let mut c = context();
    assert!(
        c.refused_meaning_change_for_test(|c| {
            c.set_inductive_params("Nat", 1);
            Ok(())
        })
        .is_some(),
        "the stable-meaning gate refuses this change; the cache is tested with it permitted"
    );
    // `keep` does not observe `Nat`'s constructor set (its body never eliminates a `Nat` or
    // names a constructor), so the gate ADMITS a new constructor here.
    assert!(c
        .refused_meaning_change_for_test(|c| {
            c.add_constructor("Other", "Nat", g("Nat"));
            Ok(())
        })
        .is_none());
    c.permit_meaning_changes_for_cache_tests();
    let mut bad = c.clone();
    bad.set_inductive_params("Nat", 1);
    assert_eq!(
        natural_normal_form(&bad, &lit(1), super::super::Transparency::All, 1000).0,
        NatReadbackOutcome::Unsupported
    );
    let mut bad = c.clone();
    bad.add_constructor("Other", "Nat", g("Nat"));
    assert_eq!(
        natural_normal_form(&bad, &lit(1), super::super::Transparency::All, 1000).0,
        NatReadbackOutcome::Unsupported
    );
    let mut changed = c.clone();
    changed
        .with_equations_suspended(|c| {
            c.declare(
                "keep",
                pi("x", g("Nat"), g("Nat")),
                lam("x", g("Nat"), app(g("Succ"), v("x"))),
            )
        })
        .unwrap();
    assert_eq!(read(&c, &app(g("keep"), lit(2))), lit(2));
    assert_eq!(read(&changed, &app(g("keep"), lit(2))), peano(1, lit(2)));
}

#[test]
fn unsupported_results_and_invalid_literal_tails_never_return_success() {
    let mut c = context();
    c.add_declaration("opaque", g("Nat"));
    for term in [
        g("opaque"),
        v("open"),
        Term::Hole,
        lit(-1),
        app(g("Succ"), g("opaque")),
        app(g("Succ"), Term::Lit(Literal::Int(2))),
        lam("x", g("Nat"), v("x")),
    ] {
        assert_eq!(
            natural_normal_form(&c, &term, super::super::Transparency::All, 1000).0,
            NatReadbackOutcome::Unsupported
        );
    }
}

#[test]
fn exhaustion_is_explicit_and_a_following_attempt_cannot_reset_fuel() {
    let c = context();
    let mut r = HeadReducer::for_nat_readback(&c, 1);
    let root = r.store.intern(&app(g("keep"), lit(7)));
    assert_eq!(r.readback_nat(root, 1600), NatReadbackOutcome::Exhausted);
    assert_eq!(r.fuel, 0);
    let root = r.store.intern(&lit(1));
    assert_eq!(r.readback_nat(root, 1600), NatReadbackOutcome::Exhausted);
    assert_eq!(r.fuel, 0);
    assert_eq!(
        natural_normal_form(&c, &lit(0), super::super::Transparency::All, 0),
        (NatReadbackOutcome::Exhausted, 0)
    );
}

#[test]
fn output_depth_is_checked_before_constructing_a_too_deep_result() {
    let c = context();
    let mut r = HeadReducer::for_nat_readback(&c, 10000);
    let root = r.store.intern(&peano(4, lit(0)));
    assert_eq!(r.readback_nat(root, 3), NatReadbackOutcome::TooDeep);
    assert!(r.fuel < 10000 && r.fuel > 0);
    let root = r.store.intern(&peano(3, lit(0)));
    assert_eq!(
        r.readback_nat(root, 3),
        NatReadbackOutcome::Normal(peano(3, lit(0)))
    );
}
#[test]
fn rebinding_canonical_successor_disables_readback_without_reinterpreting_it() {
    let c = context();
    let term = app(g("Succ"), lit(2));
    assert_eq!(read(&c, &term), term);
    let mut changed = c.clone();
    assert!(
        changed
            .refused_meaning_change_for_test(|c| {
                c.with_equations_suspended(|c| {
                    c.declare(
                        "Succ",
                        pi("n", g("Nat"), g("Nat")),
                        lam("n", g("Nat"), v("n")),
                    )
                })?;
                Ok(())
            })
            .is_some(),
        "the stable-meaning gate refuses this change; the cache is tested with it permitted"
    );
    changed.permit_meaning_changes_for_cache_tests();
    changed
        .with_equations_suspended(|c| {
            c.declare(
                "Succ",
                pi("n", g("Nat"), g("Nat")),
                lam("n", g("Nat"), v("n")),
            )
        })
        .unwrap();
    assert!(!crate::int_inductive::canonical_nat_environment(&changed));
    assert_eq!(
        natural_normal_form(&changed, &term, super::super::Transparency::All, 1000),
        (NatReadbackOutcome::Unsupported, 1000)
    );
    assert_eq!(read(&c, &term), term);
}
