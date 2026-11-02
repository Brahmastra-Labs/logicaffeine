//! LOGOS-LAPACK extended surface — rank-k update, triangular inverse, norms, and the
//! SVD-powered drivers (2-norm condition number, numerical rank, Moore–Penrose pseudo-inverse).
//!
//! `cond`/`rank`/`pinv` reuse the `svd` kernel, so the whole singular-value machinery pays off
//! across the library. Exact cases use `assert_all_tiers`; `pinv` (built on iterative Jacobi) uses
//! a tolerance harness that verifies `A·A⁺ == I` through the BLAS `gemm`.

mod common;
use common::{assert_all_tiers, run_all_tiers};

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
    if inner.trim().is_empty() {
        return Vec::new();
    }
    inner
        .split(',')
        .map(|s| s.trim().parse::<f64>().expect("float"))
        .collect()
}
fn assert_close(got: &[f64], want: &[f64], tol: f64) {
    assert_eq!(got.len(), want.len(), "len: {got:?} vs {want:?}");
    for (i, (g, e)) in got.iter().zip(want.iter()).enumerate() {
        assert!(
            (g - e).abs() <= tol * (1.0 + e.abs()),
            "elem {i}: {g} vs {e}"
        );
    }
}

// ---- BLAS L3: syrk (C := alpha·A·Aᵀ + beta·C) ----

#[test]
fn syrk_symmetric_rank_k() {
    // A=[[1,2],[3,4]] → A·Aᵀ = [[5,11],[11,25]].
    assert_all_tiers(
        "## Main\nShow syrk(2, 2, 1.0, [1.0, 2.0, 3.0, 4.0], 0.0, [0.0, 0.0, 0.0, 0.0]).\n",
        "[5, 11, 11, 25]",
    );
}

// ---- trtri: inverse of an upper-triangular matrix ----

#[test]
fn trtri_inverts_upper_triangular() {
    // U=[[2,1],[0,4]] → U⁻¹ = [[0.5, -0.125],[0, 0.25]] (dyadic → exact).
    assert_all_tiers(
        "## Main\nShow trtri(2, [2.0, 1.0, 0.0, 4.0]).\n",
        "[0.5, -0.125, 0, 0.25]",
    );
}

// ---- lange: Frobenius norm ----

#[test]
fn lange_frobenius_norm() {
    // ‖diag(3,4)‖_F = sqrt(9+16) = 5.
    assert_all_tiers("## Main\nShow lange(2, 2, [3.0, 0.0, 0.0, 4.0]).\n", "5");
}

// ---- cond: 2-norm condition number (σ_max / σ_min) ----

#[test]
fn cond_two_norm() {
    // diag(2,4): singular values {2,4} → cond = 4/2 = 2.
    assert_all_tiers("## Main\nShow cond(2, [2.0, 0.0, 0.0, 4.0]).\n", "2");
}

// ---- rank: numerical rank (count of nonzero singular values) ----

#[test]
fn rank_counts_nonzero_singular_values() {
    // diag(3,0,5) is rank 2.
    assert_all_tiers(
        "## Main\nShow rank(3, 3, [3.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 5.0]).\n",
        "2",
    );
}

// ---- pinv: Moore–Penrose pseudo-inverse (via SVD) ----

#[test]
fn pinv_of_square_is_inverse() {
    // For invertible A, A·A⁺ = I. A=[[4,7],[2,6]].
    let a = seq_literal(&[4.0, 7.0, 2.0, 6.0]);
    let src = format!(
        "## Main\nLet p be pinv(2, 2, {a}).\nShow gemm(2, 2, 2, 1.0, {a}, p, 0.0, [0.0, 0.0, 0.0, 0.0]).\n"
    );
    let out = run_all_tiers(&src);
    assert!(out.error.is_none(), "pinv errored: {:?}", out.error);
    assert_close(&parse_seq_floats(&out.output), &[1.0, 0.0, 0.0, 1.0], 1e-9);
}
