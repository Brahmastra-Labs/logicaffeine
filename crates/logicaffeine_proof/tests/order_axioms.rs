//! The Int order theory — the trusted base for linear-arithmetic certificates.
//! `m ≤ n` is the shallow Prop `Eq Bool (le m n) true`. These tests confirm the
//! order axioms (`le_trans`, `le_add_mono`, `le_mul_nonneg`) are real, applicable
//! kernel proofs, and that a Farkas-style step — scaling a hypothesis by a ground
//! non-negative multiplier whose `0 ≤ k` side-condition is DECIDED BY COMPUTATION
//! (`refl`) — reconstructs and type-checks. This is the certificate machinery the
//! engine's linarith strategy will assemble from an Omega/Fourier-Motzkin decision.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, is_subtype, Context, Literal, Term};

fn ctx() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c
}
fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn int() -> Term {
    g("Int")
}
fn lit(n: i64) -> Term {
    Term::Lit(Literal::Int(n))
}
fn app(f: Term, xs: Vec<Term>) -> Term {
    xs.into_iter()
        .fold(f, |acc, x| Term::App(Box::new(acc), Box::new(x)))
}
/// `a ≤ b`  ≡  `Eq Bool (le a b) true`
fn le(a: Term, b: Term) -> Term {
    let le_ab = app(g("le"), vec![a, b]);
    app(g("Eq"), vec![g("Bool"), le_ab, g("true")])
}

#[test]
fn le_trans_chains_two_hypotheses() {
    let mut c = ctx();
    for x in ["x", "y", "z"] {
        c.add_declaration(x, int());
    }
    c.add_declaration("hxy", le(g("x"), g("y")));
    c.add_declaration("hyz", le(g("y"), g("z")));

    // le_trans x y z hxy hyz : x ≤ z
    let cert = app(g("le_trans"), vec![g("x"), g("y"), g("z"), g("hxy"), g("hyz")]);
    let inferred = infer_type(&c, &cert).expect("le_trans application type-checks");
    assert!(
        is_subtype(&c, &inferred, &le(g("x"), g("z"))),
        "le_trans should prove x ≤ z, got {}",
        inferred
    );
}

#[test]
fn le_add_mono_adds_two_inequalities() {
    let mut c = ctx();
    for x in ["a", "b", "p", "q"] {
        c.add_declaration(x, int());
    }
    c.add_declaration("hab", le(g("a"), g("b")));
    c.add_declaration("hpq", le(g("p"), g("q")));

    // le_add_mono a b p q hab hpq : (a + p) ≤ (b + q)
    let cert = app(
        g("le_add_mono"),
        vec![g("a"), g("b"), g("p"), g("q"), g("hab"), g("hpq")],
    );
    let inferred = infer_type(&c, &cert).expect("le_add_mono application type-checks");
    let goal = le(app(g("add"), vec![g("a"), g("p")]), app(g("add"), vec![g("b"), g("q")]));
    assert!(
        is_subtype(&c, &inferred, &goal),
        "le_add_mono should prove (a+p) ≤ (b+q), got {}",
        inferred
    );
}

#[test]
fn farkas_scale_by_ground_nonneg_multiplier() {
    let mut c = ctx();
    c.add_declaration("x", int());
    c.add_declaration("y", int());
    c.add_declaration("hxy", le(g("x"), g("y")));

    // The side condition `0 ≤ 2` is decided by COMPUTATION: le 0 2 ⇝ true, so
    // `refl Bool true : Eq Bool true true` proves `Eq Bool (le 0 2) true`.
    let refl_true = app(g("refl"), vec![g("Bool"), g("true")]);
    assert!(
        is_subtype(&c, &infer_type(&c, &refl_true).unwrap(), &le(lit(0), lit(2))),
        "0 ≤ 2 must be decided by computation"
    );

    // le_mul_nonneg 2 x y (0≤2) (x≤y) : (2*x) ≤ (2*y)
    let cert = app(
        g("le_mul_nonneg"),
        vec![lit(2), g("x"), g("y"), refl_true, g("hxy")],
    );
    let inferred = infer_type(&c, &cert).expect("le_mul_nonneg application type-checks");
    let goal = le(app(g("mul"), vec![lit(2), g("x")]), app(g("mul"), vec![lit(2), g("y")]));
    assert!(
        is_subtype(&c, &inferred, &goal),
        "scaling x ≤ y by 2 should prove 2x ≤ 2y, got {}",
        inferred
    );
}

#[test]
fn unsound_scaling_by_negative_is_rejected() {
    // `le_mul_nonneg` requires `0 ≤ k`. For k = -1 that side condition is FALSE by
    // computation (le 0 (-1) ⇝ false), so `refl Bool true` cannot supply it — the
    // certificate does not type-check. (Scaling an inequality by a negative number
    // flips it; the non-negativity guard is what makes the axiom sound.)
    let mut c = ctx();
    c.add_declaration("x", int());
    c.add_declaration("y", int());
    c.add_declaration("hxy", le(g("x"), g("y")));
    let refl_true = app(g("refl"), vec![g("Bool"), g("true")]);
    // Try to use refl (which proves true=true) where `0 ≤ -1` is required.
    let bogus = app(
        g("le_mul_nonneg"),
        vec![lit(-1), g("x"), g("y"), refl_true, g("hxy")],
    );
    assert!(
        infer_type(&c, &bogus).is_err(),
        "scaling by a negative multiplier must not type-check (0 ≤ -1 is false)"
    );
}
