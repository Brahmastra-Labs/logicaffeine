//! =============================================================================
//! PHASE 80: REDUCTION SOUNDNESS — CAPTURE-AVOIDANCE & DE BRUIJN BETA
//! =============================================================================
//!
//! Regression pins for Bug Report #1. The proof kernel's beta reduction must be
//! a sound equivalence: it must not capture free variables (named representation)
//! and must decrement surviving free de Bruijn indices when a binder is removed
//! (reflected representation). A wrong normal form here makes definitional
//! equality / the `DCompute` tactic certify a FALSE equality.

use logicaffeine_kernel::{normalize, Context, Literal, Term, Universe};

/// BUG-002 (Critical): the reflected de Bruijn beta (`syn_beta` = `syn_subst arg 0 body`)
/// never decrements surviving free variables after the binder is eliminated.
/// `(λ. SVar 1) (SVar 7)` must reduce to `SVar 0`, not `SVar 1`.
#[test]
fn reflected_beta_decrements_surviving_free_vars() {
    let ctx = Context::new();
    fn g(n: &str) -> Term {
        Term::Global(n.into())
    }
    fn app(f: Term, x: Term) -> Term {
        Term::App(Box::new(f), Box::new(x))
    }
    let svar = |k: i64| app(g("SVar"), Term::Lit(Literal::Int(k)));
    let slam = |t: Term, b: Term| app(app(g("SLam"), t), b);
    let sapp = |f: Term, x: Term| app(app(g("SApp"), f), x);

    // redex = (\T. SVar 1) (SVar 7), where SVar 1 is free: it refers to the
    // binder one level OUTSIDE this lambda. Once the lambda's binder is
    // eliminated by beta, that reference must shift down: SVar 1 -> SVar 0.
    let redex = sapp(slam(svar(0), svar(1)), svar(7));
    let eval = app(app(g("syn_eval"), Term::Lit(Literal::Int(5))), redex);
    let result = normalize(&ctx, &eval);

    assert_eq!(
        result,
        svar(0),
        "surviving free var not decremented after binder removal: got {:?}",
        result
    );
    assert_ne!(
        result,
        svar(1),
        "got SVar 1 (one index too high) — binder removed but free var left un-shifted"
    );
}

/// BUG-003 (Critical): named-variable `substitute` (the engine of beta, fix
/// unfolding, and definitional equality) is not capture-avoiding. `(λf. λx. f x) x`
/// must reduce to `λx'. x x'` (free `x` applied to the renamed bound param), not
/// the captured `λx. x x`.
#[test]
fn beta_reduction_is_capture_avoiding() {
    let ctx = Context::new();
    let ty = || Box::new(Term::Sort(Universe::Type(0)));

    // (λf. λx. f x) x   — the trailing/argument x is a FREE variable.
    let inner_body = Term::App(
        Box::new(Term::Var("f".into())),
        Box::new(Term::Var("x".into())),
    );
    let lam_x = Term::Lambda {
        param: "x".into(),
        param_type: ty(),
        body: Box::new(inner_body),
    };
    let lam_f = Term::Lambda {
        param: "f".into(),
        param_type: ty(),
        body: Box::new(lam_x),
    };
    let redex = Term::App(Box::new(lam_f), Box::new(Term::Var("x".into())));

    let result = normalize(&ctx, &redex);

    // After capture-avoiding beta reduction the result must be a lambda whose
    // body applies the FREE x (head) to the BOUND parameter (arg), and the bound
    // parameter must have been renamed away from "x" (otherwise the free x was
    // captured, collapsing two distinct variables into one).
    match &result {
        Term::Lambda { param, body, .. } => match body.as_ref() {
            Term::App(f, a) => {
                assert_eq!(**f, Term::Var("x".into()), "head must remain the FREE x");
                assert_eq!(**a, Term::Var(param.clone()), "arg must be the BOUND param");
                assert_ne!(
                    param, "x",
                    "VARIABLE CAPTURE: inner binder captured the free x; got `λx. x x` \
                     instead of capture-avoiding `λx'. x x'`"
                );
            }
            other => panic!("unexpected lambda body, expected an application: {:?}", other),
        },
        other => panic!("unexpected normal form, expected a lambda: {:?}", other),
    }
}
