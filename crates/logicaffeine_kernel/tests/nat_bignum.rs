//! Nat bignum acceleration (K6) — a `Literal::Nat(n)` is a compact form of `Succ^n Zero`.
//! The kernel bridges the two in `extract_constructor` (so a recursor computes on it) and
//! in `def_eq` (so `Nat(n) ≡ Succ^n Zero`), in BOTH kernels. These tests pin the bridge
//! against genuine Peano numerals exhaustively.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    defeq_for_test, double_check, infer_type, normalize, recheck, BigInt, Context, DoubleCheck,
    Literal, Term, Universe,
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
fn lam(p: &str, t: Term, b: Term) -> Term {
    Term::Lambda { param: p.to_string(), param_type: Box::new(t), body: Box::new(b) }
}
fn fix(name: &str, body: Term) -> Term {
    Term::Fix { name: name.to_string(), body: Box::new(body) }
}
fn match_(d: Term, motive: Term, cases: Vec<Term>) -> Term {
    Term::Match { discriminant: Box::new(d), motive: Box::new(motive), cases }
}
fn nat() -> Term {
    g("Nat")
}
fn succ(x: Term) -> Term {
    app(g("Succ"), x)
}
/// The genuine unary Peano numeral `Succ^n Zero`.
fn peano(n: u64) -> Term {
    (0..n).fold(g("Zero"), |acc, _| succ(acc))
}
/// The compact `Nat` literal for `n`.
fn nat_lit(n: u64) -> Term {
    Term::Lit(Literal::Nat(BigInt::from_i64(n as i64)))
}
fn std_ctx() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    ctx
}

#[test]
fn nat_literal_is_defeq_to_peano_exhaustively() {
    // For every n in 0..64: `Nat(n) ≡ Succ^n Zero` (both orientations), and `Nat(n)` is
    // NOT equal to `Succ^{n+1} Zero` — the bridge is exact, not approximate.
    let ctx = std_ctx();
    for n in 0u64..64 {
        assert!(defeq_for_test(&ctx, &nat_lit(n), &peano(n)), "Nat({n}) ≡ Succ^{n} Zero");
        assert!(defeq_for_test(&ctx, &peano(n), &nat_lit(n)), "symmetric at {n}");
        assert!(
            !defeq_for_test(&ctx, &nat_lit(n), &peano(n + 1)),
            "Nat({n}) must NOT equal Succ^{} Zero",
            n + 1
        );
        // A Nat literal is well-typed as `Nat`.
        assert_eq!(infer_type(&ctx, &nat_lit(n)).unwrap(), nat(), "Nat({n}) : Nat");
    }
}

#[test]
fn recursor_computes_on_nat_literals_agreeing_with_peano() {
    // `double : Nat → Nat` by structural recursion. It must compute identically whether the
    // argument is the compact `Nat(n)` or the unary `Succ^n Zero` — for both it yields
    // `Succ^{2n} Zero`. This exercises the `extract_constructor` bridge (match on `Nat(n)`).
    let ctx = std_ctx();
    let double = fix(
        "rec",
        lam(
            "n",
            nat(),
            match_(
                v("n"),
                lam("_", nat(), nat()),
                vec![g("Zero"), lam("k", nat(), succ(succ(app(v("rec"), v("k")))))],
            ),
        ),
    );
    for n in 0u64..24 {
        let on_lit = normalize(&ctx, &app(double.clone(), nat_lit(n)));
        let on_peano = normalize(&ctx, &app(double.clone(), peano(n)));
        assert_eq!(on_lit, on_peano, "double agrees on Nat({n}) and Succ^{n} Zero");
        assert!(
            defeq_for_test(&ctx, &on_lit, &peano(2 * n)),
            "double(Nat({n})) = Succ^{} Zero",
            2 * n
        );
    }
}

#[test]
fn matching_a_nat_literal_selects_the_right_branch() {
    // `isZero : Nat → Bool` — the match on a `Nat` literal picks `Zero`/`Succ` correctly.
    let ctx = std_ctx();
    let is_zero = lam(
        "n",
        nat(),
        match_(v("n"), lam("_", nat(), g("Bool")), vec![g("true"), lam("_", nat(), g("false"))]),
    );
    assert_eq!(normalize(&ctx, &app(is_zero.clone(), nat_lit(0))), g("true"), "isZero (Nat 0) = true");
    assert_eq!(normalize(&ctx, &app(is_zero.clone(), nat_lit(7))), g("false"), "isZero (Nat 7) = false");
    // And agrees with the Peano form.
    assert_eq!(normalize(&ctx, &app(is_zero.clone(), peano(7))), g("false"), "isZero (Succ^7 Zero) = false");
}

#[test]
fn nat_bridge_is_two_kernel_verified() {
    // A term that type-checks ONLY via the Peano bridge: a function expecting a proof of
    // `Eq Nat (Nat 3) (Succ^3 Zero)` is applied to `refl Nat (Succ^3 Zero)`, whose type
    // `Eq Nat (Succ^3 Zero) (Succ^3 Zero)` matches only because `Nat 3 ≡ Succ^3 Zero`. BOTH
    // kernels must accept it (the re-checker has its own copy of the bridge) — if either
    // lacked it, `double_check` would report Disagree.
    let ctx = std_ctx();
    let eq_ty = apps(g("Eq"), &[nat(), nat_lit(3), peano(3)]);
    let refl_proof = apps(g("refl"), &[nat(), peano(3)]);
    let coerce = app(lam("h", eq_ty, g("Zero")), refl_proof);

    assert_eq!(infer_type(&ctx, &coerce).unwrap(), nat(), "the coercion type-checks to Nat");
    match double_check(&ctx, &coerce) {
        DoubleCheck::Agreed => {}
        other => panic!("both kernels must agree via the Nat bridge, got {other:?}"),
    }
}

#[test]
fn negative_nat_literal_is_rejected_and_never_loops() {
    // AUDIT FIX: a `Nat` literal is non-negative. A negative one — only reachable from
    // untrusted serialized input — must (a) be REJECTED by both kernels' type inference,
    // and (b) never send the peel toward −∞: the bridge collapses `n ≤ 0` to `Zero`, so
    // `match`/`def_eq` TERMINATE instead of hanging the checker.
    let ctx = std_ctx();
    let neg = Term::Lit(Literal::Nat(BigInt::from_i64(-5)));

    assert!(infer_type(&ctx, &neg).is_err(), "main kernel rejects a negative Nat literal");
    assert!(recheck(&ctx, &neg).is_err(), "re-checker rejects a negative Nat literal");

    // Loop-safety: a match on it terminates (collapses to the Zero branch).
    let is_zero = lam(
        "n",
        nat(),
        match_(v("n"), lam("_", nat(), g("Bool")), vec![g("true"), lam("_", nat(), g("false"))]),
    );
    assert_eq!(normalize(&ctx, &app(is_zero, neg.clone())), g("true"), "negative Nat peels to Zero, no ∞ loop");
    // def_eq terminates too (does not hang).
    assert!(!defeq_for_test(&ctx, &neg, &peano(3)), "def_eq terminates on a negative Nat literal");
}

#[test]
fn large_nat_literal_stays_compact_and_computes() {
    // A Nat literal far past any unary representation (10^9) still type-checks, and
    // `isZero` on it is `false` by the ONE-step bridge — no 10^9-node expansion.
    let ctx = std_ctx();
    let huge = Term::Lit(Literal::Nat(BigInt::from_i64(1_000_000_000)));
    assert_eq!(infer_type(&ctx, &huge).unwrap(), nat(), "Nat(10^9) : Nat");
    let is_zero = lam(
        "n",
        nat(),
        match_(v("n"), lam("_", nat(), g("Bool")), vec![g("true"), lam("_", nat(), g("false"))]),
    );
    assert_eq!(normalize(&ctx, &app(is_zero, huge)), g("false"), "isZero (Nat 10^9) = false, no unary blowup");
}
