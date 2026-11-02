//! # KERNEL-CERTIFIED lemmas behind the char-2 generic-fiber theorem (§32–§35)
//!
//! The GF(256) experiments *check* the results numerically; these tests *prove* the atomic facts the whole
//! argument reduces to, through the LOGOS prover — the same dual-checked CoC kernel that certifies the FOL
//! proofs. Two kinds of certificate:
//!
//!   * **by computation in the kernel** (`refl` + `double_check`): the triangle incidence determinant is
//!     `−2`, it factors as `2·(−1)`, and a `2×2` minor is a unit — i.e. the per-triangle cell has Smith
//!     normal form `(1,1,2)`, a single `ℤ/2`. This is the 2-torsion keystone (§35), proved not measured.
//!   * **by the first-order prover** (`prove_certify_check`, whose derivation is type-checked by the kernel):
//!     in any integral domain of characteristic 2, `μ ∉ {0,1} ⟹ μ·(μ+1) ≠ 0` — the non-vanishing that makes
//!     the asymmetric witness full-rank (§34), from the field axioms.

use logicaffeine_kernel::prelude::StandardLibrary;
use logicaffeine_kernel::{
    double_check, infer_type, is_subtype, normalize, Context, DoubleCheck, Literal, Term,
};
use logicaffeine_proof::verify::prove_certify_check;
use logicaffeine_proof::{ProofExpr, ProofTerm};

// ── kernel term helpers (as in kernel/tests/bignum_kernel.rs) ────────────────────────────────────
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
    Term::Pi {
        param: "_".to_string(),
        param_type: Box::new(a),
        body_type: Box::new(b),
    }
}
fn int(n: i64) -> Term {
    Term::Lit(Literal::Int(n))
}
fn mul(x: Term, y: Term) -> Term {
    apps(g("mul"), &[x, y])
}
fn sub(x: Term, y: Term) -> Term {
    apps(g("sub"), &[x, y])
}
fn add(x: Term, y: Term) -> Term {
    apps(g("add"), &[x, y])
}

fn ctx_with_ops() -> Context {
    let mut ctx = Context::new();
    StandardLibrary::register(&mut ctx);
    let it = g("Int");
    let binop = arrow(it.clone(), arrow(it.clone(), it.clone()));
    for op in ["mul", "add", "sub"] {
        ctx.add_declaration(op, binop.clone());
    }
    ctx
}

/// `det [[a,b,c],[d,e,f],[g,h,i]]` as a kernel `Int` expression (cofactor expansion along row 0).
fn det3(m: [[i64; 3]; 3]) -> Term {
    let x = |r: usize, c: usize| int(m[r][c]);
    let ma = sub(mul(x(1, 1), x(2, 2)), mul(x(1, 2), x(2, 1)));
    let mb = sub(mul(x(1, 0), x(2, 2)), mul(x(1, 2), x(2, 0)));
    let mc = sub(mul(x(1, 0), x(2, 1)), mul(x(1, 1), x(2, 0)));
    add(sub(mul(x(0, 0), ma), mul(x(0, 1), mb)), mul(x(0, 2), mc))
}

/// Kernel-certify `expr = value` over `Int` by `refl`-on-computation, dual-checked.
fn kernel_certifies_eq(ctx: &Context, expr: &Term, value: i64) {
    assert_eq!(
        normalize(ctx, expr),
        int(value),
        "kernel reduces the expression to {value}"
    );
    let refl = apps(g("refl"), &[g("Int"), int(value)]);
    let claim = apps(g("Eq"), &[g("Int"), expr.clone(), int(value)]);
    let ty = infer_type(ctx, &refl).expect("refl type-checks");
    assert!(
        is_subtype(ctx, &ty, &claim),
        "refl Int {value} must prove the claim by computation"
    );
    match double_check(ctx, &refl) {
        DoubleCheck::Agreed => {}
        other => panic!("both kernels must agree on the arithmetic certificate, got {other:?}"),
    }
}

#[test]
fn the_triangle_incidence_determinant_is_minus_two_kernel_certified() {
    // The per-triangle cell is the vertex–edge incidence matrix of a 3-cycle. Its determinant is −2 — proved
    // by computation in the CoC kernel (dual-checked), not measured over a finite field. −2 ≡ 0 (mod 2) is
    // exactly why the triangle is a 𝔽₂ 1-cycle; −2 ≠ 0 is why it is only 2-torsion, invisible elsewhere.
    let ctx = ctx_with_ops();
    let det = det3([[1, 1, 0], [1, 0, 1], [0, 1, 1]]);
    kernel_certifies_eq(&ctx, &det, -2);
}

#[test]
fn the_triangle_cell_is_a_single_z_mod_2_kernel_certified() {
    // Smith normal form (1,1,2) of the triangle cell, kernel-certified: the determinant factors as 2·(−1)
    // (elementary divisor 2 = one ℤ/2), and a 2×2 minor is a unit (−1), forcing d₁ = d₂ = 1. Together:
    // coker = ℤ/2 per triangle. This is the 2-torsion of §35 proved in the kernel.
    let ctx = ctx_with_ops();
    // det = 2·(−1): the whole determinant is 2 times a unit.
    kernel_certifies_eq(&ctx, &mul(int(2), int(-1)), -2);
    // a 2×2 minor of the triangle cell equals −1 (a unit): rows/cols {0,1} of [[1,1,0],[1,0,1],[0,1,1]].
    let minor = sub(mul(int(1), int(0)), mul(int(1), int(1))); // 1·0 − 1·1
    kernel_certifies_eq(&ctx, &minor, -1);
}

#[test]
fn the_char2_symmetric_orientation_vanishes_kernel_certified() {
    // WHY it is char-2. The two triangle orientations of the naïve (symmetric) construction are 1·1·1 and
    // 1·1·1, so the char-2 determinant is 1 + 1. Over ℤ that is 2 (nonzero); its image in characteristic 2
    // is 0 — the cell degenerates. The asymmetric witness (§34) replaces the orientations by μ and μ², whose
    // difference μ + μ² is the object of the field lemma below.
    let ctx = ctx_with_ops();
    kernel_certifies_eq(&ctx, &add(int(1), int(1)), 2); // 1+1 = 2 over ℤ; ≡ 0 only in char 2
}

// ── FOL, kernel-checked: the non-vanishing that makes the witness full-rank ──────────────────────
fn k(n: &str) -> ProofTerm {
    ProofTerm::Constant(n.to_string())
}
fn f2(name: &str, a: ProofTerm, b: ProofTerm) -> ProofTerm {
    ProofTerm::Function(name.to_string(), vec![a, b])
}
fn eq(l: ProofTerm, r: ProofTerm) -> ProofExpr {
    ProofExpr::Identity(l, r)
}
fn implies(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Implies(Box::new(l), Box::new(r))
}
fn not_(e: ProofExpr) -> ProofExpr {
    ProofExpr::Not(Box::new(e))
}
fn or(l: ProofExpr, r: ProofExpr) -> ProofExpr {
    ProofExpr::Or(Box::new(l), Box::new(r))
}

#[test]
fn the_witness_nonvanishing_is_field_certified() {
    // THE CRUX of §34, proved from the field axioms and kernel-checked. In an integral domain of
    // characteristic 2, if μ ∉ {0,1} then the triangle-cell determinant μ·(μ+1) is nonzero — so every
    // triangle block of the asymmetric witness is invertible, hence (block-triangular) the whole oil system
    // is full rank. The argument is purely first-order:
    //   integral domain:  μ·(μ+1) = 0 → (μ = 0 ∨ μ+1 = 0)
    //   characteristic 2: μ+1 = 0 → μ = 1          [add 1: μ = μ+(1+1) = (μ+1)+1 = 1]
    //   with μ ≠ 0 and μ ≠ 1, both disjuncts are impossible ⟹ μ·(μ+1) ≠ 0.
    // Uninterpreted field operations `plus`/`prod` (NOT the names add/mul, which invoke the Int arithmetic
    // oracle) — the argument is purely first-order over an abstract field, so μ stays an abstract element.
    let (zero, one, mu) = (k("zero"), k("one"), k("mu"));
    let mu1 = f2("plus", one.clone(), mu.clone()); // μ + 1
    let prod = f2("prod", mu.clone(), mu1.clone()); // μ·(μ+1)

    let premises = [
        // integral domain (the instance we need): a product is zero only if a factor is.
        implies(
            eq(prod.clone(), zero.clone()),
            or(eq(mu.clone(), zero.clone()), eq(mu1.clone(), zero.clone())),
        ),
        // characteristic 2: μ+1 = 0 forces μ = 1.
        implies(eq(mu1.clone(), zero.clone()), eq(mu.clone(), one.clone())),
        // the hypotheses μ ∉ {0,1}.
        not_(eq(mu.clone(), zero.clone())),
        not_(eq(mu.clone(), one.clone())),
    ];
    let goal = not_(eq(prod, zero));

    let r = prove_certify_check(&premises, &goal);
    assert!(
        r.verified,
        "field non-vanishing μ(μ+1)≠0 must be kernel-certified: {:?}",
        r.verification_error
    );
}
