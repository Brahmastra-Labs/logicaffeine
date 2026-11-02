use super::*;
fn g(n: &str) -> Db {
    Db::Global(n.into())
}
fn t(n: &str) -> Term {
    Term::Global(n.into())
}
fn app(f: Db, a: Db) -> Db {
    Db::App(DbBox::new(f), DbBox::new(a))
}
fn context() -> Context {
    let mut c = Context::new();
    c.add_declaration("N", Term::Sort(Universe::Type(0)));
    for n in ["zero", "one"] {
        c.add_declaration(n, t("N"));
    }
    let ty = Term::Pi {
        param: "x".into(),
        param_type: Box::new(t("N")),
        body_type: Box::new(t("N")),
    };
    for (name, body) in [("id", Term::Var("x".into())), ("erase", t("zero"))] {
        c.declare(
            name,
            ty.clone(),
            Term::Lambda {
                param: "x".into(),
                param_type: Box::new(t("N")),
                body: Box::new(body),
            },
        )
        .unwrap();
    }
    c.declare("alias", t("N"), t("zero")).unwrap();
    c
}
struct Restore(usize, usize, usize, bool, bool);
impl Restore {
    fn new() -> Self {
        Self(
            WHNF_FUEL.with(|v| v.get()),
            WHNF_DEPTH.with(|v| v.get()),
            LOCAL_CONTEXT_BUDGET_DEPTH.with(|v| v.get()),
            WHNF_EXHAUSTED.with(|v| v.get()),
            ACTIVE.with(|v| v.get()),
        )
    }
}
impl Drop for Restore {
    fn drop(&mut self) {
        WHNF_FUEL.with(|v| v.set(self.0));
        WHNF_DEPTH.with(|v| v.set(self.1));
        LOCAL_CONTEXT_BUDGET_DEPTH.with(|v| v.set(self.2));
        WHNF_EXHAUSTED.with(|v| v.set(self.3));
        ACTIVE.with(|v| v.set(self.4));
    }
}
#[test]
fn unequal_arguments_are_not_a_rejection_of_erasing_applications() {
    let c = context();
    let a = app(g("erase"), g("zero"));
    let b = app(g("erase"), g("one"));
    assert!(!equivalent(&c, &[], &a, &b));
    assert!(!WHNF_EXHAUSTED.with(|v| v.get()));
    assert!(def_eq(&c, &[], &a, &b));
    assert!(!equivalent(
        &c,
        &[],
        &app(g("id"), g("zero")),
        &app(g("id"), g("one"))
    ));
}
#[test]
fn local_indices_and_context_rebinding_remain_distinct() {
    let c = context();
    let locals = vec![g("N"), g("N")];
    let a = app(g("id"), Db::Var(0));
    let b = app(g("id"), Db::Var(1));
    assert!(!equivalent(&c, &locals, &a, &b));
    assert!(equivalent(&c, &locals, &a, &a));
    assert!(equivalent(
        &c,
        &[],
        &app(g("id"), g("alias")),
        &app(g("id"), g("zero"))
    ));
    let mut other = c.clone();
    other.declare("alias", t("N"), t("one")).unwrap();
    assert!(!equivalent(
        &other,
        &[],
        &app(g("id"), g("alias")),
        &app(g("id"), g("zero"))
    ));
}
#[test]
fn nested_exhaustion_is_charged_once_and_erasing_fallback_can_finish() {
    let mut c = context();
    let mut body = t("zero");
    for i in 0..80 {
        body = Term::Let {
            name: format!("x{i}"),
            ty: Box::new(t("N")),
            value: Box::new(t("zero")),
            body: Box::new(body),
        };
    }
    c.declare("long", t("N"), body).unwrap();
    let a = app(g("erase"), app(g("id"), g("long")));
    let b = app(g("erase"), app(g("id"), g("zero")));
    let _restore = Restore::new();
    WHNF_EXHAUSTED.with(|v| v.set(false));
    WHNF_FUEL.with(|v| v.set(64));
    WHNF_DEPTH.with(|v| v.set(0));
    LOCAL_CONTEXT_BUDGET_DEPTH.with(|v| v.set(1));
    assert!(!equivalent(&c, &[], &a, &b));
    assert_eq!(
        WHNF_FUEL.with(|v| v.get()),
        48,
        "all nested reductions share exactly sixteen available steps"
    );
    assert_eq!(LOCAL_CONTEXT_BUDGET_DEPTH.with(|v| v.get()), 1);
    assert!(!WHNF_EXHAUSTED.with(|v| v.get()));
    assert!(!ACTIVE.with(|v| v.get()));
    assert!(
        def_eq(&c, &[], &a, &b),
        "the ordinary erasing conversion retains enough fuel"
    );
    assert!(!WHNF_EXHAUSTED.with(|v| v.get()));
}
#[test]
fn preexisting_exhaustion_and_tiny_owned_budgets_are_preserved() {
    let c = context();
    let a = app(g("id"), g("alias"));
    let b = app(g("id"), g("zero"));
    let _restore = Restore::new();
    for (fuel, exhausted) in [(100, true), (0, false), (3, false)] {
        WHNF_FUEL.with(|v| v.set(fuel));
        WHNF_DEPTH.with(|v| v.set(1));
        WHNF_EXHAUSTED.with(|v| v.set(exhausted));
        assert!(!equivalent(&c, &[], &a, &b));
        assert_eq!(WHNF_FUEL.with(|v| v.get()), fuel);
        assert_eq!(WHNF_EXHAUSTED.with(|v| v.get()), exhausted);
        assert!(!ACTIVE.with(|v| v.get()));
    }
}
#[test]
fn inactive_fuel_state_is_restored_and_repeated_calls_have_no_answers_cached() {
    let c = context();
    let a = app(g("id"), g("alias"));
    let b = app(g("id"), g("zero"));
    let _restore = Restore::new();
    WHNF_DEPTH.with(|v| v.set(0));
    LOCAL_CONTEXT_BUDGET_DEPTH.with(|v| v.set(0));
    WHNF_FUEL.with(|v| v.set(7));
    assert!(equivalent(&c, &[], &a, &b));
    assert_eq!(WHNF_FUEL.with(|v| v.get()), 7);
    assert!(!equivalent(&c, &[], &a, &app(g("id"), g("one"))));
    assert_eq!(WHNF_FUEL.with(|v| v.get()), 7);
}
#[test]
fn explicit_universes_and_arity_are_part_of_head_identity() {
    let mut c = context();
    let u = Universe::Var("u".into());
    let ty = Term::Pi {
        param: "A".into(),
        param_type: Box::new(Term::Sort(u.clone())),
        body_type: Box::new(Term::Pi {
            param: "x".into(),
            param_type: Box::new(Term::Var("A".into())),
            body_type: Box::new(Term::Var("A".into())),
        }),
    };
    let body = Term::Lambda {
        param: "A".into(),
        param_type: Box::new(Term::Sort(u)),
        body: Box::new(Term::Lambda {
            param: "x".into(),
            param_type: Box::new(Term::Var("A".into())),
            body: Box::new(Term::Var("x".into())),
        }),
    };
    c.declare_universe_poly("poly", vec!["u".into()], ty, body)
        .unwrap();
    let call = |u, x| {
        app(
            app(
                Db::Const {
                    name: "poly".into(),
                    levels: vec![u],
                },
                g("N"),
            ),
            g(x),
        )
    };
    assert!(equivalent(
        &c,
        &[],
        &call(Universe::Type(0), "alias"),
        &call(Universe::Type(0), "zero")
    ));
    assert!(!equivalent(
        &c,
        &[],
        &call(Universe::Type(0), "alias"),
        &call(Universe::Type(1), "zero")
    ));
    assert!(!equivalent(&c, &[], &app(g("id"), g("zero")), &g("id")));
}
#[test]
fn nested_scope_cleanup_survives_unwind() {
    let _restore = Restore::new();
    WHNF_DEPTH.with(|v| v.set(0));
    LOCAL_CONTEXT_BUDGET_DEPTH.with(|v| v.set(1));
    WHNF_FUEL.with(|v| v.set(80));
    let result = std::panic::catch_unwind(|| {
        let _outer = Scope::enter().unwrap();
        let _inner = Scope::enter().unwrap();
        WHNF_FUEL.with(|v| v.set(v.get() - 3));
        WHNF_EXHAUSTED.with(|v| v.set(true));
        panic!("probe unwind");
    });
    assert!(result.is_err());
    assert_eq!(WHNF_FUEL.with(|v| v.get()), 77);
    assert_eq!(LOCAL_CONTEXT_BUDGET_DEPTH.with(|v| v.get()), 1);
    assert!(!WHNF_EXHAUSTED.with(|v| v.get()));
    assert!(!ACTIVE.with(|v| v.get()));
}

#[test]
fn folded_function_definition_alignment_avoids_whnf_input_copies() {
    let mut c = context();
    let function_ty = Term::Pi {
        param: "x".into(),
        param_type: Box::new(t("N")),
        body_type: Box::new(t("N")),
    };
    c.add_declaration("step", function_ty.clone());
    c.add_declaration(
        "observe",
        Term::Pi {
            param: "f".into(),
            param_type: Box::new(function_ty.clone()),
            body_type: Box::new(t("N")),
        },
    );
    let mut body = Term::Var("x".into());
    for _ in 0..40 {
        body = Term::App(Box::new(t("step")), Box::new(body));
    }
    let body = Term::Lambda {
        param: "x".into(),
        param_type: Box::new(t("N")),
        body: Box::new(body),
    };
    c.declare("function_alias", function_ty, body.clone())
        .unwrap();
    let explicit = to_db(&body, &mut Vec::new()).unwrap();
    let a = app(g("observe"), g("function_alias"));
    let b = app(g("observe"), explicit);
    WHNF_INPUT_CLONES.with(|v| v.set(Some(0)));
    let equal = def_eq(&c, &[], &a, &b);
    let copies = WHNF_INPUT_CLONES.with(|v| v.replace(None).unwrap());
    assert!(equal);
    assert_eq!(
        copies, 0,
        "closed function alignment should not copy ordinary WHNF inputs"
    );
}

#[test]
fn function_alignment_requires_existing_scope_and_preserves_failed_work() {
    let c = context();
    let body = to_db(c.get_definition_body("id").unwrap(), &mut Vec::new()).unwrap();
    let _restore = Restore::new();
    WHNF_EXHAUSTED.with(|v| v.set(false));
    WHNF_FUEL.with(|v| v.set(800));
    ACTIVE.with(|v| v.set(false));
    assert!(!align_definitions(&c, &g("id"), &body));
    assert_eq!(WHNF_FUEL.with(|v| v.get()), 800);
    ACTIVE.with(|v| v.set(true));
    assert!(align_definitions(&c, &g("id"), &body));
    let after = WHNF_FUEL.with(|v| v.get());
    assert!(after < 800 && after > 0);
    assert!(!align_definitions(&c, &g("id"), &g("one")));
    assert!(WHNF_FUEL.with(|v| v.get()) < after);
    WHNF_EXHAUSTED.with(|v| v.set(true));
    let before = WHNF_FUEL.with(|v| v.get());
    assert!(!align_definitions(&c, &g("id"), &body));
    assert_eq!(WHNF_FUEL.with(|v| v.get()), before);
    assert!(WHNF_EXHAUSTED.with(|v| v.get()));
}
#[test]
fn widened_delta_refuses_open_and_deep_function_bodies() {
    let mut c = context();
    let function_ty = Term::Pi {
        param: "x".into(),
        param_type: Box::new(t("N")),
        body_type: Box::new(t("N")),
    };
    c.add_definition(
        "open_function".into(),
        function_ty.clone(),
        Term::Lambda {
            param: "x".into(),
            param_type: Box::new(t("N")),
            body: Box::new(Term::Var("outside".into())),
        },
    );
    assert!(delta::reduce_with_limit(&c, &g("open_function"), &mut 4096, 512).is_none());
    let mut body = t("zero");
    let mut ty = t("N");
    for i in 0..80 {
        body = Term::Lambda {
            param: format!("x{i}"),
            param_type: Box::new(t("N")),
            body: Box::new(body),
        };
        ty = Term::Pi {
            param: format!("x{i}"),
            param_type: Box::new(t("N")),
            body_type: Box::new(ty),
        };
    }
    c.add_definition("deep_function".into(), ty, body);
    let mut fuel = 4096;
    assert!(delta::reduce_with_limit(&c, &g("deep_function"), &mut fuel, 512).is_none());
    assert!(fuel < 4096 && fuel > 0);
}
#[test]
fn widened_function_alignment_respects_universe_arity_and_order() {
    let mut c = context();
    let u = Universe::Var("u".into());
    let body = Term::Lambda {
        param: "A".into(),
        param_type: Box::new(Term::Sort(u.clone())),
        body: Box::new(Term::Var("A".into())),
    };
    let ty = Term::Pi {
        param: "A".into(),
        param_type: Box::new(Term::Sort(u.clone())),
        body_type: Box::new(Term::Sort(u)),
    };
    c.declare_universe_poly("universe_id", vec!["u".into(), "v".into()], ty, body)
        .unwrap();
    let _restore = Restore::new();
    ACTIVE.with(|v| v.set(true));
    WHNF_EXHAUSTED.with(|v| v.set(false));
    let target = Db::Lam(
        DbBox::new(Db::Sort(Universe::Type(0))),
        DbBox::new(Db::Var(0)),
    );
    for (levels, expected) in [
        (vec![Universe::Type(0), Universe::Type(1)], true),
        (vec![Universe::Type(1), Universe::Type(0)], false),
        (vec![Universe::Type(0)], false),
    ] {
        WHNF_FUEL.with(|v| v.set(4096));
        assert_eq!(
            align_definitions(
                &c,
                &Db::Const {
                    name: "universe_id".into(),
                    levels
                },
                &target
            ),
            expected
        );
        assert!(WHNF_FUEL.with(|v| v.get()) < 4096);
    }
}
#[test]
fn widened_probe_stops_recursive_alias_chains_without_spending_caller_reserve() {
    let mut c = context();
    let mut previous = "id".to_string();
    let ty = Term::Pi {
        param: "x".into(),
        param_type: Box::new(t("N")),
        body_type: Box::new(t("N")),
    };
    for i in 0..200 {
        let name = format!("function_chain.{i}");
        c.add_definition(name.clone(), ty.clone(), t(&previous));
        previous = name;
    }
    let _restore = Restore::new();
    WHNF_EXHAUSTED.with(|v| v.set(false));
    let mut fuel = 4096;
    assert!(!borrowed_congruence_with_limit(
        Some(&c),
        &g(&previous),
        &g("id"),
        &mut fuel,
        512,
        128
    ));
    assert!(fuel > 0 && fuel < 4096);
    assert!(!WHNF_EXHAUSTED.with(|v| v.get()));
    assert!(def_eq(&c, &[], &g(&previous), &g("id")));
}
#[test]
fn reserved_function_fuel_returns_only_unspent_work_on_unwind() {
    let _restore = Restore::new();
    WHNF_FUEL.with(|v| v.set(123));
    let result = std::panic::catch_unwind(|| {
        let mut reserved = ReservedFuel(WHNF_FUEL.with(|v| v.replace(0)));
        assert_eq!(WHNF_FUEL.with(|v| v.get()), 0);
        reserved.0 -= 19;
        panic!("alignment unwind");
    });
    assert!(result.is_err());
    assert_eq!(WHNF_FUEL.with(|v| v.get()), 104);
}
