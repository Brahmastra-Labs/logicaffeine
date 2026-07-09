//! B4 — QUOTIENT TYPES. `Quot A r` (opaque, non-inductive), `Quot_mk`, `Quot_lift` (with
//! the definitional computation rule `Quot_lift … (Quot_mk a) ≡ f a`), `Quot_ind`, and the
//! `Quot_sound` axiom identifying related representatives. The computation rule lives in
//! both kernels, so quotient proofs are two-kernel verified.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    double_check, infer_type, is_subtype, normalize, Context, DoubleCheck, Term,
};

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn v(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn lam(p: &str, ty: Term, body: Term) -> Term {
    Term::Lambda { param: p.to_string(), param_type: Box::new(ty), body: Box::new(body) }
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}
fn nat_lit(n: usize) -> Term {
    let mut t = g("Zero");
    for _ in 0..n {
        t = app(g("Succ"), t);
    }
    t
}
fn eqn(a: Term, b: Term) -> Term {
    app(app(app(g("Eq"), g("Nat")), a), b)
}
fn quot(a: Term, r: Term) -> Term {
    app(app(g("Quot"), a), r)
}
fn quot_mk(a: Term, r: Term, x: Term) -> Term {
    app(app(app(g("Quot_mk"), a), r), x)
}
fn quot_lift(a: Term, r: Term, b: Term, f: Term, h: Term, q: Term) -> Term {
    app(app(app(app(app(app(g("Quot_lift"), a), r), b), f), h), q)
}
/// `r := λx y. Eq Nat x y` (the equality relation) and `r_true := λx y. True` (total).
fn r_eq() -> Term {
    lam("x", g("Nat"), lam("y", g("Nat"), eqn(v("x"), v("y"))))
}
fn r_true() -> Term {
    lam("x", g("Nat"), lam("y", g("Nat"), g("True")))
}

#[test]
fn quotient_primitives_are_registered_and_well_typed() {
    let ctx = std_ctx();
    for name in ["Quot", "Quot_mk", "Quot_lift", "Quot_ind", "Quot_sound"] {
        assert!(
            infer_type(&ctx, &g(name)).is_ok(),
            "{name} must be registered and well-typed: {:?}",
            infer_type(&ctx, &g(name))
        );
    }
}

#[test]
fn quot_lift_computes_on_a_representative() {
    // Lift `Succ` over the equality quotient (respected by `succ_cong`): applying it to the
    // class of `2` computes to `Succ 2 = 3`.
    let ctx = std_ctx();
    let f = lam("x", g("Nat"), app(g("Succ"), v("x")));
    let lift = quot_lift(
        g("Nat"),
        r_eq(),
        g("Nat"),
        f,
        g("succ_cong"),
        quot_mk(g("Nat"), r_eq(), nat_lit(2)),
    );
    assert!(infer_type(&ctx, &lift).is_ok(), "the lift must type-check: {:?}", infer_type(&ctx, &lift));
    assert_eq!(normalize(&ctx, &lift), nat_lit(3), "Quot_lift Succ … (mk 2) must compute to 3");
}

#[test]
fn quot_sound_identifies_distinct_representatives() {
    // Under the TOTAL relation, distinct naturals `2` and `3` are EQUAL in the quotient —
    // the whole point of a quotient. `Quot_sound Nat r_true 2 3 I` proves it.
    let ctx = std_ctx();
    let sound = app(
        app(app(app(app(g("Quot_sound"), g("Nat")), r_true()), nat_lit(2)), nat_lit(3)),
        g("I"),
    );
    let ty = infer_type(&ctx, &sound).expect("Quot_sound must type-check");
    let expected = eqn_at(
        quot(g("Nat"), r_true()),
        quot_mk(g("Nat"), r_true(), nat_lit(2)),
        quot_mk(g("Nat"), r_true(), nat_lit(3)),
    );
    assert!(
        is_subtype(&ctx, &ty, &expected),
        "Quot_sound proves (mk 2 = mk 3) in the quotient\n  got: {ty}"
    );
}

/// `Eq (Quot …) x y` for the quotient-typed equality above.
fn eqn_at(ty: Term, x: Term, y: Term) -> Term {
    app(app(app(g("Eq"), ty), x), y)
}

#[test]
fn quot_computation_rule_is_two_kernel_verified() {
    // `Eq_sym Nat (Quot_lift … (mk 2)) 3 (refl Nat 3)` type-checks only via the computation
    // rule `Quot_lift Succ … (mk 2) ≡ Succ 2 ≡ 3` — and BOTH kernels must apply it.
    let ctx = std_ctx();
    let f = lam("x", g("Nat"), app(g("Succ"), v("x")));
    let lift = quot_lift(
        g("Nat"),
        r_eq(),
        g("Nat"),
        f,
        g("succ_cong"),
        quot_mk(g("Nat"), r_eq(), nat_lit(2)),
    );
    let refl3 = app(app(g("refl"), g("Nat")), nat_lit(3));
    let term = app(app(app(app(g("Eq_sym"), g("Nat")), lift), nat_lit(3)), refl3);
    assert!(
        infer_type(&ctx, &term).is_ok(),
        "the computation rule lets the refl witness check: {:?}",
        infer_type(&ctx, &term)
    );
    assert_eq!(
        double_check(&ctx, &term),
        DoubleCheck::Agreed,
        "the quotient computation rule must be two-kernel verified"
    );
}
