use super::super::{Transparency, Unreduced};
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
fn builds() -> usize {
    NAT_READBACK_STORE_BUILDS.with(Cell::get)
}
fn reset_builds() {
    NAT_READBACK_STORE_BUILDS.with(|n| n.set(0));
}
fn checked(c: &Context, t: &Term, ty: &Term) {
    assert_eq!(crate::infer_type(c, t).unwrap(), *ty);
    assert_eq!(crate::recheck(c, t).unwrap(), *ty);
}
fn balanced(depth: usize) -> Term {
    if depth == 0 {
        return g("Zero");
    }
    apps(
        lam("x", g("Nat"), lam("y", g("Nat"), v("x"))),
        &[balanced(depth - 1), balanced(depth - 1)],
    )
}

#[test]
fn bounded_metrics_preserve_size_depth_and_refuse_wide_children_before_interning() {
    let c = context();
    assert_eq!(
        super::super::bounded_conversion_metrics(&peano(3, g("Zero"))),
        Some((7, 3, true))
    );
    let exact = Term::Match {
        discriminant: Box::new(g("Zero")),
        motive: Box::new(g("Nat")),
        cases: vec![g("Zero"); 4093],
    };
    assert_eq!(
        super::super::bounded_conversion_metrics(&exact),
        Some((4096, 1, true))
    );
    assert_eq!(super::super::bounded_conversion_size(&exact), Some(4096));
    let over = Term::Match {
        discriminant: Box::new(g("Zero")),
        motive: Box::new(g("Nat")),
        cases: vec![g("Zero"); 4094],
    };
    assert!(super::super::bounded_conversion_size(&over).is_none());
    let terms = [
        Term::Match {
            discriminant: Box::new(g("Zero")),
            motive: Box::new(g("Nat")),
            cases: vec![g("Zero"); 10_000],
        },
        Term::MutualFix {
            defs: (0..10_000).map(|i| (format!("f{i}"), g("Zero"))).collect(),
            index: 0,
        },
    ];
    for term in terms {
        reset_builds();
        assert!(super::super::bounded_conversion_metrics(&term).is_none());
        assert_eq!(
            natural_normal_form(&c, &term, Transparency::All, 1000),
            (NatReadbackOutcome::Unsupported, 1000)
        );
        assert_eq!(
            builds(),
            0,
            "wide rejected input must not create/intern a store"
        );
    }
}

#[test]
fn zero_fuel_and_complex_levels_decline_before_store_creation() {
    let c = context();
    reset_builds();
    assert_eq!(
        natural_normal_form(&c, &lit(0), Transparency::All, 0),
        (NatReadbackOutcome::Exhausted, 0)
    );
    assert_eq!(builds(), 0);
    let universe = Term::Sort(crate::Universe::Succ(Box::new(crate::Universe::Type(0))));
    assert!(!natural_readback_admits(&universe));
    assert_eq!(
        natural_normal_form(&c, &universe, Transparency::All, 100),
        (NatReadbackOutcome::Unsupported, 100)
    );
    assert_eq!(builds(), 0);
}

#[test]
fn borrowed_large_delta_body_is_refused_before_copying_into_the_store() {
    let mut c = context();
    let body = balanced(10);
    assert!(super::super::bounded_conversion_size(&body).is_none());
    c.with_equations_suspended(|c| c.declare("large", g("Nat"), body))
        .unwrap();
    assert!(c.get_definition_body("large").is_some());
    let mut r = HeadReducer::for_nat_readback(&c, 1000);
    let head = r.store.intern(&g("large"));
    let before = r.store.len();
    assert!(r.global(head).is_none());
    assert_eq!(
        r.store.len(),
        before,
        "declining a body cannot intern its large tree"
    );
}

#[test]
fn borrowed_constructor_headers_and_large_inventory_keep_conservative_fallback() {
    let mut c = context();
    c.add_inductive("Packet", Term::Sort(crate::Universe::Type(0)));
    c.add_constructor("wide_header", "Packet", pi("x", balanced(10), g("Packet")));
    let mut r = HeadReducer::for_nat_readback(&c, 1000);
    let head = r.store.intern(&g("wide_header"));
    let before = r.store.len();
    assert!(r.constructor(head).is_none());
    assert_eq!(r.store.len(), before);
    drop(r);
    assert_eq!(c.constructor_inventory_upper_bound("Nat"), 2);
    assert!(
        c.refused_meaning_change_for_test(|c| {
            c.add_constructor("extra0", "Nat", g("Nat"));
            Ok(())
        })
        .is_some(),
        "the stable-meaning gate refuses this change; the cache is tested with it permitted"
    );
    c.permit_meaning_changes_for_cache_tests();
    for i in 0..4100 {
        c.add_constructor(&format!("extra{i}"), "Nat", g("Nat"));
    }
    assert_eq!(c.constructor_inventory_upper_bound("Nat"), 4102);
    reset_builds();
    assert_eq!(
        natural_normal_form(&c, &lit(0), Transparency::All, 1000),
        (NatReadbackOutcome::Unsupported, 1000)
    );
    assert_eq!(builds(), 0);
}

#[test]
fn eligibility_is_saturated_monomorphic_natural_result_only() {
    let mut c = context();
    c.add_declaration("opaque", g("Nat"));
    assert!(natural_result_candidate(
        &c,
        &app(g("keep"), lit(2)),
        Transparency::All
    ));
    for term in [
        lit(2),
        g("Zero"),
        g("opaque"),
        g("keep"),
        app(app(g("keep"), lit(2)), lit(3)),
        lam("x", g("Nat"), v("x")),
        pi("x", g("Nat"), g("Nat")),
        Term::Sort(crate::Universe::Prop),
        Term::Const {
            name: "keep".into(),
            levels: vec![],
        },
    ] {
        assert!(!natural_result_candidate(&c, &term, Transparency::All));
    }
    assert!(!natural_result_candidate(
        &c,
        &app(g("keep"), lit(2)),
        Transparency::Excluding(&|_| false)
    ));
    reset_builds();
    assert_eq!(
        crate::normalize_checked(&c, &g("opaque")).unwrap(),
        g("opaque")
    );
    assert_eq!(builds(), 0);
}

#[test]
fn public_hook_preserves_exact_literal_constructor_and_mixed_tail_syntax() {
    let c = context();
    for expected in [
        lit(0),
        lit(3),
        g("Zero"),
        peano(3, g("Zero")),
        peano(2, lit(1)),
    ] {
        let term = app(g("keep"), expected.clone());
        checked(&c, &term, &g("Nat"));
        reset_builds();
        assert_eq!(crate::normalize_checked(&c, &term).unwrap(), expected);
        assert_eq!(
            builds(),
            1,
            "eligible normalization must use one outer observer"
        );
    }
}

#[test]
fn public_nat_program_results_equal_ordinary_tree_normal_forms() {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    for name in [
        "Nat_add", "Nat_sub", "Nat_mul", "Nat_pow", "Nat_div", "Nat_mod",
    ] {
        for (a, b) in [(0, 0), (2, 0), (3, 2)] {
            let term = apps(g(name), &[lit(a), lit(b)]);
            let ordinary =
                super::super::normalize_tree_with_spent(&c, &term, Transparency::All, 0).unwrap();
            assert_eq!(
                crate::normalize_checked(&c, &term).unwrap(),
                ordinary,
                "{name} {a} {b}"
            );
        }
    }
}

#[test]
fn proof_field_projection_is_exact_but_demanded_opaque_proof_is_unsupported() {
    let mut c = context();
    c.add_inductive("Proof", Term::Sort(crate::Universe::Prop));
    c.add_constructor("proof_intro", "Proof", g("Proof"));
    c.add_declaration("opaque_proof", g("Proof"));
    c.add_inductive("Packet", Term::Sort(crate::Universe::Type(0)));
    c.add_constructor(
        "packet",
        "Packet",
        pi("h", g("Proof"), pi("n", g("Nat"), g("Packet"))),
    );
    let projection = Term::Match {
        discriminant: Box::new(apps(g("packet"), &[g("opaque_proof"), peano(3, lit(1))])),
        motive: Box::new(lam("p", g("Packet"), g("Nat"))),
        cases: vec![lam("h", g("Proof"), lam("n", g("Nat"), v("n")))],
    };
    checked(&c, &projection, &g("Nat"));
    assert_eq!(
        natural_normal_form(&c, &projection, Transparency::All, 10_000).0,
        NatReadbackOutcome::Normal(peano(3, lit(1)))
    );
    let demand = Term::Match {
        discriminant: Box::new(g("opaque_proof")),
        motive: Box::new(lam("p", g("Proof"), g("Nat"))),
        cases: vec![g("Zero")],
    };
    checked(&c, &demand, &g("Nat"));
    assert_eq!(
        natural_normal_form(&c, &demand, Transparency::All, 10_000).0,
        NatReadbackOutcome::Unsupported
    );
}

#[test]
fn checked_nonfirst_structural_column_uses_the_actual_recursive_argument() {
    let mut c = context();
    let body = lam(
        "inert",
        g("Nat"),
        lam(
            "n",
            g("Nat"),
            Term::Match {
                discriminant: Box::new(v("n")),
                motive: Box::new(lam("n0", g("Nat"), g("Nat"))),
                cases: vec![
                    g("Zero"),
                    lam(
                        "p",
                        g("Nat"),
                        app(g("Succ"), apps(v("rec"), &[v("inert"), v("p")])),
                    ),
                ],
            },
        ),
    );
    assert_eq!(
        crate::termination::structural_columns(&c, &body).first(),
        Some(&1)
    );
    let fix = Term::Fix {
        name: "rec".into(),
        body: Box::new(body),
    };
    c.with_equations_suspended(|c| {
        c.declare(
            "second_column",
            pi("inert", g("Nat"), pi("n", g("Nat"), g("Nat"))),
            fix,
        )
    })
    .unwrap();
    let term = apps(g("second_column"), &[lit(77), lit(5)]);
    checked(&c, &term, &g("Nat"));
    assert_eq!(
        crate::normalize_checked(&c, &term).unwrap(),
        peano(5, g("Zero"))
    );
}

#[test]
fn unsuccessful_observation_spends_fuel_and_tree_fallback_cannot_reset_it() {
    let mut c = context();
    c.add_declaration("opaque", g("Nat"));
    let term = app(g("keep"), g("opaque"));
    let (answer, left) = natural_normal_form(&c, &term, Transparency::All, 1000);
    assert_eq!(answer, NatReadbackOutcome::Unsupported);
    assert!(left < 1000 && left > 0);
    let nested = app(g("keep"), app(g("keep"), lit(2)));
    assert!(matches!(
        super::super::normalize_tree_with_spent(
            &c,
            &nested,
            Transparency::All,
            super::super::MAX_REDUCTION_STEPS - 1
        ),
        Err((_, Unreduced::BudgetSpent))
    ));
    assert!(matches!(
        super::super::normalize_tree_with_spent(
            &c,
            &nested,
            Transparency::All,
            super::super::MAX_REDUCTION_STEPS
        ),
        Err((_, Unreduced::BudgetSpent))
    ));
    assert_eq!(crate::normalize_checked(&c, &term).unwrap(), g("opaque"));
}

#[test]
fn public_hook_retains_excluded_transparency_and_registered_rewrite_paths() {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    let term = apps(g("Nat_add"), &[lit(2), lit(3)]);
    let excluded = |_: &str| true;
    reset_builds();
    assert_eq!(
        crate::reduction::normalize_checked_at(&c, &term, Transparency::Excluding(&excluded))
            .unwrap(),
        term
    );
    assert_eq!(builds(), 0);
    c.declare(
        "readback_zero",
        apps(
            g("Eq"),
            &[
                g("Nat"),
                apps(g("Nat_add"), &[g("Zero"), g("Zero")]),
                g("Zero"),
            ],
        ),
        apps(g("refl"), &[g("Nat"), g("Zero")]),
    )
    .unwrap();
    c.add_simp_lemma("readback_zero").unwrap();
    reset_builds();
    let ordinary =
        super::super::normalize_tree_with_spent(&c, &term, Transparency::All, 0).unwrap();
    assert_eq!(crate::normalize_checked(&c, &term).unwrap(), ordinary);
    assert_eq!(builds(), 0);
}
