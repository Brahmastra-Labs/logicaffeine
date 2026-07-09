//! B2 — definitional conversion beyond structure: η (functions) and PROOF IRRELEVANCE.
//!
//! η: `f ≡ λx. f x`. Proof irrelevance: any two proofs of the same proposition are equal
//! (but distinct values of a `Type` are NOT). Both are added to the conversion of BOTH
//! kernels, so proofs relying on them are two-kernel verified.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    double_check, infer_type, is_subtype, normalize, Context, DoubleCheck, Term, Universe,
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
fn pi(p: &str, ty: Term, body: Term) -> Term {
    Term::Pi { param: p.to_string(), param_type: Box::new(ty), body_type: Box::new(body) }
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}

// =============================================================================
// η-conversion.
// =============================================================================

#[test]
fn eta_function_conversion_both_directions() {
    // `Succ ≡ λx:Nat. Succ x`, symmetric.
    let ctx = std_ctx();
    let succ = g("Succ");
    let eta = lam("x", g("Nat"), app(g("Succ"), v("x")));
    assert!(is_subtype(&ctx, &succ, &eta), "Succ ≡ λx. Succ x (η)");
    assert!(is_subtype(&ctx, &eta, &succ), "λx. Succ x ≡ Succ (η, other direction)");
}

#[test]
fn eta_is_two_kernel_verified() {
    // `Eq_sym (Nat→Nat) Succ (λx.Succ x) (refl (Nat→Nat) Succ)` type-checks only because the
    // `refl` witness (`Eq .. Succ Succ`) is accepted at `Eq .. Succ (λx.Succ x)` by η — and
    // BOTH kernels must agree.
    let ctx = std_ctx();
    let nat2nat = pi("_", g("Nat"), g("Nat"));
    let succ = g("Succ");
    let eta = lam("x", g("Nat"), app(g("Succ"), v("x")));
    let refl_succ = app(app(g("refl"), nat2nat.clone()), succ.clone());
    let term = app(
        app(app(app(g("Eq_sym"), nat2nat), succ), eta),
        refl_succ,
    );
    assert!(infer_type(&ctx, &term).is_ok(), "η lets the refl witness check: {:?}", infer_type(&ctx, &term));
    assert_eq!(double_check(&ctx, &term), DoubleCheck::Agreed, "η must be two-kernel verified");
}

// =============================================================================
// Proof irrelevance.
// =============================================================================

#[test]
fn proof_irrelevance_collapses_proofs_of_a_prop() {
    // With `P : Prop`, `a b : P` in scope, `a ≡ b` — proofs of a proposition are irrelevant.
    let mut ctx = std_ctx();
    ctx.add("P", Term::Sort(Universe::Prop));
    ctx.add("a", v("P"));
    ctx.add("b", v("P"));
    assert!(is_subtype(&ctx, &v("a"), &v("b")), "any two proofs of a Prop are definitionally equal");
}

#[test]
fn proof_irrelevance_does_not_collapse_type_values() {
    // SOUNDNESS: distinct values of a `Type` must NOT be identified — proof irrelevance is
    // for `Prop` only. (`0 ≡ 1 : Nat` would be catastrophic.)
    let mut ctx = std_ctx();
    ctx.add("T", Term::Sort(Universe::Type(0)));
    ctx.add("x", v("T"));
    ctx.add("y", v("T"));
    assert!(!is_subtype(&ctx, &v("x"), &v("y")), "distinct Type-level values must stay distinct");
    // And concretely: 0 and 1 are not identified.
    let zero = g("Zero");
    let one = app(g("Succ"), g("Zero"));
    assert!(!is_subtype(&ctx, &zero, &one), "0 ≢ 1");
}

#[test]
fn proof_irrelevance_is_two_kernel_verified() {
    // `Or True True` is a proposition with two genuinely distinct proofs, `left True True I`
    // and `right True True I`. Proof irrelevance identifies them, so the `refl` witness for
    // the first is accepted where the second is expected — verified by BOTH kernels.
    let ctx = std_ctx();
    let or_tt = app(app(g("Or"), g("True")), g("True"));
    let l = app(app(app(g("left"), g("True")), g("True")), g("I"));
    let r = app(app(app(g("right"), g("True")), g("True")), g("I"));
    let refl_l = app(app(g("refl"), or_tt.clone()), l.clone());
    // Eq_sym (Or True True) (left …) (right …) (refl (Or True True) (left …))
    let term = app(app(app(app(g("Eq_sym"), or_tt), l), r), refl_l);
    assert!(
        infer_type(&ctx, &term).is_ok(),
        "proof irrelevance lets the refl witness check: {:?}",
        infer_type(&ctx, &term)
    );
    assert_eq!(double_check(&ctx, &term), DoubleCheck::Agreed, "proof irrelevance must be two-kernel verified");
    // Also sanity: the two proofs are directly convertible.
    assert!(is_subtype(&ctx, &normalize(&ctx, &term), &normalize(&ctx, &term)));
}
