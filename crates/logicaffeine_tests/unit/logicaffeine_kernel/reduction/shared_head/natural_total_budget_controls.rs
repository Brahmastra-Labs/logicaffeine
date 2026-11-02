use super::super::{Transparency, Unreduced};
use super::*;
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn n(x: i64) -> Term {
    Term::Lit(Literal::Nat(crate::BigInt::from_i64(x)))
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn lam(x: &str, b: Term) -> Term {
    Term::Lambda {
        param: x.into(),
        param_type: Box::new(g("Nat")),
        body: Box::new(b),
    }
}
fn ctx() -> Context {
    let mut c = Context::new();
    c.set_auto_auxiliaries(false);
    c.add_inductive("Nat", Term::Sort(crate::Universe::Type(0)));
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
    c.with_equations_suspended(|c| {
        c.declare(
            "keep",
            Term::Pi {
                param: "n".into(),
                param_type: Box::new(g("Nat")),
                body_type: Box::new(g("Nat")),
            },
            lam("n", v("n")),
        )
    })
    .unwrap();
    c
}
fn reset() {
    super::super::TREE_REDUCTION_CALLS.with(|n| n.set(0));
}
fn calls() -> usize {
    super::super::TREE_REDUCTION_CALLS.with(std::cell::Cell::get)
}
#[test]
fn checked_remainder_finishes_in_the_existing_total_without_tree_fallback() {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    let term = app(app(g("Nat_mod"), n(255)), n(256));
    assert_eq!(crate::infer_type(&c, &term).unwrap(), g("Nat"));
    assert_eq!(crate::recheck(&c, &term).unwrap(), g("Nat"));
    let expected = (0..255).fold(g("Zero"), |tail, _| app(g("Succ"), tail));
    let total = super::super::MAX_REDUCTION_STEPS as u64;
    let (result, left) = natural_normal_form(&c, &term, Transparency::All, total);
    assert_eq!(result, NatReadbackOutcome::Normal(expected.clone()));
    assert!(left <= total);
    eprintln!("actual full-budget remainder transitions: {}", total - left);
    reset();
    assert_eq!(crate::normalize_checked(&c, &term).unwrap(), expected);
    assert_eq!(
        calls(),
        0,
        "successful shared normalization must not fall through to copying reduction"
    );
}
#[test]
fn exhausted_shared_budget_permits_zero_further_tree_reducer_calls() {
    let c = ctx();
    let term = app(g("keep"), n(3));
    let (result, left) = natural_normal_form(&c, &term, Transparency::All, 1);
    assert_eq!(result, NatReadbackOutcome::Exhausted);
    assert_eq!(left, 0);
    // Model the last unit of the same total; the first MAX-1 units are already spent.
    let spent = super::super::MAX_REDUCTION_STEPS - 1 + (1 - left as usize);
    reset();
    assert!(matches!(
        super::super::normalize_tree_with_spent(&c, &term, Transparency::All, spent),
        Err((_, Unreduced::BudgetSpent))
    ));
    assert_eq!(calls(), 0);
}
#[test]
fn unsupported_natural_observation_retains_fallback_and_remaining_budget() {
    let mut c = ctx();
    c.add_declaration("opaque", g("Nat"));
    let term = app(g("keep"), g("opaque"));
    let total = super::super::MAX_REDUCTION_STEPS as u64;
    let (result, left) = natural_normal_form(&c, &term, Transparency::All, total);
    assert_eq!(result, NatReadbackOutcome::Unsupported);
    assert!(left > 0 && left < total);
    reset();
    assert_eq!(
        super::super::normalize_tree_with_spent(
            &c,
            &term,
            Transparency::All,
            (total - left) as usize
        )
        .unwrap(),
        g("opaque")
    );
    assert!(calls() > 0);
    assert_eq!(crate::normalize_checked(&c, &term).unwrap(), g("opaque"));
}
#[test]
fn raw_beta_cycle_keeps_cyclic_diagnostic_without_publishing_a_cyclic_definition() {
    let c = ctx();
    // normalize_checked accepts raw syntax; this deliberately untyped input
    // exercises divergence reporting, not declaration or proof acceptance.
    let omega = lam("x", app(v("x"), v("x")));
    let term = app(g("keep"), app(omega.clone(), omega));
    assert!(crate::infer_type(&c, &term).is_err());
    assert!(crate::recheck(&c, &term).is_err());
    assert!(matches!(
        crate::normalize_checked(&c, &term),
        Err((_, Unreduced::Cyclic))
    ));
}
