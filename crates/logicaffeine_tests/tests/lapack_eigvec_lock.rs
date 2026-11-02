//! LOGOS-LAPACK eigenvectors of a general real matrix — `eigVectors` (reference LAPACK `DGEEV`
//! with vectors, real-spectrum path). From the Schur form `A = Z·T·Zᵀ`: each eigenvector `y` of the
//! triangular `T` is found by back-substitution, then mapped back as `x = Z·y`. Scoped to matrices
//! with a REAL spectrum (complex eigenvalues would need complex arithmetic, which LOGOS lacks).
//! Returns `SymEig{vals, vecs}`. Locked by the defining residual `A·V − V·diag(λ) == 0`.

mod common;
use common::run_all_tiers;

fn float_lit(x: f64) -> String {
    let s = format!("{x}");
    if s.contains('.') || s.contains('e') || s.contains('E') {
        s
    } else {
        format!("{s}.0")
    }
}
fn seq_literal(xs: &[f64]) -> String {
    format!(
        "[{}]",
        xs.iter()
            .map(|x| float_lit(*x))
            .collect::<Vec<_>>()
            .join(", ")
    )
}
fn parse_seq_floats(out: &str) -> Vec<f64> {
    let t = out.trim();
    let inner = t
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .expect("bracketed Seq");
    inner
        .split(',')
        .map(|s| s.trim().parse::<f64>().expect("float"))
        .collect()
}

// (A·V − V·diag(λ))[i][k] = Σ_j A[i][j]·V[j][k] − λ[k]·V[i][k]  — must vanish for eigenpairs.
const EIG_RESIDUAL: &str = "\
## To eigRes (n: Int, a: Seq of Float, v: Seq of Float, lam: Seq of Float) -> Seq of Float:\n\
\x20   Let mutable out be a new Seq of Float.\n\
\x20   Repeat for i from 1 to n:\n\
\x20       Repeat for k from 1 to n:\n\
\x20           Let mutable av be 0.0.\n\
\x20           Repeat for j from 1 to n:\n\
\x20               Set av to av + (item ((i - 1) * n + j) of a) * (item ((j - 1) * n + k) of v).\n\
\x20           Let r be av - (item k of lam) * (item ((i - 1) * n + k) of v).\n\
\x20           Push r to out.\n\
\x20   Return out.\n";

fn check_eigvectors(n: usize, a: &[f64]) {
    let src = format!(
        "{EIG_RESIDUAL}\n## Main\nLet e be eigVectors({n}, {a}).\nLet vv be e's vecs.\nLet ll be e's vals.\n\
         Show eigRes({n}, {a}, vv, ll).\n",
        a = seq_literal(a)
    );
    let out = run_all_tiers(&src);
    assert!(out.error.is_none(), "eigVectors errored: {:?}", out.error);
    let res = parse_seq_floats(&out.output);
    assert_eq!(res.len(), n * n);
    for (idx, v) in res.iter().enumerate() {
        assert!(v.abs() <= 1e-6, "residual[{idx}] = {v} should vanish");
    }
}

#[test]
fn eigvectors_of_symmetric_3x3() {
    // Symmetric (real spectrum). A·V = V·diag(λ).
    check_eigvectors(3, &[4.0, 1.0, 1.0, 1.0, 3.0, 1.0, 1.0, 1.0, 2.0]);
}

#[test]
fn eigvectors_of_triangular_real_spectrum() {
    // Lower-triangular → real eigenvalues {2,3,4}; distinct → well-conditioned eigenvectors.
    check_eigvectors(3, &[2.0, 0.0, 0.0, 1.0, 3.0, 0.0, 1.0, 1.0, 4.0]);
}

#[test]
fn eigvectors_of_diagonal() {
    // diag(5,3,7): eigenvectors are the axes.
    check_eigvectors(3, &[5.0, 0.0, 0.0, 0.0, 3.0, 0.0, 0.0, 0.0, 7.0]);
}
