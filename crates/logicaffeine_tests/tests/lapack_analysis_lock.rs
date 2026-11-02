//! LOGOS-LAPACK matrix analysis — `spectralRadius` (max |eigenvalue|, via `geev`), `matrixNorm2`
//! (operator 2-norm = largest singular value, via `svd`), `isPosDef` (positive-definiteness test,
//! via `syev`). Native LOGOS scalar/predicate summaries built on the eigen/SVD kernels.

mod common;
use common::assert_all_tiers;

#[test]
fn spectral_radius_of_rotation_is_one() {
    // [[0,-1],[1,0]] has eigenvalues ±i, magnitude 1.
    assert_all_tiers(
        "## Main\nShow spectralRadius(2, [0.0, -1.0, 1.0, 0.0]).\n",
        "1",
    );
}

#[test]
fn spectral_radius_of_diagonal() {
    // diag(2,-3) → max(|2|,|-3|) = 3.
    assert_all_tiers(
        "## Main\nShow spectralRadius(2, [2.0, 0.0, 0.0, -3.0]).\n",
        "3",
    );
}

#[test]
fn matrix_norm2_is_largest_singular_value() {
    // ‖diag(3,5)‖₂ = 5.
    assert_all_tiers("## Main\nShow matrixNorm2(2, [3.0, 0.0, 0.0, 5.0]).\n", "5");
}

#[test]
fn is_pos_def_true_for_spd() {
    // [[2,1],[1,2]] has eigenvalues 1 and 3 → positive definite.
    assert_all_tiers("## Main\nShow isPosDef(2, [2.0, 1.0, 1.0, 2.0]).\n", "true");
}

#[test]
fn is_pos_def_false_for_indefinite() {
    // diag(1,-1) has a negative eigenvalue → not positive definite.
    assert_all_tiers(
        "## Main\nShow isPosDef(2, [1.0, 0.0, 0.0, -1.0]).\n",
        "false",
    );
}
