//! η-unification (E5) — a function and its η-expansion must unify: `λx. f x ≡ f`. This
//! lets the elaborator solve metavariables that appear η-expanded on one side.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{defeq_for_test, unify, Context, MetaCtx, Term};

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn v(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn lam(p: &str, t: Term, b: Term) -> Term {
    Term::Lambda { param: p.to_string(), param_type: Box::new(t), body: Box::new(b) }
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}

#[test]
fn function_unifies_with_its_eta_expansion() {
    let ctx = std_ctx();
    let mut mctx = MetaCtx::new();
    // `λx:Nat. Succ x  =?=  Succ`
    let eta = lam("x", g("Nat"), app(g("Succ"), v("x")));
    assert!(unify(&ctx, &mut mctx, &eta, &g("Succ")), "λx. Succ x must unify with Succ (η)");
    // Symmetric.
    let mut mctx2 = MetaCtx::new();
    assert!(unify(&ctx, &mut mctx2, &g("Succ"), &eta), "Succ must unify with λx. Succ x (η)");
}

#[test]
fn eta_solves_a_metavariable_under_a_binder() {
    // `λx:Nat. ?M x  =?=  Succ`  ⇒  η reduces to `?M x =?= Succ x`, pattern-solving `?M := Succ`.
    let ctx = std_ctx();
    let mut mctx = MetaCtx::new();
    let m = mctx.fresh();
    let lhs = lam("x", g("Nat"), app(m.clone(), v("x")));
    assert!(unify(&ctx, &mut mctx, &lhs, &g("Succ")), "η + pattern must solve ?M");
    // ?M is solved to `λx. Succ x` (the η-expanded pattern solution), which is
    // definitionally equal to `Succ`.
    if let Term::Var(name) = &m {
        let sol = mctx.solution(name).expect("?M is solved");
        assert!(defeq_for_test(&ctx, sol, &g("Succ")), "?M ≡ Succ (got {sol})");
    } else {
        panic!("fresh metavariable should be a Var");
    }
}

#[test]
fn distinct_functions_do_not_unify_via_eta() {
    // η must not equate genuinely different functions: `λx. Succ x` ≠ `Zero`.
    let ctx = std_ctx();
    let mut mctx = MetaCtx::new();
    let eta = lam("x", g("Nat"), app(g("Succ"), v("x")));
    assert!(!unify(&ctx, &mut mctx, &eta, &g("Zero")), "η must not unify Succ with Zero");
}
