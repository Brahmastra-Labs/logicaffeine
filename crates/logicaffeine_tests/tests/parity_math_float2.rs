//! Stdlib-parity north-star corpus — second float/special-function wave for the `math` domain.
//! Closes unclaimed C `math.h`, Rust `f64`, and Python `math` symbols. Each test is a real
//! `assert_all_tiers` behavioral spec. EXACT-valued capabilities (recip/fract/powi/divEuclid/
//! remEuclid/rint) assert full precision; the approximation capabilities (logBase/gamma/lgamma)
//! assert `floor(f(x) * SCALE)` at boundary-robust scales whose integer value is AUTHORITATIVE
//! (verified equal to Python's `math` at those floors). Byte-identical tw ≡ VM ≡ AOT ≡ C ≡ WASM.
#![allow(unused_imports, dead_code)]

mod common;
use common::assert_all_tiers;

/// Reciprocal 1/x. rust: f64.recip
#[test]
fn parity_math_recip() {
    assert_all_tiers(
        "## Main\nShow recip(4.0).\nShow recip(8.0).\nShow recip(2.0).\n",
        "0.25\n0.125\n0.5",
    );
}

/// Fractional part keeping sign, x - trunc(x). rust: f64.fract
#[test]
fn parity_math_fract() {
    assert_all_tiers(
        "## Main\nShow fract(3.75).\nShow fract(0.0 - 3.75).\nShow fract(5.0).\n",
        "0.75\n-0.75\n0",
    );
}

/// Logarithm to an arbitrary base, ln(x)/ln(base). rust: f64.log
/// floor at ×100 (mid-integer, boundary-robust): Python floor(log(10,2)*100)=332, floor(log(50,10)*100)=169.
#[test]
fn parity_math_log_base() {
    assert_all_tiers(
        "## Main\nShow floor(logBase(10.0, 2.0) * 100.0).\nShow floor(logBase(50.0, 10.0) * 100.0).\n",
        "332\n169",
    );
}

/// Integer power x^n (n may be negative). rust: f64.powi
#[test]
fn parity_math_powi() {
    assert_all_tiers(
        "## Main\nShow powi(2.0, 10).\nShow powi(2.0, 0 - 3).\nShow powi(3.0, 0).\nShow powi(5.0, 3).\n",
        "1024\n0.125\n1\n125",
    );
}

/// Euclidean division: q with a - q*b in [0, |b|). rust: f64.div_euclid
#[test]
fn parity_math_div_euclid() {
    assert_all_tiers(
        "## Main\nShow divEuclid(7.0, 3.0).\nShow divEuclid(0.0 - 7.0, 3.0).\nShow divEuclid(7.0, 0.0 - 3.0).\nShow divEuclid(0.0 - 7.0, 0.0 - 3.0).\n",
        "2\n-3\n-2\n3",
    );
}

/// Euclidean remainder, always in [0, |b|). rust: f64.rem_euclid
#[test]
fn parity_math_rem_euclid() {
    assert_all_tiers(
        "## Main\nShow remEuclid(7.0, 3.0).\nShow remEuclid(0.0 - 7.0, 3.0).\nShow remEuclid(7.0, 0.0 - 3.0).\nShow remEuclid(0.0 - 7.0, 0.0 - 3.0).\n",
        "1\n2\n1\n2",
    );
}

/// Round half to even (banker's rounding), Real result. c: math.h.rint / math.h.nearbyint
#[test]
fn parity_math_rint() {
    assert_all_tiers(
        "## Main\nShow rint(2.5).\nShow rint(3.5).\nShow rint(0.0 - 2.5).\nShow rint(2.4).\nShow rint(2.6).\n",
        "2\n4\n-2\n2\n3",
    );
}

/// Round half to even, Int result. c: math.h.lrint
#[test]
fn parity_math_lrint() {
    assert_all_tiers(
        "## Main\nShow lrint(2.5).\nShow lrint(3.5).\nShow lrint(0.0 - 2.5).\n",
        "2\n4\n-2",
    );
}

/// Gamma function (Lanczos g=7). c: math.h.tgamma · py: math.gamma
/// floor at ×1000 (Lanczos ≈1e-13, verified equal to Python's math.gamma at these floors).
#[test]
fn parity_math_gamma() {
    assert_all_tiers(
        "## Main\nShow floor(gamma(2.5) * 1000.0).\nShow floor(gamma(0.5) * 1000.0).\nShow floor(gamma(3.5) * 1000.0).\n",
        "1329\n1772\n3323",
    );
}

/// Log-gamma ln|Γ(x)| (Lanczos, direct so large args don't overflow). c: math.h.lgamma · py: math.lgamma
#[test]
fn parity_math_lgamma() {
    assert_all_tiers(
        "## Main\nShow floor(lgamma(5.0) * 1000.0).\nShow floor(lgamma(100.0) * 100.0).\nShow floor(lgamma(0.5) * 1000.0).\n",
        "3178\n35913\n572",
    );
}

/// True iff x is a normal float (finite, nonzero, not subnormal). c: math.h.isnormal
#[test]
fn parity_math_isnormal() {
    assert_all_tiers(
        "## Main\nShow isNormal(1.0).\nShow isNormal(0.0).\nShow isNormal(1.5).\n",
        "true\nfalse\ntrue",
    );
}

/// IEEE-754 classification as a label. c: math.h.fpclassify
#[test]
fn parity_math_fpclassify() {
    assert_all_tiers(
        "## Main\nShow fpclassify(1.0).\nShow fpclassify(0.0).\n",
        "normal\nzero",
    );
}
