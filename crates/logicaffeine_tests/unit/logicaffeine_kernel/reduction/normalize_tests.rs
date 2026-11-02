//! Lib-unit coverage for `normalize` — the β/ζ/δ reduction engine `def_eq` trusts. Pins the
//! reduction rules to known normal forms and locks idempotence (a normal form is a fixpoint).
use super::*;
use crate::prelude::StandardLibrary;
use crate::term::{Literal, Term, Universe};

fn ctx() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c
}
fn sort(n: u32) -> Term {
    Term::Sort(Universe::Type(n))
}
fn g(s: &str) -> Term {
    Term::Global(s.to_string())
}
fn v(s: &str) -> Term {
    Term::Var(s.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn nat(n: u64) -> Term {
    Term::Lit(Literal::Nat(logicaffeine_base::BigInt::from_u64(n)))
}
fn lam(p: &str, ty: Term, b: Term) -> Term {
    Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(ty),
        body: Box::new(b),
    }
}
fn pi(p: &str, ty: Term, b: Term) -> Term {
    Term::Pi {
        param: p.to_string(),
        param_type: Box::new(ty),
        body_type: Box::new(b),
    }
}

#[test]
fn normalize_beta_reduces_applied_lambdas() {
    let c = ctx();
    // (λx:Nat. x) 5  →  5
    assert_eq!(
        normalize(&c, &app(lam("x", g("Nat"), v("x")), nat(5))),
        nat(5)
    );
    // ((λA:Type0. λx:A. x) Nat) 5  →  5   (polymorphic identity)
    let id = lam("A", sort(0), lam("x", v("A"), v("x")));
    assert_eq!(normalize(&c, &app(app(id, g("Nat")), nat(5))), nat(5));
    // (λx:Nat. λy:Nat. x) 3 7  →  3   (K drops its second argument)
    let k = lam("x", g("Nat"), lam("y", g("Nat"), v("x")));
    assert_eq!(normalize(&c, &app(app(k, nat(3)), nat(7))), nat(3));
}

#[test]
fn normalize_zeta_reduces_let_and_delta_unfolds_definitions() {
    let mut c = ctx();
    // ζ: let x := 5 in x  →  5
    let zeta = Term::Let {
        name: "x".to_string(),
        ty: Box::new(g("Nat")),
        value: Box::new(nat(5)),
        body: Box::new(v("x")),
    };
    assert_eq!(normalize(&c, &zeta), nat(5));
    // δ: a transparent definition unfolds to its body.
    c.add_definition("five".to_string(), g("Nat"), nat(5));
    assert_eq!(normalize(&c, &g("five")), nat(5));
    // An opaque inductive does NOT unfold.
    assert_eq!(normalize(&c, &g("Nat")), g("Nat"));
}

#[test]
fn normalize_is_idempotent_on_normal_forms() {
    let c = ctx();
    for t in [
        sort(0),
        g("Nat"),
        nat(9),
        app(g("Succ"), nat(1)),
        pi("x", g("Nat"), g("Nat")),
    ] {
        let nf = normalize(&c, &t);
        assert_eq!(
            normalize(&c, &nf),
            nf,
            "normalize must be a fixpoint on {t:?}"
        );
    }
}

// ---- DEFECT-9 / T11-005: the soundness doors must not certify from an unfinished
// ---- reduction. These reach the private doors directly, because the hazard is invisible
// ---- from outside: a false POSITIVE needs two reductions abandoned at the same place,
// ---- and a δ-cycle is the one shape that reaches that state cheaply.

fn int_g() -> Term {
    g("Int")
}

/// `simp_compute_check` decides an equality by normalising both sides and comparing, and
/// `try_dsimp_solve_conclude` ACCEPTS PROOFS through it. When a side has no normal form at
/// all, "they came out equal" is not evidence of anything — the same code path with a
/// spent step budget is a false positive that certifies a wrong proof.
#[test]
fn the_reflection_door_refuses_a_side_whose_reduction_never_finishes() {
    let mut c = ctx();
    // A reduction that FINISHES is accepted: `def six := 6` and `6 = 6`.
    c.add_definition("six".to_string(), int_g(), Term::Lit(Literal::Int(6)));
    let finished = term_to_syntax(&app(
        app(app(g("Eq"), int_g()), g("six")),
        Term::Lit(Literal::Int(6)),
    ))
    .expect("reflects");
    assert!(
        simp_compute_check(&c, &finished),
        "a finished reduction must still decide the equality"
    );

    // …and a δ-CYCLE is refused, even though the two sides are the SAME term and the
    // equation is true: the door reached that verdict without finishing a reduction, and
    // it cannot tell that apart from a budget that ran out on two different terms.
    c.add_definition("loop_a".to_string(), int_g(), g("loop_b"));
    c.add_definition("loop_b".to_string(), int_g(), g("loop_a"));
    let cyclic = term_to_syntax(&app(app(app(g("Eq"), int_g()), g("loop_a")), g("loop_a")))
        .expect("reflects");
    assert!(
        !simp_compute_check(&c, &cyclic),
        "a term with NO normal form must not be certified by `normalise both and compare`"
    );

    // Only one side cyclic is refused too — the plainly-unsound direction.
    let half = term_to_syntax(&app(
        app(app(g("Eq"), int_g()), g("loop_a")),
        Term::Lit(Literal::Int(6)),
    ))
    .expect("reflects");
    assert!(!simp_compute_check(&c, &half));
}

/// The reflected evaluator's honest channel: `Ok` only when the evaluation FINISHED,
/// `Err(Some(partial))` when the fuel ran out — and the object-language face
/// (`try_syn_eval_reduce`, whose fuel the user writes in the term) still hands back the
/// partial, because there the budget is visible to the caller.
#[test]
fn syn_eval_reports_exhaustion_and_still_answers_the_object_language() {
    let c = ctx();
    let slit = |n: i64| app(g("SLit"), Term::Lit(Literal::Int(n)));
    let sname = |s: &str| app(g("SName"), Term::Lit(Literal::Text(s.to_string())));
    let sapp = |f: Term, x: Term| app(app(g("SApp"), f), x);
    // `add (add 1 2) 4` in the reflected language: two arithmetic steps.
    let two_steps = sapp(
        sapp(sname("add"), sapp(sapp(sname("add"), slit(1)), slit(2))),
        slit(4),
    );

    assert_eq!(
        syn_eval_checked(&c, 8, &two_steps),
        Ok(slit(7)),
        "with fuel to spare the evaluation finishes and says so"
    );
    // A SPENT budget is not a normal form, and the honest channel says which it is. Zero
    // fuel is the sharpest form of the same state: the old shape returned `Some(term)`
    // here, byte-identical to the answer for a value.
    assert_eq!(
        syn_eval_checked(&c, 0, &two_steps),
        Err(Some(two_steps.clone())),
        "no fuel means the evaluation never ran — not that the term was already a value"
    );
    assert_eq!(
        syn_eval_checked(&c, 0, &slit(7)),
        Err(Some(slit(7))),
        "…and it says so even when the term happens to BE a value"
    );
    // The object-language face keeps its semantics: a spent budget yields how far it got,
    // because there the budget is written in the term and the caller can see it.
    assert_eq!(
        try_syn_eval_reduce(&c, 0, &two_steps),
        Some(two_steps.clone())
    );
    assert_eq!(try_syn_eval_reduce(&c, 8, &two_steps), Some(slit(7)));
}

/// The reflected object language must model the kernel's OWN `div`/`mod`. If it truncated
/// while the kernel rounded the other way, `try_dcompute_conclude` would certify
/// `Eq Int (div (-7) 2) (-3)` — false under the kernel's arithmetic.
#[test]
fn the_reflected_arithmetic_is_the_kernels_own_euclidean_division() {
    let c = ctx();
    let slit = |n: i64| app(g("SLit"), Term::Lit(Literal::Int(n)));
    let sname = |s: &str| app(g("SName"), Term::Lit(Literal::Text(s.to_string())));
    let sapp = |f: Term, x: Term| app(app(g("SApp"), f), x);
    for (a, b, q, r) in [
        (-7i64, 2i64, -4i64, 1i64),
        (7, -2, -3, 1),
        (-7, -2, 4, 1),
        (5, 0, 0, 5),
    ] {
        let d = sapp(sapp(sname("div"), slit(a)), slit(b));
        let m = sapp(sapp(sname("mod"), slit(a)), slit(b));
        assert_eq!(
            syn_eval_checked(&c, 8, &d),
            Ok(slit(q)),
            "reflected div({a},{b})"
        );
        assert_eq!(
            syn_eval_checked(&c, 8, &m),
            Ok(slit(r)),
            "reflected mod({a},{b})"
        );
        // …and it agrees with the kernel itself, which is the property that matters.
        let kd = normalize(
            &c,
            &app(
                app(g("div"), Term::Lit(Literal::Int(a))),
                Term::Lit(Literal::Int(b)),
            ),
        );
        assert_eq!(kd, Term::Lit(Literal::Int(q)), "kernel div({a},{b})");
    }
}
