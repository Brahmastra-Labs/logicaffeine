//! Stdlib-parity — `randGauss` (Python random.gauss / normalvariate): deterministic normal-
//! distribution sample via Box-Muller over two LCG uniforms. Values computed against CPython's
//! sqrt/log/cos at seed 42 (z ≈ -1.37096). Floor-scaled at safe margins. tw ≡ VM ≡ AOT-Rust ≡ C.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Deterministic normal sample (Box-Muller). py: random.gauss / random.normalvariate
#[test]
fn parity_rand_gauss() {
    let src = "## Main\n\
        Show floor(randGauss(42, 0.0, 1.0) * 10.0).\n\
        Show floor(randGauss(42, 100.0, 15.0) * 100.0).\n";
    assert_all_tiers(src, "-14\n7943");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "-14\n7943");
}
