//! LOGOS-LAPACK real Schur decomposition — `schur` (`A = Z·T·Zᵀ`, `Z` orthogonal, `T` quasi-upper-
//! triangular), reference LAPACK `DGEES`. The `geev` shifted-QR iteration, now accumulating the
//! orthogonal factor `Z`. Locked by reconstruction `Z·T·Zᵀ == A` and orthogonality `Z·Zᵀ == I`,
//! computed in-LOGOS via `gemm`/`transpose`.

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

const ZEROS9: &str = "[0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]";

#[test]
fn schur_reconstructs_symmetric() {
    // A symmetric (real spectrum → QR converges). Verify Z·T·Zᵀ ≈ A.
    let a = seq_literal(&[4.0, 1.0, 1.0, 1.0, 3.0, 1.0, 1.0, 1.0, 2.0]);
    let src = format!(
        "## Main\nLet s be schur(3, {a}).\nLet t be s's t.\nLet z be s's z.\nLet zt be transpose(3, 3, z).\n\
         Let zt2 be gemm(3, 3, 3, 1.0, t, zt, 0.0, {ZEROS9}).\n\
         Show gemm(3, 3, 3, 1.0, z, zt2, 0.0, {ZEROS9}).\n"
    );
    assert_close(
        &run_seq(&src),
        &[4.0, 1.0, 1.0, 1.0, 3.0, 1.0, 1.0, 1.0, 2.0],
        1e-7,
    );
}

#[test]
fn schur_z_is_orthogonal() {
    // Z·Zᵀ ≈ I.
    let a = seq_literal(&[4.0, 1.0, 1.0, 1.0, 3.0, 1.0, 1.0, 1.0, 2.0]);
    let src = format!(
        "## Main\nLet s be schur(3, {a}).\nLet z be s's z.\nLet zt be transpose(3, 3, z).\n\
         Show gemm(3, 3, 3, 1.0, z, zt, 0.0, {ZEROS9}).\n"
    );
    assert_close(
        &run_seq(&src),
        &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        1e-7,
    );
}

#[test]
fn schur_triangular_form_has_eigenvalues_on_diagonal() {
    // Upper-triangular A is already in Schur form: T's diagonal = A's = {2,3,4}, Z = I.
    let a = seq_literal(&[2.0, 1.0, 5.0, 0.0, 3.0, 1.0, 0.0, 0.0, 4.0]);
    let src = format!("## Main\nLet s be schur(3, {a}).\nShow s's t.\n");
    let t = run_seq(&src);
    let mut diag = vec![t[0], t[4], t[8]];
    diag.sort_by(|a, b| a.partial_cmp(b).unwrap());
    assert_close(&diag, &[2.0, 3.0, 4.0], 1e-7);
    // sub-diagonal entries vanish (real spectrum → strictly upper-triangular).
    for &(i, j) in &[(2usize, 1usize), (3, 1), (3, 2)] {
        assert!(
            t[(i - 1) * 3 + (j - 1)].abs() <= 1e-7,
            "T[{i}][{j}] should vanish"
        );
    }
}
