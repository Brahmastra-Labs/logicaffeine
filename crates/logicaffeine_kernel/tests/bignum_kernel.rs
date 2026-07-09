//! Bignum kernel arithmetic (K6) — arbitrary-precision integer computation, so
//! `refl`-by-computation survives past 2⁶³. `Literal::BigInt` is a PARALLEL, CANONICAL
//! representation: only values that overflow `i64` use it, small numbers keep the fast
//! `i64` path, and every integer has exactly one literal — so definitional equality stays
//! sound. Serialized as a decimal string, appended to the enum so old certificates load
//! unchanged.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    double_check, infer_type, is_subtype, normalize, BigInt, Context, DoubleCheck, Literal, Term,
};

fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn apps(f: Term, xs: &[Term]) -> Term {
    xs.iter().fold(f, |a, x| app(a, x.clone()))
}
fn arrow(a: Term, b: Term) -> Term {
    Term::Pi { param: "_".to_string(), param_type: Box::new(a), body_type: Box::new(b) }
}
fn big(lit: BigInt) -> Term {
    Term::Lit(Literal::BigInt(lit))
}
fn int(n: i64) -> Term {
    Term::Lit(Literal::Int(n))
}
/// 10^k as a `BigInt`.
fn ten_pow(k: usize) -> BigInt {
    BigInt::parse_decimal(&format!("1{}", "0".repeat(k))).unwrap()
}

/// Standard prelude plus the ground integer operators `mul`/`add`/`sub`/`le` typed
/// `Int → Int → …` (their reduction is the kernel's primitive arithmetic).
fn ctx_with_ops() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    let int = g("Int");
    let binop = arrow(int.clone(), arrow(int.clone(), int.clone()));
    for op in ["mul", "add", "sub"] {
        ctx.add_declaration(op, binop.clone());
    }
    ctx.add_declaration("le", arrow(int.clone(), arrow(int, g("Bool"))));
    ctx
}

#[test]
fn refl_on_huge_arithmetic() {
    // THE HEADLINE: `10^30 * 10^30 = 10^60` proven BY `refl` — the kernel reduces the
    // product exactly, past the `i64` ceiling, so `refl Int 10^60` checks against
    // `Eq Int (mul 10^30 10^30) 10^60`. On the old i64-only kernel this multiplication
    // overflowed and got stuck; here it computes.
    let ctx = ctx_with_ops();
    let huge = big(ten_pow(30));
    let prod = apps(g("mul"), &[huge.clone(), huge]);
    let ten60 = big(ten_pow(60));

    // (a) It reduces to 10^60 exactly.
    assert_eq!(normalize(&ctx, &prod), ten60.clone(), "10^30 * 10^30 = 10^60 by reduction");

    // (b) `refl Int 10^60 : Eq Int (mul 10^30 10^30) 10^60` — proof by computation.
    let refl_proof = apps(g("refl"), &[g("Int"), ten60.clone()]);
    let claim = apps(g("Eq"), &[g("Int"), prod, ten60]);
    let proof_ty = infer_type(&ctx, &refl_proof).expect("refl type-checks");
    assert!(
        is_subtype(&ctx, &proof_ty, &claim),
        "refl must prove 10^30*10^30 = 10^60 by computation.\n proof_ty = {proof_ty}\n claim = {claim}"
    );

    // (c) Both kernels certify that proof (each reduces the bignum product independently).
    match double_check(&ctx, &refl_proof) {
        DoubleCheck::Agreed => {}
        other => panic!("both kernels must agree on the bignum refl proof, got {other:?}"),
    }
}

#[test]
fn i64_overflow_promotes_to_bigint() {
    // `10^10 * 10^10 = 10^20` — the operands fit `i64` but the product does not. The fast
    // path overflows and the kernel redoes it exactly, yielding the BigInt result rather
    // than getting stuck.
    let ctx = ctx_with_ops();
    let e10 = int(10_000_000_000); // 10^10, fits i64
    let prod = apps(g("mul"), &[e10.clone(), e10]);
    assert_eq!(normalize(&ctx, &prod), big(ten_pow(20)), "10^10 * 10^10 promotes to BigInt 10^20");
}

#[test]
fn results_are_canonicalized_no_bigint_that_fits_i64() {
    // Canonical-form invariant: a bignum computation whose RESULT fits `i64` is demoted
    // back to `Int` — `10^20 - 10^20` is `Int(0)`, never `BigInt(0)`. Without this a value
    // could have two literals and definitional equality would break.
    let ctx = ctx_with_ops();
    let a = big(ten_pow(20));
    let diff = apps(g("sub"), &[a.clone(), a]);
    assert_eq!(normalize(&ctx, &diff), int(0), "10^20 - 10^20 = Int(0), canonicalized");

    // And an `Int`/`BigInt` mix computes and canonicalizes: 10^20 - (10^20 - 5) = 5 : Int.
    let inner = apps(g("sub"), &[big(ten_pow(20)), int(5)]); // 10^20 - 5 (a BigInt)
    let outer = apps(g("sub"), &[big(ten_pow(20)), inner]);
    assert_eq!(normalize(&ctx, &outer), int(5), "mixed Int/BigInt arithmetic canonicalizes to Int(5)");
}

#[test]
fn bignum_comparison_decides_by_computation() {
    // `le 10^30 10^60` reduces to `true`, `le 10^60 10^30` to `false` — exact ordering of
    // arbitrary-precision integers, the substrate for large linear-arithmetic certificates.
    let ctx = ctx_with_ops();
    let lo = big(ten_pow(30));
    let hi = big(ten_pow(60));
    assert_eq!(normalize(&ctx, &apps(g("le"), &[lo.clone(), hi.clone()])), g("true"), "10^30 ≤ 10^60");
    assert_eq!(normalize(&ctx, &apps(g("le"), &[hi, lo])), g("false"), "¬(10^60 ≤ 10^30)");
}

#[test]
fn small_arithmetic_stays_on_the_fast_i64_path() {
    // Regression: ordinary small arithmetic is unchanged — `2 * 3 = 6` as `Int`, never a
    // BigInt. The bignum path must not perturb the common case.
    let ctx = ctx_with_ops();
    assert_eq!(normalize(&ctx, &apps(g("mul"), &[int(2), int(3)])), int(6), "2 * 3 = Int(6)");
    assert_eq!(normalize(&ctx, &apps(g("add"), &[int(40), int(2)])), int(42), "40 + 2 = Int(42)");
}

// --- Serde format: appended variant, old certificates unchanged ----------------

#[cfg(feature = "serde")]
mod serde_format {
    use super::*;
    use logicaffeine_kernel::certificate::Certificate;

    #[test]
    fn existing_literal_encoding_is_byte_unchanged() {
        // The careful part: adding `BigInt` must NOT change how existing literals encode,
        // or every prior certificate would fail to load. `Int(5)` is still `{"Int":5}`.
        assert_eq!(serde_json::to_string(&Literal::Int(5)).unwrap(), r#"{"Int":5}"#);
        assert_eq!(serde_json::to_string(&Literal::Text("x".into())).unwrap(), r#"{"Text":"x"}"#);
    }

    #[test]
    fn old_format_certificate_still_deserializes() {
        // A hand-written pre-BigInt certificate JSON (only `Int` literals) must still load
        // and re-check — proving the format extension is backward compatible.
        let json = r#"{
            "proof_term": {"Lit":{"Int":42}},
            "claimed_type": {"Global":"Int"},
            "prelude_version": "logos-coc-1"
        }"#;
        let cert: Certificate = serde_json::from_str(json).expect("old-format certificate loads");
        assert_eq!(cert.proof_term, Term::Lit(Literal::Int(42)));
        assert_eq!(cert.claimed_type, Term::Global("Int".to_string()));
    }

    #[test]
    fn bigint_literal_round_trips_as_decimal_string() {
        // The new variant serializes as a decimal string (stable across the internal limb
        // layout) and round-trips exactly.
        let lit = Literal::BigInt(ten_pow(60));
        let json = serde_json::to_string(&lit).unwrap();
        assert_eq!(json, format!(r#"{{"BigInt":"1{}"}}"#, "0".repeat(60)));
        let back: Literal = serde_json::from_str(&json).unwrap();
        assert_eq!(back, lit, "BigInt literal round-trips through serde");
    }

    #[test]
    fn nat_literal_round_trips_as_decimal_string() {
        // The `Nat` variant is likewise a decimal string, appended so it perturbs nothing
        // before it.
        let lit = Literal::Nat(BigInt::from_i64(1_000_000));
        let json = serde_json::to_string(&lit).unwrap();
        assert_eq!(json, r#"{"Nat":"1000000"}"#);
        let back: Literal = serde_json::from_str(&json).unwrap();
        assert_eq!(back, lit, "Nat literal round-trips through serde");
    }
}
