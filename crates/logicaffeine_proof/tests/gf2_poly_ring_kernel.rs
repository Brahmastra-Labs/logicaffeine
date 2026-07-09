//! **The single-variable multilinear GF(2) polynomial ring, in the kernel.**
//!
//! `gf2_ring_kernel` proved the *coefficient field* laws over `Bool`. This lifts them to actual polynomials:
//! the multilinear ring `GF(2)[X]/(X²−X)`, whose elements are `a + bX` (`a`,`b : Bool`), built as a kernel
//! inductive `Poly1` with constructor `mk a b`. Addition is coefficient-wise `xor`; multiplication uses
//! `X² = X` (idempotent), so `(a₀+b₀X)(a₁+b₁X) = a₀a₁ + (a₀b₁ + b₁a₀... )X` collapses the `X²` term. Then two
//! facts are PROVEN as kernel theorems:
//!
//!   - `atom = one` as a GENUINE polynomial identity: `(1 + X) + X = 1` in the ring (both reduce to `mk true
//!     false`), by `Reflexivity` — upgrading the previous *pointwise-on-Bool* atom collapse to an equality of
//!     polynomials.
//!   - `∀p:Poly1. p · one = p` — the multiplicative identity for ARBITRARY polynomials, by case analysis on
//!     the two coefficients (the coefficient-field laws discharge each leaf).
//!
//! This is the base case of the `n`-variable lift: `MPoly(n+1) ≅ MPoly(n) × MPoly(n)` (a poly in `n+1` vars
//! is `p₀ + X_{n+1} p₁`), so the same shape — reduce `·one` on the pair to `·one` on each half via the IH —
//! carries the multiplicative identity to all `n` by induction on the variable count.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{infer_type, is_subtype, Context, Term, Universe};

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
fn xor(a: Term, b: Term) -> Term {
    app2(g("xor"), a, b)
}
fn and(a: Term, b: Term) -> Term {
    app2(g("and2"), a, b)
}
fn poly1() -> Term {
    g("Poly1")
}
/// `mk a b` — the polynomial `a + bX`.
fn mk(a: Term, b: Term) -> Term {
    app2(g("mk"), a, b)
}
/// `Eq Poly1 a b`.
fn eqp(a: Term, b: Term) -> Term {
    app(app2(g("Eq"), poly1(), a), b)
}
/// `refl Poly1 x`.
fn refl_p(x: Term) -> Term {
    app2(g("refl"), poly1(), x)
}
fn padd(a: Term, b: Term) -> Term {
    app2(g("padd"), a, b)
}
fn pmul(a: Term, b: Term) -> Term {
    app2(g("pmul"), a, b)
}
/// `one = 1 + 0·X`.
fn pone() -> Term {
    mk(tt(), ff())
}
/// `X = 0 + 1·X`.
fn px() -> Term {
    mk(ff(), tt())
}
/// The `Bool → Bool → Poly1` motive shape for a non-dependent `Poly1`-valued match.
fn poly_ret() -> Term {
    lam("_", poly1(), poly1())
}

fn gf2_poly_context() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);

    // GF(2) = (Bool, xor, and2), as in gf2_ring_kernel. Constructor order [true, false].
    let not_b = mtch(kvar("b"), lam("_", boolt(), boolt()), vec![ff(), tt()]);
    let xor_body = lam(
        "a",
        boolt(),
        lam("b", boolt(), mtch(kvar("a"), lam("_", boolt(), boolt()), vec![not_b, kvar("b")])),
    );
    ctx.add_definition("xor".to_string(), pi("a", boolt(), pi("b", boolt(), boolt())), xor_body);
    let and_body = lam(
        "a",
        boolt(),
        lam("b", boolt(), mtch(kvar("a"), lam("_", boolt(), boolt()), vec![kvar("b"), ff()])),
    );
    ctx.add_definition("and2".to_string(), pi("a", boolt(), pi("b", boolt(), boolt())), and_body);

    // Poly1 : Type — the multilinear ring GF(2)[X]/(X²−X), elements `mk a b` = a + bX.
    ctx.add_inductive("Poly1", Term::Sort(Universe::Type(0)));
    ctx.add_constructor("mk", "Poly1", pi("a", boolt(), pi("b", boolt(), poly1())));

    // padd (mk a1 b1) (mk a2 b2) = mk (a1⊕a2) (b1⊕b2) — coefficient-wise addition.
    let padd_inner = mtch(
        kvar("q"),
        poly_ret(),
        vec![lam(
            "a2",
            boolt(),
            lam("b2", boolt(), mk(xor(kvar("a1"), kvar("a2")), xor(kvar("b1"), kvar("b2")))),
        )],
    );
    let padd_body = lam(
        "p",
        poly1(),
        lam(
            "q",
            poly1(),
            mtch(kvar("p"), poly_ret(), vec![lam("a1", boolt(), lam("b1", boolt(), padd_inner))]),
        ),
    );
    ctx.add_definition("padd".to_string(), pi("p", poly1(), pi("q", poly1(), poly1())), padd_body);

    // pmul (mk a1 b1) (mk a2 b2) = mk (a1·a2) ((a1·b2) ⊕ (b1·a2) ⊕ (b1·b2)) — using X² = X.
    let hi = xor(
        xor(and(kvar("a1"), kvar("b2")), and(kvar("b1"), kvar("a2"))),
        and(kvar("b1"), kvar("b2")),
    );
    let pmul_inner = mtch(
        kvar("q"),
        poly_ret(),
        vec![lam("a2", boolt(), lam("b2", boolt(), mk(and(kvar("a1"), kvar("a2")), hi)))],
    );
    let pmul_body = lam(
        "p",
        poly1(),
        lam(
            "q",
            poly1(),
            mtch(kvar("p"), poly_ret(), vec![lam("a1", boolt(), lam("b1", boolt(), pmul_inner))]),
        ),
    );
    ctx.add_definition("pmul".to_string(), pi("p", poly1(), pi("q", poly1(), poly1())), pmul_body);
    ctx
}

fn proves(ctx: &Context, proof: &Term, law: &Term) -> bool {
    match infer_type(ctx, proof) {
        Ok(ty) => is_subtype(ctx, &ty, law) && is_subtype(ctx, law, &ty),
        Err(_) => false,
    }
}

#[test]
fn the_polynomial_ring_is_well_formed() {
    let ctx = gf2_poly_context();
    assert!(matches!(infer_type(&ctx, &g("padd")), Ok(Term::Pi { .. })), "padd : Poly1 → Poly1 → Poly1");
    assert!(matches!(infer_type(&ctx, &g("pmul")), Ok(Term::Pi { .. })), "pmul : Poly1 → Poly1 → Poly1");
    assert!(matches!(infer_type(&ctx, &pone()), Ok(_)), "one = mk true false : Poly1");
    assert!(matches!(infer_type(&ctx, &px()), Ok(_)), "X = mk false true : Poly1");
}

#[test]
fn atom_is_one_as_a_genuine_polynomial_identity() {
    let ctx = gf2_poly_context();
    // atom = (1 + X) + X, a real element of the ring; it must EQUAL one as polynomials (not just pointwise).
    let atom = padd(padd(pone(), px()), px());
    let law = eqp(atom.clone(), pone());
    // Both sides reduce to `mk true false`, so `refl Poly1 one` proves it.
    let proof = refl_p(pone());
    assert!(proves(&ctx, &proof, &law), "(1+X)+X = 1 as a polynomial identity — kernel-proven");
}

#[test]
fn multiplicative_identity_for_all_polynomials() {
    let ctx = gf2_poly_context();
    // ∀p:Poly1. pmul p one = p — the multiplicative identity for ARBITRARY polynomials, by case analysis on
    // the two coefficients (each leaf discharged by the coefficient-field laws, via computation).
    let law = pi("p", poly1(), eqp(pmul(kvar("p"), pone()), kvar("p")));

    // Inner match on b, for a fixed leading coefficient `af` (a concrete Bool term).
    let inner = |af: Term| {
        mtch(
            kvar("b"),
            lam("b", boolt(), eqp(pmul(mk(af.clone(), kvar("b")), pone()), mk(af.clone(), kvar("b")))),
            vec![refl_p(mk(af.clone(), tt())), refl_p(mk(af, ff()))],
        )
    };
    let proof = lam(
        "p",
        poly1(),
        mtch(
            kvar("p"),
            lam("p", poly1(), eqp(pmul(kvar("p"), pone()), kvar("p"))),
            vec![lam(
                "a",
                boolt(),
                lam(
                    "b",
                    boolt(),
                    mtch(
                        kvar("a"),
                        lam(
                            "a",
                            boolt(),
                            eqp(pmul(mk(kvar("a"), kvar("b")), pone()), mk(kvar("a"), kvar("b"))),
                        ),
                        vec![inner(tt()), inner(ff())],
                    ),
                ),
            )],
        ),
    );
    assert!(proves(&ctx, &proof, &law), "∀p. p · one = p — kernel-proven for all polynomials");
}

#[test]
fn a_false_polynomial_law_is_rejected() {
    let ctx = gf2_poly_context();
    // FALSE: ∀p. pmul p one = one (fails whenever p ≠ one). The case-analysis proof cannot type-check.
    let law = pi("p", poly1(), eqp(pmul(kvar("p"), pone()), pone()));
    let inner = |af: Term| {
        mtch(
            kvar("b"),
            lam("b", boolt(), eqp(pmul(mk(af.clone(), kvar("b")), pone()), pone())),
            vec![refl_p(mk(af.clone(), tt())), refl_p(mk(af, ff()))],
        )
    };
    let proof = lam(
        "p",
        poly1(),
        mtch(
            kvar("p"),
            lam("p", poly1(), eqp(pmul(kvar("p"), pone()), pone())),
            vec![lam(
                "a",
                boolt(),
                lam(
                    "b",
                    boolt(),
                    mtch(
                        kvar("a"),
                        lam("a", boolt(), eqp(pmul(mk(kvar("a"), kvar("b")), pone()), pone())),
                        vec![inner(tt()), inner(ff())],
                    ),
                ),
            )],
        ),
    );
    assert!(!proves(&ctx, &proof, &law), "a false polynomial law must be rejected by the kernel");
}
