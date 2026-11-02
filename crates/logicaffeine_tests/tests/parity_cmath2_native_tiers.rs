//! Full-audit: the second cmath wave (polar/isfinite/isinf/isnan/isclose/log10 + inverse trig) on
//! the NATIVE tiers (AOT-Rust + C), beyond the tw/VM `assert_all_tiers` cover. Proves the complex
//! square-root parts, branch-cut compositions, tuple return (cPolar) and `re`/`im` extraction lower
//! identically through AOT-Rust and C. C expected values are AUTHORITATIVE (Python `cmath`).
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_c_output, assert_compiled_equals_interpreted};

const ALL: &str = "## Main
Show floor(re(cAsinh(0.5, 0.5)) * 1000.0).
Show floor(im(cAcosh(2.0, 1.0)) * 1000.0).
Show floor(re(cAtanh(0.3, 0.7)) * 1000.0).
Show floor(im(cAsin(0.5, 0.5)) * 1000.0).
Show floor(re(cAcos(0.5, 0.5)) * 1000.0).
Show floor(im(cAtan(2.0, 1.0)) * 1000.0).
Show cIsClose(1.0, 2.0, 1.0, 2.0).
Show cLog10(1.0, 0.0).
Show cPolar(5.0, 0.0).
";

#[test]
fn cmath2_batch_on_aot_rust() {
    assert_compiled_equals_interpreted(ALL);
}

#[test]
fn cmath2_batch_on_c() {
    assert_c_output(ALL, "530\n507\n199\n530\n1118\n173\ntrue\n0\n(5, 0)");
}
