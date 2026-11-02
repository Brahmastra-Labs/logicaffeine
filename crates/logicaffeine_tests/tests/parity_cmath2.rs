//! Stdlib-parity north-star corpus — second `complex`/cmath wave. Closes unclaimed Python `cmath`
//! symbols (polar, isfinite/isinf/isnan, isclose, log10) and the complex inverse-trig family
//! (asin/acos/atan/asinh/acosh/atanh). Complex results are extracted with the built-in `re`/`im`
//! and floor-scaled at ×1000; those integers are AUTHORITATIVE — verified equal to Python `cmath`
//! (branch-cut correct). Byte-identical tw ≡ VM ≡ AOT ≡ C ≡ WASM.
#![allow(unused_imports, dead_code)]

mod common;
use common::assert_all_tiers;

/// Polar coordinates (r, phi) of a complex number. py: cmath.polar
#[test]
fn parity_cmath_polar() {
    assert_all_tiers(
        "## Main\nShow cPolar(5.0, 0.0).\nShow cPolar(0.0, 0.0).\n",
        "(5, 0)\n(0, 0)",
    );
}

/// True iff both components are finite. py: cmath.isfinite
#[test]
fn parity_cmath_isfinite() {
    assert_all_tiers("## Main\nShow cIsFinite(3.0, 4.0).\n", "true");
}

/// True iff either component is infinite. py: cmath.isinf
#[test]
fn parity_cmath_isinf() {
    assert_all_tiers("## Main\nShow cIsInf(3.0, 4.0).\n", "false");
}

/// True iff either component is NaN. py: cmath.isnan
#[test]
fn parity_cmath_isnan() {
    assert_all_tiers("## Main\nShow cIsNaN(3.0, 4.0).\n", "false");
}

/// Complex isclose (rel_tol 1e-9, abs_tol 0). py: cmath.isclose
#[test]
fn parity_cmath_isclose() {
    assert_all_tiers(
        "## Main\nShow cIsClose(1.0, 2.0, 1.0, 2.0).\nShow cIsClose(1.0, 0.0, 2.0, 0.0).\n",
        "true\nfalse",
    );
}

/// Complex base-10 logarithm ln(z)/ln(10). py: cmath.log10
#[test]
fn parity_cmath_log10() {
    assert_all_tiers("## Main\nShow cLog10(1.0, 0.0).\n", "0");
}

/// Complex inverse hyperbolic sine. py: cmath.asinh
#[test]
fn parity_cmath_asinh() {
    assert_all_tiers(
        "## Main\nShow cAsinh(0.0, 0.0).\nShow floor(re(cAsinh(0.5, 0.5)) * 1000.0).\nShow floor(im(cAsinh(0.5, 0.5)) * 1000.0).\n",
        "0\n530\n452",
    );
}

/// Complex inverse hyperbolic cosine. py: cmath.acosh
#[test]
fn parity_cmath_acosh() {
    assert_all_tiers(
        "## Main\nShow cAcosh(1.0, 0.0).\nShow floor(re(cAcosh(2.0, 1.0)) * 1000.0).\nShow floor(im(cAcosh(2.0, 1.0)) * 1000.0).\n",
        "0\n1469\n507",
    );
}

/// Complex inverse hyperbolic tangent. py: cmath.atanh
#[test]
fn parity_cmath_atanh() {
    assert_all_tiers(
        "## Main\nShow cAtanh(0.0, 0.0).\nShow floor(re(cAtanh(0.3, 0.7)) * 1000.0).\nShow floor(im(cAtanh(0.3, 0.7)) * 1000.0).\n",
        "0\n199\n639",
    );
}

/// Complex arcsine. py: cmath.asin
#[test]
fn parity_cmath_asin() {
    assert_all_tiers(
        "## Main\nShow cAsin(0.0, 0.0).\nShow floor(re(cAsin(0.5, 0.5)) * 1000.0).\nShow floor(im(cAsin(0.5, 0.5)) * 1000.0).\n",
        "0\n452\n530",
    );
}

/// Complex arccosine. py: cmath.acos
#[test]
fn parity_cmath_acos() {
    assert_all_tiers(
        "## Main\nShow cAcos(1.0, 0.0).\nShow floor(re(cAcos(0.5, 0.5)) * 1000.0).\nShow floor(im(cAcos(0.5, 0.5)) * 1000.0).\n",
        "0\n1118\n-531",
    );
}

/// Complex arctangent. py: cmath.atan
#[test]
fn parity_cmath_atan() {
    assert_all_tiers(
        "## Main\nShow cAtan(0.0, 0.0).\nShow floor(re(cAtan(2.0, 1.0)) * 1000.0).\nShow floor(im(cAtan(2.0, 1.0)) * 1000.0).\n",
        "0\n1178\n173",
    );
}
