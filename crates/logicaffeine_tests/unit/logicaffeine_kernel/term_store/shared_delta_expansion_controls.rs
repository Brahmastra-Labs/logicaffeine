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
fn identity(t: Term) -> Term {
    app(lam("x", g("N"), v("x")), t)
}
fn context(count: usize) -> Context {
    let mut c = Context::new();
    c.add_declaration("N", Term::Sort(Universe::Type(0)));
    for name in ["zero", "one"] {
        c.add_declaration(name, g("N"));
    }
    c.add_declaration("hold", pi("n", g("N"), g("N")));
    c.with_equations_suspended(|c| {
        for i in 0..count {
            let body = if i == 0 {
                g("zero")
            } else {
                g(&format!("alias.{}", i - 1))
            };
            c.declare(&format!("alias.{i}"), g("N"), body).unwrap();
        }
    });
    c
}
fn operands(count: usize) -> (Term, Term) {
    let value = g(&format!("alias.{}", count - 1));
    (
        app(g("hold"), value.clone()),
        app(g("hold"), identity(value)),
    )
}
fn counted(c: &Context, a: &Term, b: &Term, budget: u64) -> (bool, usize) {
    struct Stop;
    impl Drop for Stop {
        fn drop(&mut self) {
            DELTA_EXPANSION_WORK.with(|n| n.set(None));
        }
    }
    DELTA_EXPANSION_WORK.with(|n| n.set(Some((0, 0))));
    let _stop = Stop;
    let answer = beta_delta_equivalent(c, a, b, budget);
    let count = DELTA_EXPANSION_WORK.with(|n| n.get().unwrap().0);
    (answer, count)
}
#[test]
fn common_closed_delta_tail_is_expanded_once_for_both_operands() {
    let count = 32;
    let c = context(count);
    let (a, b) = operands(count);
    for term in [&a, &b] {
        assert_eq!(crate::infer_type(&c, term).unwrap(), g("N"));
        assert_eq!(crate::recheck(&c, term).unwrap(), g("N"));
    }
    for (a, b) in [(&a, &b), (&b, &a)] {
        let (equal, visits) = counted(&c, a, b, 1024);
        assert!(equal);
        assert!(
            visits <= count + 16,
            "one shared tail plus the two small wrappers, got {visits}"
        );
    }
}
#[test]
fn common_tail_work_stays_linear_as_checked_declaration_chains_grow() {
    for count in [8, 16, 48] {
        let c = context(count);
        let (a, b) = operands(count);
        let (equal, visits) = counted(&c, &a, &b, 1024);
        assert!(equal);
        assert!(
            visits <= count + 16,
            "chain length {count}: {visits} expansion visits"
        );
    }
}
#[test]
fn every_uncached_visit_uses_the_same_remaining_fuel_without_replenishment() {
    let c = context(8);
    let (a, b) = operands(8);
    let (_, complete) = counted(&c, &a, &b, 1024);
    for budget in [0, 1, 2, complete as u64 - 1] {
        let (equal, visits) = counted(&c, &a, &b, budget);
        assert!(!equal, "unfinished expansion cannot prove conversion");
        assert!(
            visits as u64 <= budget,
            "the right operand cannot replenish fuel"
        );
    }
    // Expansion can consume the last debit while beta still has work to do.
    assert!(!counted(&c, &a, &b, complete as u64).0);
    assert!(
        counted(&c, &a, &b, 1024).0,
        "a failed call publishes no state to the retry"
    );
}
#[test]
fn completed_repeated_dependencies_are_not_confused_with_active_cycles() {
    let mut c = context(12);
    let (a, b) = operands(12);
    c.with_equations_suspended(|c| {
        assert!(c.declare("cycle", g("N"), g("cycle")).is_err());
    });
    assert!(c.get_definition_body("cycle").is_none());
    assert!(!beta_delta_equivalent(
        &c,
        &app(g("hold"), g("cycle")),
        &a,
        1024
    ));
    assert!(beta_delta_equivalent(&c, &a, &b, 1024));
}
#[test]
fn successful_expansion_does_not_skip_a_distinct_right_operand_obligation() {
    let c = context(12);
    let (a, b) = operands(12);
    assert!(beta_delta_equivalent(&c, &a, &b, 1024));
    for rhs in [
        app(g("hold"), identity(g("one"))),
        app(g("other"), identity(g("alias.11"))),
        g("zero"),
    ] {
        assert!(!beta_delta_equivalent(&c, &a, &rhs, 1024));
    }
}
#[test]
fn completed_results_do_not_cross_context_mutation_or_store_lifetimes() {
    let mut c = context(12);
    let (a, b) = operands(12);
    let zero = app(g("hold"), g("zero"));
    assert!(beta_delta_equivalent(&c, &a, &zero, 1024));
    let saved = c.clone();
    c.permit_meaning_changes_for_cache_tests();
    c.with_equations_suspended(|c| c.declare("alias.0", g("N"), g("one")))
        .unwrap();
    assert!(!beta_delta_equivalent(&c, &a, &zero, 1024));
    assert!(beta_delta_equivalent(&c, &a, &b, 1024));
    assert!(beta_delta_equivalent(&saved, &a, &zero, 1024));
}
#[test]
fn shared_closed_expansion_under_binders_preserves_capture_avoidance() {
    let c = context(12);
    let redex = app(lam("x", g("N"), lam("y", g("N"), v("x"))), v("y"));
    let a = lam("outer", g("N"), app(app(g("pair"), g("alias.11")), redex));
    let correct = lam(
        "renamed",
        g("N"),
        app(app(g("pair"), g("alias.11")), lam("fresh", g("N"), v("y"))),
    );
    let captured = lam(
        "renamed",
        g("N"),
        app(app(g("pair"), g("alias.11")), lam("y", g("N"), v("y"))),
    );
    assert!(beta_delta_equivalent(&c, &a, &correct, 1024));
    assert!(!beta_delta_equivalent(&c, &a, &captured, 1024));
}
#[test]
fn explicit_universe_instances_and_bare_polymorphic_names_stay_distinct() {
    let mut c = context(8);
    c.add_universe_poly_declaration("poly", vec!["u".into()], pi("n", g("N"), g("N")))
        .unwrap();
    let at = |u| Term::Const {
        name: "poly".into(),
        levels: vec![Universe::Type(u)],
    };
    let left = app(at(0), g("alias.7"));
    assert!(beta_delta_equivalent(
        &c,
        &left,
        &app(at(0), identity(g("alias.7"))),
        1024
    ));
    assert!(!beta_delta_equivalent(
        &c,
        &left,
        &app(at(1), identity(g("alias.7"))),
        1024
    ));
    assert!(!beta_delta_equivalent(
        &c,
        &app(g("poly"), g("alias.7")),
        &left,
        1024
    ));
}
#[test]
fn open_definition_bodies_remain_ineligible_even_beneath_a_matching_binder() {
    let mut c = context(8).extend("external", g("N"));
    c.with_equations_suspended(|c| c.declare("open", g("N"), v("external")))
        .unwrap();
    let a = lam("external", g("N"), app(g("hold"), g("open")));
    let b = lam("external", g("N"), app(g("hold"), identity(g("open"))));
    assert!(!beta_delta_equivalent(&c, &a, &b, 1024));
}
#[test]
fn holes_floats_and_raw_match_shapes_cannot_become_completed_evidence() {
    let c = context(8);
    for unsupported in [
        Term::Hole,
        Term::Lit(Literal::Float(1.5)),
        Term::Match {
            discriminant: Box::new(g("zero")),
            motive: Box::new(g("N")),
            cases: vec![g("zero")],
        },
    ] {
        let a = app(g("hold"), unsupported.clone());
        let b = app(g("hold"), identity(unsupported));
        assert!(!beta_delta_equivalent(&c, &a, &b, 1024));
    }
}
#[test]
fn named_match_boundaries_keep_exact_names_and_arguments() {
    let mut c = context(8);
    c.add_inductive("Flag", Term::Sort(Universe::Type(0)));
    c.add_constructor("off", "Flag", g("Flag"));
    c.add_constructor("on", "Flag", g("Flag"));
    let ty = pi("flag", g("Flag"), g("N"));
    let body = lam(
        "flag",
        g("Flag"),
        Term::Match {
            discriminant: Box::new(v("flag")),
            motive: Box::new(lam("_", g("Flag"), g("N"))),
            cases: vec![g("zero"), g("one")],
        },
    );
    c.with_equations_suspended(|c| {
        c.declare("boundary", ty.clone(), body.clone()).unwrap();
        c.declare("different", ty, body).unwrap();
    });
    let a = app(g("hold"), app(g("boundary"), g("off")));
    assert!(beta_delta_equivalent(
        &c,
        &a,
        &app(g("hold"), identity(app(g("boundary"), g("off")))),
        1024
    ));
    assert!(!beta_delta_equivalent(
        &c,
        &a,
        &app(g("hold"), identity(app(g("different"), g("off")))),
        1024
    ));
    assert!(!beta_delta_equivalent(
        &c,
        &a,
        &app(g("hold"), identity(app(g("boundary"), g("on")))),
        1024
    ));
}
