//! Completed L2/L3 BLAS surface — real (`blas.lg`) and complex (`clapack.lg`). Hand-computed values,
//! every check on the tree-walker≡VM AND the Rust-AOT tier.
mod common;
use common::{assert_all_tiers, assert_compiled_equals_interpreted};

fn both(src: &str, expected: &str) {
    assert_all_tiers(src, expected);
    assert_compiled_equals_interpreted(src);
}

#[test]
fn symv_symmetric_matvec() {
    // A=[[2,1],[1,2]], x=[1,1] ⇒ [3,3].
    both(
        "## Main\nShow symv(2, 1.0, [2.0,1.0,1.0,2.0], [1.0,1.0], 0.0, [0.0,0.0]).\n",
        "[3, 3]",
    );
}
#[test]
fn trmv_upper_triangular_matvec() {
    // A=[[2,1],[0,3]] (upper), x=[1,1] ⇒ [3,3].
    both(
        "## Main\nShow trmv(2, [2.0,1.0,0.0,3.0], [1.0,1.0]).\n",
        "[3, 3]",
    );
}
#[test]
fn syr_rank1_symmetric() {
    // x=[1,2] ⇒ x·xᵀ = [[1,2],[2,4]].
    both(
        "## Main\nShow syr(2, 1.0, [1.0,2.0], [0.0,0.0,0.0,0.0]).\n",
        "[1, 2, 2, 4]",
    );
}
#[test]
fn trmm_triangular_times_identity() {
    // A=[[2,1],[0,3]]·I = A.
    both(
        "## Main\nShow trmm(2, 2, 1.0, [2.0,1.0,0.0,3.0], [1.0,0.0,0.0,1.0]).\n",
        "[2, 1, 0, 3]",
    );
}
#[test]
fn trsm_solve_against_self_is_identity() {
    // A·X = A ⇒ X = I.
    both(
        "## Main\nShow trsm(2, 2, 1.0, [2.0,1.0,0.0,3.0], [2.0,1.0,0.0,3.0]).\n",
        "[1, 0, 0, 1]",
    );
}
#[test]
fn symm_symmetric_times_identity() {
    both("## Main\nShow symm(2, 2, 1.0, [2.0,1.0,1.0,2.0], [1.0,0.0,0.0,1.0], 0.0, [0.0,0.0,0.0,0.0]).\n", "[2, 1, 1, 2]");
}
#[test]
fn syr2k_rank2k_update() {
    // A=[1;2], B=[3;4] (2×1) ⇒ A·Bᵀ+B·Aᵀ = [[6,10],[10,16]].
    both(
        "## Main\nShow syr2k(2, 1, 1.0, [1.0,2.0], [3.0,4.0], 0.0, [0.0,0.0,0.0,0.0]).\n",
        "[6, 10, 10, 16]",
    );
}

#[test]
fn cgeru_unconjugated_rank1() {
    // x=[1+i,2], y=[3,4i] ⇒ x·yᵀ = [[3+3i,-4+4i],[6,8i]].
    both("## Main\nShow cgeru(2, 2, complex(1.0, 0.0), [complex(1.0, 1.0),complex(2.0, 0.0)], [complex(3.0, 0.0),complex(0.0, 4.0)], [complex(0.0, 0.0),complex(0.0, 0.0),complex(0.0, 0.0),complex(0.0, 0.0)]).\n", "[3+3i, -4+4i, 6, 8i]");
}
#[test]
fn cgerc_conjugated_rank1() {
    // x·yᴴ with y=[3,4i] ⇒ conj → [[3+3i,4-4i],[6,-8i]].
    both("## Main\nShow cgerc(2, 2, complex(1.0, 0.0), [complex(1.0, 1.0),complex(2.0, 0.0)], [complex(3.0, 0.0),complex(0.0, 4.0)], [complex(0.0, 0.0),complex(0.0, 0.0),complex(0.0, 0.0),complex(0.0, 0.0)]).\n", "[3+3i, 4-4i, 6, -8i]");
}
#[test]
fn chemv_hermitian_matvec() {
    // A=[[2,i],[-i,2]], x=[1,1] ⇒ [2+i, 2-i].
    both("## Main\nShow chemv(2, complex(1.0, 0.0), [complex(2.0, 0.0),complex(0.0, 1.0),complex(0.0, -1.0),complex(2.0, 0.0)], [complex(1.0, 0.0),complex(1.0, 0.0)], complex(0.0, 0.0), [complex(0.0, 0.0),complex(0.0, 0.0)]).\n", "[2+i, 2-i]");
}
#[test]
fn chemm_hermitian_times_identity() {
    both("## Main\nShow chemm(2, 2, complex(1.0, 0.0), [complex(2.0, 0.0),complex(0.0, 1.0),complex(0.0, -1.0),complex(2.0, 0.0)], [complex(1.0, 0.0),complex(0.0, 0.0),complex(0.0, 0.0),complex(1.0, 0.0)], complex(0.0, 0.0), [complex(0.0, 0.0),complex(0.0, 0.0),complex(0.0, 0.0),complex(0.0, 0.0)]).\n", "[2, i, -i, 2]");
}
#[test]
fn cherk_hermitian_rank_k() {
    // A=[1+i;2-i] (2×1) ⇒ A·Aᴴ = [[2,1+3i],[1-3i,5]].
    both("## Main\nShow cherk(2, 1, 1.0, [complex(1.0, 1.0),complex(2.0, -1.0)], 0.0, [complex(0.0, 0.0),complex(0.0, 0.0),complex(0.0, 0.0),complex(0.0, 0.0)]).\n", "[2, 1+3i, 1-3i, 5]");
}
#[test]
fn ctrmm_triangular_times_identity() {
    both("## Main\nShow ctrmm(2, 2, complex(1.0, 0.0), [complex(2.0, 0.0),complex(1.0, 0.0),complex(0.0, 0.0),complex(3.0, 0.0)], [complex(1.0, 0.0),complex(0.0, 0.0),complex(0.0, 0.0),complex(1.0, 0.0)]).\n", "[2, 1, 0, 3]");
}
#[test]
fn ctrsm_solve_against_self_is_identity() {
    both("## Main\nShow ctrsm(2, 2, complex(1.0, 0.0), [complex(2.0, 0.0),complex(1.0, 0.0),complex(0.0, 0.0),complex(3.0, 0.0)], [complex(2.0, 0.0),complex(1.0, 0.0),complex(0.0, 0.0),complex(3.0, 0.0)]).\n", "[1, 0, 0, 1]");
}
