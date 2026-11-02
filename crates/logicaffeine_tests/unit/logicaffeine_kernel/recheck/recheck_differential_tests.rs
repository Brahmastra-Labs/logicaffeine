//! The de Bruijn criterion as a lib-unit battery: the independent re-checker must AGREE with
//! the main kernel on well-typed terms and REJECT ill-typed ones exactly as the main kernel
//! does. A mutation to the de Bruijn machinery (to_db/shift/subst/whnf/def_eq/infer) that
//! changes a verdict surfaces here as a `Disagree`. Match/Fix paths are covered by the
//! integration suite (tests/recheck.rs); this locks the Sort/Var/Global/Pi/Lambda/App/Lit core.
use super::*;
use crate::prelude::StandardLibrary;
use crate::term::{Literal, Term, Universe};

fn ctx() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c
}
fn sort(n: u32) -> Term {
    Term::Sort(Universe::Type(n))
}
fn g(s: &str) -> Term {
    Term::Global(s.to_string())
}
fn v(s: &str) -> Term {
    Term::Var(s.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn nat(n: u64) -> Term {
    Term::Lit(Literal::Nat(logicaffeine_base::BigInt::from_u64(n)))
}
fn lam(p: &str, ty: Term, b: Term) -> Term {
    Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(ty),
        body: Box::new(b),
    }
}
fn pi(p: &str, ty: Term, b: Term) -> Term {
    Term::Pi {
        param: p.to_string(),
        param_type: Box::new(ty),
        body_type: Box::new(b),
    }
}
fn arrow(a: Term, b: Term) -> Term {
    pi("_", a, b)
}

#[test]
fn the_two_kernels_never_disagree_on_the_well_typed_battery() {
    let c = ctx();
    let terms = vec![
        sort(0),
        g("Nat"),
        nat(0),
        nat(7),
        g("Succ"),
        lam("x", g("Nat"), v("x")),
        pi("x", g("Nat"), g("Nat")),
        app(g("Succ"), nat(2)),
        app(lam("x", g("Nat"), v("x")), nat(3)),
        lam("A", sort(0), lam("x", v("A"), v("x"))), // polymorphic identity
        lam(
            "f",
            arrow(g("Nat"), g("Nat")),
            lam("x", g("Nat"), app(v("f"), app(v("f"), v("x")))),
        ), // twice
        app(
            app(lam("A", sort(0), lam("x", v("A"), v("x"))), g("Nat")),
            nat(9),
        ), // id Nat 9
    ];
    for t in &terms {
        let dc = double_check(&c, t);
        assert!(
            !matches!(dc, DoubleCheck::Disagree(_)),
            "the independent kernels DISAGREED on a well-typed term {t:?}: {dc:?}",
        );
        assert!(
            recheck(&c, t).is_ok(),
            "the re-checker rejected the well-typed term {t:?}"
        );
    }
}

#[test]
fn the_re_checker_rejects_ill_typed_terms_like_the_main_kernel() {
    let c = ctx();
    let ill = vec![
        app(g("Nat"), nat(1)),                    // Nat is not a function
        app(lam("x", g("Nat"), v("x")), sort(0)), // argument Type0 ≠ Nat
        app(g("Succ"), sort(0)),                  // Succ expects a Nat
        lam("x", g("Nat"), app(v("x"), v("x"))),  // x:Nat applied to itself
    ];
    for t in &ill {
        let dc = double_check(&c, t);
        assert!(
            !matches!(dc, DoubleCheck::Disagree(_)),
            "kernels disagreed on an ill-typed term {t:?}: {dc:?} (one wrongly accepted it)",
        );
        assert!(
            recheck(&c, t).is_err(),
            "the re-checker wrongly ACCEPTED ill-typed {t:?}"
        );
    }
}

#[test]
fn double_check_reaches_full_agreement_on_canonical_terms() {
    let c = ctx();
    // Not vacuous: must be full Agreement (both accept + def-eq types), not "incomplete".
    assert_eq!(
        double_check(&c, &lam("x", g("Nat"), v("x"))),
        DoubleCheck::Agreed
    );
    assert_eq!(double_check(&c, &nat(42)), DoubleCheck::Agreed);
}
