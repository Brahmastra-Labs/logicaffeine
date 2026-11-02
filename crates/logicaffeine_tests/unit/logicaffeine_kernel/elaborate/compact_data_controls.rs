use super::*;
use crate::prelude::StandardLibrary;
use crate::{Literal, Reducibility};
fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn app(f: Term, a: Term) -> Term {
    Term::App(Box::new(f), Box::new(a))
}
fn apps(f: Term, args: Vec<Term>) -> Term {
    args.into_iter().fold(f, app)
}
fn nat(n: u64) -> Term {
    Term::Lit(Literal::Nat(crate::BigInt::from_u64(n)))
}
fn pi(n: &str, a: Term, b: Term) -> Term {
    Term::Pi {
        param: n.into(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn eq(ty: Term, a: Term, b: Term) -> Term {
    apps(g("Eq"), vec![ty, a, b])
}
fn context() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    // Checked aliases deliberately use unrelated names. Recurrence evidence,
    // never the spelling of an imported operation, authorizes computation.
    c.declare(
        "DataGate.power",
        pi("a", g("Nat"), pi("b", g("Nat"), g("Nat"))),
        g("Nat_pow"),
    )
    .unwrap();
    c.declare(
        "DataGate.order",
        pi("a", g("Nat"), pi("b", g("Nat"), g("Bool"))),
        g("natLt"),
    )
    .unwrap();
    assert!(c.pending_obligations().is_empty());
    c
}
fn decision(exponent: u64) -> Term {
    let power = apps(g("DataGate.power"), vec![nat(2), nat(exponent)]);
    let less = apps(g("DataGate.order"), vec![nat(0), power]);
    let p = eq(g("Bool"), less.clone(), g("true"));
    apps(
        g("decide"),
        vec![p, apps(g("decEqBool"), vec![less, g("true")])],
    )
}
fn reset() {
    LAZY_UNIFY_FULL_NORMALIZATIONS.with(|n| n.set(0));
}
fn count() -> usize {
    LAZY_UNIFY_FULL_NORMALIZATIONS.with(|n| n.get())
}
#[test]
fn dependent_decision_conversion_avoids_full_normalizing_the_common_power() {
    let c = context();
    let d = decision(8);
    assert_eq!(crate::infer_type(&c, &d).unwrap(), g("Bool"));
    assert_eq!(crate::recheck(&c, &d).unwrap(), g("Bool"));
    let left = eq(g("Bool"), d.clone(), d.clone());
    let right = eq(g("Bool"), d, g("true"));
    reset();
    assert!(unify(&c, &mut MetaCtx::new(), &left, &right));
    assert_eq!(
        count(),
        0,
        "checked compact evaluation must settle the demanded Boolean without full normalization"
    );
}
#[test]
fn computed_false_cannot_license_a_true_claim() {
    let c = context();
    let p = eq(g("Nat"), nat(0), nat(1));
    let d = apps(
        g("decide"),
        vec![p, apps(g("decEqNat"), vec![nat(0), nat(1)])],
    );
    assert_eq!(crate::infer_type(&c, &d).unwrap(), g("Bool"));
    assert!(!unify(&c, &mut MetaCtx::new(), &d, &g("true")));
    assert!(unify(&c, &mut MetaCtx::new(), &d, &g("false")));
}
#[test]
fn irreducible_checked_wrapper_still_blocks_elaborator_conversion() {
    let mut c = context();
    c.declare("DataGate.secret", g("Bool"), g("true")).unwrap();
    c.set_reducibility("DataGate.secret", Reducibility::Irreducible);
    assert!(!unify(
        &c,
        &mut MetaCtx::new(),
        &g("DataGate.secret"),
        &g("true")
    ));
}
#[test]
fn unrelated_irreducibility_keeps_the_existing_fallback_available() {
    let mut c = context();
    c.set_reducibility("unrelated.attribute", Reducibility::Irreducible);
    reset();
    assert!(unify(&c, &mut MetaCtx::new(), &decision(3), &g("true")));
    assert!(count() > 0);
}
#[test]
fn open_names_and_pending_holes_are_not_computational_evidence() {
    let c = context();
    assert!(!unify(
        &c,
        &mut MetaCtx::new(),
        &Term::Var("free".into()),
        &g("true")
    ));
    let mut m = MetaCtx::new();
    let pending = m.fresh();
    assert!(unify(&c, &mut m, &pending, &g("true")));
    let Term::Var(n) = pending else {
        unreachable!()
    };
    assert_eq!(m.solution(&n), Some(&g("true")));
}
#[test]
fn active_simp_rules_retain_the_old_computational_path() {
    let mut c = context();
    let v = Term::Var("x".into());
    let lam = |b: Term| Term::Lambda {
        param: "x".into(),
        param_type: Box::new(g("Nat")),
        body: Box::new(b),
    };
    c.declare("DataGate.id", pi("x", g("Nat"), g("Nat")), lam(v.clone()))
        .unwrap();
    let ty = pi(
        "x",
        g("Nat"),
        eq(g("Nat"), app(g("DataGate.id"), v.clone()), v.clone()),
    );
    c.declare("DataGate.rule", ty, lam(app(app(g("refl"), g("Nat")), v)))
        .unwrap();
    c.add_simp_lemma("DataGate.rule").unwrap();
    reset();
    assert!(unify(&c, &mut MetaCtx::new(), &decision(3), &g("true")));
    assert!(count() > 0);
}
