//! LOGOS-LAPACK real matrix power — `matPowReal` (`Aᵗ = expm(t · logm(A))` for real `t`). The
//! capstone of the matrix-function family: it composes `logm` and `expm`, so `Aᵗ` for fractional
//! `t` is exact. `matPowReal(A, ½)` must equal `sqrtm(A)`; `matPowReal(A, 2)` must equal `A²`.
//! Tolerance harness.

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
fn mat_pow_real_half_is_square_root() {
    // diag(4,9)^½ = diag(2,3).
    assert_close(
        &run_seq("## Main\nShow matPowReal(2, [4.0, 0.0, 0.0, 9.0], 0.5).\n"),
        &[2.0, 0.0, 0.0, 3.0],
        1e-6,
    );
}

#[test]
fn mat_pow_real_two_is_square() {
    // diag(2,3)^2 = diag(4,9).
    assert_close(
        &run_seq("## Main\nShow matPowReal(2, [2.0, 0.0, 0.0, 3.0], 2.0).\n"),
        &[4.0, 0.0, 0.0, 9.0],
        1e-6,
    );
}

#[test]
fn mat_pow_real_half_matches_sqrtm_nondiagonal() {
    // For SPD A, A^½ (via expm∘logm) must equal sqrtm(A) (via Denman–Beavers).
    let a = seq_literal(&[5.0, 4.0, 4.0, 5.0]);
    let src = format!(
        "## Main\nLet p be matPowReal(2, {a}, 0.5).\nLet r be sqrtm(2, {a}).\n\
         Show matsub(2, 2, p, r).\n"
    );
    assert_close(&run_seq(&src), &[0.0, 0.0, 0.0, 0.0], 1e-6);
}
