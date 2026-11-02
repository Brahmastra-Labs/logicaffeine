//! LOGOS-LAPACK Cholesky/triangular utilities — `potrs` (solve from an existing Cholesky factor),
//! `potri` (SPD inverse via Cholesky), `trtriLower` (lower-triangular inverse). All native LOGOS,
//! reusing the factorization primitives. Exact dyadic cases use `assert_all_tiers`; the others use
//! a tolerance harness verifying the defining identity through the BLAS `gemm`.

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
fn run_seq(src: &str) -> Vec<f64> {
    let out = run_all_tiers(src);
    assert!(out.error.is_none(), "errored: {:?}\n{src}", out.error);
    parse_seq_floats(&out.output)
}

#[test]
fn potrs_solves_from_cholesky_factor() {
    // L=[[2,0],[1,2]] (so A=L·Lᵀ=[[4,2],[2,5]]); solve A·x=[6,7] → x=[1,1].
    let src = format!(
        "## Main\nShow potrs(2, {}, {}).\n",
        seq_literal(&[2.0, 0.0, 1.0, 2.0]),
        seq_literal(&[6.0, 7.0])
    );
    assert_close(&run_seq(&src), &[1.0, 1.0], 1e-10);
}

#[test]
fn potri_inverts_spd_matrix() {
    // A=[[4,2],[2,5]]; verify A·potri(A) ≈ I via gemm.
    let a = seq_literal(&[4.0, 2.0, 2.0, 5.0]);
    let src = format!(
        "## Main\nLet ai be potri(2, {a}).\nShow gemm(2, 2, 2, 1.0, {a}, ai, 0.0, [0.0, 0.0, 0.0, 0.0]).\n"
    );
    assert_close(&run_seq(&src), &[1.0, 0.0, 0.0, 1.0], 1e-9);
}

#[test]
fn trtri_lower_inverts_lower_triangular() {
    // L=[[2,0],[1,4]] → L⁻¹ = [[0.5,0],[-0.125,0.25]] (dyadic → exact).
    assert_all_tiers(
        "## Main\nShow trtriLower(2, [2.0, 0.0, 1.0, 4.0]).\n",
        "[0.5, 0, -0.125, 0.25]",
    );
}
