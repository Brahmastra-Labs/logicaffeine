//! A conversion's "not equal" is a decision only when none of the kernel work it relied on stopped
//! for want of a resource. `observe_resource_stops` is how a caller asks — the Lean reader turns a
//! refusal with a recorded stop into a `ResourceLimit` decline and one without into an
//! `InvalidProof` rejection. These locks pin the record itself: what it sees, what it does not, and
//! that it survives the kernel's own sized-stack threads.
use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    defeq_for_test, discard_resource_stops, is_subtype, normalize_checked, observe_resource_stops,
    Context, Literal, Term, Unreduced,
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
fn lam(n: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}
fn pi(n: &str, ty: Term, body: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(ty),
        body_type: Box::new(body),
    }
}
fn nat(n: u64) -> Term {
    Term::Lit(Literal::Nat(logicaffeine_kernel::BigInt::from_u64(n)))
}

/// `countdown n = Zero`, one structural step per unit: no acceleration recognizes it, so its
/// normal form at `2²¹` lies beyond the reduction budget.
fn context() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    let body = lam(
        "n",
        g("Nat"),
        Term::Match {
            discriminant: Box::new(v("n")),
            motive: Box::new(lam("_", g("Nat"), g("Nat"))),
            cases: vec![g("Zero"), lam("k", g("Nat"), app(v("countdown"), v("k")))],
        },
    );
    c.add_definition(
        "countdown".into(),
        pi("_", g("Nat"), g("Nat")),
        Term::Fix {
            name: "countdown".into(),
            body: Box::new(body),
        },
    );
    c.add_declaration("opaque_nat", g("Nat"));
    c
}
/// A cycle with no normal form at all. The δ door refuses `a := b; b := a`, so the loop is made
/// of `@[simp]` rewrites, `spin0 x = spin1 x` and `spin1 x = spin0 x` — in a context of its own,
/// because a registered rewrite changes how every other conversion runs.
fn cyclic_context() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    for f in ["spin0", "spin1"] {
        c.add_declaration(f, pi("_", g("Nat"), g("Nat")));
    }
    for (l, r) in [("spin0", "spin1"), ("spin1", "spin0")] {
        let rule = format!("{l}_rot");
        let eq = app(
            app(app(g("Eq"), g("Nat")), app(g(l), v("x"))),
            app(g(r), v("x")),
        );
        c.add_declaration(&rule, pi("x", g("Nat"), eq));
        c.add_simp_lemma(&rule)
            .unwrap_or_else(|e| panic!("{rule}: {e:?}"));
    }
    c
}
fn beyond_budget() -> Term {
    app(g("countdown"), nat(1 << 21))
}
/// The same term under enough β-redexes that a kernel entry runs it on a sized-stack thread.
fn deep(t: Term) -> Term {
    (0..48).fold(t, |t, _| app(lam("y", g("Nat"), v("y")), t))
}

#[test]
fn a_decided_refutation_records_no_stop() {
    let c = context();
    let (equal, stopped) =
        observe_resource_stops(|| defeq_for_test(&c, &g("Zero"), &app(g("Succ"), g("Zero"))));
    assert!(!equal);
    assert_eq!(stopped, None, "Zero ≢ Succ Zero is decided");
    let (equal, stopped) =
        observe_resource_stops(|| defeq_for_test(&c, &app(g("countdown"), nat(5)), &nat(0)));
    assert!(equal);
    assert_eq!(stopped, None);
    let (equal, stopped) =
        observe_resource_stops(|| defeq_for_test(&c, &app(g("countdown"), nat(5)), &nat(1)));
    assert!(!equal);
    assert_eq!(
        stopped, None,
        "a completed evaluation decides the refutation"
    );
}

#[test]
fn a_refusal_after_a_spent_budget_records_the_stop() {
    let c = context();
    let (equal, stopped) =
        observe_resource_stops(|| defeq_for_test(&c, &beyond_budget(), &g("opaque_nat")));
    assert!(!equal, "the kernel could not show them equal");
    assert_eq!(
        stopped,
        Some(Unreduced::BudgetSpent),
        "…and says it did not decide that they differ"
    );
}

#[test]
fn a_stop_on_a_sized_stack_thread_is_carried_back_to_its_caller() {
    let c = context();
    let term = deep(beyond_budget());
    // `is_subtype` sizes its stack for the deeper input and runs on a thread of the kernel's own;
    // the reduction that spends its budget runs there, and its record must come back.
    let (equal, stopped) = observe_resource_stops(|| is_subtype(&c, &term, &g("opaque_nat")));
    assert!(!equal);
    assert_eq!(stopped, Some(Unreduced::BudgetSpent));
}

#[test]
fn a_term_too_deep_to_descend_is_a_stop_not_a_difference() {
    let c = context();
    let too_deep = (0..2000).fold(g("Zero"), |t, _| app(g("Succ"), t));
    let (equal, stopped) =
        observe_resource_stops(|| defeq_for_test(&c, &too_deep, &g("opaque_nat")));
    assert!(!equal);
    assert_eq!(stopped, Some(Unreduced::TooDeep));
}

#[test]
fn a_cycle_is_a_proof_of_no_normal_form_and_never_a_resource_stop() {
    let c = cyclic_context();
    let (result, stopped) =
        observe_resource_stops(|| normalize_checked(&c, &app(g("spin0"), g("Zero"))));
    assert!(matches!(result, Err((_, Unreduced::Cyclic))), "{result:?}");
    assert_eq!(stopped, None);
}

#[test]
fn observations_nest_and_an_inner_stop_reaches_the_enclosing_one() {
    let c = context();
    let (inner, outer) = observe_resource_stops(|| {
        let decided = observe_resource_stops(|| defeq_for_test(&c, &g("Zero"), &g("Zero")));
        let undecided =
            observe_resource_stops(|| defeq_for_test(&c, &beyond_budget(), &g("opaque_nat")));
        (decided, undecided)
    });
    assert_eq!(inner.0, (true, None), "a nested observation starts clean");
    assert_eq!(inner.1, (false, Some(Unreduced::BudgetSpent)));
    assert_eq!(
        outer,
        Some(Unreduced::BudgetSpent),
        "the enclosing observation learns of the inner stop"
    );
    // …and a fresh observation afterwards does not inherit it.
    let (_, after) = observe_resource_stops(|| defeq_for_test(&c, &g("Zero"), &g("Zero")));
    assert_eq!(after, None);
}

#[test]
fn discarded_work_never_reaches_the_enclosing_observation() {
    // A debug differential that re-decides an already decided pair must not turn its verdict into
    // an undecided one by running out of budget itself.
    let c = context();
    let (equal, stopped) = observe_resource_stops(|| {
        let equal = defeq_for_test(&c, &g("Zero"), &app(g("Succ"), g("Zero")));
        discard_resource_stops(|| defeq_for_test(&c, &beyond_budget(), &g("opaque_nat")));
        equal
    });
    assert!(!equal);
    assert_eq!(stopped, None);
    // …and it does not erase a stop the enclosing work had already recorded.
    let (_, stopped) = observe_resource_stops(|| {
        defeq_for_test(&c, &beyond_budget(), &g("opaque_nat"));
        discard_resource_stops(|| defeq_for_test(&c, &g("Zero"), &g("Zero")));
    });
    assert_eq!(stopped, Some(Unreduced::BudgetSpent));
}
