//! Stdlib-parity — `randGamma` (gammavariate) + `randBeta` (betavariate): rejection-sampled
//! distributions over LCG uniforms. randGamma = Marsaglia-Tsang (shape/scale, incl. the alpha<1
//! boost branch); randBeta = y1/(y1+y2) of two gamma draws. Golden values from the canonical
//! tree-walker oracle, floor-scaled.
//!
//! TIER SCOPE — tw/VM/AOT-Rust/C. `ln`, `cos` and `exp` are `math.lg`'s own LOGOS definitions on every
//! tier, so each rejection loop takes the same iterations everywhere. `randBeta` also pins a
//! specialization: its second draw `randGamma(seed + 7919, beta, 1.0)` is cloned for the literal scale
//! while the shape stays the caller's `beta`.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Gamma distribution (Marsaglia-Tsang), alpha>=1 and the alpha<1 boost branch. py: random.gammavariate
#[test]
fn parity_rand_gamma() {
    let src = "## Main\n\
        Show floor(randGamma(42, 2.0, 1.0) * 1000.0).\n\
        Show floor(randGamma(42, 5.0, 2.0) * 1000.0).\n\
        Show floor(randGamma(7, 0.5, 1.0) * 1000.0).\n";
    assert_all_tiers(src, "449\n4574\n4");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "449\n4574\n4");
}

/// Beta distribution (ratio of two gamma draws). py: random.betavariate
#[test]
fn parity_rand_beta() {
    let src = "## Main\n\
        Show floor(randBeta(42, 2.0, 3.0) * 1000.0).\n\
        Show floor(randBeta(7, 0.5, 0.5) * 1000.0).\n";
    assert_all_tiers(src, "201\n9");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "201\n9");
}

/// Von Mises circular distribution (Best-Fisher rejection), incl. the kappa→0 uniform branch and
/// acos-based angle. py: random.vonmisesvariate
#[test]
fn parity_rand_vonmises() {
    let src = "## Main\n\
        Show floor(randVonmises(42, 0.0, 2.0) * 1000.0).\n\
        Show floor(randVonmises(42, 3.14159, 5.0) * 1000.0).\n\
        Show floor(randVonmises(7, 1.0, 0.5) * 1000.0).\n\
        Show floor(randVonmises(99, 0.0, 0.0000001) * 1000.0).\n";
    assert_all_tiers(src, "5804\n2833\n746\n5295");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "5804\n2833\n746\n5295");
}

/// Beta alone on the C tier: one call site, so the specialized second draw is the only clone.
#[test]
fn parity_rand_beta_c() {
    assert_c_output(
        "## Main\nShow floor(randBeta(42, 2.0, 3.0) * 1000.0).\n",
        "201",
    );
}
