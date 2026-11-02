//! **Exact complex linear algebra over the Gaussian rationals ℚ[i].**
//!
//! LOGOS has a first-class exact `Complex` (`complex(re, im)` over `Rational` parts, exact `+ − × ÷`).
//! The sqrt-free complex operations — dot product, GEMM, Cramer/Gaussian solve, determinant — are
//! therefore EXACT, with no rounding. This is the complex linear algebra LAPACK's f64 `Z` routines can
//! only approximate: here `A·x − b` is IDENTICALLY zero, not `1e-16`. `complex()` requires EXACT
//! operands (integers/rationals), which is the whole point.

mod common;
use common::assert_all_tiers;

#[test]
fn exact_complex_dot_product() {
    // ⟨[1+2i, 3], [i, 2+2i]⟩ = (1+2i)i + 3(2+2i) = (−2+i) + (6+6i) = 4+7i, exactly, cross-tier.
    let src = "\
## To cdot (n: Int, x: Seq, y: Seq) -> Complex:
    Let mutable acc be complex(0, 0).
    Repeat for i from 1 to n:
        Set acc to acc + (item i of x) * (item i of y).
    Return acc.

## Main
Let mutable x be a new Seq.
Push complex(1, 2) to x.
Push complex(3, 0) to x.
Let mutable y be a new Seq.
Push complex(0, 1) to y.
Push complex(2, 2) to y.
Show cdot(2, x, y).
";
    assert_all_tiers(src, "4+7i");
}

#[test]
fn exact_complex_2x2_solve_has_identically_zero_residual() {
    // A = [[2,1],[1,2]], b = [1+i, 2]. Cramer's rule gives x = [2i/3, 1 − i/3] — FRACTIONAL Gaussian
    // rationals, held EXACTLY. The reconstruction A·x − b is 0 + 0i in BOTH components (prints "0"),
    // with zero rounding — the exact complex solve LAPACK's Z routines cannot produce.
    let src = "\
## To csolve2 (a: Seq, b: Seq) -> Seq:
    Let a00 be item 1 of a.
    Let a01 be item 2 of a.
    Let a10 be item 3 of a.
    Let a11 be item 4 of a.
    Let b0 be item 1 of b.
    Let b1 be item 2 of b.
    Let det be a00 * a11 - a01 * a10.
    Let x0 be (b0 * a11 - a01 * b1) / det.
    Let x1 be (a00 * b1 - b0 * a10) / det.
    Let mutable x be a new Seq.
    Push x0 to x.
    Push x1 to x.
    Return x.

## Main
Let mutable a be a new Seq.
Push complex(2, 0) to a.
Push complex(1, 0) to a.
Push complex(1, 0) to a.
Push complex(2, 0) to a.
Let mutable b be a new Seq.
Push complex(1, 1) to b.
Push complex(2, 0) to b.
Let x be csolve2(a, b).
Let r0 be (item 1 of a) * (item 1 of x) + (item 2 of a) * (item 2 of x).
Let r1 be (item 3 of a) * (item 1 of x) + (item 4 of a) * (item 2 of x).
Show r0 - (item 1 of b).
Show r1 - (item 2 of b).
";
    assert_all_tiers(src, "0\n0");
}
