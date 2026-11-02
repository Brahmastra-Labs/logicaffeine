//! Conversion refutes two closed data values whose weak-head constructors differ WITHOUT the
//! full-normalization fallback — Lean compares constructors after whnf and never normalizes. The
//! fallback used to spend the whole reduction budget twice (theorems folded, then unfolded) on a
//! Bool-valued certified comparison before refusing it (`lean_scale_nat_compare`: about 60 s per
//! false comparison in a release build). Proof irrelevance, literal/constructor bridging and open
//! terms keep their exact answers.
use super::*;
use crate::{BigInt, Literal, Universe};

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn lam(n: &str, t: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(t),
        body: Box::new(b),
    }
}
fn pi(n: &str, t: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(t),
        body_type: Box::new(b),
    }
}
fn lit(n: u64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_u64(n)))
}
fn on_nat(d: Term, zero: Term, succ: Term) -> Term {
    Term::Match {
        discriminant: Box::new(d),
        motive: Box::new(lam("_", g("Nat"), g("Bool"))),
        cases: vec![zero, succ],
    }
}

/// Lean's two-sided `Nat.ble`: `| 0, _ => true | _+1, 0 => false | n+1, m+1 => ble n m`.
fn environment() -> Context {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    let body = Term::Fix {
        name: "ble.rec".into(),
        body: Box::new(lam(
            "n",
            g("Nat"),
            lam(
                "m",
                g("Nat"),
                on_nat(
                    v("n"),
                    on_nat(v("m"), g("true"), lam("m1", g("Nat"), g("true"))),
                    lam(
                        "n1",
                        g("Nat"),
                        on_nat(
                            v("m"),
                            g("false"),
                            lam("m1", g("Nat"), app(app(v("ble.rec"), v("n1")), v("m1"))),
                        ),
                    ),
                ),
            ),
        )),
    };
    c.declare(
        "lean_ble",
        pi("n", g("Nat"), pi("m", g("Nat"), g("Bool"))),
        body,
    )
    .unwrap();
    c
}

fn full_normalizations(c: &Context, a: &Term, b: &Term) -> (bool, usize) {
    CONVERSION_FULL_NORMALIZATIONS.with(|count| count.set(0));
    let equal = def_eq(c, a, b);
    (
        equal,
        CONVERSION_FULL_NORMALIZATIONS.with(|count| count.get()),
    )
}

#[test]
fn a_false_closed_comparison_is_refuted_without_full_normalization() {
    let c = environment();
    for (a, b, answer) in [
        (30u64, 40u64, "true"),
        (40, 30, "false"),
        (1_114_112, 4_294_967_296, "true"),
    ] {
        let call = app(app(g("lean_ble"), lit(a)), lit(b));
        let wrong = if answer == "true" { "false" } else { "true" };
        // The first use certifies `lean_ble`'s equations (once per context, by conversion); the
        // comparison measured below is the refusal alone.
        assert!(def_eq(&c, &call, &g(answer)), "ble {a} {b} is {answer}");
        let (equal, normalizations) = full_normalizations(&c, &call, &g(wrong));
        assert!(!equal, "ble {a} {b} is not {wrong}");
        assert_eq!(
            normalizations, 0,
            "ble {a} {b} ≡ {wrong} is refuted by its weak-head constructor"
        );
    }
}

#[test]
fn distinct_naturals_are_refuted_by_value_and_equal_spellings_are_not() {
    let c = environment();
    let succ = |t: Term| app(g("Succ"), t);
    assert_eq!(full_normalizations(&c, &lit(3), &succ(lit(3))), (false, 0));
    assert!(def_eq(&c, &lit(3), &succ(lit(2))), "3 is Succ 2");
    assert!(def_eq(&c, &lit(0), &g("Zero")), "0 is Zero");
    assert!(
        def_eq(&c, &succ(succ(g("Zero"))), &lit(2)),
        "Succ (Succ Zero) is 2"
    );
}

#[test]
fn two_proofs_of_one_proposition_stay_equal() {
    let mut c = environment();
    c.add_inductive("Either", Term::Sort(Universe::Prop));
    c.add_constructor("Either.left", "Either", g("Either"));
    c.add_constructor("Either.right", "Either", g("Either"));
    assert!(
        def_eq(&c, &g("Either.left"), &g("Either.right")),
        "proof irrelevance equates the two constructors"
    );
}

#[test]
fn an_open_comparison_is_not_refuted_by_observation() {
    let c = environment().extend("x", g("Nat"));
    let call = app(app(g("lean_ble"), v("x")), lit(5));
    assert!(
        !crate::reduction::constructors_disagree(&c, &call, &g("false"), 1 << 20),
        "observation never refutes an open term"
    );
    // The lazy loop's R4 does: `lean_ble x 5` is stuck for good on the local `x`, and `false` is a
    // rigid constructor.
    assert_eq!(
        lazy_delta::lazy_delta_conversion(&c, &call, &g("false")),
        Some(false)
    );
    let (equal, normalizations) = full_normalizations(&c, &call, &g("false"));
    assert!(!equal);
    assert_eq!(
        normalizations, 0,
        "refuted by the stuck-term rule, not by normalizing both sides"
    );
    // The closed comparison is the evaluator's: `lean_ble 7 5` is observed to be `false`.
    let closed = app(app(g("lean_ble"), lit(7)), lit(5));
    assert!(crate::reduction::constructor_equivalent(
        &c,
        &closed,
        &g("false"),
        1 << 20
    ));
    assert!(crate::reduction::constructors_disagree(
        &c,
        &closed,
        &g("true"),
        1 << 20
    ));
    assert!(def_eq(&c, &closed, &g("false")) && !def_eq(&c, &closed, &g("true")));
}
