use super::*;
use crate::prelude::StandardLibrary;
use crate::{Reducibility, Universe};

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn lam(n: &str, a: Term, b: Term) -> Term {
    Term::Lambda {
        param: n.into(),
        param_type: Box::new(a),
        body: Box::new(b),
    }
}
fn sort() -> Term {
    Term::Sort(Universe::Type(0))
}

// A checked dependent telescope whose second domain computes to Nat. Small
// beta/delta work gives a deterministic baseline failure, not an OOM test.
fn setup() -> (Context, Term, Term) {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c.declare(
        "PiExposure.Family",
        pi("n", g("Nat"), sort()),
        lam("n", g("Nat"), g("Nat")),
    )
    .unwrap();
    let family = app(g("PiExposure.Family"), v("n"));
    let ty = pi("n", g("Nat"), pi("x", family.clone(), g("Nat")));
    c.declare(
        "PiExposure.keep",
        ty.clone(),
        lam("n", g("Nat"), lam("x", family, v("n"))),
    )
    .unwrap();
    assert!(c.pending_obligations().is_empty());
    let residual = pi("x", app(g("PiExposure.Family"), g("Zero")), g("Nat"));
    (c, ty, residual)
}

#[test]
fn visible_dependent_pi_keeps_unneeded_computational_domain_structured() {
    let (c, ty, residual) = setup();
    let (term, actual) = elaborate_app_against(
        &c,
        &mut MetaCtx::new(),
        &g("PiExposure.keep"),
        &ty,
        &[ParamKind::Explicit],
        &[g("Zero")],
        None,
    )
    .unwrap();
    assert_eq!(term, app(g("PiExposure.keep"), g("Zero")));
    assert_eq!(
        actual, residual,
        "Pi exposure must not normalize the unused dependent suffix"
    );
    let main = crate::infer_type(&c, &term).unwrap();
    let independent = crate::recheck(&c, &term).unwrap();
    assert!(crate::is_subtype(&c, &main, &actual));
    assert!(crate::is_subtype(&c, &independent, &actual));
}

#[test]
fn explicit_arguments_still_undergo_conversion_and_independent_checking() {
    let (c, ty, _) = setup();
    let (term, actual) = elaborate_app_against(
        &c,
        &mut MetaCtx::new(),
        &g("PiExposure.keep"),
        &ty,
        &[ParamKind::Explicit; 2],
        &[g("Zero"), g("Zero")],
        None,
    )
    .unwrap();
    assert_eq!(term, app(app(g("PiExposure.keep"), g("Zero")), g("Zero")));
    assert_eq!(actual, g("Nat"));
    assert_eq!(crate::infer_type(&c, &term).unwrap(), g("Nat"));
    assert_eq!(crate::recheck(&c, &term).unwrap(), g("Nat"));
}

#[test]
fn mismatched_explicit_argument_is_still_refused() {
    let (c, ty, _) = setup();
    let result = elaborate_app_against(
        &c,
        &mut MetaCtx::new(),
        &g("PiExposure.keep"),
        &ty,
        &[ParamKind::Explicit; 2],
        &[g("Zero"), g("Nat")],
        None,
    );
    assert!(result.is_err());
    let wrong = app(app(g("PiExposure.keep"), g("Zero")), g("Nat"));
    assert!(crate::infer_type(&c, &wrong).is_err());
    assert!(crate::recheck(&c, &wrong).is_err());
}

#[test]
fn reducible_alias_exposes_pi_and_irreducible_alias_stays_hidden() {
    let (mut c, ty, _) = setup();
    c.declare("PiExposure.Alias", sort(), ty).unwrap();
    let call = |c: &Context| {
        elaborate_app_against(
            c,
            &mut MetaCtx::new(),
            &g("PiExposure.keep"),
            &g("PiExposure.Alias"),
            &[ParamKind::Explicit],
            &[g("Zero")],
            None,
        )
    };
    let (term, actual) = call(&c).unwrap();
    assert_eq!(term, app(g("PiExposure.keep"), g("Zero")));
    assert!(crate::is_subtype(&c, &actual, &pi("x", g("Nat"), g("Nat"))));
    c.set_reducibility("PiExposure.Alias", Reducibility::Irreducible);
    assert!(matches!(call(&c), Err(KernelError::NotAFunction(_))));
}

#[test]
fn solved_meta_is_instantiated_before_visible_pi_exposure() {
    let (c, ty, residual) = setup();
    let mut m = MetaCtx::new();
    let hole = m.fresh();
    let Term::Var(name) = &hole else {
        unreachable!()
    };
    m.solve(name, ty);
    let (term, actual) = elaborate_spine(
        &c,
        &mut m,
        &g("PiExposure.keep"),
        &hole,
        &[Fill::Given(g("Zero"))],
        None,
    )
    .unwrap();
    assert_eq!(term, app(g("PiExposure.keep"), g("Zero")));
    assert_eq!(actual, residual);
    let mut unsolved = MetaCtx::new();
    let hole = unsolved.fresh();
    assert!(matches!(
        elaborate_spine(
            &c,
            &mut unsolved,
            &g("PiExposure.keep"),
            &hole,
            &[Fill::Given(g("Zero"))],
            None
        ),
        Err(KernelError::NotAFunction(_))
    ));
}

#[test]
fn active_simp_context_retains_full_normalization_fallback() {
    let (mut c, ty, _) = setup();
    c.declare(
        "PiExposure.id",
        pi("n", g("Nat"), g("Nat")),
        lam("n", g("Nat"), v("n")),
    )
    .unwrap();
    let eq = app(
        app(app(g("Eq"), g("Nat")), app(g("PiExposure.id"), v("n"))),
        v("n"),
    );
    c.declare(
        "PiExposure.id.rule",
        pi("n", g("Nat"), eq),
        lam("n", g("Nat"), app(app(g("refl"), g("Nat")), v("n"))),
    )
    .unwrap();
    c.add_simp_lemma("PiExposure.id.rule").unwrap();
    assert!(!c.simp_rules().is_empty());
    let (_, actual) = elaborate_spine(
        &c,
        &mut MetaCtx::new(),
        &g("PiExposure.keep"),
        &ty,
        &[Fill::Given(g("Zero"))],
        None,
    )
    .unwrap();
    // With active rewrites the old full elaborator normalizer is deliberately kept.
    assert_eq!(actual, pi("x", g("Nat"), g("Nat")));
}
