use super::*;
use crate::prelude::StandardLibrary;
use crate::{Literal, Reducibility};
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
fn power(n: u64) -> Term {
    app(app(g("Nat_pow"), nat(2)), nat(n))
}
fn pair(n: u64) -> (Context, Term, Term) {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c.declare("ScopedGate.bound", g("Nat"), power(n)).unwrap();
    assert!(c.pending_obligations().is_empty());
    c.add("n", g("Nat"));
    let prefix = app(g("natLt"), v("n"));
    (
        c,
        app(prefix.clone(), g("ScopedGate.bound")),
        app(prefix, power(n)),
    )
}
fn reset() {
    LAZY_UNIFY_FULL_NORMALIZATIONS.with(|n| n.set(0));
}
fn count() -> usize {
    LAZY_UNIFY_FULL_NORMALIZATIONS.with(|n| n.get())
}
#[test]
fn actual_context_local_and_closed_alias_avoid_full_normalization() {
    let (c, a, b) = pair(8);
    assert_eq!(crate::infer_type(&c, &a).unwrap(), g("Bool"));
    assert_eq!(crate::recheck(&c, &b).unwrap(), g("Bool"));
    reset();
    assert!(unify_in(&c, &mut MetaCtx::new(), &[], &a, &b));
    assert_eq!(
        count(),
        0,
        "Context-bound open prefix must preserve the shared closed power"
    );
}
#[test]
fn reverse_alias_orientation_preserves_the_same_closed_power() {
    let (c, a, b) = pair(8);
    reset();
    assert!(unify(&c, &mut MetaCtx::new(), &b, &a));
    assert_eq!(count(), 0);
}
#[test]
fn distinct_context_locals_do_not_become_equal() {
    let (mut c, _, _) = pair(3);
    c.add("m", g("Nat"));
    assert!(!unify(&c, &mut MetaCtx::new(), &v("n"), &v("m")));
}
#[test]
fn irreducible_alias_stays_opaque_under_an_open_prefix() {
    let (mut c, a, b) = pair(3);
    c.set_reducibility("ScopedGate.bound", Reducibility::Irreducible);
    assert!(!unify(&c, &mut MetaCtx::new(), &a, &b));
}
#[test]
fn pending_meta_assignment_still_uses_the_existing_unifier() {
    let (c, _, _) = pair(3);
    let mut m = MetaCtx::new();
    let pending = m.fresh();
    assert!(unify(&c, &mut m, &pending, &v("n")));
    let Term::Var(name) = pending else {
        unreachable!()
    };
    assert_eq!(m.solution(&name), Some(&v("n")));
}
