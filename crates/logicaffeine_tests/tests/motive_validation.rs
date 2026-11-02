//! Semantic boundaries for future lazy type-head exposure.
//! These fixtures do not establish a performance result without work counters.
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    defeq_for_test, infer_type, recheck, Context, MutualInductive, Term, Universe,
};

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn pi(n: &str, ty: Term, body: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(ty),
        body_type: Box::new(body),
    }
}
fn lam(n: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}
fn sort() -> Term {
    Term::Sort(Universe::Type(0))
}
fn context() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}
fn type_alias(ctx: &mut Context, ty: Term, mode: usize) -> Term {
    match mode {
        0 => ty,
        1 => app(lam("T", sort(), v("T")), ty),
        2 => Term::Let {
            name: "T".into(),
            ty: Box::new(sort()),
            value: Box::new(ty),
            body: Box::new(v("T")),
        },
        _ => {
            ctx.declare("HeadAlias", sort(), ty).unwrap();
            g("HeadAlias")
        }
    }
}
fn both_check(ctx: &Context, term: &Term, expected: &Term) {
    let actual = infer_type(ctx, term).expect("main kernel checks fixture");
    let independent = recheck(ctx, term).expect("independent kernel checks fixture");
    assert!(defeq_for_test(ctx, &actual, expected));
    assert!(defeq_for_test(ctx, &independent, expected));
}

#[test]
fn direct_beta_let_and_delta_function_type_heads_are_interchangeable() {
    for mode in 0..4 {
        let mut ctx = context();
        let domain = type_alias(&mut ctx, pi("n", g("Nat"), g("Nat")), mode);
        let term = lam("f", domain.clone(), app(v("f"), g("Zero")));
        both_check(&ctx, &term, &pi("f", domain, g("Nat")));
    }
}
#[test]
fn hidden_function_type_does_not_skip_argument_checking() {
    for mode in 0..4 {
        let mut ctx = context();
        let domain = type_alias(&mut ctx, pi("n", g("Nat"), g("Nat")), mode);
        let term = lam("f", domain, app(v("f"), sort()));
        assert!(infer_type(&ctx, &term).is_err());
        assert!(recheck(&ctx, &term).is_err());
    }
}
fn carrier_context() -> Context {
    let mut ctx = context();
    ctx.add_mutual_inductives(&[MutualInductive {
        name: "HeadCarrier".into(),
        sort: pi("A", sort(), sort()),
        num_params: 1,
        constructors: vec![(
            "HeadPack".into(),
            pi("A", sort(), pi("x", v("A"), app(g("HeadCarrier"), v("A")))),
        )],
    }])
    .unwrap();
    ctx.declare("ElementAlias", sort(), g("Nat")).unwrap();
    ctx
}
fn carrier_match(domain: Term, motive_domain: Term, bad_case: bool) -> Term {
    lam(
        "major",
        domain,
        Term::Match {
            discriminant: Box::new(v("major")),
            motive: Box::new(lam("scrutinee", motive_domain, g("Nat"))),
            cases: vec![lam(
                "element",
                g("ElementAlias"),
                if bad_case { sort() } else { v("element") },
            )],
        },
    )
}
#[test]
fn match_head_exposure_preserves_reducible_uniform_parameters() {
    for mode in 0..4 {
        let mut ctx = carrier_context();
        let carrier = app(g("HeadCarrier"), g("ElementAlias"));
        let domain = type_alias(&mut ctx, carrier.clone(), mode);
        let term = carrier_match(domain.clone(), carrier, false);
        both_check(&ctx, &term, &pi("major", domain, g("Nat")));
    }
}
#[test]
fn matching_inductive_head_does_not_erase_uniform_parameter_mismatch() {
    let mut ctx = carrier_context();
    let domain = type_alias(&mut ctx, app(g("HeadCarrier"), g("ElementAlias")), 3);
    let term = carrier_match(domain, app(g("HeadCarrier"), g("Bool")), false);
    assert!(infer_type(&ctx, &term).is_err());
    assert!(recheck(&ctx, &term).is_err());
}
#[test]
fn matching_inductive_head_does_not_skip_case_body_checking() {
    let mut ctx = carrier_context();
    let carrier = app(g("HeadCarrier"), g("ElementAlias"));
    let domain = type_alias(&mut ctx, carrier.clone(), 3);
    let term = carrier_match(domain, carrier, true);
    assert!(infer_type(&ctx, &term).is_err());
    assert!(recheck(&ctx, &term).is_err());
}
fn indexed_context() -> Context {
    let mut ctx = context();
    ctx.add_mutual_inductives(&[MutualInductive {
        name: "HeadAtZero".into(),
        sort: pi("n", g("Nat"), sort()),
        num_params: 0,
        constructors: vec![("HeadAtZero.mk".into(), app(g("HeadAtZero"), g("Zero")))],
    }])
    .unwrap();
    ctx
}
fn indexed_match(domain: Term, wrong_index: bool) -> Term {
    let index = if wrong_index {
        app(g("Succ"), v("n"))
    } else {
        v("n")
    };
    lam(
        "major",
        domain,
        Term::Match {
            discriminant: Box::new(v("major")),
            motive: Box::new(lam(
                "n",
                g("Nat"),
                lam("scrutinee", app(g("HeadAtZero"), index), g("Nat")),
            )),
            cases: vec![g("Zero")],
        },
    )
}
#[test]
fn indexed_match_head_exposure_preserves_convertible_index() {
    let mut ctx = indexed_context();
    let index = app(lam("x", g("Nat"), v("x")), g("Zero"));
    let domain = type_alias(&mut ctx, app(g("HeadAtZero"), index), 3);
    let term = indexed_match(domain.clone(), false);
    both_check(&ctx, &term, &pi("major", domain, g("Nat")));
}
#[test]
fn indexed_match_head_exposure_does_not_launder_wrong_motive_index() {
    let mut ctx = indexed_context();
    let domain = type_alias(&mut ctx, app(g("HeadAtZero"), g("Zero")), 3);
    let term = indexed_match(domain, true);
    assert!(infer_type(&ctx, &term).is_err());
    assert!(recheck(&ctx, &term).is_err());
}

#[test]
fn oversized_delta_type_head_retains_general_checking_fallback() {
    let mut ctx = context();
    let mut result = g("Nat");
    for i in 0..11 {
        result = pi(&format!("n{i}"), result.clone(), result);
    }
    // More than4096 nodes but shallow, valid, and already in normal form.
    let domain = type_alias(&mut ctx, pi("n", g("Nat"), result.clone()), 3);
    let term = lam("f", domain.clone(), app(v("f"), g("Zero")));
    both_check(&ctx, &term, &pi("f", domain, result));
}
#[test]
fn registered_rewrite_context_retains_general_type_head_semantics() {
    let mut ctx = context();
    ctx.declare(
        "HeadIdentity",
        pi("n", g("Nat"), g("Nat")),
        lam("n", g("Nat"), v("n")),
    )
    .unwrap();
    let lhs = app(g("HeadIdentity"), v("n"));
    let equation = app(app(app(g("Eq"), g("Nat")), lhs), v("n"));
    let proof = app(app(g("refl"), g("Nat")), v("n"));
    ctx.declare(
        "HeadIdentity.rule",
        pi("n", g("Nat"), equation),
        lam("n", g("Nat"), proof),
    )
    .unwrap();
    ctx.add_simp_lemma("HeadIdentity.rule").unwrap();
    assert!(!ctx.simp_rules().is_empty());
    let domain = type_alias(&mut ctx, pi("n", g("Nat"), g("Nat")), 3);
    let term = lam(
        "f",
        domain.clone(),
        app(v("f"), app(g("HeadIdentity"), g("Zero"))),
    );
    both_check(&ctx, &term, &pi("f", domain, g("Nat")));
}

#[test]
fn direct_indexed_motive_domain_is_independently_checked() {
    let ctx = indexed_context();
    let term = indexed_match(app(g("HeadAtZero"), g("Zero")), true);
    let main = infer_type(&ctx, &term);
    let independent = recheck(&ctx, &term);
    eprintln!("minimal wrong-index motive: main={main:?}, independent={independent:?}");
    assert!(
        independent.is_err(),
        "independent checker must check major domain before beta erasure"
    );
    assert!(
        main.is_err(),
        "main checker must check major domain before beta erasure"
    );
}

fn reject_in_both(ctx: &Context, term: &Term) {
    let main = infer_type(ctx, term);
    let independent = recheck(ctx, term);
    assert!(
        main.is_err() && independent.is_err(),
        "malformed motive: main={main:?}; independent={independent:?}"
    );
}
#[test]
fn indexed_motive_index_domain_is_checked_before_erasing_unused_index() {
    let ctx = indexed_context();
    let domain = app(g("HeadAtZero"), g("Zero"));
    let term = lam(
        "major",
        domain.clone(),
        Term::Match {
            discriminant: Box::new(v("major")),
            motive: Box::new(lam("n", g("Bool"), lam("scrutinee", domain, g("Nat")))),
            cases: vec![g("Zero")],
        },
    );
    reject_in_both(&ctx, &term);
}
#[test]
fn indexed_motive_must_work_for_generic_indices_not_only_current_major() {
    let ctx = indexed_context();
    let domain = app(g("HeadAtZero"), g("Zero"));
    let term = lam(
        "major",
        domain.clone(),
        Term::Match {
            discriminant: Box::new(v("major")),
            motive: Box::new(lam("n", g("Nat"), lam("scrutinee", domain, g("Nat")))),
            cases: vec![g("Zero")],
        },
    );
    reject_in_both(&ctx, &term);
}
#[test]
fn indexed_motive_codomain_must_be_a_sort() {
    let ctx = indexed_context();
    let term = lam(
        "major",
        app(g("HeadAtZero"), g("Zero")),
        Term::Match {
            discriminant: Box::new(v("major")),
            motive: Box::new(lam(
                "n",
                g("Nat"),
                lam("scrutinee", app(g("HeadAtZero"), v("n")), g("Zero")),
            )),
            cases: vec![g("Zero")],
        },
    );
    reject_in_both(&ctx, &term);
}
#[test]
fn empty_indexed_family_still_checks_generic_motive_domain() {
    let mut ctx = context();
    ctx.add_mutual_inductives(&[MutualInductive {
        name: "EmptyIndex".into(),
        sort: pi("n", g("Nat"), sort()),
        num_params: 0,
        constructors: vec![],
    }])
    .unwrap();
    let term = lam(
        "major",
        app(g("EmptyIndex"), g("Zero")),
        Term::Match {
            discriminant: Box::new(v("major")),
            motive: Box::new(lam(
                "n",
                g("Nat"),
                lam(
                    "scrutinee",
                    app(g("EmptyIndex"), app(g("Succ"), v("n"))),
                    g("Nat"),
                ),
            )),
            cases: vec![],
        },
    );
    reject_in_both(&ctx, &term);
}
fn pruned_match(wrong_major: bool) -> (Context, Term) {
    let mut ctx = context();
    ctx.add_mutual_inductives(&[MutualInductive {
        name: "TwoIndices".into(),
        sort: pi("n", g("Nat"), sort()),
        num_params: 0,
        constructors: vec![
            ("TwoIndices.zero".into(), app(g("TwoIndices"), g("Zero"))),
            (
                "TwoIndices.one".into(),
                app(g("TwoIndices"), app(g("Succ"), g("Zero"))),
            ),
        ],
    }])
    .unwrap();
    let motive_index = if wrong_major { g("Zero") } else { v("n") };
    let term = lam(
        "major",
        app(g("TwoIndices"), g("Zero")),
        Term::Match {
            discriminant: Box::new(v("major")),
            motive: Box::new(lam(
                "n",
                g("Nat"),
                lam("scrutinee", app(g("TwoIndices"), motive_index), g("Nat")),
            )),
            cases: vec![g("Zero")],
        },
    );
    (ctx, term)
}
#[test]
fn pruned_indexed_coverage_preserves_valid_generic_motive() {
    let (ctx, term) = pruned_match(false);
    both_check(
        &ctx,
        &term,
        &pi("major", app(g("TwoIndices"), g("Zero")), g("Nat")),
    );
}
#[test]
fn pruned_indexed_coverage_cannot_hide_non_generic_motive() {
    let (ctx, term) = pruned_match(true);
    reject_in_both(&ctx, &term);
}

fn poly_family(level: Universe, parameter: Term, a: Term, x: Term) -> Term {
    app(
        app(
            app(
                Term::Const {
                    name: "DependentIndices".into(),
                    levels: vec![level],
                },
                parameter,
            ),
            a,
        ),
        x,
    )
}
fn dependent_polymorphic_match(fault: usize) -> (Context, Term) {
    let mut ctx = context();
    let u = Universe::Var("u".into());
    let su = Term::Sort(u.clone());
    ctx.add_universe_poly_inductives(
        &["u".into()],
        &[MutualInductive {
            name: "DependentIndices".into(),
            sort: pi(
                "B",
                su.clone(),
                pi("A", su.clone(), pi("x", v("A"), Term::Sort(u.succ()))),
            ),
            num_params: 1,
            constructors: vec![(
                "DependentIndices.mk".into(),
                pi(
                    "B",
                    su.clone(),
                    pi(
                        "A",
                        su,
                        pi("x", v("A"), poly_family(u, v("B"), v("A"), v("x"))),
                    ),
                ),
            )],
        }],
    )
    .unwrap();
    let level = Universe::Type(0);
    let domain = poly_family(level.clone(), g("Nat"), g("Nat"), g("Zero"));
    let motive_level = if fault == 2 { Universe::Type(1) } else { level };
    let motive_parameter = if fault == 3 { g("Bool") } else { g("Nat") };
    let index_domain = if fault == 1 { g("Nat") } else { v("A") };
    let term = lam(
        "major",
        domain,
        Term::Match {
            discriminant: Box::new(v("major")),
            motive: Box::new(lam(
                "A",
                sort(),
                lam(
                    "x",
                    index_domain,
                    lam(
                        "scrutinee",
                        poly_family(motive_level, motive_parameter, v("A"), v("x")),
                        g("Nat"),
                    ),
                ),
            )),
            cases: vec![lam("A", sort(), lam("x", v("A"), g("Zero")))],
        },
    );
    (ctx, term)
}
#[test]
fn dependent_index_domains_and_explicit_universe_parameters_are_supported() {
    let (ctx, term) = dependent_polymorphic_match(0);
    let domain = poly_family(Universe::Type(0), g("Nat"), g("Nat"), g("Zero"));
    both_check(&ctx, &term, &pi("major", domain, g("Nat")));
}
#[test]
fn dependent_index_domain_cannot_be_specialized_to_current_argument() {
    let (ctx, term) = dependent_polymorphic_match(1);
    reject_in_both(&ctx, &term);
}
#[test]
fn indexed_major_universe_instance_must_match_exactly() {
    let (ctx, term) = dependent_polymorphic_match(2);
    reject_in_both(&ctx, &term);
}
#[test]
fn indexed_major_uniform_parameter_must_match_exactly() {
    let (ctx, term) = dependent_polymorphic_match(3);
    reject_in_both(&ctx, &term);
}

fn empty_nonindexed_match(value_valued: bool) -> (Context, Term) {
    let mut ctx = context();
    ctx.add_mutual_inductives(&[MutualInductive {
        name: "HeadEmpty".into(),
        sort: sort(),
        num_params: 0,
        constructors: vec![],
    }])
    .unwrap();
    let term = lam(
        "major",
        g("HeadEmpty"),
        Term::Match {
            discriminant: Box::new(v("major")),
            motive: Box::new(lam(
                "scrutinee",
                g("HeadEmpty"),
                if value_valued { g("Zero") } else { g("Nat") },
            )),
            cases: vec![],
        },
    );
    (ctx, term)
}
#[test]
fn empty_nonindexed_family_accepts_type_valued_motive() {
    let (ctx, term) = empty_nonindexed_match(false);
    both_check(&ctx, &term, &pi("major", g("HeadEmpty"), g("Nat")));
}
#[test]
fn empty_nonindexed_family_rejects_value_valued_motive() {
    let (ctx, term) = empty_nonindexed_match(true);
    reject_in_both(&ctx, &term);
}
#[test]
fn generic_motive_binders_do_not_capture_outer_dependent_parameters() {
    let (ctx, _) = dependent_polymorphic_match(0);
    let outer = v("__motive_index0");
    let outer_value = v("__motive_major");
    let domain = poly_family(Universe::Type(0), outer.clone(), outer.clone(), outer_value);
    let motive = lam(
        "A",
        sort(),
        lam(
            "x",
            v("A"),
            lam(
                "scrutinee",
                poly_family(Universe::Type(0), outer.clone(), v("A"), v("x")),
                g("Nat"),
            ),
        ),
    );
    let matched = Term::Match {
        discriminant: Box::new(v("major")),
        motive: Box::new(motive),
        cases: vec![lam("A", sort(), lam("x", v("A"), g("Zero")))],
    };
    let term = lam(
        "__motive_index0",
        sort(),
        lam(
            "__motive_major",
            outer.clone(),
            lam("major", domain.clone(), matched),
        ),
    );
    let expected = pi(
        "__motive_index0",
        sort(),
        pi("__motive_major", outer, pi("major", domain, g("Nat"))),
    );
    both_check(&ctx, &term, &expected);
}
fn two_generic_indices(swapped: bool) -> (Context, Term) {
    let mut ctx = context();
    let family = |n: Term, m: Term| app(app(g("DiagonalIndex"), n), m);
    ctx.add_mutual_inductives(&[MutualInductive {
        name: "DiagonalIndex".into(),
        sort: pi("n", g("Nat"), pi("m", g("Nat"), sort())),
        num_params: 0,
        constructors: vec![(
            "DiagonalIndex.mk".into(),
            pi("n", g("Nat"), family(v("n"), v("n"))),
        )],
    }])
    .unwrap();
    let domain = family(g("Zero"), g("Zero"));
    let major_domain = if swapped {
        family(v("m"), v("n"))
    } else {
        family(v("n"), v("m"))
    };
    let term = lam(
        "major",
        domain,
        Term::Match {
            discriminant: Box::new(v("major")),
            motive: Box::new(lam(
                "n",
                g("Nat"),
                lam("m", g("Nat"), lam("scrutinee", major_domain, g("Nat"))),
            )),
            cases: vec![lam("n", g("Nat"), g("Zero"))],
        },
    );
    (ctx, term)
}
#[test]
fn multiple_generic_indices_preserve_their_declared_order() {
    let (ctx, term) = two_generic_indices(false);
    both_check(
        &ctx,
        &term,
        &pi(
            "major",
            app(app(g("DiagonalIndex"), g("Zero")), g("Zero")),
            g("Nat"),
        ),
    );
}
#[test]
fn equal_actual_indices_do_not_hide_swapped_generic_motive_indices() {
    let (ctx, term) = two_generic_indices(true);
    reject_in_both(&ctx, &term);
}
