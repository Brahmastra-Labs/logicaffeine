//! LOGOS-LAPACK matrix toolkit — `eye`, `transpose`, `matadd`, `matsub`, `hadamard`, `outer`,
//! `diagOf`, and `sqrtm` (matrix square root via the Denman–Beavers iteration, inv-only, no
//! transcendentals). The elementwise/structural ops are exact (`assert_all_tiers`); `sqrtm` is
//! locked by `X·X == A` through the BLAS `gemm`.

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

#[test]
fn eye_builds_identity() {
    assert_all_tiers("## Main\nShow eye(3).\n", "[1, 0, 0, 0, 1, 0, 0, 0, 1]");
}

#[test]
fn transpose_swaps_indices() {
    // [[1,2,3],[4,5,6]]ᵀ = [[1,4],[2,5],[3,6]].
    assert_all_tiers(
        "## Main\nShow transpose(2, 3, [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]).\n",
        "[1, 4, 2, 5, 3, 6]",
    );
}

#[test]
fn matadd_and_matsub() {
    assert_all_tiers(
        "## Main\nShow matadd(2, 2, [1.0, 2.0, 3.0, 4.0], [5.0, 6.0, 7.0, 8.0]).\n",
        "[6, 8, 10, 12]",
    );
    assert_all_tiers(
        "## Main\nShow matsub(2, 2, [5.0, 6.0, 7.0, 8.0], [1.0, 2.0, 3.0, 4.0]).\n",
        "[4, 4, 4, 4]",
    );
}

#[test]
fn hadamard_elementwise_product() {
    assert_all_tiers(
        "## Main\nShow hadamard(2, 2, [1.0, 2.0, 3.0, 4.0], [2.0, 2.0, 2.0, 2.0]).\n",
        "[2, 4, 6, 8]",
    );
}

#[test]
fn outer_product() {
    // [1,2] ⊗ [3,4]ᵀ = [[3,4],[6,8]].
    assert_all_tiers(
        "## Main\nShow outerProduct(2, 2, [1.0, 2.0], [3.0, 4.0]).\n",
        "[3, 4, 6, 8]",
    );
}

#[test]
fn diag_of_extracts_diagonal() {
    assert_all_tiers("## Main\nShow diagOf(2, [1.0, 2.0, 3.0, 4.0]).\n", "[1, 4]");
}

#[test]
fn sqrtm_squares_back_to_a() {
    // X = sqrtm(A); verify X·X ≈ A via gemm. A=[[5,4],[4,5]] (SPD).
    let a = seq_literal(&[5.0, 4.0, 4.0, 5.0]);
    let src = format!(
        "## Main\nLet x be sqrtm(2, {a}).\nShow gemm(2, 2, 2, 1.0, x, x, 0.0, [0.0, 0.0, 0.0, 0.0]).\n"
    );
    let out = run_all_tiers(&src);
    assert!(out.error.is_none(), "sqrtm errored: {:?}", out.error);
    assert_close(&parse_seq_floats(&out.output), &[5.0, 4.0, 4.0, 5.0], 1e-8);
}

#[test]
fn sqrtm_of_diagonal() {
    // sqrt(diag(4,9)) = diag(2,3).
    let out = run_all_tiers("## Main\nShow sqrtm(2, [4.0, 0.0, 0.0, 9.0]).\n");
    assert!(out.error.is_none(), "sqrtm errored: {:?}", out.error);
    assert_close(&parse_seq_floats(&out.output), &[2.0, 0.0, 0.0, 3.0], 1e-8);
}
