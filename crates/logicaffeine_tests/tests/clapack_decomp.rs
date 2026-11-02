//! Complex LAPACK decompositions (Wave 2): cgetri (inverse), cgeqrf (QR), clange1 (1-norm),
//! cgecon (condition). Hand-computed values, all tiers + AOT.
mod common;
use common::{assert_all_tiers, assert_compiled_equals_interpreted};

fn both(src: &str, expected: &str) {
    assert_all_tiers(src, expected);
    assert_compiled_equals_interpreted(src);
}

#[test]
fn cgetri_diagonal_complex_inverse() {
    // A=diag(1+i, 1-i) ⇒ A⁻¹ = diag(0.5-0.5i, 0.5+0.5i).
    both("## Main\nShow cgetri(2, [complex(1.0, 1.0), complex(0.0, 0.0), complex(0.0, 0.0), complex(1.0, -1.0)]).\n",
         "[0.5-0.5i, 0, 0, 0.5+0.5i]");
}

#[test]
fn cgeqrf_antidiagonal_qr() {
    // A=[[0,5],[5,0]] ⇒ Q=[[0,1],[1,0]], R=[[5,0],[0,5]] (exact).
    let src = "\
## Main
Let f be cgeqrf(2, 2, [complex(0.0, 0.0), complex(5.0, 0.0), complex(5.0, 0.0), complex(0.0, 0.0)]).
Show f's q.
Show f's r.
";
    both(src, "[0, 1, 1, 0]\n[5, 0, 0, 5]");
}

#[test]
fn clange1_max_column_sum() {
    // A=[[1,2],[3,4]] ⇒ column sums 4, 6 ⇒ max 6.
    both("## Main\nShow clange1(2, [complex(1.0, 0.0), complex(2.0, 0.0), complex(3.0, 0.0), complex(4.0, 0.0)]).\n", "6");
}

#[test]
fn cgecon_one_norm_condition() {
    // A=diag(2,4) ⇒ ‖A‖₁·‖A⁻¹‖₁ = 4·0.5 = 2.
    both("## Main\nShow cgecon(2, [complex(2.0, 0.0), complex(0.0, 0.0), complex(0.0, 0.0), complex(4.0, 0.0)]).\n", "2");
}
