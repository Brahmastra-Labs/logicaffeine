//! Stdlib-parity — sincos (math.Sincos) + inf/nan constants (math.Inf/NaN, math.inf/nan).
//! inf/nan produced by float overflow; tested via isInf/isFinite/isNaN to dodge display-format
//! divergence. Byte-identical all tiers.
#![allow(unused_imports, dead_code)]
mod common;
use common::{assert_all_tiers, assert_c_output};

#[test]
fn parity_math_sincos() {
    assert_all_tiers("## Main\nShow sincos(0.0).\n", "(0, 1)");
}
#[test]
fn parity_math_inf() {
    assert_all_tiers("## Main\nShow isInf(mathInf(1)).\nShow isInf(mathInf(0 - 1)).\nShow isFinite(mathInf(1)).\n", "true\ntrue\nfalse");
}
#[test]
fn parity_math_nan() {
    assert_all_tiers(
        "## Main\nShow isNaN(mathNan()).\nShow isFinite(mathNan()).\n",
        "true\nfalse",
    );
}
#[test]
fn math_consts_on_c() {
    assert_c_output(
        "## Main\nShow isInf(mathInf(1)).\nShow isNaN(mathNan()).\n",
        "true\ntrue",
    );
}
