//! Stdlib-parity north-star corpus - domain: complex
//! LOGOS has a built-in `Complex` value type + `complex(re, im)` builtin; arithmetic and magnitude
//! work natively (no cmath library). Extraction/transcendental-on-complex are not yet surfaced.
#![allow(unused_imports, dead_code)]
mod common;
use common::assert_all_tiers;
/// Construct a complex number. py: builtins.complex
#[test]
fn parity_complex_make() {
    assert_all_tiers("## Main\nShow complex(3, 4).\n", "3+4i");
}
/// Magnitude of a complex number. go: math/cmplx.Abs
#[test]
fn parity_complex_abs() {
    assert_all_tiers("## Main\nShow abs(complex(3, 4)).\n", "5");
}
/// Complex arithmetic (add + the imaginary unit i·i = −1). go/py: native `+`/`*` on complex
#[test]
fn parity_complex_arith() {
    assert_all_tiers(
        "## Main\nShow complex(1, 2) + complex(3, 4).\nShow complex(0, 1) * complex(0, 1).\n",
        "4+6i\n-1",
    );
}

// Complex transcendentals (math.lg), computed from the real transcendentals via the standard
// formulas — z = x + yi. Tested at inputs whose results are exact so Show is byte-stable.

/// Complex exponential e^z = e^x(cos y + i sin y). py: cmath.exp · go: math/cmplx.Exp
#[test]
fn parity_cmath_exp() {
    assert_all_tiers("## Main\nShow cExp(0.0, 0.0).\n", "1");
}
/// Complex natural log ln|z| + i·arg z. py: cmath.log · go: math/cmplx.Log
#[test]
fn parity_cmath_log() {
    assert_all_tiers("## Main\nShow cLog(1.0, 0.0).\n", "0");
}
/// Principal complex square root. py: cmath.sqrt · go: math/cmplx.Sqrt
#[test]
fn parity_cmath_sqrt() {
    assert_all_tiers("## Main\nShow cSqrt(4.0, 0.0).\n", "2");
}
/// Complex sine. py: cmath.sin · go: math/cmplx.Sin
#[test]
fn parity_cmath_sin() {
    assert_all_tiers("## Main\nShow cSin(0.0, 0.0).\n", "0");
}
/// Complex cosine. py: cmath.cos · go: math/cmplx.Cos
#[test]
fn parity_cmath_cos() {
    assert_all_tiers("## Main\nShow cCos(0.0, 0.0).\n", "1");
}
/// Complex hyperbolic sine. py: cmath.sinh · go: math/cmplx.Sinh
#[test]
fn parity_cmath_sinh() {
    assert_all_tiers("## Main\nShow cSinh(0.0, 0.0).\n", "0");
}
/// Complex hyperbolic cosine. py: cmath.cosh · go: math/cmplx.Cosh
#[test]
fn parity_cmath_cosh() {
    assert_all_tiers("## Main\nShow cCosh(0.0, 0.0).\n", "1");
}
/// Complex tangent. py: cmath.tan · go: math/cmplx.Tan
#[test]
fn parity_cmath_tan() {
    assert_all_tiers("## Main\nShow cTan(0.0, 0.0).\n", "0");
}
/// Complex hyperbolic tangent. py: cmath.tanh · go: math/cmplx.Tanh
#[test]
fn parity_cmath_tanh() {
    assert_all_tiers("## Main\nShow cTanh(0.0, 0.0).\n", "0");
}
/// Phase (argument) of z. py: cmath.phase · go: math/cmplx.Phase
#[test]
fn parity_cmath_phase() {
    assert_all_tiers(
        "## Main\nShow floor(cPhase(0.0, 1.0) * 1000.0).\nShow floor(cPhase(1.0, 0.0) * 1000.0).\n",
        "1570\n0",
    );
}
/// Complex from polar coordinates. py: cmath.rect · go: math/cmplx.Rect
#[test]
fn parity_cmath_rect() {
    assert_all_tiers("## Main\nShow cRect(2.0, 0.0).\n", "2");
}
/// Complex conjugate. py: builtins.complex.conjugate · go: math/cmplx.Conj
#[test]
fn parity_cmath_conj() {
    assert_all_tiers("## Main\nShow cConj(3, 4).\nShow cConj(5, 0).\n", "3-4i\n5");
}
