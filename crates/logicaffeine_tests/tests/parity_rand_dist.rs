//! Stdlib-parity — more deterministic seeded distributions: `randExpo` (expovariate), `randPareto`
//! (paretovariate), `randWeibull` (weibullvariate). One transcendental each over a uniform.
//! Values computed against CPython at seed 42 (u≈0.38699). tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Exponential distribution. py: random.expovariate
#[test]
fn parity_rand_expo() {
    let src = "## Main\nShow floor(randExpo(42, 1.0) * 1000.0).\n";
    assert_all_tiers(src, "489");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "489");
}

/// Pareto distribution. py: random.paretovariate
#[test]
fn parity_rand_pareto() {
    let src = "## Main\nShow floor(randPareto(42, 2.0) * 1000.0).\n";
    assert_all_tiers(src, "1277");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "1277");
}

/// Weibull distribution. py: random.weibullvariate
#[test]
fn parity_rand_weibull() {
    let src = "## Main\nShow floor(randWeibull(42, 1.0, 2.0) * 1000.0).\n";
    assert_all_tiers(src, "699");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "699");
}
