//! Lean's exact conversion refuses a pair as an INVALID PROOF only when the conversion it ran
//! decided that the sides differ; when a reduction the conversion relied on stopped for want of a
//! resource, the refusal is a RESOURCE LIMIT — the first time and every time the pair is asked.
use super::*;
use crate::prelude::StandardLibrary;
use crate::term_store::{KernelCall, TermStore};
use crate::Literal;

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn nat(n: u64) -> Term {
    Term::Lit(Literal::Nat(crate::BigInt::from_u64(n)))
}

/// `countdown n = Zero`, one structural step per unit; at `2²¹` its normal form lies beyond the
/// reduction budget.
fn context() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    let body = Term::Lambda {
        param: "n".into(),
        param_type: Box::new(g("Nat")),
        body: Box::new(Term::Match {
            discriminant: Box::new(v("n")),
            motive: Box::new(Term::Lambda {
                param: "_".into(),
                param_type: Box::new(g("Nat")),
                body: Box::new(g("Nat")),
            }),
            cases: vec![
                g("Zero"),
                Term::Lambda {
                    param: "k".into(),
                    param_type: Box::new(g("Nat")),
                    body: Box::new(app(v("countdown"), v("k"))),
                },
            ],
        }),
    };
    let ty = Term::Pi {
        param: "_".into(),
        param_type: Box::new(g("Nat")),
        body_type: Box::new(g("Nat")),
    };
    c.add_definition(
        "countdown".into(),
        ty,
        Term::Fix {
            name: "countdown".into(),
            body: Box::new(body),
        },
    );
    c.add_declaration("opaque_nat", g("Nat"));
    c
}
fn beyond_budget() -> Term {
    app(g("countdown"), nat(1 << 21))
}

fn declined_as_undecided(verdict: &Verdict) {
    assert!(matches!(verdict, Verdict::Decline(_)), "{verdict:?}");
    assert_eq!(
        verdict.reason().code(),
        DiagnosticCode::ResourceLimit,
        "{verdict:?}"
    );
    assert!(
        verdict.reason().message().contains("undecided"),
        "{verdict:?}"
    );
}
fn rejected_as_invalid(verdict: &Verdict) {
    assert!(matches!(verdict, Verdict::Reject(_)), "{verdict:?}");
    assert_eq!(
        verdict.reason().code(),
        DiagnosticCode::InvalidProof,
        "{verdict:?}"
    );
}

#[test]
fn a_comparison_left_undecided_by_a_spent_budget_is_a_resource_limit() {
    let c = context();
    declined_as_undecided(&exact(&c, &beyond_budget(), &g("opaque_nat")).unwrap_err());
}

#[test]
fn a_comparison_decided_at_a_constructor_is_rejected_whatever_its_other_arguments_cost() {
    // `Succ (countdown 2²¹)` against `Zero`: the heads refute each other at once. The side that
    // cannot be normalized within the budget was never needed, so this is a decision.
    let c = context();
    rejected_as_invalid(&exact(&c, &app(g("Succ"), beyond_budget()), &g("Zero")).unwrap_err());
    rejected_as_invalid(&exact(&c, &app(g("countdown"), nat(5)), &nat(1)).unwrap_err());
    exact(&c, &app(g("countdown"), nat(5)), &nat(0))
        .expect("a completed evaluation proves equality");
}

#[test]
fn an_undecided_comparison_is_never_replayed_from_the_cache_as_a_rejection() {
    let c = context();
    let call = KernelCall::begin();
    let mut store = TermStore::new();
    let mut equal = HashMap::new();
    let env = store.env(&[]);
    for attempt in 0..2 {
        let verdict = lean_exact_cached(
            &c,
            &beyond_budget(),
            &g("opaque_nat"),
            &mut store,
            &mut equal,
            &call,
            env,
        )
        .expect_err("the kernel cannot show these equal");
        assert!(
            matches!(verdict, Verdict::Decline(_)),
            "attempt {attempt}: {verdict:?}"
        );
        declined_as_undecided(&verdict);
    }
    // A decided refutation is cached, and replayed as the same rejection.
    for _ in 0..2 {
        rejected_as_invalid(
            &lean_exact_cached(
                &c,
                &g("Zero"),
                &app(g("Succ"), g("Zero")),
                &mut store,
                &mut equal,
                &call,
                env,
            )
            .expect_err("Zero is not Succ Zero"),
        );
    }
}
