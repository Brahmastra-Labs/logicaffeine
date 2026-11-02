//! Stdlib-parity — `erfinv` (Go math.Erfinv) + `erfcinv` (Go math.Erfcinv): inverse error and
//! inverse complementary error functions via Giles' rational approximation. Closed-form (ln/sqrt +
//! Horner), floor-scaled. Verified: erf(erfinv(x)) ≈ x, erfinv(0.5)≈0.47694. tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Inverse error function. go: math.Erfinv
#[test]
fn parity_math_erfinv() {
    let src = "## Main\n\
        Show floor(erfinv(0.5) * 1000.0).\n\
        Show floor(erfinv(0.9) * 1000.0).\n\
        Show floor(erfinv(0.0 - 0.5) * 1000.0).\n\
        Show floor(erfinv(0.99) * 1000.0).\n";
    assert_all_tiers(src, "476\n1163\n-477\n1821");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "476\n1163\n-477\n1821");
}

/// Inverse complementary error function. go: math.Erfcinv
#[test]
fn parity_math_erfcinv() {
    let src = "## Main\n\
        Show floor(erfcinv(0.5) * 1000.0).\n\
        Show floor(erfcinv(0.1) * 1000.0).\n";
    assert_all_tiers(src, "476\n1163");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "476\n1163");
}
