//! LOGOS-BLAS — the `blas.lg` stdlib surface (Track A: faithful f64 dense linear algebra).
//!
//! We port the numerical *algorithm* from the netlib reference but discard its 1978-era
//! hand-optimization. The reference `DDOT` carries jack dongarra's manual 5-way loop unroll
//! (`DTEMP + DX(I)*DY(I) + DX(I+1)*DY(I+1) + …`) plus a `MOD(N,5)` clean-up loop plus stride
//! gymnastics — all pipeline hand-tuning from LINPACK, 3/11/78. The LOGOS `dot` is the naive
//! definition `sum += x[i]*y[i]`; the supercompiler does the unrolling automatically and
//! provably. Every kernel must auto-import on first reference and agree byte-for-byte across
//! the tree-walker and VM tiers (Invariant II); `fmt_f64`'s shortest-round-trip Display makes
//! byte-equality ⇔ bit-equality.

mod common;
use common::{assert_all_tiers, assert_compiled_equals_interpreted};

// ---- BLAS Level 1: dot ----

#[test]
fn dot_products_two_vectors() {
    // Referencing `dot` alone must pull in blas.lg (auto-import).
    // dot(3, [1,2,3], [4,5,6]) = 4 + 10 + 18 = 32.
    assert_all_tiers(
        "## Main\nShow dot(3, [1.0, 2.0, 3.0], [4.0, 5.0, 6.0]).\n",
        "32",
    );
}

#[test]
fn dot_keeps_fractional_precision() {
    // dot(2, [0.5, 0.25], [1.0, 1.0]) = 0.75 — exercises fmt_f64 on a non-integral result.
    assert_all_tiers("## Main\nShow dot(2, [0.5, 0.25], [1.0, 1.0]).\n", "0.75");
}

#[test]
fn dot_of_empty_prefix_is_zero() {
    // n <= 0 → empty sum → 0 (reference DDOT: `IF (N.LE.0) RETURN` with DDOT = 0.0d0).
    assert_all_tiers("## Main\nShow dot(0, [1.0, 2.0], [3.0, 4.0]).\n", "0");
}

// ---- BLAS Level 1: axpy (y := alpha*x + y), scal (x := alpha*x), nrm2 (‖x‖₂) ----

#[test]
fn axpy_scales_and_adds() {
    // axpy(3, 2, [1,2,3], [10,20,30]) = [12, 24, 36] (reference DAXPY).
    assert_all_tiers(
        "## Main\nShow axpy(3, 2.0, [1.0, 2.0, 3.0], [10.0, 20.0, 30.0]).\n",
        "[12, 24, 36]",
    );
}

#[test]
fn axpy_with_alpha_zero_is_identity() {
    // alpha = 0 → y unchanged (DAXPY fast-path `IF (DA.EQ.0.0d0) RETURN`).
    assert_all_tiers(
        "## Main\nShow axpy(3, 0.0, [1.0, 2.0, 3.0], [10.0, 20.0, 30.0]).\n",
        "[10, 20, 30]",
    );
}

#[test]
fn scal_multiplies_in_place() {
    // scal(3, 0.5, [2,4,6]) = [1, 2, 3] (reference DSCAL).
    assert_all_tiers(
        "## Main\nShow scal(3, 0.5, [2.0, 4.0, 6.0]).\n",
        "[1, 2, 3]",
    );
}

#[test]
fn nrm2_is_euclidean_length() {
    // nrm2(2, [3,4]) = sqrt(9+16) = 5 (reference DNRM2, sans the overflow-scaling).
    assert_all_tiers("## Main\nShow nrm2(2, [3.0, 4.0]).\n", "5");
}

// ---- BLAS Level 2: gemv (y := alpha·A·x + beta·y), A is m×n row-major ----

#[test]
fn gemv_matrix_vector_product() {
    // A = [[1,2],[3,4]] (row-major [1,2,3,4]), x = [1,1], alpha=1, beta=0, y=[0,0].
    // A·x = [1·1+2·1, 3·1+4·1] = [3, 7].
    assert_all_tiers(
        "## Main\nShow gemv(2, 2, 1.0, [1.0, 2.0, 3.0, 4.0], [1.0, 1.0], 0.0, [0.0, 0.0]).\n",
        "[3, 7]",
    );
}

#[test]
fn gemv_accumulates_into_beta_y() {
    // beta=2, y=[1,1] → 2·y + A·x = [2+3, 2+7] = [5, 9].
    assert_all_tiers(
        "## Main\nShow gemv(2, 2, 1.0, [1.0, 2.0, 3.0, 4.0], [1.0, 1.0], 2.0, [1.0, 1.0]).\n",
        "[5, 9]",
    );
}

// ---- BLAS Level 3: gemm (C := alpha·A·B + beta·C), the flagship ----

#[test]
fn gemm_times_identity_is_a() {
    // A·I == A, the canonical GEMM property. A = [[1,2],[3,4]], I = [[1,0],[0,1]].
    assert_all_tiers(
        "## Main\nShow gemm(2, 2, 2, 1.0, [1.0, 2.0, 3.0, 4.0], [1.0, 0.0, 0.0, 1.0], 0.0, [0.0, 0.0, 0.0, 0.0]).\n",
        "[1, 2, 3, 4]",
    );
}

#[test]
fn gemm_general_product() {
    // A=[[1,2],[3,4]], B=[[5,6],[7,8]] → A·B = [[19,22],[43,50]] = [19, 22, 43, 50].
    assert_all_tiers(
        "## Main\nShow gemm(2, 2, 2, 1.0, [1.0, 2.0, 3.0, 4.0], [5.0, 6.0, 7.0, 8.0], 0.0, [0.0, 0.0, 0.0, 0.0]).\n",
        "[19, 22, 43, 50]",
    );
}

#[test]
fn gemm_applies_alpha_and_beta() {
    // alpha=1, beta=2, C=[[1,1],[1,1]] → 2·C + A·B = [2+19, 2+22, 2+43, 2+50] = [21, 24, 45, 52].
    assert_all_tiers(
        "## Main\nShow gemm(2, 2, 2, 1.0, [1.0, 2.0, 3.0, 4.0], [5.0, 6.0, 7.0, 8.0], 2.0, [1.0, 1.0, 1.0, 1.0]).\n",
        "[21, 24, 45, 52]",
    );
}

#[test]
fn gemm_agrees_on_the_rust_aot_tier() {
    // The cross-tier lock reaches the compiled Rust AOT backend too, not just interp/VM.
    assert_compiled_equals_interpreted(
        "## Main\nShow gemm(2, 2, 2, 1.0, [1.0, 2.0, 3.0, 4.0], [5.0, 6.0, 7.0, 8.0], 0.0, [0.0, 0.0, 0.0, 0.0]).\n",
    );
}

// ---- BLAS Level 1 completion: asum (Σ|xᵢ|), iamax (argmax|xᵢ|) ----

#[test]
fn asum_sums_absolute_values() {
    // asum(3, [1, -2, 3]) = 1 + 2 + 3 = 6 (reference DASUM).
    assert_all_tiers("## Main\nShow asum(3, [1.0, -2.0, 3.0]).\n", "6");
}

#[test]
fn iamax_finds_largest_magnitude_index() {
    // iamax(3, [1, -5, 3]) = 2 (1-based index of |-5|, the largest magnitude; reference IDAMAX).
    assert_all_tiers("## Main\nShow iamax(3, [1.0, -5.0, 3.0]).\n", "2");
}

// ---- BLAS Level 2 completion: ger (rank-1 update A := alpha·x·yᵀ + A) ----

#[test]
fn ger_rank_one_update() {
    // x=[1,2], y=[3,4], alpha=1, A=0 → A[i][j] = x[i]·y[j] = [[3,4],[6,8]].
    assert_all_tiers(
        "## Main\nShow ger(2, 2, 1.0, [1.0, 2.0], [3.0, 4.0], [0.0, 0.0, 0.0, 0.0]).\n",
        "[3, 4, 6, 8]",
    );
}

// ---- Triangular solves (forward/back substitution) — reused by LU/Cholesky drivers ----

#[test]
fn trsv_lower_forward_substitution() {
    // L=[[2,0],[1,3]], b=[4,11] → x1=4/2=2, x2=(11-1·2)/3=3. (DTRSV uplo='L').
    assert_all_tiers(
        "## Main\nShow trsvLower(2, [2.0, 0.0, 1.0, 3.0], [4.0, 11.0]).\n",
        "[2, 3]",
    );
}

#[test]
fn trsv_upper_back_substitution() {
    // U=[[2,1],[0,3]], b=[8,9] → x2=9/3=3, x1=(8-1·3)/2=2.5. (DTRSV uplo='U').
    assert_all_tiers(
        "## Main\nShow trsvUpper(2, [2.0, 1.0, 0.0, 3.0], [8.0, 9.0]).\n",
        "[2.5, 3]",
    );
}
