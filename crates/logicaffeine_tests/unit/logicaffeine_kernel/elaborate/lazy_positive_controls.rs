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
fn apps(f: Term, args: &[Term]) -> Term {
    args.iter().cloned().fold(f, app)
}
fn context() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c
}
fn reset() {
    LAZY_UNIFY_FULL_NORMALIZATIONS.with(|n| n.set(0));
}
fn count() -> usize {
    LAZY_UNIFY_FULL_NORMALIZATIONS.with(|n| n.get())
}

// Models exactly the observed mismatch in wrappers while leaving a shared
// computation intact. Fixture declarations are checked; no new trusted rule.
fn pair() -> (Context, Term, Term) {
    let mut c = context();
    c.add_declaration(
        "Congruence.Order",
        pi("m", g("Nat"), pi("n", g("Nat"), Term::Sort(Universe::Prop))),
    );
    c.add_declaration(
        "Congruence.Decidable",
        pi(
            "p",
            Term::Sort(Universe::Prop),
            Term::Sort(Universe::Type(0)),
        ),
    );
    c.declare(
        "Congruence.lt",
        pi("m", g("Nat"), pi("n", g("Nat"), Term::Sort(Universe::Prop))),
        lam(
            "m",
            g("Nat"),
            lam(
                "n",
                g("Nat"),
                apps(g("Congruence.Order"), &[app(g("Succ"), v("m")), v("n")]),
            ),
        ),
    )
    .unwrap();
    c.declare(
        "Congruence.identity",
        pi("m", g("Nat"), g("Nat")),
        lam("m", g("Nat"), v("m")),
    )
    .unwrap();
    c.declare(
        "Congruence.shared",
        pi("m", g("Nat"), g("Nat")),
        lam("m", g("Nat"), app(g("Succ"), v("m"))),
    )
    .unwrap();
    let common = app(g("Congruence.shared"), g("Zero"));
    let left = app(
        g("Congruence.Decidable"),
        apps(
            g("Congruence.Order"),
            &[app(g("Succ"), g("Zero")), common.clone()],
        ),
    );
    let right = app(
        g("Congruence.Decidable"),
        apps(
            g("Congruence.lt"),
            &[app(g("Congruence.identity"), g("Zero")), common],
        ),
    );
    assert!(crate::infer_type(&c, &left).is_ok());
    assert!(crate::recheck(&c, &right).is_ok());
    (c, left, right)
}
#[test]
fn closed_wrapper_conversion_does_not_fully_normalize_shared_argument() {
    let (c, a, b) = pair();
    reset();
    assert!(unify(&c, &mut MetaCtx::new(), &a, &b));
    assert_eq!(
        count(),
        0,
        "the common expensive argument must not enter full normalization"
    );
}
#[test]
fn positive_beta_conversion_also_keeps_the_old_fallback_cold() {
    let c = context();
    let a = app(lam("n", g("Nat"), v("n")), g("Zero"));
    reset();
    assert!(unify(&c, &mut MetaCtx::new(), &a, &g("Zero")));
    assert_eq!(count(), 0);
}
#[test]
fn irreducible_wrapper_is_not_unfolded_by_positive_probe() {
    let (mut c, a, b) = pair();
    c.set_reducibility("Congruence.lt", Reducibility::Irreducible);
    reset();
    assert!(!unify(&c, &mut MetaCtx::new(), &a, &b));
    assert!(count() > 0);
}
#[test]
fn nested_irreducible_dependency_remains_folded() {
    let mut c = context();
    c.declare("Congruence.inner", g("Nat"), g("Zero")).unwrap();
    c.declare("Congruence.outer", g("Nat"), g("Congruence.inner"))
        .unwrap();
    c.set_reducibility("Congruence.inner", Reducibility::Irreducible);
    reset();
    assert!(!unify(
        &c,
        &mut MetaCtx::new(),
        &g("Congruence.outer"),
        &g("Zero")
    ));
    assert!(count() > 0);
}
#[test]
fn distinct_shared_arguments_are_not_mistaken_for_common_evidence() {
    let (c, a, mut b) = pair();
    let Term::App(_, proposition) = &mut b else {
        unreachable!()
    };
    let Term::App(_, last) = proposition.as_mut() else {
        unreachable!()
    };
    *last = Box::new(app(g("Congruence.shared"), app(g("Succ"), g("Zero"))));
    reset();
    assert!(!unify(&c, &mut MetaCtx::new(), &a, &b));
    assert!(count() > 0);
}
#[test]
fn active_simp_rules_keep_the_existing_normalizer_path() {
    let (mut c, a, b) = pair();
    let lhs = app(g("Congruence.identity"), v("n"));
    let eq = apps(g("Eq"), &[g("Nat"), lhs, v("n")]);
    c.declare(
        "Congruence.identity.rule",
        pi("n", g("Nat"), eq),
        lam("n", g("Nat"), apps(g("refl"), &[g("Nat"), v("n")])),
    )
    .unwrap();
    c.add_simp_lemma("Congruence.identity.rule").unwrap();
    reset();
    assert!(unify(&c, &mut MetaCtx::new(), &a, &b));
    assert!(count() > 0);
}
#[test]
fn metavariable_assignment_is_left_to_original_unifier() {
    let c = context();
    let mut m = MetaCtx::new();
    let hole = m.fresh();
    reset();
    assert!(unify(&c, &mut m, &hole, &g("Nat")));
    assert!(count() > 0);
    let Term::Var(n) = hole else { unreachable!() };
    assert_eq!(m.solution(&n), Some(&g("Nat")));
}
#[test]
fn binder_capture_and_open_names_never_gain_positive_evidence() {
    let c = context();
    let a = lam("x", g("Nat"), v("x"));
    let b = lam("y", g("Nat"), g("Zero"));
    assert!(!unify(&c, &mut MetaCtx::new(), &a, &b));
    assert!(!unify(&c, &mut MetaCtx::new(), &v("unbound"), &g("Zero")));
}
