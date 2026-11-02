use super::*;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn pair() -> (Term, Term) {
    (
        Term::App(
            Box::new(Term::Lambda {
                param: "n".into(),
                param_type: Box::new(g("Nat")),
                body: Box::new(Term::Var("n".into())),
            }),
            Box::new(g("Zero")),
        ),
        g("Zero"),
    )
}
fn context() -> Context {
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
    c
}
#[test]
fn registration_reuses_conversion_but_still_checks_the_body() {
    let mut c = context();
    let (a, b) = pair();
    c.add_inductive(
        "Indexed",
        Term::Pi {
            param: "n".into(),
            param_type: Box::new(g("Nat")),
            body_type: Box::new(Term::Sort(Universe::Type(0))),
        },
    );
    let app = |t| Term::App(Box::new(g("Indexed")), Box::new(t));
    let actual = app(a);
    let expected = app(b);
    c.add_declaration("witness", actual.clone());
    let mut work = 0;
    c.declare_with_validation::<crate::KernelError>(
        "checked",
        expected.clone(),
        g("witness"),
        |ctx, ty, _| {
            assert!(crate::type_checker::def_eq(ctx, &actual, ty));
            work = COMPUTATIONS.with(|n| n.get());
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(
        COMPUTATIONS.with(|n| n.get()),
        work,
        "registration must reuse the kernel-earned exact conversion"
    );
    assert_eq!(c.get_definition_type("checked"), Some(&expected));
    assert!(c
        .declare_with_validation::<crate::KernelError>("forged", g("Nat"), g("Succ"), |_, _, _| Ok(
            ()
        ))
        .is_err());
    assert!(!c.is_definition("forged"));
}
#[test]
fn validation_errors_are_atomic_and_cannot_be_deferred() {
    let mut c = context();
    c.declare("old", g("Nat"), g("Zero")).unwrap();
    let result = c.declare_with_validation::<crate::KernelError>(
        "old",
        g("Nat"),
        g("missing"),
        |_, _, _| {
            Err(crate::KernelError::CertificationError(
                "source mismatch".into(),
            ))
        },
    );
    assert!(result.is_err());
    assert_eq!(c.get_definition_body("old"), Some(&g("Zero")));
    assert!(c.pending_obligations().is_empty());
}
#[test]
fn contexts_and_shadowed_locals_cannot_share_evidence() {
    let c = context();
    let (a, b) = pair();
    scope(&c, || {
        assert!(crate::type_checker::def_eq(&c, &a, &b));
        assert!(!known(&c.clone(), &a, &b));
        assert!(!known(&c.extend("x", g("Nat")), &a, &b));
        assert!(!known(&c, &a, &g("Succ")));
    });
    let mut changed = c;
    changed.add_definition("Zero".into(), g("Nat"), g("missing"));
    scope(&changed, || assert!(!known(&changed, &a, &b)));
}
#[test]
fn nested_scopes_and_panics_restore_only_their_own_evidence() {
    let c = context();
    let other = context();
    let (a, b) = pair();
    scope(&c, || {
        assert!(crate::type_checker::def_eq(&c, &a, &b));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            scope(&other, || {
                assert!(!known(&c, &a, &b));
                assert!(crate::type_checker::def_eq(&other, &a, &b));
                panic!("scope exit")
            })
        }));
        assert!(result.is_err());
        assert!(known(&c, &a, &b));
        assert!(!known(&other, &a, &b));
    });
    assert!(!known(&c, &a, &b));
    assert!(!known(&other, &a, &b));
}
#[test]
fn failed_conversion_and_cumulative_subtyping_are_not_exact_evidence() {
    let c = context();
    let a = Term::Sort(Universe::Type(0));
    let b = Term::Sort(Universe::Type(1));
    scope(&c, || {
        assert!(crate::is_subtype(&c, &a, &b));
        assert!(!known(&c, &a, &b));
        assert!(!crate::type_checker::def_eq(&c, &a, &b));
        assert!(!known(&c, &a, &b));
        assert!(!crate::type_checker::def_eq(&c, &g("Zero"), &g("Succ")));
        assert!(!known(&c, &g("Zero"), &g("Succ")));
    });
}
#[test]
fn admission_bounds_names_width_depth_universes_and_unfaithful_literals() {
    let small = g("Zero");
    assert!(admit(&small, &small).is_some());
    let wide = Term::Match {
        discriminant: Box::new(small.clone()),
        motive: Box::new(small.clone()),
        cases: vec![small.clone(); MAX_PAIR_NODES],
    };
    assert!(admit(&wide, &small).is_none());
    let name = g(&"x".repeat(MAX_PAYLOAD + 1));
    assert!(admit(&name, &small).is_none());
    let mut deep = small.clone();
    for _ in 0..=MAX_DEPTH {
        deep = Term::App(Box::new(g("f")), Box::new(deep));
    }
    assert!(admit(&deep, &small).is_none());
    let levels = Term::Const {
        name: "f".into(),
        levels: vec![Universe::Prop; MAX_PAIR_NODES + 1],
    };
    assert!(admit(&levels, &small).is_none());
    let mut level = Universe::Prop;
    for _ in 0..=MAX_DEPTH {
        level = Universe::Succ(Box::new(level));
    }
    assert!(admit(&Term::Sort(level), &small).is_none());
    assert!(admit(&Term::Lit(Literal::Float(f64::NAN)), &small).is_none());
    assert!(admit(&Term::Hole, &small).is_none());
}
#[test]
fn cache_eviction_does_not_recycle_term_ids_into_old_proofs() {
    let c = context();
    let (a, b) = pair();
    scope(&c, || {
        assert!(crate::type_checker::def_eq(&c, &a, &b));
        ACTIVE.with(|slot| slot.borrow_mut().as_mut().unwrap().payload = MAX_STORE_PAYLOAD);
        assert!(!known(&c, &g("different"), &g("not-equal")));
        assert!(!known(&c, &a, &b));
        assert!(crate::type_checker::def_eq(&c, &a, &b));
        assert!(known(&c, &a, &b));
    });
}

#[test]
fn conversion_evidence_stays_on_its_own_thread() {
    let c = context();
    let (a, b) = pair();
    scope(&c, || {
        assert!(crate::type_checker::def_eq(&c, &a, &b));
        std::thread::scope(|threads| {
            threads
                .spawn(|| {
                    assert!(!known(&c, &a, &b));
                    assert!(crate::type_checker::def_eq(&c, &a, &b));
                    assert!(!known(&c, &a, &b));
                })
                .join()
                .unwrap();
        });
        assert!(known(&c, &a, &b));
    });
}

#[test]
fn prechecked_bodies_keep_cycle_and_pending_registration_guards() {
    let mut c = context();
    c.declare("old", g("Nat"), g("Zero")).unwrap();
    let mut visited = false;
    let result =
        c.declare_with_validation::<crate::KernelError>("old", g("Nat"), g("old"), |_, _, _| {
            visited = true;
            Ok(())
        });
    assert!(visited && result.is_err());
    assert_eq!(c.get_definition_body("old"), Some(&g("Zero")));
    assert!(c.pending_obligations().is_empty());
    c.declare_with_validation::<crate::KernelError>("later", g("Nat"), g("missing"), |_, _, _| {
        Ok(())
    })
    .unwrap();
    assert!(
        !c.pending_obligations().is_empty(),
        "ordinary repairable obligations must remain visible"
    );
}
