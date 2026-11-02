//! Full-audit: the second math float/special wave (recip/fract/logBase/powi/divEuclid/remEuclid/
//! rint/lrint/gamma/lgamma/isNormal/fpclassify) on the NATIVE tiers (AOT-Rust + C), beyond the
//! tree-walker/VM `assert_all_tiers` cover. Proves the Lanczos series, Euclidean division, and
//! round-half-even lower identically through the AOT-Rust and C backends. C expected values are
//! AUTHORITATIVE (verified against Python's `math`).
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_c_output, assert_compiled_equals_interpreted};

/// One program exercising every capability — AOT-Rust output must equal the interpreter's.
const ALL: &str = "## Main
Show recip(4.0).
Show fract(0.0 - 3.75).
Show floor(logBase(10.0, 2.0) * 100.0).
Show powi(2.0, 0 - 3).
Show divEuclid(0.0 - 7.0, 3.0).
Show remEuclid(0.0 - 7.0, 3.0).
Show rint(2.5).
Show lrint(3.5).
Show floor(gamma(2.5) * 1000.0).
Show floor(lgamma(100.0) * 100.0).
Show isNormal(0.0).
Show fpclassify(1.0).
";

#[test]
fn float2_batch_on_aot_rust() {
    assert_compiled_equals_interpreted(ALL);
}

#[test]
fn float2_batch_on_c() {
    assert_c_output(
        ALL,
        "0.25\n-0.75\n332\n0.125\n-3\n2\n2\n4\n1329\n35913\nfalse\nnormal",
    );
}
