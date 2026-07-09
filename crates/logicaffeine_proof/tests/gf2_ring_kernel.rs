//! **GF(2) constructed in the kernel — the ring laws behind `ring_step` become theorems.**
//!
//! The `finite_randomness_kernel_integration` proof took the ring facts (`ring_step`, `atom = one`) as
//! *axioms* justified by the polynomial model. This file discharges their content into the kernel: GF(2) — the
//! two-element field the multilinear ring's coefficients live in — is built as a kernel inductive (on `Bool`),
//! its `+` (`xor`) and `·` (`and`) are DEFINED (computable via `Match`), and the ring laws are PROVEN as
//! kernel-checked theorems, not asserted:
//!
//!   - `and a true = a`                    — the multiplicative identity (the `·one` in `ring_step`)
//!   - `xor a a = false`                   — CHARACTERISTIC 2 (`a + a = 0`), the defining GF(2) fact
//!   - `xor a false = a`                   — the additive identity
//!   - `xor (xor true x) x = true`         — THE ATOM COLLAPSE `(1 ⊕ x) ⊕ x = 1`, i.e. `atom = one`
//!
//! Each theorem's proof is a `λ`+`match` whose motive IS the law statement; a successful `infer_type` (checked
//! against the explicit law with `is_subtype` both ways) means the kernel verified it. A false law's proof is
//! REJECTED by `infer_type` — soundness is kernel-enforced.
//!
//! Faithfulness: the atom collapse `(1+X)+X = 1` holds in the multilinear ring because the `X`-coefficient
//! cancels (`1+1 = 0` in GF(2)); on the Boolean cube a multilinear polynomial equals its evaluation function,
//! so the pointwise `∀x:Bool. (1⊕x)⊕x = 1` proven here IS that polynomial identity. What remains for full-full
//! integration is the `n`-variable polynomial ring as a kernel inductive with a computable `·` (so the
//! *multiplicative* identity lifts from the coefficient field to arbitrary polynomials) — the coefficient-field
//! laws proven here are its foundation.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, is_subtype, Context, Term, Universe};

// ── kernel term helpers ──────────────────────────────────────────────────────────────────────────────
fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn kvar(n: &str) -> Term {
    Term::Var(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn app2(f: Term, x: Term, y: Term) -> Term {
    app(app(f, x), y)
}
fn lam(param: &str, ty: Term, body: Term) -> Term {
    Term::Lambda { param: param.to_string(), param_type: Box::new(ty), body: Box::new(body) }
}
fn pi(param: &str, ty: Term, body: Term) -> Term {
    Term::Pi { param: param.to_string(), param_type: Box::new(ty), body_type: Box::new(body) }
}
fn mtch(disc: Term, motive: Term, cases: Vec<Term>) -> Term {
    Term::Match { discriminant: Box::new(disc), motive: Box::new(motive), cases }
}
fn boolt() -> Term {
    g("Bool")
}
fn tt() -> Term {
    g("true")
}
fn ff() -> Term {
    g("false")
}
/// `Eq Bool a b : Prop`.
fn eqb(a: Term, b: Term) -> Term {
    app(app2(g("Eq"), boolt(), a), b)
}
/// `refl Bool x : Eq Bool x x`.
fn refl_b(x: Term) -> Term {
    app2(g("refl"), boolt(), x)
}
fn xor(a: Term, b: Term) -> Term {
    app2(g("xor"), a, b)
}
fn and(a: Term, b: Term) -> Term {
    app2(g("and2"), a, b)
}

/// A `Bool → Bool → Bool` type.
fn binop_ty() -> Term {
    pi("a", boolt(), pi("b", boolt(), boolt()))
}

/// The kernel context with GF(2) = (Bool, xor, and2) defined and computable.
fn gf2_context() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx); // Bool : Type, true/false, Eq, refl

    // xor a b : characteristic-2 addition. Bool constructor order is [true, false].
    //   xor a b = match a { true => (match b { true => false, false => true }),  false => b }
    let not_b = mtch(kvar("b"), lam("_", boolt(), boolt()), vec![ff(), tt()]);
    let xor_body = lam(
        "a",
        boolt(),
        lam("b", boolt(), mtch(kvar("a"), lam("_", boolt(), boolt()), vec![not_b, kvar("b")])),
    );
    ctx.add_definition("xor".to_string(), binop_ty(), xor_body);

    // and2 a b : multiplication.  and2 a b = match a { true => b, false => false }
    let and_body = lam(
        "a",
        boolt(),
        lam("b", boolt(), mtch(kvar("a"), lam("_", boolt(), boolt()), vec![kvar("b"), ff()])),
    );
    ctx.add_definition("and2".to_string(), binop_ty(), and_body);
    ctx
}

/// A universally-quantified law `∀v:Bool. Eq Bool (lhs v) (rhs v)`, its explicit type, and a proof by
/// `Bool` case analysis with the law as the match motive (`refl` per case; the kernel reduces `xor`/`and2`).
fn law(v: &str, lhs: impl Fn(Term) -> Term, rhs: impl Fn(Term) -> Term, case_true: Term, case_false: Term) -> (Term, Term) {
    let stmt = pi(v, boolt(), eqb(lhs(kvar(v)), rhs(kvar(v))));
    let motive = lam(v, boolt(), eqb(lhs(kvar(v)), rhs(kvar(v))));
    let proof = lam(v, boolt(), mtch(kvar(v), motive, vec![case_true, case_false]));
    (stmt, proof)
}

/// The kernel verifies `proof : law` — infer its type and check it is definitionally the law statement.
fn proves(ctx: &Context, proof: &Term, law: &Term) -> bool {
    match infer_type(ctx, proof) {
        Ok(ty) => is_subtype(ctx, &ty, law) && is_subtype(ctx, law, &ty),
        Err(_) => false,
    }
}

#[test]
fn gf2_ring_laws_are_kernel_theorems() {
    let ctx = gf2_context();

    // `xor` and `and2` are well-typed Bool operations (the construction itself type-checks).
    assert!(matches!(infer_type(&ctx, &g("xor")), Ok(Term::Pi { .. })), "xor : Bool → Bool → Bool");
    assert!(matches!(infer_type(&ctx, &g("and2")), Ok(Term::Pi { .. })), "and2 : Bool → Bool → Bool");

    // Multiplicative identity: ∀a. and2 a true = a.  (rhs is `a`, so the case witnesses are the constructors.)
    let (mul_one, p) = law("a", |a| and(a.clone(), tt()), |a| a, refl_b(tt()), refl_b(ff()));
    assert!(proves(&ctx, &p, &mul_one), "and2 a true = a is a kernel theorem");

    // Characteristic 2: ∀a. xor a a = false.  (both cases reduce to false.)
    let (add_self, p) = law("a", |a| xor(a.clone(), a), |_| ff(), refl_b(ff()), refl_b(ff()));
    assert!(proves(&ctx, &p, &add_self), "xor a a = false (characteristic 2) is a kernel theorem");

    // Additive identity: ∀a. xor a false = a.
    let (add_zero, p) = law("a", |a| xor(a, ff()), |a| a, refl_b(tt()), refl_b(ff()));
    assert!(proves(&ctx, &p, &add_zero), "xor a false = a is a kernel theorem");

    // THE ATOM COLLAPSE: ∀x. xor (xor true x) x = true — i.e. (1 ⊕ x) ⊕ x = 1, so `atom = one`. This is the
    // characteristic-2 fact that was a model-checked axiom (`ring_step`/`atom = one`), now kernel-PROVEN.
    let (atom_one, p) =
        law("x", |x| xor(xor(tt(), x.clone()), x), |_| tt(), refl_b(tt()), refl_b(tt()));
    assert!(proves(&ctx, &p, &atom_one), "the atom (1⊕x)⊕x collapses to 1 — kernel-proven, not assumed");
}

#[test]
fn a_false_gf2_law_is_rejected_by_the_kernel() {
    let ctx = gf2_context();
    // FALSE law: ∀a. and2 a true = true (fails at a = false: and2 false true = false ≠ true). The same
    // case-analysis proof shape cannot type-check — at the false case, `refl` would need type
    // `Eq Bool false true`, which the kernel rejects (false ≢ true). Soundness is kernel-enforced.
    let (bogus, p) = law("a", |a| and(a, tt()), |_| tt(), refl_b(tt()), refl_b(ff()));
    assert!(!proves(&ctx, &p, &bogus), "a false law must NOT be provable — the kernel rejects it");
    // And even a well-typed proof of a DIFFERENT statement is not accepted as this false law.
    assert!(infer_type(&ctx, &p).is_err() || !is_subtype(&ctx, &infer_type(&ctx, &p).unwrap(), &bogus),
        "no term certifies the false law");
    let _ = Universe::Type(0);
}
