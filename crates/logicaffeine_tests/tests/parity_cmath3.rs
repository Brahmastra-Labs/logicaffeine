//! Stdlib-parity north-star corpus — third `complex`/cmath wave. Closes the LAST unclaimed
//! `go math/cmplx` symbols (Cot, Pow, Inf, NaN) and Python `cmath` symbols (infj, nanj).
//!
//! `cCot`/`cPow` are ordinary floating-point complex functions: finite results extracted with the
//! built-in `re`/`im` and floor-scaled ×1000. Those integers are AUTHORITATIVE — computed against
//! Python's `cmath` (the provenance) at inputs with a comfortable floor margin (>0.2). Byte-identical
//! tw ≡ VM ≡ AOT-Rust ≡ C.
//!
//! `cInf`/`cNan`/`cInfj`/`cNanj` produce NON-FINITE complex values. The interpreter/VM/AOT-Rust
//! `ComplexF` is a real `{re: f64, im: f64}`, so inf/nan ride through natively — but the C tier
//! stores `ComplexF` as exact dyadic rationals (`rat_from_f64`), which cannot represent inf/nan.
//! They are therefore proven on tw ≡ VM ≡ AOT-Rust only (an honest boundary of the exact-rational
//! C float-complex tier), and asserted via the `cIsInf`/`cIsNaN`/`isInf`/`isNaN` predicates to
//! dodge non-finite display divergence.
#![allow(unused_imports, dead_code)]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Complex cotangent `cos(z)/sin(z)`. go: math/cmplx.Cot
#[test]
fn parity_cmath_cot() {
    let src = "## Main\n\
        Show floor(re(cCot(0.6, 0.9)) * 1000.0).\n\
        Show floor(im(cCot(0.6, 0.9)) * 1000.0).\n\
        Show floor(re(cCot(0.7, 0.0)) * 1000.0).\n\
        Show floor(im(cCot(0.7, 0.0)) * 1000.0).\n";
    assert_all_tiers(src, "339\n-1072\n1187\n0");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "339\n-1072\n1187\n0");
}

/// Complex power `z^w = exp(w·log z)`, principal branch, with the `0^w` edge cases. go: math/cmplx.Pow
#[test]
fn parity_cmath_pow() {
    let src = "## Main\n\
        Show floor(re(cPow(2.0, 0.0, 0.7, 0.3)) * 1000.0).\n\
        Show floor(im(cPow(2.0, 0.0, 0.7, 0.3)) * 1000.0).\n\
        Show floor(re(cPow(2.0, 0.0, 0.5, 0.0)) * 1000.0).\n\
        Show floor(im(cPow(2.0, 0.0, 0.5, 0.0)) * 1000.0).\n\
        Show floor(re(cPow(0.0, 0.0, 0.0, 0.0)) * 1000.0).\n\
        Show floor(re(cPow(0.0, 0.0, 3.0, 0.0)) * 1000.0).\n";
    assert_all_tiers(src, "1589\n335\n1414\n0\n1000\n0");
    assert_compiled_equals_interpreted(src);
    assert_c_output(src, "1589\n335\n1414\n0\n1000\n0");
}

/// The complex infinity `∞ + ∞i`. go: math/cmplx.Inf
#[test]
fn parity_cmath_complex_inf() {
    let src = "## Main\n\
        Show cIsInf(re(cInf()), im(cInf())).\n\
        Show isInf(re(cInf())).\n\
        Show isInf(im(cInf())).\n";
    assert_all_tiers(src, "true\ntrue\ntrue");
    assert_compiled_equals_interpreted(src);
}

/// The complex NaN `NaN + NaN·i`. go: math/cmplx.NaN
#[test]
fn parity_cmath_complex_nan() {
    let src = "## Main\n\
        Show cIsNaN(re(cNan()), im(cNan())).\n\
        Show isNaN(re(cNan())).\n\
        Show isNaN(im(cNan())).\n";
    assert_all_tiers(src, "true\ntrue\ntrue");
    assert_compiled_equals_interpreted(src);
}

/// The imaginary-axis infinity `0 + ∞i`. py: cmath.infj
#[test]
fn parity_cmath_infj() {
    let src = "## Main\n\
        Show floor(re(cInfj()) * 1000.0).\n\
        Show isFinite(re(cInfj())).\n\
        Show isInf(im(cInfj())).\n";
    assert_all_tiers(src, "0\ntrue\ntrue");
    assert_compiled_equals_interpreted(src);
}

/// The imaginary-axis NaN `0 + NaN·i`. py: cmath.nanj
#[test]
fn parity_cmath_nanj() {
    let src = "## Main\n\
        Show floor(re(cNanj()) * 1000.0).\n\
        Show isFinite(re(cNanj())).\n\
        Show isNaN(im(cNanj())).\n";
    assert_all_tiers(src, "0\ntrue\ntrue");
    assert_compiled_equals_interpreted(src);
}
