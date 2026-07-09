//! B3d — `native_decide`: discharge a `decide` goal by evaluating the decision procedure
//! with the fast CBV evaluator, trusting the result (via `ofReduceBool`) instead of having
//! the kernel re-normalize it.
//!
//! The evaluator is trusted, so the first test is a DIFFERENTIAL check that it agrees with
//! the kernel's own `normalize` — the confidence backing the trust boundary.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{eval_bool, infer_type, is_subtype, native_decide, normalize, Context, Term};

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
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
fn dec_eq_nat(a: Term, b: Term) -> Term {
    app(app(g("decEqNat"), a), b)
}

#[test]
fn reduce_bool_and_of_reduce_bool_are_registered() {
    let ctx = std_ctx();
    assert!(infer_type(&ctx, &g("reduceBool")).is_ok(), "reduceBool must be typed");
    assert!(infer_type(&ctx, &g("ofReduceBool")).is_ok(), "ofReduceBool must be typed");
}

#[test]
fn evaluator_agrees_with_normalize() {
    // The trust check: for every `decEqNat a b`, the fast evaluator's verdict must match
    // BOTH the ground truth (`a == b`) AND what the kernel's `normalize` computes.
    let ctx = std_ctx();
    for a in 0..5usize {
        for b in 0..5usize {
            let decide_app =
                app(app(g("decide"), eqn(nat_lit(a), nat_lit(b))), dec_eq_nat(nat_lit(a), nat_lit(b)));
            let eval = eval_bool(&ctx, &decide_app);
            let norm = normalize(&ctx, &decide_app);
            let expected = a == b;
            assert_eq!(eval, Some(expected), "evaluator: {a} = {b} should be {expected}");
            let norm_bool = match norm {
                ref t if *t == g("true") => true,
                ref t if *t == g("false") => false,
                other => panic!("normalize gave a non-Bool: {other}"),
            };
            assert_eq!(norm_bool, expected, "normalize: {a} = {b} should be {expected}");
            assert_eq!(eval, Some(norm_bool), "evaluator must AGREE with normalize on {a}={b}");
        }
    }
}

#[test]
fn native_decide_proves_arithmetic_and_typechecks_via_the_hook() {
    // `native_decide` produces a proof of `7 = 7`, and the (main) kernel accepts it —
    // exercising the `reduceBool` reduction hook (which runs the fast evaluator).
    let ctx = std_ctx();
    let prop = eqn(nat_lit(7), nat_lit(7));
    let proof = native_decide(&ctx, &prop, &dec_eq_nat(nat_lit(7), nat_lit(7)))
        .expect("native_decide should prove 7 = 7");
    let ty = infer_type(&ctx, &proof).expect("the native_decide proof must type-check");
    assert!(is_subtype(&ctx, &ty, &prop), "the proof has type Eq Nat 7 7");
}

#[test]
fn native_decide_is_fail_closed() {
    // A false goal: the evaluator returns `false`, so `native_decide` declines — it can
    // never fabricate a proof of `7 = 8`.
    let ctx = std_ctx();
    let prop = eqn(nat_lit(7), nat_lit(8));
    assert!(
        native_decide(&ctx, &prop, &dec_eq_nat(nat_lit(7), nat_lit(8))).is_none(),
        "native_decide must decline a false goal"
    );
}

#[test]
fn native_decide_scales_past_kernel_decide_range() {
    // A larger numeral the fast evaluator handles comfortably; the proof still type-checks
    // via the hook (the kernel runs the evaluator once rather than normalizing the fixpoint
    // by substitution).
    let ctx = std_ctx();
    let prop = eqn(nat_lit(60), nat_lit(60));
    let proof = native_decide(&ctx, &prop, &dec_eq_nat(nat_lit(60), nat_lit(60)))
        .expect("native_decide should prove 60 = 60");
    let ty = infer_type(&ctx, &proof).expect("large native_decide proof must type-check");
    assert!(is_subtype(&ctx, &ty, &prop), "the proof has type Eq Nat 60 60");
}

// ---- Int primitives in the evaluator (the `le(2,5) = true` substrate) ----

fn int_lit(n: i64) -> Term {
    Term::Lit(logicaffeine_kernel::Literal::Int(n))
}

#[test]
fn evaluator_computes_int_primitives_like_normalize() {
    // Differential trust check for the Int fragment: on every ground
    // comparison, the fast evaluator's Bool verdict must match what the
    // kernel's own normalize computes via `try_primitive_reduce`.
    let ctx = std_ctx();
    let pairs: [(i64, i64); 7] =
        [(0, 0), (2, 5), (5, 2), (-3, 3), (7, 7), (i64::MAX, i64::MIN), (-1, 0)];
    for op in ["le", "lt", "ge", "gt"] {
        for (a, b) in pairs {
            let t = app(app(g(op), int_lit(a)), int_lit(b));
            let via_eval = eval_bool(&ctx, &t);
            let via_normalize = match normalize(&ctx, &t) {
                Term::Global(n) if n == "true" => Some(true),
                Term::Global(n) if n == "false" => Some(false),
                _ => None,
            };
            assert_eq!(via_eval, via_normalize, "{op}({a},{b})");
            assert!(via_eval.is_some(), "{op}({a},{b}) must decide");
        }
    }
}

#[test]
fn evaluator_int_arithmetic_agrees_with_normalize() {
    // add/sub/mul/div/mod on ground Ints, compared through a comparison so the
    // result surfaces as a Bool: `le(add(2,3), 5) = true`, etc. Since K6, i64 overflow
    // PROMOTES to exact arbitrary precision (both engines identically), so it computes
    // rather than getting stuck; division by zero stays undecided in both.
    let ctx = std_ctx();
    let cases: Vec<(Term, Option<bool>)> = vec![
        (app(app(g("le"), app(app(g("add"), int_lit(2)), int_lit(3))), int_lit(5)), Some(true)),
        (app(app(g("lt"), app(app(g("mul"), int_lit(4)), int_lit(5))), int_lit(20)), Some(false)),
        (app(app(g("ge"), app(app(g("sub"), int_lit(10)), int_lit(3))), int_lit(7)), Some(true)),
        (app(app(g("le"), app(app(g("div"), int_lit(9)), int_lit(2))), int_lit(4)), Some(true)),
        (app(app(g("le"), app(app(g("mod"), int_lit(9)), int_lit(2))), int_lit(1)), Some(true)),
        // Overflow now PROMOTES to exact BigInt (K6): i64::MAX + 1 = 2^63 (> 0), so
        // `le(2^63, 0) = false` — computed identically in both engines, not stuck.
        (
            app(app(g("le"), app(app(g("add"), int_lit(i64::MAX)), int_lit(1))), int_lit(0)),
            Some(false),
        ),
        // Division by zero: stuck in both engines.
        (
            app(app(g("le"), app(app(g("div"), int_lit(1)), int_lit(0))), int_lit(0)),
            None,
        ),
    ];
    for (t, expected) in cases {
        let via_eval = eval_bool(&ctx, &t);
        let via_normalize = match normalize(&ctx, &t) {
            Term::Global(n) if n == "true" => Some(true),
            Term::Global(n) if n == "false" => Some(false),
            _ => None,
        };
        assert_eq!(via_eval, via_normalize, "differential: {t}");
        assert_eq!(via_eval, expected, "expected verdict: {t}");
    }
}

#[test]
fn native_decide_proves_ground_int_comparison() {
    // The end-to-end route `decide` rides: Eq Bool (le 2 5) true via
    // decEqBool, kernel-checked.
    let ctx = std_ctx();
    let le25 = app(app(g("le"), int_lit(2)), int_lit(5));
    let prop = app(app(app(g("Eq"), g("Bool")), le25.clone()), g("true"));
    let inst = app(app(g("decEqBool"), le25), g("true"));
    let proof = native_decide(&ctx, &prop, &inst).expect("le(2,5)=true must decide");
    let ty = infer_type(&ctx, &proof).expect("proof must type-check");
    assert!(is_subtype(&ctx, &ty, &prop));
}

#[test]
fn native_decide_declines_false_int_comparison() {
    let ctx = std_ctx();
    let lt52 = app(app(g("lt"), int_lit(5)), int_lit(2));
    let prop = app(app(app(g("Eq"), g("Bool")), lt52.clone()), g("true"));
    let inst = app(app(g("decEqBool"), lt52), g("true"));
    assert!(
        native_decide(&ctx, &prop, &inst).is_none(),
        "a false comparison must be declined"
    );
}
