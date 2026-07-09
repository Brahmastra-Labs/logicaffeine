//! R4 — higher-order PATTERN (Miller) unification, locked in by TDD.
//!
//! General higher-order unification is undecidable; the Miller PATTERN fragment —
//! `?M x̄ =?= t` where `?M` is a metavariable applied to DISTINCT bound variables — is
//! decidable, with the unique solution `?M := λx̄. t`. These tests pin it: the solver
//! produces constant functions, the identity, and argument permutations; it respects the
//! occurs-check and rejects right-hand sides that mention un-abstractable variables; it
//! recognizes non-patterns (repeated or non-variable arguments) and leaves them to
//! first-order unification; and — the contrast — it solves exactly what first-order
//! unification cannot.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{instantiate, normalize, unify, unify_in, Context, MetaCtx, Term};

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn v(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}
fn lctx(pairs: &[(&str, Term)]) -> Vec<(String, Term)> {
    pairs.iter().map(|(n, t)| (n.to_string(), t.clone())).collect()
}
/// Instantiate `t` with the solved metavariables and reduce — the realized result.
fn solved(ctx: &Context, mctx: &MetaCtx, t: &Term) -> Term {
    normalize(ctx, &instantiate(t, mctx))
}

#[test]
fn pattern_solves_a_constant_function() {
    // ?M x =?= Nat   (x : Nat in scope)   ⇒   ?M := λx:Nat. Nat
    let ctx = std_ctx();
    let mut m = MetaCtx::new();
    let mv = m.fresh();
    let l = lctx(&[("x", g("Nat"))]);
    assert!(unify_in(&ctx, &mut m, &l, &app(mv.clone(), v("x")), &g("Nat")));
    // Applying the solution to ANY argument yields Nat (it's constant).
    assert_eq!(solved(&ctx, &m, &app(mv, g("Zero"))), g("Nat"));
}

#[test]
fn pattern_solves_the_identity() {
    // ?M x =?= x   ⇒   ?M := λx:Nat. x   (the identity)
    let ctx = std_ctx();
    let mut m = MetaCtx::new();
    let mv = m.fresh();
    let l = lctx(&[("x", g("Nat"))]);
    assert!(unify_in(&ctx, &mut m, &l, &app(mv.clone(), v("x")), &v("x")));
    // `?M Zero` must reduce to `Zero`.
    assert_eq!(solved(&ctx, &m, &app(mv, g("Zero"))), g("Zero"));
}

#[test]
fn pattern_solves_an_argument_permutation() {
    // ?M x y =?= f y x   ⇒   ?M := λx.λy. f y x.  `?M a b` must reduce to `f b a`.
    let ctx = std_ctx();
    let mut m = MetaCtx::new();
    let mv = m.fresh();
    let l = lctx(&[("x", g("Nat")), ("y", g("Bool"))]);
    let lhs = app(app(mv.clone(), v("x")), v("y"));
    let rhs = app(app(g("f"), v("y")), v("x"));
    assert!(unify_in(&ctx, &mut m, &l, &lhs, &rhs));
    // Apply the solution to fresh arguments and check the swap held.
    let applied = app(app(mv, g("Zero")), g("true"));
    assert_eq!(solved(&ctx, &m, &applied), app(app(g("f"), g("true")), g("Zero")));
}

#[test]
fn pattern_respects_the_occurs_check() {
    // ?M x =?= Succ (?M x)  is cyclic — must be rejected, not solved.
    let ctx = std_ctx();
    let mut m = MetaCtx::new();
    let mv = m.fresh();
    let l = lctx(&[("x", g("Nat"))]);
    let lhs = app(mv.clone(), v("x"));
    let rhs = app(g("Succ"), app(mv, v("x")));
    assert!(!unify_in(&ctx, &mut m, &l, &lhs, &rhs), "?M x := Succ (?M x) is cyclic");
}

#[test]
fn pattern_rejects_an_out_of_scope_right_hand_side() {
    // ?M x =?= y, where y is in scope but is NOT a pattern argument: `λx. y` would leave
    // `y` free, so the pattern is unsolvable.
    let ctx = std_ctx();
    let mut m = MetaCtx::new();
    let mv = m.fresh();
    let l = lctx(&[("x", g("Nat")), ("y", g("Nat"))]);
    assert!(
        !unify_in(&ctx, &mut m, &l, &app(mv, v("x")), &v("y")),
        "cannot abstract `x` and keep a free `y`"
    );
}

#[test]
fn repeated_arguments_are_not_a_pattern() {
    // ?M x x =?= x is NOT a Miller pattern (the arguments are not distinct); the pattern
    // solver declines, leaving first-order unification (which cannot solve it) — so the
    // whole unification fails rather than guessing.
    let ctx = std_ctx();
    let mut m = MetaCtx::new();
    let mv = m.fresh();
    let l = lctx(&[("x", g("Nat"))]);
    assert!(!unify_in(&ctx, &mut m, &l, &app(app(mv, v("x")), v("x")), &v("x")));
}

#[test]
fn a_non_variable_argument_is_not_a_pattern() {
    // ?M Zero =?= Zero — the argument is a constructor, not a bound variable, so this is
    // not a pattern; first-order decomposition (`?M =?= Zero` ∧ `Zero =?= ...`) governs.
    let ctx = std_ctx();
    let mut m = MetaCtx::new();
    let mv = m.fresh();
    let l = lctx(&[("x", g("Nat"))]);
    // Not a pattern; falls through to structural unification, which fails on Zero =?= Nat.
    assert!(!unify_in(&ctx, &mut m, &l, &app(mv, g("Zero")), &g("Nat")));
}

#[test]
fn pattern_unification_solves_what_first_order_cannot() {
    // THE contrast: `?M x =?= Nat`. With NO local context (first-order `unify`) this fails
    // — `?M x` is an application, `Nat` is not. With the bound variable `x` in context,
    // pattern unification solves it.
    let ctx = std_ctx();

    let mut first_order = MetaCtx::new();
    let mv1 = first_order.fresh();
    assert!(
        !unify(&ctx, &mut first_order, &app(mv1, v("x")), &g("Nat")),
        "first-order unification cannot solve a metavariable application"
    );

    let mut higher_order = MetaCtx::new();
    let mv2 = higher_order.fresh();
    let l = lctx(&[("x", g("Nat"))]);
    assert!(
        unify_in(&ctx, &mut higher_order, &l, &app(mv2, v("x")), &g("Nat")),
        "pattern unification solves it"
    );
}
