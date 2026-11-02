//! Differential coverage for the SECOND kernel's REDUCTION machinery (`recheck.rs`
//! `try_builtin`/`whnf`/`def_eq`). The existing double_check tests type-CHECK terms but rarely force
//! the re-checker to REDUCE — in particular, a `refl Int r` proof infers `Eq Int r r`, so the
//! arithmetic in the *claim* is reduced only by the MAIN kernel's `is_subtype`, never by recheck.
//!
//! The trick here: put the arithmetic in a COERCION the application type-check must resolve.
//! `(λ(h : Eq Int (op a b) r). h) (refl Int r)` type-checks iff `op a b ≡ r`, so BOTH kernels must
//! reduce `op a b` — recheck via `try_builtin`, the main kernel via `try_primitive_reduce`. A mutated
//! recheck arithmetic op computes a different value ⇒ its def-eq fails ⇒ it REJECTS a proof the main
//! kernel accepts ⇒ `double_check` returns `Disagree`. So every arm of `try_builtin` is pinned.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    double_check, infer_type, normalize, BigInt, Context, DoubleCheck, Literal, Term,
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
fn apps(f: Term, xs: &[Term]) -> Term {
    xs.iter().fold(f, |a, x| app(a, x.clone()))
}
fn arrow(a: Term, b: Term) -> Term {
    Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn lam(p: &str, ty: Term, body: Term) -> Term {
    Term::Lambda {
        param: p.to_string(),
        param_type: Box::new(ty),
        body: Box::new(body),
    }
}
fn int(n: i64) -> Term {
    Term::Lit(Literal::Int(n))
}
fn big(k: usize) -> Term {
    Term::Lit(Literal::BigInt(
        BigInt::parse_decimal(&format!("1{}", "0".repeat(k))).unwrap(),
    ))
}

/// Prelude + the ground operators, each typed so `op a b` is a well-formed redex the kernel
/// reduces by primitive arithmetic.
fn ctx_with_ops() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    let intt = g("Int");
    let binop = arrow(intt.clone(), arrow(intt.clone(), intt.clone()));
    let cmp = arrow(intt.clone(), arrow(intt.clone(), g("Bool")));
    for op in ["add", "sub", "mul", "div", "mod"] {
        ctx.add_declaration(op, binop.clone());
    }
    for op in ["le", "lt", "ge", "gt"] {
        ctx.add_declaration(op, cmp.clone());
    }
    ctx
}

/// `(λ(h : Eq T (redex) result). h) (refl T result)` — its well-typedness forces BOTH kernels to
/// reduce `redex` to `result`. Returns the coercion application.
fn coercion(ty: Term, redex: Term, result: Term) -> Term {
    let claim = apps(g("Eq"), &[ty.clone(), redex, result.clone()]);
    let refl_proof = apps(g("refl"), &[ty, result]);
    app(lam("h", claim, v("h")), refl_proof)
}

fn assert_both_kernels_agree(ctx: &Context, redex: Term, result: Term, ty: Term, label: &str) {
    // Self-check: the MAIN kernel actually reduces the redex to the result (so the coercion is
    // well-typed and the test is not vacuous).
    assert_eq!(
        normalize(ctx, &redex),
        result.clone(),
        "main kernel reduces {label}"
    );
    // The re-checker must agree — a mutated `try_builtin` arm makes it Disagree.
    let term = coercion(ty, redex, result);
    assert_eq!(
        double_check(ctx, &term),
        DoubleCheck::Agreed,
        "both kernels must certify the {label} coercion (recheck's try_builtin must match)",
    );
}

#[test]
fn recheck_evaluates_integer_arithmetic_exactly_like_the_main_kernel() {
    let ctx = ctx_with_ops();
    let i = g("Int");
    // i64 fast path — each op with a checkable result.
    assert_both_kernels_agree(
        &ctx,
        apps(g("add"), &[int(3), int(5)]),
        int(8),
        i.clone(),
        "add 3 5 = 8",
    );
    assert_both_kernels_agree(
        &ctx,
        apps(g("sub"), &[int(10), int(4)]),
        int(6),
        i.clone(),
        "sub 10 4 = 6",
    );
    assert_both_kernels_agree(
        &ctx,
        apps(g("mul"), &[int(6), int(7)]),
        int(42),
        i.clone(),
        "mul 6 7 = 42",
    );
    assert_both_kernels_agree(
        &ctx,
        apps(g("div"), &[int(20), int(3)]),
        int(6),
        i.clone(),
        "div 20 3 = 6",
    );
    assert_both_kernels_agree(
        &ctx,
        apps(g("mod"), &[int(20), int(3)]),
        int(2),
        i.clone(),
        "mod 20 3 = 2",
    );
    // Negatives (sign handling in every op).
    assert_both_kernels_agree(
        &ctx,
        apps(g("add"), &[int(-3), int(5)]),
        int(2),
        i.clone(),
        "add -3 5 = 2",
    );
    assert_both_kernels_agree(
        &ctx,
        apps(g("mul"), &[int(-6), int(7)]),
        int(-42),
        i.clone(),
        "mul -6 7 = -42",
    );
}

#[test]
fn recheck_evaluates_bignum_arithmetic_exactly_like_the_main_kernel() {
    let ctx = ctx_with_ops();
    let i = g("Int");
    // BigInt path (operands and/or result exceed i64) — the independent arbitrary-precision copy.
    assert_both_kernels_agree(
        &ctx,
        apps(g("mul"), &[big(30), big(30)]),
        big(60),
        i.clone(),
        "10^30 * 10^30 = 10^60",
    );
    assert_both_kernels_agree(
        &ctx,
        apps(g("add"), &[big(30), big(30)]),
        Term::Lit(Literal::BigInt(
            BigInt::parse_decimal(&format!("2{}", "0".repeat(30))).unwrap(),
        )),
        i.clone(),
        "10^30 + 10^30 = 2·10^30",
    );
    assert_both_kernels_agree(
        &ctx,
        apps(g("div"), &[big(60), big(30)]),
        big(30),
        i.clone(),
        "10^60 / 10^30 = 10^30",
    );
}

#[test]
fn recheck_evaluates_integer_comparisons_exactly_like_the_main_kernel() {
    let ctx = ctx_with_ops();
    let b = g("Bool");
    // Each comparison, both truth values (pins le/lt/ge/gt AND their true/false arms).
    assert_both_kernels_agree(
        &ctx,
        apps(g("le"), &[int(3), int(5)]),
        g("true"),
        b.clone(),
        "le 3 5 = true",
    );
    assert_both_kernels_agree(
        &ctx,
        apps(g("le"), &[int(5), int(3)]),
        g("false"),
        b.clone(),
        "le 5 3 = false",
    );
    assert_both_kernels_agree(
        &ctx,
        apps(g("lt"), &[int(3), int(5)]),
        g("true"),
        b.clone(),
        "lt 3 5 = true",
    );
    assert_both_kernels_agree(
        &ctx,
        apps(g("lt"), &[int(5), int(5)]),
        g("false"),
        b.clone(),
        "lt 5 5 = false",
    );
    assert_both_kernels_agree(
        &ctx,
        apps(g("ge"), &[int(5), int(3)]),
        g("true"),
        b.clone(),
        "ge 5 3 = true",
    );
    assert_both_kernels_agree(
        &ctx,
        apps(g("ge"), &[int(3), int(5)]),
        g("false"),
        b.clone(),
        "ge 3 5 = false",
    );
    assert_both_kernels_agree(
        &ctx,
        apps(g("gt"), &[int(5), int(3)]),
        g("true"),
        b.clone(),
        "gt 5 3 = true",
    );
    assert_both_kernels_agree(
        &ctx,
        apps(g("gt"), &[int(3), int(5)]),
        g("false"),
        b.clone(),
        "gt 3 5 = false",
    );
    // BigInt comparison path.
    assert_both_kernels_agree(
        &ctx,
        apps(g("lt"), &[big(30), big(60)]),
        g("true"),
        b.clone(),
        "10^30 < 10^60",
    );
}

// --- def_eq / whnf / subst / shift: reduction + congruence, forced through coercions ---

fn nat_ty() -> Term {
    g("Nat")
}
fn zero() -> Term {
    g("Zero")
}
fn succ(t: Term) -> Term {
    app(g("Succ"), t)
}
/// `Succ^n Zero`.
fn succ_n(n: u64) -> Term {
    (0..n).fold(zero(), |t, _| succ(t))
}
fn nat_lit(n: u64) -> Term {
    Term::Lit(Literal::Nat(BigInt::parse_decimal(&n.to_string()).unwrap()))
}

/// The coercion `(λh:Eq T redex result. h) (refl T result)` MUST type-check (redex ≡ result) and
/// both kernels must certify it — a mutated recheck def-eq/reduction arm makes it Disagree.
fn assert_coercion_agrees(ctx: &Context, ty: Term, redex: Term, result: Term, label: &str) {
    let term = coercion(ty, redex, result);
    assert!(
        infer_type(ctx, &term).is_ok(),
        "the {label} coercion is well-typed in the main kernel"
    );
    assert_eq!(
        double_check(ctx, &term),
        DoubleCheck::Agreed,
        "both kernels must certify the {label} coercion",
    );
}

/// An ILL-TYPED coercion (redex ≢ result): BOTH kernels must REJECT it, so double_check is Agreed
/// (Err, Err). A congruence mutant that weakens `&&` to `||` makes recheck wrongly ACCEPT ⇒ Disagree.
fn assert_ill_typed_agrees(ctx: &Context, ty: Term, redex: Term, result: Term, label: &str) {
    let term = coercion(ty, redex, result);
    assert!(
        infer_type(ctx, &term).is_err(),
        "the {label} coercion MUST be ill-typed (a real mismatch)"
    );
    assert_eq!(
        double_check(ctx, &term),
        DoubleCheck::Agreed,
        "both kernels must REJECT the ill-typed {label} coercion (recheck must not wrongly accept)",
    );
}

#[test]
fn recheck_nat_peano_bridge_matches_the_main_kernel() {
    // `Nat(n)` literal ≡ `Succ^n Zero`, both directions — pins def_eq's Peano arm + db_nat_peano_*.
    let ctx = {
        let mut c = Context::new();
        StandardLibrary::register(&mut c);
        c
    };
    assert_eq!(
        normalize(&ctx, &succ_n(3)),
        succ_n(3),
        "Succ^3 Zero is already normal"
    );
    assert_coercion_agrees(&ctx, nat_ty(), succ_n(2), nat_lit(2), "Nat(2) ≡ Succ² Zero");
    assert_coercion_agrees(&ctx, nat_ty(), nat_lit(3), succ_n(3), "Succ³ Zero ≡ Nat(3)");
    assert_coercion_agrees(&ctx, nat_ty(), nat_lit(0), zero(), "Nat(0) ≡ Zero");
    // A genuine inequality: Succ² Zero ≢ Nat(3). Ill-typed ⇒ both reject; the Nat arm must not
    // wrongly equate different counts.
    assert_ill_typed_agrees(&ctx, nat_ty(), succ_n(2), nat_lit(3), "Succ² Zero ≢ Nat(3)");
}

#[test]
fn recheck_beta_reduction_matches_the_main_kernel() {
    // β-redexes force recheck's subst/shift/whnf. `(λx:Nat. x) Zero ≡ Zero`.
    let ctx = {
        let mut c = Context::new();
        StandardLibrary::register(&mut c);
        c
    };
    let id = lam("x", nat_ty(), v("x"));
    assert_eq!(
        normalize(&ctx, &app(id.clone(), zero())),
        zero(),
        "(λx.x) Zero = Zero"
    );
    assert_coercion_agrees(
        &ctx,
        nat_ty(),
        app(id.clone(), zero()),
        zero(),
        "(λx.x) Zero ≡ Zero",
    );
    // A binder OVER a redex — the substituted value must be shifted under the outer λ.
    // ((λf:Nat→Nat. λy:Nat. f y) Succ) Zero  ≡  Succ Zero.
    let twice = lam(
        "f",
        arrow(nat_ty(), nat_ty()),
        lam("y", nat_ty(), app(v("f"), v("y"))),
    );
    let applied = app(app(twice, g("Succ")), zero());
    assert_eq!(
        normalize(&ctx, &applied),
        succ(zero()),
        "nested β = Succ Zero"
    );
    assert_coercion_agrees(
        &ctx,
        nat_ty(),
        applied,
        succ(zero()),
        "nested β ≡ Succ Zero",
    );
}

#[test]
fn recheck_eta_and_congruence_match_the_main_kernel() {
    // η: `Succ ≡ λx:Nat. Succ x` — pins the def_eq η-guards (one side λ, the other not).
    let ctx = {
        let mut c = Context::new();
        StandardLibrary::register(&mut c);
        c
    };
    let nat2nat = arrow(nat_ty(), nat_ty());
    let eta = lam("x", nat_ty(), succ(v("x")));
    assert_coercion_agrees(
        &ctx,
        nat2nat.clone(),
        g("Succ"),
        eta.clone(),
        "Succ ≡ λx. Succ x (η)",
    );
    // Lambda congruence — identical λs agree; differing λs must NOT (pins the `&&` in the Lam arm).
    assert_coercion_agrees(
        &ctx,
        nat2nat.clone(),
        eta.clone(),
        eta.clone(),
        "λx. Succ x ≡ itself",
    );
    assert_ill_typed_agrees(
        &ctx,
        nat2nat,
        lam("x", nat_ty(), v("x")),
        eta,
        "λx.x ≢ λx.Succ x",
    );
    // Application congruence — `Succ Zero ≢ Succ (Succ Zero)`: same head, different arg. Pins the
    // `def_eq(f1,f2) && def_eq(a1,a2)` conjunction (a `||` mutant would equate them on the head alone).
    assert_ill_typed_agrees(
        &ctx,
        nat_ty(),
        succ(zero()),
        succ(succ(zero())),
        "Succ Zero ≢ Succ² Zero",
    );
}
