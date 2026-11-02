//! Kernel-certified linear algebra: turn an exact solve into a KERNEL-CHECKED PROOF TERM.
//!
//! [`crate::linalg`] solves `A·x = b` exactly over ℚ and re-checks the residual. This module goes
//! one level deeper: it emits, for each row, an honest kernel proof term of type `Eq Int lhsᵢ rhsᵢ`,
//! where the row identity `Σⱼ Aᵢⱼ·xⱼ = bᵢ` has been cleared of denominators to an INTEGER identity
//!
//! ```text
//! bdᵢ · (Σⱼ Anᵢⱼ · Xnⱼ)  =  Da_i · Dx · bnᵢ
//! ```
//!
//! and proven by the ring-axiom oracle [`crate::arith::prove_int_eq`], then RE-CHECKED by the
//! kernel's own `infer_type`. This is the same trust path the kernel's Gödel/Chaitin theorems ride:
//! the oracle is untrusted search, the kernel is the judge. A false identity is unprovable (the
//! oracle returns `None`) and a forged proof fails the type-check — fail-closed. The current arith
//! oracle is i64-wide, so systems whose cleared integers exceed i64 return `None` (an honest limit,
//! not a wrong answer; a BigInt oracle would lift it).

use crate::linalg;
use logicaffeine_base::Rational;
use logicaffeine_kernel::{
    infer_type, normalize, prelude::StandardLibrary, Context, Literal, Term,
};

fn global(s: &str) -> Term {
    Term::Global(s.to_string())
}
fn app(f: Term, x: Term) -> Term {
    Term::App(Box::new(f), Box::new(x))
}
fn app2(f: Term, x: Term, y: Term) -> Term {
    app(app(f, x), y)
}
fn lit(n: i64) -> Term {
    Term::Lit(Literal::Int(n))
}
fn add(a: Term, b: Term) -> Term {
    app2(global("add"), a, b)
}
fn mul(a: Term, b: Term) -> Term {
    app2(global("mul"), a, b)
}
/// The proposition `Eq Int l r`.
fn eq_int(l: Term, r: Term) -> Term {
    app(app(app(global("Eq"), global("Int")), l), r)
}

/// A kernel context of its own carrying the ring/field/order axioms the oracle proves against: a
/// copy-on-write clone of the standard library's one registration.
pub fn ring_context() -> Context {
    StandardLibrary::context().clone()
}

fn gcd(a: i128, b: i128) -> i128 {
    let (mut a, mut b) = (a.abs(), b.abs());
    while b != 0 {
        let t = a % b;
        a = b;
        b = t;
    }
    a
}
fn lcm(a: i128, b: i128) -> Option<i128> {
    if a == 0 || b == 0 {
        return Some(0);
    }
    let g = gcd(a, b);
    (a / g).checked_mul(b).map(|v| v.abs())
}
fn fit(v: i128) -> Option<i64> {
    if (i64::MIN as i128..=i64::MAX as i128).contains(&v) {
        Some(v as i64)
    } else {
        None
    }
}
/// (numerator, denominator) of a rational as i128, or None if either exceeds i64 (oracle width).
fn nd(r: &Rational) -> Option<(i128, i128)> {
    Some((
        r.numerator().to_i64()? as i128,
        r.denominator().to_i64()? as i128,
    ))
}

/// Scale a rational vector to integers: `vₖ · D` where `D = lcm` of the denominators. Returns the
/// integer vector and `D`, or `None` past i64.
fn clear_vec(v: &[Rational]) -> Option<(Vec<i128>, i128)> {
    let nds: Vec<(i128, i128)> = v.iter().map(nd).collect::<Option<_>>()?;
    let mut d: i128 = 1;
    for &(_, den) in &nds {
        d = lcm(d, den)?;
    }
    let ints: Vec<i128> = nds
        .iter()
        .map(|&(num, den)| num.checked_mul(d / den))
        .collect::<Option<_>>()?;
    Some((ints, d))
}

/// Prove the dot-product identity `Σₖ psₖ·qsₖ = r` as ONE kernel-checked term (type
/// `Eq Int lhs rhs`). Clears denominators to `rd·Σₖ (Pnₖ·Qnₖ) = Dp·Dq·rn` — structure preserved —
/// proves it with the untrusted oracle, then RE-CHECKS with the kernel. `None` if false or past i64.
/// This one identity underlies every certificate below: a solve is a row·x = b, a factorization
/// entry is Lrow·Ucol = (PA)ᵢⱼ, an inverse entry is Arow·(A⁻¹col) = Iᵢⱼ.
pub fn prove_dot(ctx: &Context, ps: &[Rational], qs: &[Rational], r: &Rational) -> Option<Term> {
    let k = ps.len();
    if k == 0 || qs.len() != k {
        return None;
    }
    let (pn, dp) = clear_vec(ps)?;
    let (qn, dq) = clear_vec(qs)?;
    let (rn, rd) = nd(r)?;
    let mut sum = mul(lit(fit(pn[0])?), lit(fit(qn[0])?));
    for i in 1..k {
        sum = add(sum, mul(lit(fit(pn[i])?), lit(fit(qn[i])?)));
    }
    let lhs = mul(lit(fit(rd)?), sum);
    let rhs = lit(fit(dp.checked_mul(dq)?.checked_mul(rn)?)?);
    // untrusted oracle search → kernel re-check that the type is exactly this proposition.
    let proof = crate::arith::prove_int_eq(ctx, &lhs, &rhs)?;
    let ty = infer_type(ctx, &proof).ok()?;
    if normalize(ctx, &ty) != normalize(ctx, &eq_int(lhs, rhs)) {
        return None;
    }
    Some(proof)
}

/// Prove `r ≠ 0` in the kernel by EXHIBITING its inverse: a kernel-checked proof of `r·(1/r) = 1`.
/// A value with a multiplicative inverse is nonzero, so this certifies nonsingularity when applied to
/// a determinant. `None` iff `r == 0` — correctly unprovable, since `0` has no inverse.
pub fn prove_nonzero(ctx: &Context, r: &Rational) -> Option<Term> {
    let inv = r.recip()?; // r == 0 → None (the claim `0 ≠ 0` is false)
    prove_dot(
        ctx,
        std::slice::from_ref(r),
        std::slice::from_ref(&inv),
        &Rational::one(),
    )
}

/// Prove `A·x = b` for a GIVEN candidate `x`, one kernel-checked proof term per row (each is the
/// row dot-product identity `Aᵢ·x = bᵢ`). `None` if any row is false (fail-closed) or past i64.
pub fn prove_solution(
    ctx: &Context,
    a: &[Vec<Rational>],
    x: &[Rational],
    b: &[Rational],
) -> Option<Vec<Term>> {
    let n = a.len();
    if n == 0 || a.iter().any(|r| r.len() != n) || x.len() != n || b.len() != n {
        return None;
    }
    (0..n).map(|i| prove_dot(ctx, &a[i], x, &b[i])).collect()
}

/// Factor `A` and PROVE the factorization in the kernel: every entry of `P·A = L·U` is a dot-product
/// proof `Lᵢ·Uⱼ = (PA)ᵢⱼ`. Returns the exact factorization and an n×n grid of kernel-checked terms.
pub fn prove_lu(ctx: &Context, a: &[Vec<Rational>]) -> Option<(linalg::Lu, Vec<Vec<Term>>)> {
    let f = linalg::lu(a)?;
    let n = a.len();
    let pa: Vec<Vec<Rational>> = f.perm.iter().map(|&i| a[i].clone()).collect();
    let ucols = linalg::transpose(&f.u); // ucols[j] = column j of U
    let mut proofs = Vec::with_capacity(n);
    for i in 0..n {
        let mut row = Vec::with_capacity(n);
        for j in 0..n {
            row.push(prove_dot(ctx, &f.l[i], &ucols[j], &pa[i][j])?);
        }
        proofs.push(row);
    }
    Some((f, proofs))
}

/// Invert `A` and PROVE it in the kernel: every entry of `A·A⁻¹ = I` is a dot-product proof
/// `Aᵢ·(A⁻¹)ⱼ = Iᵢⱼ` (the off-diagonal `= 0` identities are proven too). `None` iff singular / past i64.
pub fn prove_inverse(
    ctx: &Context,
    a: &[Vec<Rational>],
) -> Option<(Vec<Vec<Rational>>, Vec<Vec<Term>>)> {
    let ainv = linalg::inverse(a)?;
    let n = a.len();
    let id = linalg::identity(n);
    let invcols = linalg::transpose(&ainv); // invcols[j] = column j of A⁻¹
    let mut proofs = Vec::with_capacity(n);
    for i in 0..n {
        let mut row = Vec::with_capacity(n);
        for j in 0..n {
            row.push(prove_dot(ctx, &a[i], &invcols[j], &id[i][j])?);
        }
        proofs.push(row);
    }
    Some((ainv, proofs))
}

/// The complete certificate that a linear system `A·x = b` has a UNIQUE exact solution: the exact
/// `x`, a kernel proof of `Aᵢ·x = bᵢ` for every row, and a kernel proof that `det A ≠ 0` (so the
/// solution is unique, not merely one of many). Every term is re-checked by the kernel. This is the
/// full "prove, not just run" object for a linear system. `None` iff singular or past the i64 width.
pub struct SystemCert {
    pub x: Vec<Rational>,
    pub solve_proofs: Vec<Term>,
    pub det: Rational,
    pub nonsingular_proof: Term,
}

/// Solve and produce the complete unique-solution certificate.
pub fn certify_system(ctx: &Context, a: &[Vec<Rational>], b: &[Rational]) -> Option<SystemCert> {
    let x = linalg::solve(a, b)?;
    let solve_proofs = prove_solution(ctx, a, &x, b)?;
    let det = linalg::det(a);
    let nonsingular_proof = prove_nonzero(ctx, &det)?;
    Some(SystemCert {
        x,
        solve_proofs,
        det,
        nonsingular_proof,
    })
}

/// Solve `A·x = b` exactly and hand back the exact `x` together with a kernel-checked proof that it
/// solves the system (one proof term per row). `None` iff singular or beyond the i64 oracle width.
pub fn prove_solve(
    ctx: &Context,
    a: &[Vec<Rational>],
    b: &[Rational],
) -> Option<(Vec<Rational>, Vec<Term>)> {
    let x = linalg::solve(a, b)?;
    let proofs = prove_solution(ctx, a, &x, b)?;
    Some((x, proofs))
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_proof/linalg_certify/tests.rs"]
mod tests;
