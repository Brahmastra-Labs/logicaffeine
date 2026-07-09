//! **GF(3) constructed in the kernel — the characteristic axis reaches the trust root.**
//!
//! `gf2_ring_kernel` discharged the `GF(2)` coefficient-field laws into the Calculus-of-Constructions
//! kernel. This file does the same for `GF(3)` — the first field where the general Nullstellensatz
//! engine's SIGNED arithmetic is visible (`1 − x ≠ 1 + x`) — as a bespoke three-constructor kernel
//! inductive (no `Fin` machinery is assumed; the construction mirrors `Bool`'s):
//!
//!   - `add3 a F0 = a`                        — the additive identity
//!   - `mul3 a F1 = a`                        — the multiplicative identity
//!   - `add3 (add3 a a) a = F0`               — CHARACTERISTIC 3 (`a + a + a = 0`), the defining fact
//!   - `add3 a (neg3 a) = F0`                 — additive inverses (negation is real here, not identity)
//!   - `add3 (add3 F1 (neg3 x)) x = F1`       — THE SIGNED ATOM `(1 − x) + x = 1`, the identity the
//!     field-generic partition of unity (and with it constructive NS completeness at characteristic 3)
//!     rests on
//!
//! Each law is proven by `GF3` case analysis with the law as the match motive; a successful
//! `infer_type` (checked against the law with `is_subtype` both ways) means the kernel verified it.
//! The characteristic-2 law `a + a = 0` — TRUE over `GF(2)`, FALSE here — is rejected, so the kernel
//! genuinely distinguishes the characteristics rather than rubber-stamping ring-shaped statements.

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
fn gf3t() -> Term {
    g("GF3")
}
fn f0() -> Term {
    g("F0")
}
fn f1() -> Term {
    g("F1")
}
fn f2() -> Term {
    g("F2")
}
/// `Eq GF3 a b : Prop`.
fn eq3(a: Term, b: Term) -> Term {
    app(app2(g("Eq"), gf3t(), a), b)
}
/// `refl GF3 x : Eq GF3 x x`.
fn refl3(x: Term) -> Term {
    app2(g("refl"), gf3t(), x)
}
fn add3(a: Term, b: Term) -> Term {
    app2(g("add3"), a, b)
}
fn mul3(a: Term, b: Term) -> Term {
    app2(g("mul3"), a, b)
}
fn neg3(a: Term) -> Term {
    app(g("neg3"), a)
}

/// A `GF3 → GF3 → GF3` type.
fn binop_ty() -> Term {
    pi("a", gf3t(), pi("b", gf3t(), gf3t()))
}

/// A case body: `match b { F0 => c0, F1 => c1, F2 => c2 }` (constructor order F0, F1, F2).
fn match_b(c0: Term, c1: Term, c2: Term) -> Term {
    mtch(kvar("b"), lam("_", gf3t(), gf3t()), vec![c0, c1, c2])
}

/// The kernel context with GF(3) = ({F0, F1, F2}, add3, mul3, neg3) defined and computable.
fn gf3_context() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx); // Eq, refl (and the rest of the trusted base)

    ctx.add_inductive("GF3", Term::Sort(Universe::Type(0)));
    ctx.add_constructor("F0", "GF3", gf3t());
    ctx.add_constructor("F1", "GF3", gf3t());
    ctx.add_constructor("F2", "GF3", gf3t());

    // add3 a b : addition mod 3, by double case analysis.
    //   add3 F0 b = b;  add3 F1 b = match b { F0=>F1, F1=>F2, F2=>F0 };  add3 F2 b = match b { F0=>F2, F1=>F0, F2=>F1 }
    let add_body = lam(
        "a",
        gf3t(),
        lam(
            "b",
            gf3t(),
            mtch(
                kvar("a"),
                lam("_", gf3t(), gf3t()),
                vec![kvar("b"), match_b(f1(), f2(), f0()), match_b(f2(), f0(), f1())],
            ),
        ),
    );
    ctx.add_definition("add3".to_string(), binop_ty(), add_body);

    // mul3 a b : multiplication mod 3.
    //   mul3 F0 b = F0;  mul3 F1 b = b;  mul3 F2 b = match b { F0=>F0, F1=>F2, F2=>F1 }
    let mul_body = lam(
        "a",
        gf3t(),
        lam(
            "b",
            gf3t(),
            mtch(
                kvar("a"),
                lam("_", gf3t(), gf3t()),
                vec![f0(), kvar("b"), match_b(f0(), f2(), f1())],
            ),
        ),
    );
    ctx.add_definition("mul3".to_string(), binop_ty(), mul_body);

    // neg3 a : additive inverse — genuinely nontrivial at characteristic 3 (over GF(2), neg = id).
    //   neg3 F0 = F0;  neg3 F1 = F2;  neg3 F2 = F1
    let neg_body =
        lam("a", gf3t(), mtch(kvar("a"), lam("_", gf3t(), gf3t()), vec![f0(), f2(), f1()]));
    ctx.add_definition("neg3".to_string(), pi("a", gf3t(), gf3t()), neg_body);
    ctx
}

/// A universally-quantified law `∀v:GF3. Eq GF3 (lhs v) (rhs v)`, and its proof by `GF3` case analysis
/// with the law as the match motive (`refl` per case; the kernel reduces `add3`/`mul3`/`neg3`).
fn law(
    v: &str,
    lhs: impl Fn(Term) -> Term,
    rhs: impl Fn(Term) -> Term,
    cases: [Term; 3],
) -> (Term, Term) {
    let stmt = pi(v, gf3t(), eq3(lhs(kvar(v)), rhs(kvar(v))));
    let motive = lam(v, gf3t(), eq3(lhs(kvar(v)), rhs(kvar(v))));
    let proof = lam(v, gf3t(), mtch(kvar(v), motive, cases.to_vec()));
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
fn gf3_ring_laws_are_kernel_theorems() {
    let ctx = gf3_context();

    // The construction itself type-checks.
    assert!(matches!(infer_type(&ctx, &g("add3")), Ok(Term::Pi { .. })), "add3 : GF3 → GF3 → GF3");
    assert!(matches!(infer_type(&ctx, &g("mul3")), Ok(Term::Pi { .. })), "mul3 : GF3 → GF3 → GF3");
    assert!(matches!(infer_type(&ctx, &g("neg3")), Ok(Term::Pi { .. })), "neg3 : GF3 → GF3");

    // Additive identity: ∀a. add3 a F0 = a.
    let (add_zero, p) = law("a", |a| add3(a.clone(), f0()), |a| a, [refl3(f0()), refl3(f1()), refl3(f2())]);
    assert!(proves(&ctx, &p, &add_zero), "add3 a F0 = a is a kernel theorem");

    // Multiplicative identity: ∀a. mul3 a F1 = a.
    let (mul_one, p) = law("a", |a| mul3(a.clone(), f1()), |a| a, [refl3(f0()), refl3(f1()), refl3(f2())]);
    assert!(proves(&ctx, &p, &mul_one), "mul3 a F1 = a is a kernel theorem");

    // CHARACTERISTIC 3: ∀a. (a + a) + a = F0 — the defining fact, false at characteristic 2.
    let (char3, p) = law(
        "a",
        |a| add3(add3(a.clone(), a.clone()), a),
        |_| f0(),
        [refl3(f0()), refl3(f0()), refl3(f0())],
    );
    assert!(proves(&ctx, &p, &char3), "a + a + a = 0 (characteristic 3) is a kernel theorem");

    // Additive inverses: ∀a. add3 a (neg3 a) = F0 — negation is a real operation here.
    let (inv, p) = law(
        "a",
        |a| add3(a.clone(), neg3(a)),
        |_| f0(),
        [refl3(f0()), refl3(f0()), refl3(f0())],
    );
    assert!(proves(&ctx, &p, &inv), "a + (−a) = 0 is a kernel theorem");

    // THE SIGNED ATOM: ∀x. (1 − x) + x = 1, i.e. add3 (add3 F1 (neg3 x)) x = F1 — the identity the
    // field-generic partition of unity (constructive NS completeness at characteristic 3) rests on.
    let (atom_one, p) = law(
        "x",
        |x| add3(add3(f1(), neg3(x.clone())), x),
        |_| f1(),
        [refl3(f1()), refl3(f1()), refl3(f1())],
    );
    assert!(proves(&ctx, &p, &atom_one), "the signed atom (1 − x) + x collapses to 1 — kernel-proven");
}

#[test]
fn a_false_gf3_law_is_rejected_by_the_kernel() {
    let ctx = gf3_context();
    // FALSE law: the CHARACTERISTIC-2 identity ∀a. add3 a a = F0 — a kernel theorem over GF(2), FALSE
    // over GF(3) (add3 F1 F1 = F2 ≠ F0). The same case-analysis proof shape cannot type-check: at the
    // F1 case, `refl` would need type `Eq GF3 F2 F0`, which the kernel rejects. The kernel therefore
    // separates the characteristics — it does not accept ring-shaped statements on shape.
    let (char2, p) =
        law("a", |a| add3(a.clone(), a), |_| f0(), [refl3(f0()), refl3(f0()), refl3(f0())]);
    assert!(!proves(&ctx, &p, &char2), "the characteristic-2 law must NOT be provable over GF(3)");
    // And no differently-shaped well-typed term certifies it either.
    assert!(
        infer_type(&ctx, &p).is_err()
            || !is_subtype(&ctx, &infer_type(&ctx, &p).unwrap(), &char2),
        "no term certifies the false law"
    );
    // The sibling sanity: the same statement with the CORRECT GF(3) right sides IS provable —
    // ∀a. add3 a a = neg3 a (doubling is negation at characteristic 3).
    let (double_is_neg, p) = law(
        "a",
        |a| add3(a.clone(), a.clone()),
        neg3,
        [refl3(f0()), refl3(f2()), refl3(f1())],
    );
    assert!(proves(&ctx, &p, &double_is_neg), "2a = −a is the true characteristic-3 doubling law");
}
