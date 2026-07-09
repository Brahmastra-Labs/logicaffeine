//! The integer order axioms — including the two `omega` adds, `lt_succ_le`
//! (discreteness) and `le_total` (totality) — must type-check with the exact
//! shapes the proof engine's Farkas/omega reconstruction relies on.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, is_subtype, Context, Literal, Term};

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn app2(f: Term, x: Term, y: Term) -> Term {
    app(app(f, x), y)
}
fn int_lit(n: i64) -> Term {
    Term::Lit(Literal::Int(n))
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}
/// `Eq Bool (op a b) true` — the shallow encoding of `a op b`.
fn cmp_prop(op: &str, a: Term, b: Term) -> Term {
    app2(app(g("Eq"), g("Bool")), app2(g(op), a, b), g("true"))
}
fn le_prop(a: Term, b: Term) -> Term {
    cmp_prop("le", a, b)
}
fn lt_prop(a: Term, b: Term) -> Term {
    cmp_prop("lt", a, b)
}

#[test]
fn discreteness_axioms_are_registered() {
    let ctx = std_ctx();
    assert!(infer_type(&ctx, &g("lt_succ_le")).is_ok(), "lt_succ_le must be typed");
    assert!(infer_type(&ctx, &g("lt_add1_le")).is_ok(), "lt_add1_le must be typed");
    assert!(infer_type(&ctx, &g("le_total")).is_ok(), "le_total must be typed");
}

#[test]
fn lt_add1_le_cancels_the_successor_bound() {
    // lt_add1_le 4 6 : (4 < 6+1) → (4 ≤ 6).
    let ctx = std_ctx();
    let applied = app2(g("lt_add1_le"), int_lit(4), int_lit(6));
    let ty = infer_type(&ctx, &applied).expect("lt_add1_le 4 6 must type-check");
    let expected = Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(lt_prop(int_lit(4), app2(g("add"), int_lit(6), int_lit(1)))),
        body_type: Box::new(le_prop(int_lit(4), int_lit(6))),
    };
    assert!(
        is_subtype(&ctx, &ty, &expected),
        "lt_add1_le 4 6 : (4 < 6+1) → (4 ≤ 6), got {ty}"
    );
}

#[test]
fn lt_succ_le_applied_yields_the_shifted_le() {
    // lt_succ_le 3 7 : (3 < 7) → (3 + 1 ≤ 7).
    let ctx = std_ctx();
    let applied = app2(g("lt_succ_le"), int_lit(3), int_lit(7));
    let ty = infer_type(&ctx, &applied).expect("lt_succ_le 3 7 must type-check");
    let expected = Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(lt_prop(int_lit(3), int_lit(7))),
        body_type: Box::new(le_prop(app2(g("add"), int_lit(3), int_lit(1)), int_lit(7))),
    };
    assert!(
        is_subtype(&ctx, &ty, &expected),
        "lt_succ_le 3 7 : (3<7) → (3+1 ≤ 7), got {ty}"
    );
}

#[test]
fn le_total_applied_is_a_disjunction() {
    // le_total 2 5 : (2 ≤ 5) ∨ (5 ≤ 2).
    let ctx = std_ctx();
    let applied = app2(g("le_total"), int_lit(2), int_lit(5));
    let ty = infer_type(&ctx, &applied).expect("le_total 2 5 must type-check");
    let expected = app2(g("Or"), le_prop(int_lit(2), int_lit(5)), le_prop(int_lit(5), int_lit(2)));
    assert!(is_subtype(&ctx, &ty, &expected), "le_total 2 5 : (2≤5) ∨ (5≤2), got {ty}");
}

