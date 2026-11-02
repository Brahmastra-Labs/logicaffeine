//! LOGOS-LAPACK orthogonalization — `polar` (polar decomposition `A = Q·H`, returning the nearest
//! orthogonal factor `Q = U·Vᵀ` via SVD) and `gramSchmidt` (orthonormalize a matrix's columns).
//! Native LOGOS. Locked by defining identities (`polar` of an SPD matrix is `I`; `gramSchmidt`
//! produces orthonormal columns) checked with a tolerance harness.

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
fn polar_of_spd_is_identity() {
    // For SPD A the polar orthogonal factor is I (A = I·A).
    let out = run_seq(&format!(
        "## Main\nShow polar(2, {}).\n",
        seq_literal(&[5.0, 4.0, 4.0, 5.0])
    ));
    assert_close(&out, &[1.0, 0.0, 0.0, 1.0], 1e-7);
}

#[test]
fn polar_factor_is_orthogonal() {
    // Q = polar(A); verify Q·Qᵀ ≈ I via transpose + gemm.
    let a = seq_literal(&[2.0, 1.0, 0.0, 3.0]);
    let src = format!(
        "## Main\nLet q be polar(2, {a}).\nLet qt be transpose(2, 2, q).\n\
         Show gemm(2, 2, 2, 1.0, q, qt, 0.0, [0.0, 0.0, 0.0, 0.0]).\n"
    );
    assert_close(&run_seq(&src), &[1.0, 0.0, 0.0, 1.0], 1e-7);
}

#[test]
fn gram_schmidt_orthonormalizes_columns() {
    // Columns [3,4] and [1,0] → Q columns [0.6,0.8] and [0.8,-0.6].
    // A row-major with those columns = [[3,1],[4,0]] = [3,1,4,0].
    let out = run_seq("## Main\nShow gramSchmidt(2, 2, [3.0, 1.0, 4.0, 0.0]).\n");
    assert_close(&out, &[0.6, 0.8, 0.8, -0.6], 1e-9);
}

#[test]
fn gram_schmidt_gives_orthonormal_columns() {
    // Verify QᵀQ ≈ I for a general 3-column input.
    let a = seq_literal(&[1.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 1.0]);
    let src = format!(
        "## Main\nLet q be gramSchmidt(3, 3, {a}).\nLet qt be transpose(3, 3, q).\n\
         Show gemm(3, 3, 3, 1.0, qt, q, 0.0, [0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]).\n"
    );
    assert_close(
        &run_seq(&src),
        &[1.0, 0.0, 0.0, 0.0, 1.0, 0.0, 0.0, 0.0, 1.0],
        1e-9,
    );
}
