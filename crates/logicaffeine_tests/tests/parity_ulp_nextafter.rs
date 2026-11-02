//! Stdlib-parity — `ulp` (Python math.ulp) + `nextafter` (Python math.nextafter / Go math.Nextafter /
//! C nextafter): unit-in-last-place and the adjacent representable double, in PURE LOGOS via
//! frexp/ldexp/copysign (no float-bit builtin). Results are exact powers of two; probed × 2^52 to
//! clean integers. Covers binade boundaries (half-ulp step) + negatives. tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Unit in the last place. py: math.ulp
#[test]
fn parity_math_ulp() {
    let src = "## Main\n\
        Show floor(ulp(1.0) * 4503599627370496.0).\n\
        Show floor(ulp(1024.0) * 4503599627370496.0).\n\
        Show floor(ulp(0.5) * 4503599627370496.0 * 2.0).\n\
        Show floor(ulp(0.0 - 2.0) * 4503599627370496.0).\n";
    assert_all_tiers(src, "1\n1024\n1\n2");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "1\n1024\n1\n2");
}

/// Next representable double toward y. py: math.nextafter · go: math.Nextafter · c: nextafter
#[test]
fn parity_math_nextafter() {
    let src = "## Main\n\
        Show floor((nextafter(1.0, 2.0) - 1.0) * 4503599627370496.0).\n\
        Show floor((2.0 - nextafter(2.0, 0.0)) * 4503599627370496.0).\n\
        Show floor((nextafter(2.0, 4.0) - 2.0) * 4503599627370496.0).\n";
    assert_all_tiers(src, "1\n1\n2");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "1\n1\n2");
}
