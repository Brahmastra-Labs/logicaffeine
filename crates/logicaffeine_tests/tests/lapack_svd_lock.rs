//! LOGOS-LAPACK SVD lock — `svd` (one-sided Jacobi: `A = U·Σ·Vᵀ`).
//!
//! One-sided Jacobi orthogonalizes the columns of `A` by accumulated plane rotations. The
//! rotation is computed from the *tangent formula* (`ζ=(β-α)/2γ`, `t=sign(ζ)/(|ζ|+√(1+ζ²))`,
//! `c=1/√(1+t²)`, `s=ct`) — **`sqrt` only, no `sin`/`cos`/`atan2`** — which is why SVD needs no
//! transcendental builtins (the reference LAPACK is trig-free for the same reason). Correctness
//! is locked by the reconstruction `U·Σ·Vᵀ == A` (order-independent) computed in-LOGOS, plus the
//! singular values of a diagonal matrix. `run_all_tiers` enforces interp==VM.

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
    let inner: Vec<String> = xs.iter().map(|x| float_lit(*x)).collect();
    format!("[{}]", inner.join(", "))
}

fn parse_seq_floats(out: &str) -> Vec<f64> {
    let t = out.trim();
    let inner = t
        .strip_prefix('[')
        .and_then(|s| s.strip_suffix(']'))
        .unwrap_or_else(|| panic!("expected a bracketed Seq, got {out:?}"));
    if inner.trim().is_empty() {
        return Vec::new();
    }
    inner
        .split(',')
        .map(|s| s.trim().parse::<f64>().expect("float element"))
        .collect()
}

fn assert_close(got: &[f64], want: &[f64], tol: f64) {
    assert_eq!(got.len(), want.len(), "length: got {got:?} want {want:?}");
    for (i, (g, e)) in got.iter().zip(want.iter()).enumerate() {
        assert!(
            (g - e).abs() <= tol * (1.0 + e.abs()),
            "element {i}: got={g} want={e} diff={:.3e}",
            (g - e).abs()
        );
    }
}

// The in-LOGOS reconstruction helper: (U·Σ·Vᵀ)[i][l] = Σ_j U[i][j]·s[j]·V[l][j].
const RECON_HELPER: &str = "\
## To reconSvd (m: Int, n: Int, u: Seq of Float, s: Seq of Float, v: Seq of Float) -> Seq of Float:\n\
\x20   Let mutable out be a new Seq of Float.\n\
\x20   Repeat for i from 1 to m:\n\
\x20       Repeat for l from 1 to n:\n\
\x20           Let mutable acc be 0.0.\n\
\x20           Repeat for j from 1 to n:\n\
\x20               Set acc to acc + (item ((i - 1) * n + j) of u) * (item j of s) * (item ((l - 1) * n + j) of v).\n\
\x20           Push acc to out.\n\
\x20   Return out.\n";

/// Factor `A` (square, `n×n`) with `svd`, reconstruct `U·Σ·Vᵀ`, and confirm it reproduces `A`.
fn check_svd_reconstructs(n: usize, a: &[f64]) {
    let src = format!(
        "{RECON_HELPER}\n\
         ## Main\n\
         Let f be svd({n}, {n}, {a}).\n\
         Let uu be f's u.\n\
         Let ss be f's s.\n\
         Let vv be f's v.\n\
         Show reconSvd({n}, {n}, uu, ss, vv).\n",
        a = seq_literal(a)
    );
    let out = run_all_tiers(&src);
    assert!(
        out.error.is_none(),
        "svd reconstruction errored: {:?}",
        out.error
    );
    let recon = parse_seq_floats(&out.output);
    assert_close(&recon, a, 1e-8);
}

#[test]
fn svd_reconstructs_2x2() {
    check_svd_reconstructs(2, &[2.0, 0.0, 0.0, 3.0]);
}

#[test]
fn svd_reconstructs_diagonal_3x3() {
    check_svd_reconstructs(3, &[4.0, 0.0, 0.0, 0.0, 3.0, 0.0, 0.0, 0.0, 5.0]);
}

#[test]
fn svd_reconstructs_general_3x3() {
    // Nonsingular general matrix (note 10, not 9, to stay full-rank).
    check_svd_reconstructs(3, &[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 10.0]);
}

#[test]
fn svd_singular_values_of_diagonal() {
    // A = diag(4, 3, 5) → singular values are {5, 4, 3} (Jacobi returns them unordered,
    // so we compare as sorted-descending sets).
    let src = format!(
        "## Main\nLet f be svd(3, 3, {}).\nShow f's s.\n",
        seq_literal(&[4.0, 0.0, 0.0, 0.0, 3.0, 0.0, 0.0, 0.0, 5.0])
    );
    let out = run_all_tiers(&src);
    assert!(out.error.is_none(), "svd errored: {:?}", out.error);
    let mut s = parse_seq_floats(&out.output);
    s.sort_by(|a, b| b.partial_cmp(a).unwrap());
    assert_close(&s, &[5.0, 4.0, 3.0], 1e-8);
}
