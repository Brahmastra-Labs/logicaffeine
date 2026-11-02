use super::*;
use crate::context::Context;
use crate::prelude::StandardLibrary;
use crate::term::Term;

fn g(s: &str) -> Term {
    Term::Global(s.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn lam(p: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}
fn or_tt() -> Term {
    app(app(g("Or"), g("True")), g("True"))
}

/// CRITIQUE #1 (open half): CIC large-elimination restriction. Matching a proof of `Or` (a Prop with
/// TWO constructors) into `Type` (here returning `Nat`) extracts computational content from a proof
/// and breaks consistency — the kernel MUST reject it.
#[test]
fn large_elimination_of_or_into_type_is_rejected() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    let case = lam("_", g("True"), g("Zero")); // λ_:True. Zero  (Zero : Nat)
    let m = Term::Match {
        discriminant: Box::new(Term::Var("h".to_string())),
        motive: Box::new(lam("_", or_tt(), g("Nat"))), // λ_:Or True True. Nat  (large)
        cases: vec![case.clone(), case],
    };
    let term = lam("h", or_tt(), m);
    assert!(
        infer_type(&ctx, &term).is_err(),
        "large elimination of Or (2 constructors) into Type must be rejected for consistency"
    );
}

/// Regression: `ex falso` — large elimination of `False` (ZERO constructors, a subsingleton) into any
/// type — MUST stay legal, or every proof-by-contradiction breaks.
#[test]
fn ex_falso_large_elimination_of_false_still_allowed() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    let m = Term::Match {
        discriminant: Box::new(Term::Var("h".to_string())),
        motive: Box::new(lam("_", g("False"), g("Nat"))), // λ_:False. Nat (large, but False is empty)
        cases: vec![],
    };
    let term = lam("h", g("False"), m);
    assert!(
        infer_type(&ctx, &term).is_ok(),
        "ex falso (large elim of empty False) must stay legal"
    );
}

/// Regression: SMALL elimination of `Or` (into a `Prop`) is always fine — the restriction must not
/// over-reach and reject ordinary propositional case analysis.
#[test]
fn small_elimination_of_or_into_prop_still_allowed() {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    let case = lam("_", g("True"), g("I")); // λ_:True. I  (I : True)
    let m = Term::Match {
        discriminant: Box::new(Term::Var("h".to_string())),
        motive: Box::new(lam("_", or_tt(), g("True"))), // λ_:Or True True. True  (small, Prop)
        cases: vec![case.clone(), case],
    };
    let term = lam("h", or_tt(), m);
    assert!(
        infer_type(&ctx, &term).is_ok(),
        "small elimination of Or into Prop must stay legal"
    );
}
