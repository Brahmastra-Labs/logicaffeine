//! Stdlib-parity — `randLogNorm` (lognormvariate) and `randTriangular` (triangular). CPython-
//! verified at seed 42. tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Log-normal distribution (exp of a normal sample). py: random.lognormvariate
#[test]
fn parity_rand_lognorm() {
    let src = "## Main\nShow floor(randLogNorm(42, 0.0, 1.0) * 1000.0).\n";
    assert_all_tiers(src, "253");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "253");
}

/// Triangular distribution over [low, high] peaking at mode. py: random.triangular
#[test]
fn parity_rand_triangular() {
    let src = "## Main\nShow floor(randTriangular(42, 0.0, 10.0, 5.0) * 100.0).\n";
    assert_all_tiers(src, "439");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "439");
}
