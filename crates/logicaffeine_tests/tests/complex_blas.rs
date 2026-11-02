//! Complex BLAS / LAPACK over the native `ComplexF` scalar (`clapack.lg`, LAPACK `Z` precision):
//! `cdotu`/`cdotc`, `cscal`/`caxpy`/`cnrm2`, `cgemv`/`cgemm`, `cconjtranspose`, and the `cgesv`
//! complex linear solve. Every kernel is checked with hand-computed values, and every check runs
//! on BOTH the tree-walker≡VM (`assert_all_tiers`) and the Rust-AOT tier
//! (`assert_compiled_equals_interpreted`) — a `Seq of ComplexF` flowing through an auto-imported
//! stdlib function must compile to byte-identical Rust output.
mod common;
use common::{assert_all_tiers, assert_compiled_equals_interpreted};

fn both(src: &str, expected: &str) {
    assert_all_tiers(src, expected);
    assert_compiled_equals_interpreted(src);
}

#[test]
fn cdotu_is_the_unconjugated_bilinear_form() {
    // (1+2i)(5+6i) + (3+4i)(7+8i) = (-7+16i) + (-11+52i) = -18+68i.
    let src = "\
## Main
Let x be [complex(1.0, 2.0), complex(3.0, 4.0)].
Let y be [complex(5.0, 6.0), complex(7.0, 8.0)].
Show cdotu(2, x, y).
";
    both(src, "-18+68i");
}

#[test]
fn cdotc_is_the_hermitian_inner_product() {
    // conj(1+2i)(5+6i) + conj(3+4i)(7+8i) = (17-4i) + (53-4i) = 70-8i.
    let src = "\
## Main
Let x be [complex(1.0, 2.0), complex(3.0, 4.0)].
Let y be [complex(5.0, 6.0), complex(7.0, 8.0)].
Show cdotc(2, x, y).
";
    both(src, "70-8i");
}

#[test]
fn cdotc_of_a_vector_with_itself_is_the_real_squared_norm() {
    // ⟨x, x⟩ = |1+2i|² + |3+4i|² = 5 + 25 = 30 (real, no imaginary part).
    let src = "\
## Main
Let x be [complex(1.0, 2.0), complex(3.0, 4.0)].
Show cdotc(2, x, x).
";
    both(src, "30");
}

#[test]
fn cscal_scales_a_complex_vector() {
    // 2·[1+2i, 3+4i] = [2+4i, 6+8i].
    let src = "\
## Main
Let x be [complex(1.0, 2.0), complex(3.0, 4.0)].
Show cscal(2, complex(2.0, 0.0), x).
";
    both(src, "[2+4i, 6+8i]");
}

#[test]
fn caxpy_is_alpha_x_plus_y() {
    // (1+i)·[1, i] + [1, 1] = [1+(1+i), 1+(i-1)] = [2+i, i].
    let src = "\
## Main
Let x be [complex(1.0, 0.0), complex(0.0, 1.0)].
Let y be [complex(1.0, 0.0), complex(1.0, 0.0)].
Show caxpy(2, complex(1.0, 1.0), x, y).
";
    both(src, "[2+i, i]");
}

#[test]
fn cnrm2_is_the_real_euclidean_norm() {
    // ‖[3+4i, 0]‖ = √(25 + 0) = 5.
    let src = "\
## Main
Let x be [complex(3.0, 4.0), complex(0.0, 0.0)].
Show cnrm2(2, x).
";
    both(src, "5");
}

#[test]
fn cgemv_is_the_complex_matrix_vector_product() {
    // A = [[1, i], [-i, 2]], x = [1, 1], alpha=1, beta=0 ⇒ [1+i, 2-i].
    let src = "\
## Main
Let a be [complex(1.0, 0.0), complex(0.0, 1.0), complex(0.0, -1.0), complex(2.0, 0.0)].
Let x be [complex(1.0, 0.0), complex(1.0, 0.0)].
Let y be [complex(0.0, 0.0), complex(0.0, 0.0)].
Show cgemv(2, 2, complex(1.0, 0.0), a, x, complex(0.0, 0.0), y).
";
    both(src, "[1+i, 2-i]");
}

#[test]
fn cgemm_multiplies_by_the_complex_identity() {
    // A·I = A. A = [[1+i, 2], [3, 4-i]] (row-major), B = I₂ ⇒ A back.
    let src = "\
## Main
Let a be [complex(1.0, 1.0), complex(2.0, 0.0), complex(3.0, 0.0), complex(4.0, -1.0)].
Let b be [complex(1.0, 0.0), complex(0.0, 0.0), complex(0.0, 0.0), complex(1.0, 0.0)].
Let c be [complex(0.0, 0.0), complex(0.0, 0.0), complex(0.0, 0.0), complex(0.0, 0.0)].
Show cgemm(2, 2, 2, complex(1.0, 0.0), a, b, complex(0.0, 0.0), c).
";
    both(src, "[1+i, 2, 3, 4-i]");
}

#[test]
fn cconjtranspose_is_the_hermitian_adjoint() {
    // A = [[1+2i, 3+4i]] (1×2) ⇒ Aᴴ = [[1-2i], [3-4i]] (2×1) = [1-2i, 3-4i].
    let src = "\
## Main
Let a be [complex(1.0, 2.0), complex(3.0, 4.0)].
Show cconjtranspose(1, 2, a).
";
    both(src, "[1-2i, 3-4i]");
}

#[test]
fn cgesv_solves_a_complex_linear_system() {
    // A = [[1+i, 0], [0, 1-i]], b = [2i, 2] ⇒ x = [1+i, 1+i] (A·x = b exactly).
    let src = "\
## Main
Let a be [complex(1.0, 1.0), complex(0.0, 0.0), complex(0.0, 0.0), complex(1.0, -1.0)].
Let b be [complex(0.0, 2.0), complex(2.0, 0.0)].
Show cgesv(2, a, b).
";
    both(src, "[1+i, 1+i]");
}

#[test]
fn cgesv_needs_a_row_interchange() {
    // A = [[0, 1], [1, 0]] — the (1,1) pivot is ZERO, so the solve MUST interchange rows or it
    // divides by zero. The anti-diagonal permutation gives x = [b₂, b₁] = [3+4i, 1+2i], exactly.
    let src = "\
## Main
Let a be [complex(0.0, 0.0), complex(1.0, 0.0), complex(1.0, 0.0), complex(0.0, 0.0)].
Let b be [complex(1.0, 2.0), complex(3.0, 4.0)].
Show cgesv(2, a, b).
";
    both(src, "[3+4i, 1+2i]");
}

// NOTE — a residual round-trip that reconstructs b via `cgemv(a, cgesv(a, b))` is deliberately
// NOT included here: it must REUSE the matrix `a` after passing it to `cgesv`, and while the
// tree-walker's value-semantics copy-on-write is intentionally gated off (the in-progress MVS
// migration — see `ensure_collection_owned` / `value_semantics_enabled`), reusing a generic-repr
// (`ListRepr::Boxed`, which a `Seq of ComplexF` uses) argument right after a call that mutates a
// local copy of it is aliasing-observable and unreliable. `cgesv` itself is value-semantic-clean;
// its correctness is proven directly by the two solves above, so the round-trip adds no coverage.

#[test]
fn cpotrf_factors_a_2x2_hermitian_positive_definite() {
    // A = [[4, 2i], [-2i, 5]] (HPD) ⇒ L = [[2, 0], [-i, 2]], since L·Lᴴ = A.
    let src = "\
## Main
Let a be [complex(4.0, 0.0), complex(0.0, 2.0), complex(0.0, -2.0), complex(5.0, 0.0)].
Show cpotrf(2, a).
";
    both(src, "[2, 0, -i, 2]");
}

#[test]
fn cpotrf_factors_a_3x3_hermitian_positive_definite() {
    // A = [[4, 2+2i, 0], [2-2i, 6, 0], [0, 0, 9]] ⇒ L = [[2,0,0],[1-i,2,0],[0,0,3]].
    let src = "\
## Main
Let a be [complex(4.0, 0.0), complex(2.0, 2.0), complex(0.0, 0.0), complex(2.0, -2.0), complex(6.0, 0.0), complex(0.0, 0.0), complex(0.0, 0.0), complex(0.0, 0.0), complex(9.0, 0.0)].
Show cpotrf(3, a).
";
    both(src, "[2, 0, 0, 1-i, 2, 0, 0, 0, 3]");
}

#[test]
fn cposv_solves_a_2x2_hermitian_positive_definite_system() {
    // A = [[4, 2i], [-2i, 5]], x = [1, 1] ⇒ b = A·x = [4+2i, 5-2i]. Recover x.
    let src = "\
## Main
Let a be [complex(4.0, 0.0), complex(0.0, 2.0), complex(0.0, -2.0), complex(5.0, 0.0)].
Let b be [complex(4.0, 2.0), complex(5.0, -2.0)].
Show cposv(2, a, b).
";
    both(src, "[1, 1]");
}

#[test]
fn cposv_solves_a_3x3_hermitian_positive_definite_system() {
    // A = [[4, 2+2i, 0], [2-2i, 6, 0], [0, 0, 9]], x = [1,1,1] ⇒ b = [6+2i, 8-2i, 9].
    let src = "\
## Main
Let a be [complex(4.0, 0.0), complex(2.0, 2.0), complex(0.0, 0.0), complex(2.0, -2.0), complex(6.0, 0.0), complex(0.0, 0.0), complex(0.0, 0.0), complex(0.0, 0.0), complex(9.0, 0.0)].
Let b be [complex(6.0, 2.0), complex(8.0, -2.0), complex(9.0, 0.0)].
Show cposv(3, a, b).
";
    both(src, "[1, 1, 1]");
}

#[test]
fn cgetrf_factorization_reused_across_multiple_rhs() {
    // Factor A once with cgetrf, solve TWO right-hand sides with cgetrs. A = diag(1+i, 1-i):
    // b₁ = [2i, 2] ⇒ [1+i, 1+i]; b₂ = [1+i, 1-i] ⇒ [1, 1]. (Struct return works in AOT via the
    // `## Definition` heading that keeps CLuResult in the prelude splice.)
    let src = "\
## Main
Let a be [complex(1.0, 1.0), complex(0.0, 0.0), complex(0.0, 0.0), complex(1.0, -1.0)].
Let f be cgetrf(2, a).
Let x1 be cgetrs(2, f, [complex(0.0, 2.0), complex(2.0, 0.0)]).
Let x2 be cgetrs(2, f, [complex(1.0, 1.0), complex(1.0, -1.0)]).
Show x1.
Show x2.
";
    both(src, "[1+i, 1+i]\n[1, 1]");
}
