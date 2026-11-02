//! A stuck single-case match is K-probed once per normalization, not once per step.
//!
//! Full normalization re-walks the whole term on every step. A single-case match whose
//! discriminant is a neutral proof stays stuck while other redexes are contracted, and the K
//! probe (infer the proof's type, compare it with the constructor's) used to be re-run on every
//! step. Init's `WellFounded.Nat.fix_eq` spent ~96% of its 463 s inside these repeated probes.
//! The probe is a pure function of the probed syntax and the local context it is probed in, so
//! its answer is remembered for the rest of that normalization; nothing else is shared.
use super::*;

fn g(n: &str) -> Term {
    Term::Global(n.into())
}
fn v(n: &str) -> Term {
    Term::Var(n.into())
}
fn ap(f: Term, a: Term) -> Term {
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
fn eq(a: Term, b: Term) -> Term {
    ap(ap(ap(g("Eq"), g("Nat")), a), b)
}
fn context() -> Context {
    let mut c = Context::new();
    crate::prelude::StandardLibrary::register(&mut c);
    c.add_declaration("x", g("Nat"));
    c.add_declaration("y", g("Nat"));
    c.add_declaration("pair", pi("_", g("Nat"), pi("_", g("Nat"), g("Nat"))));
    c.add_definition("unfold0".into(), g("Nat"), g("Zero"));
    for i in 1..=40 {
        c.add_definition(
            format!("unfold{i}"),
            g("Nat"),
            g(&format!("unfold{}", i - 1)),
        );
    }
    assert!(
        c.k_constructor("Eq").is_some(),
        "Eq is a K-like family in the prelude"
    );
    c
}
/// `match h return (λ _ : Eq Nat x y. Nat) with [Zero]` — the motive is fixed syntax.
fn stuck(h: Term) -> Term {
    Term::Match {
        discriminant: Box::new(h),
        motive: Box::new(lam("_", eq(g("x"), g("y")), g("Nat"))),
        cases: vec![g("Zero")],
    }
}
/// `n` successive delta steps (no beta/zeta shortcut applies): `n` normalization steps with
/// the stuck match left in place.
fn redexes(n: usize) -> Term {
    g(&format!("unfold{n}"))
}
fn probes<R>(f: impl FnOnce() -> R) -> (R, usize) {
    K_PROBES.with(|c| c.set(0));
    let r = f();
    (r, K_PROBES.with(|c| c.get()))
}

#[test]
fn a_stuck_match_is_probed_once_while_other_redexes_are_contracted() {
    let mut c = context();
    c.add_declaration("h", eq(g("x"), g("y")));
    let term = ap(ap(g("pair"), stuck(g("h"))), redexes(40));
    let (normal, count) = probes(|| normalize_checked(&c, &term));
    assert_eq!(
        normal.unwrap(),
        ap(ap(g("pair"), stuck(g("h"))), g("Zero")),
        "the neutral match stays, every redex is contracted"
    );
    assert!(
        count <= 2,
        "the same stuck match was K-probed {count} times"
    );
}

#[test]
fn a_remembered_probe_never_crosses_a_different_local_context() {
    // Identical discriminant and motive syntax under two binders of different types: the proof
    // `h : Eq Nat x y` is stuck, the proof `h : Eq Nat x x` is K-reducible to `refl`.
    let c = context();
    let apart = lam("h", eq(g("x"), g("y")), stuck(v("h")));
    let same = lam("h", eq(g("x"), g("x")), stuck(v("h")));
    let term = ap(
        ap(g("pair"), ap(ap(g("pair"), apart.clone()), redexes(8))),
        same,
    );
    let normal = normalize_checked(&c, &term).unwrap();
    let expected = ap(
        ap(g("pair"), ap(ap(g("pair"), apart), g("Zero"))),
        lam("h", eq(g("x"), g("x")), g("Zero")),
    );
    assert_eq!(
        normal, expected,
        "K fires under the equal-endpoint binder only"
    );
}

#[test]
fn remembering_probes_never_changes_a_normal_form() {
    let mut c = context();
    c.add_declaration("h", eq(g("x"), g("y")));
    c.add_declaration("k", eq(g("x"), g("x")));
    for term in [
        ap(ap(g("pair"), stuck(g("h"))), redexes(12)),
        ap(ap(g("pair"), stuck(g("k"))), redexes(12)),
        ap(
            ap(
                g("pair"),
                stuck(ap(lam("p", eq(g("x"), g("x")), v("p")), g("k"))),
            ),
            stuck(g("h")),
        ),
    ] {
        let mut reference = term.clone();
        while let Some(next) = reduce_step_opt(&c, &reference, Transparency::All) {
            reference = next;
        }
        assert_eq!(
            normalize_checked(&c, &term).unwrap(),
            reference,
            "input {term:?}"
        );
    }
}
