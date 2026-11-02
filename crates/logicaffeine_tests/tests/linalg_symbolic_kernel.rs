//! **General-matrix linear-algebra theorems, kernel-certified, ∀ over all entries (fixed shape).**
//!
//! The ∀n induction file proves facts for every SIZE; this file proves facts for every MATRIX of a
//! fixed size — the matrix entries are opaque `Int` atoms, and each entry identity is discharged by
//! `arith::prove_int_eq` (a symbolic ring-polynomial identity) and re-checked by the kernel. Because
//! the atoms are universally quantified, a proof holds for ALL matrices with those entries.
//!
//! The headlines are **Cayley–Hamilton** — every 2×2 matrix satisfies its own characteristic
//! polynomial `A² − tr(A)·A + det(A)·I = 0` — and **determinant multiplicativity** `det(A·B) =
//! det(A)·det(B)` for a GENERAL 2×2 pair: signed-monomial identities the oracle could NOT prove until
//! the constructive coefficient-aware monomial sort (+ literal 0/1 annihilation for the `det·I` term).
//! Also: the adjugate/inverse identity `A·adj(A) = det(A)·I`, **Cramer's rule** `det(Aᵢ) = det(A)·xᵢ`
//! (2×2 and 3×3), **Newton's identity** `tr(A²) = tr(A)² − 2·det(A)`, determinant homogeneity
//! `det(cA)=c²det(A)`, matmul associativity `(A·B)·C = A·(B·C)`, distributivity, `(A·B)ᵀ=BᵀAᵀ`, `det(Aᵀ)=det(A)`.

use logicaffeine_kernel::{
    infer_type, normalize, prelude::StandardLibrary, Context, Literal, Term,
};
use logicaffeine_proof::arith::prove_int_eq;

// ── kernel Int term helpers ────────────────────────────────────────────────────────────────────────
fn g(n: &str) -> Term {
    Term::Global(n.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn app2(f: Term, x: Term, y: Term) -> Term {
    app(app(f, x), y)
}
fn kadd(a: Term, b: Term) -> Term {
    app2(g("add"), a, b)
}
fn kmul(a: Term, b: Term) -> Term {
    app2(g("mul"), a, b)
}
fn lit(n: i64) -> Term {
    Term::Lit(Literal::Int(n))
}
/// Subtraction via the additive convention `a + (−1)·b` (keeps everything in add/mul for the oracle).
fn ksub(a: Term, b: Term) -> Term {
    kadd(a, kmul(lit(-1), b))
}
fn eq_int(l: Term, r: Term) -> Term {
    app(app(app(g("Eq"), g("Int")), l), r)
}

// ── symbolic 2×2 matrices over opaque Int atoms ──────────────────────────────────────────────────
type M2 = Vec<Vec<Term>>;

fn ring_ctx() -> Context {
    let mut c = Context::new();
    StandardLibrary::register(&mut c);
    c
}
/// Declare a fresh n×n matrix of opaque Int entries `p00 p01 …`.
fn matrix_n(prefix: &str, n: usize, ctx: &mut Context) -> M2 {
    (0..n)
        .map(|i| {
            (0..n)
                .map(|j| {
                    let name = format!("{prefix}{i}{j}");
                    ctx.add_declaration(&name, g("Int"));
                    g(&name)
                })
                .collect()
        })
        .collect()
}
/// A fresh 2×2 matrix (`p00 p01 p10 p11`).
fn matrix(prefix: &str, ctx: &mut Context) -> M2 {
    matrix_n(prefix, 2, ctx)
}
/// General n×m·m×k matrix product `Σₗ aᵢₗ·bₗⱼ` (dimensions read from the operands).
fn matmul(a: &M2, b: &M2) -> M2 {
    let (n, k, m) = (a.len(), b.len(), b[0].len());
    (0..n)
        .map(|i| {
            (0..m)
                .map(|j| {
                    let mut acc = kmul(a[i][0].clone(), b[0][j].clone());
                    for l in 1..k {
                        acc = kadd(acc, kmul(a[i][l].clone(), b[l][j].clone()));
                    }
                    acc
                })
                .collect()
        })
        .collect()
}
fn matadd(a: &M2, b: &M2) -> M2 {
    (0..a.len())
        .map(|i| {
            (0..a[0].len())
                .map(|j| kadd(a[i][j].clone(), b[i][j].clone()))
                .collect()
        })
        .collect()
}
fn transpose(a: &M2) -> M2 {
    let (n, m) = (a.len(), a[0].len());
    (0..m)
        .map(|i| (0..n).map(|j| a[j][i].clone()).collect())
        .collect()
}
/// det of a 2×2: `a00·a11 − a01·a10`.
fn det2(a: &M2) -> Term {
    ksub(
        kmul(a[0][0].clone(), a[1][1].clone()),
        kmul(a[0][1].clone(), a[1][0].clone()),
    )
}
/// det of a 3×3 by cofactor expansion along row 0 (signs +,−,+).
fn det3(a: &M2) -> Term {
    let minor = |r0: usize, r1: usize, c0: usize, c1: usize| {
        ksub(
            kmul(a[r0][c0].clone(), a[r1][c1].clone()),
            kmul(a[r0][c1].clone(), a[r1][c0].clone()),
        )
    };
    let t0 = kmul(a[0][0].clone(), minor(1, 2, 1, 2));
    let t1 = kmul(a[0][1].clone(), minor(1, 2, 0, 2));
    let t2 = kmul(a[0][2].clone(), minor(1, 2, 0, 1));
    kadd(ksub(t0, t1), t2)
}
/// Adjugate (classical adjoint) of a 2×2: `[[a11, −a01], [−a10, a00]]`.
fn adjugate2(a: &M2) -> M2 {
    vec![
        vec![a[1][1].clone(), kmul(lit(-1), a[0][1].clone())],
        vec![kmul(lit(-1), a[1][0].clone()), a[0][0].clone()],
    ]
}
/// The 2×2 scalar matrix `s·I = [[s, 0], [0, s]]`.
fn scalar_diag2(s: &Term) -> M2 {
    vec![vec![s.clone(), lit(0)], vec![lit(0), s.clone()]]
}
/// Declare a fresh length-n column vector of opaque Int entries `p0 p1 …`.
fn vector(prefix: &str, n: usize, ctx: &mut Context) -> Vec<Term> {
    (0..n)
        .map(|i| {
            let name = format!("{prefix}{i}");
            ctx.add_declaration(&name, g("Int"));
            g(&name)
        })
        .collect()
}
/// Matrix–vector product `Σⱼ aᵢⱼ·xⱼ`.
fn matvec(a: &M2, x: &[Term]) -> Vec<Term> {
    (0..a.len())
        .map(|i| {
            let mut acc = kmul(a[i][0].clone(), x[0].clone());
            for j in 1..x.len() {
                acc = kadd(acc, kmul(a[i][j].clone(), x[j].clone()));
            }
            acc
        })
        .collect()
}
/// `A` with column `col` replaced by vector `v` — Cramer's replaced-column matrix Aᵢ.
fn replace_col(a: &M2, col: usize, v: &[Term]) -> M2 {
    let mut m = a.clone();
    for (i, row) in m.iter_mut().enumerate() {
        row[col] = v[i].clone();
    }
    m
}
/// Scale a matrix by a scalar term (entrywise `s·mᵢⱼ`).
fn smul(s: &Term, m: &M2) -> M2 {
    (0..2)
        .map(|i| (0..2).map(|j| kmul(s.clone(), m[i][j].clone())).collect())
        .collect()
}
/// Entrywise `aᵢⱼ − bᵢⱼ`.
fn matsub(a: &M2, b: &M2) -> M2 {
    (0..2)
        .map(|i| {
            (0..2)
                .map(|j| ksub(a[i][j].clone(), b[i][j].clone()))
                .collect()
        })
        .collect()
}
/// trace of a 2×2: `a00 + a11`.
fn trace2(a: &M2) -> Term {
    kadd(a[0][0].clone(), a[1][1].clone())
}
/// The 2×2 identity as literal entries.
fn identity2() -> M2 {
    vec![vec![lit(1), lit(0)], vec![lit(0), lit(1)]]
}
/// Assert every entry of a 2×2 matrix is the literal 0 (the zero matrix), each kernel-certified.
fn assert_zero_matrix(ctx: &Context, m: &M2) {
    for i in 0..2 {
        for j in 0..2 {
            assert_int_eq(ctx, &m[i][j], &lit(0));
        }
    }
}

/// Prove `l = r` in the kernel (oracle + `infer_type` re-check), asserting the type is `Eq Int l r`.
fn assert_int_eq(ctx: &Context, l: &Term, r: &Term) {
    let proof = prove_int_eq(ctx, l, r)
        .unwrap_or_else(|| panic!("oracle found no proof:\n {l:?}\n= {r:?}"));
    let ty = infer_type(ctx, &proof).unwrap_or_else(|e| panic!("kernel rejected the proof: {e:?}"));
    assert_eq!(
        normalize(ctx, &ty),
        normalize(ctx, &eq_int(l.clone(), r.clone())),
        "proof type mismatch"
    );
}
/// Prove two matrices equal entrywise, each a kernel-checked identity.
fn assert_mat_eq(ctx: &Context, x: &M2, y: &M2) {
    for i in 0..x.len() {
        for j in 0..x[0].len() {
            assert_int_eq(ctx, &x[i][j], &y[i][j]);
        }
    }
}

#[test]
fn matmul_is_associative_for_all_2x2() {
    // (A·B)·C = A·(B·C) — every entry a degree-3 polynomial identity in the 12 opaque entries.
    let mut ctx = ring_ctx();
    let a = matrix("a", &mut ctx);
    let b = matrix("b", &mut ctx);
    let c = matrix("c", &mut ctx);
    assert_mat_eq(
        &ctx,
        &matmul(&matmul(&a, &b), &c),
        &matmul(&a, &matmul(&b, &c)),
    );
}

#[test]
fn matmul_distributes_over_addition_for_all_2x2() {
    // (A + B)·C = A·C + B·C
    let mut ctx = ring_ctx();
    let a = matrix("a", &mut ctx);
    let b = matrix("b", &mut ctx);
    let c = matrix("c", &mut ctx);
    assert_mat_eq(
        &ctx,
        &matmul(&matadd(&a, &b), &c),
        &matadd(&matmul(&a, &c), &matmul(&b, &c)),
    );
}

#[test]
fn transpose_of_product_for_all_2x2() {
    // (A·B)ᵀ = Bᵀ·Aᵀ
    let mut ctx = ring_ctx();
    let a = matrix("a", &mut ctx);
    let b = matrix("b", &mut ctx);
    assert_mat_eq(
        &ctx,
        &transpose(&matmul(&a, &b)),
        &matmul(&transpose(&b), &transpose(&a)),
    );
}

#[test]
fn determinant_is_multiplicative_for_all_2x2() {
    // det(A·B) = det(A)·det(B) — the degree-4 identity in 8 entries WITH signed monomials (det has a
    // subtraction). THE determinant theorem, kernel-certified, for ALL 2×2 matrices.
    let mut ctx = ring_ctx();
    let a = matrix("a", &mut ctx);
    let b = matrix("b", &mut ctx);
    assert_int_eq(&ctx, &det2(&matmul(&a, &b)), &kmul(det2(&a), det2(&b)));
}

#[test]
fn cayley_hamilton_holds_for_all_2x2() {
    // Every 2×2 matrix satisfies its own characteristic polynomial: A² − tr(A)·A + det(A)·I = 0 (the
    // zero matrix), entrywise. Each entry is a signed-monomial ring identity in the 4 opaque entries
    // that cancels identically to 0 — reachable now via the coefficient-aware monomial sort. THE
    // Cayley–Hamilton theorem, kernel-certified for ALL 2×2 matrices.
    let mut ctx = ring_ctx();
    let a = matrix("a", &mut ctx);
    let tr = trace2(&a);
    let det = det2(&a);
    let p = matadd(
        &matsub(&matmul(&a, &a), &smul(&tr, &a)),
        &smul(&det, &identity2()),
    );
    assert_zero_matrix(&ctx, &p);
}

#[test]
fn adjugate_identity_for_all_2x2() {
    // A·adj(A) = adj(A)·A = det(A)·I — the Laplace/adjugate identity: the algebraic core of the
    // closed-form inverse A⁻¹ = adj(A)/det(A) and Cramer's rule. Signed-monomial ring identities in
    // the 4 opaque entries (off-diagonals cancel to 0, diagonals collapse to det), kernel-certified
    // for ALL 2×2 matrices — with NO nonsingularity premise (it holds even when det = 0).
    let mut ctx = ring_ctx();
    let a = matrix("a", &mut ctx);
    let adj = adjugate2(&a);
    let det_i = scalar_diag2(&det2(&a));
    assert_mat_eq(&ctx, &matmul(&a, &adj), &det_i);
    assert_mat_eq(&ctx, &matmul(&adj, &a), &det_i);
}

#[test]
fn cramers_rule_holds_for_all_2x2() {
    // Cramer's rule: for A·x = b, the solution component is xᵢ = det(Aᵢ)/det(A), where Aᵢ replaces
    // column i of A with b. Stated UNCONDITIONALLY (no division, no nonsingularity premise) by
    // substituting the defining constraint b := A·x — so `det(Aᵢ) = det(A)·xᵢ` is a pure signed-monomial
    // degree-3 ring identity in the 6 opaque entries {a.., x.}, kernel-certified for ALL A and x.
    let mut ctx = ring_ctx();
    let a = matrix("a", &mut ctx);
    let x = vector("x", 2, &mut ctx);
    let b = matvec(&a, &x);
    for i in 0..2 {
        let ai = replace_col(&a, i, &b);
        assert_int_eq(&ctx, &det2(&ai), &kmul(det2(&a), x[i].clone()));
    }
}

#[test]
fn cramers_rule_holds_for_all_3x3() {
    // Cramer's rule at 3×3: `det(Aᵢ) = det(A)·xᵢ` with b := A·x — a signed degree-4 identity in the 12
    // matrix + 3 vector entries. Larger than the 2×2 case but far short of the degree-6 det(A·B)=… blow-up.
    let mut ctx = ring_ctx();
    let a = matrix_n("a", 3, &mut ctx);
    let x = vector("x", 3, &mut ctx);
    let b = matvec(&a, &x);
    for i in 0..3 {
        let ai = replace_col(&a, i, &b);
        assert_int_eq(&ctx, &det3(&ai), &kmul(det3(&a), x[i].clone()));
    }
}

#[test]
#[ignore = "degree-6 identity in 18 vars → ~6.5 min, ~3.5 GB, deep stack; a deliberately-run heavy proof"]
fn determinant_is_multiplicative_for_all_3x3() {
    // det(A·B) = det(A)·det(B) for a GENERAL 3×3 pair — a degree-6 identity in the 18 opaque entries
    // (det3 = 6 signed degree-3 terms; det3(A·B) expands the left side to thousands of signed
    // monomials). CERTIFIED — proving the constructive coefficient-aware sort scales past the 2×2
    // degree-4 case — but the proof term is enormous, so it runs on its own deep stack and is
    // #[ignore]d out of the fast loop. Correctness, not speed, is the point.
    std::thread::Builder::new()
        .stack_size(1 << 30)
        .spawn(|| {
            let mut ctx = ring_ctx();
            let a = matrix_n("a", 3, &mut ctx);
            let b = matrix_n("b", 3, &mut ctx);
            assert_int_eq(&ctx, &det3(&matmul(&a, &b)), &kmul(det3(&a), det3(&b)));
        })
        .unwrap()
        .join()
        .unwrap();
}

#[test]
fn determinant_is_transpose_invariant_for_all_2x2() {
    // det(Aᵀ) = det(A): `a00·a11 − a10·a01` = `a00·a11 − a01·a10` for ALL entries (the crossed
    // monomial reorders under commutativity). A ring identity, kernel-certified.
    let mut ctx = ring_ctx();
    let a = matrix("a", &mut ctx);
    assert_int_eq(&ctx, &det2(&transpose(&a)), &det2(&a));
}

#[test]
fn newtons_identity_for_all_2x2() {
    // tr(A²) = tr(A)² − 2·det(A) — the n=2 Newton–Girard identity relating the power sum tr(A²) to the
    // elementary symmetric functions tr(A) and det(A) (which are exactly the characteristic-polynomial
    // coefficients). A signed degree-2 ring identity in the 4 entries, kernel-certified for ALL 2×2.
    let mut ctx = ring_ctx();
    let a = matrix("a", &mut ctx);
    let a2 = matmul(&a, &a);
    let trace_of_a_squared = kadd(a2[0][0].clone(), a2[1][1].clone());
    let tr = trace2(&a);
    let rhs = ksub(kmul(tr.clone(), tr), kmul(lit(2), det2(&a)));
    assert_int_eq(&ctx, &trace_of_a_squared, &rhs);
}

#[test]
fn determinant_is_degree_2_homogeneous_for_all_2x2() {
    // det(c·A) = c²·det(A) — scaling a 2×2 matrix by c scales its determinant by c². The determinant
    // is a degree-2 homogeneous form in the entries. Kernel-certified for ALL c and A.
    let mut ctx = ring_ctx();
    let a = matrix("a", &mut ctx);
    ctx.add_declaration("c", g("Int"));
    let c = g("c");
    let scaled = smul(&c, &a);
    let rhs = kmul(kmul(c.clone(), c), det2(&a));
    assert_int_eq(&ctx, &det2(&scaled), &rhs);
}

#[test]
fn a_false_matrix_identity_is_unprovable() {
    // Matrix multiplication is NOT commutative: (A·B)[0][0] = a00·b00 + a01·b10 ≠ b00·a00 + b01·a10
    // = (B·A)[0][0] for general entries. The oracle must NOT fabricate a proof — fail-closed.
    let mut ctx = ring_ctx();
    let a = matrix("a", &mut ctx);
    let b = matrix("b", &mut ctx);
    let ab = matmul(&a, &b);
    let ba = matmul(&b, &a);
    assert!(
        prove_int_eq(&ctx, &ab[0][0], &ba[0][0]).is_none(),
        "matmul commutativity is FALSE in general — must be unprovable"
    );
}
