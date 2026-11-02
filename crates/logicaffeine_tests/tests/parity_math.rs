//! Stdlib-parity north-star corpus — domain: math
//!
//! Auto-scaffolded from benchmarks/parity/catalog/superset.json by
//! scripts/bench/gen-parity-corpus.py. Each test mirrors one deduplicated capability of the
//! C/Python/Rust/Go (+Java) superset. `#[ignore]` = parity-pending (not yet implemented);
//! the tree stays green. To LAND a capability: implement it, replace the body with a real
//! `assert_all_tiers` behavioral spec, DELETE the `#[ignore]` line, flip the catalog entry
//! to `status: green`, and bump GREEN_FLOOR in parity_catalog_lock.rs.
#![allow(unused_imports, dead_code)]

mod common;
use common::assert_all_tiers;

/// Exact integer exponentiation via square-and-multiply (not the f64 `pow`).
/// provenance — python: builtins.pow · go: math/big.Exp
#[test]
fn parity_math_pow_exact() {
    assert_all_tiers(
        "## Main\nShow powExact(2, 10).\nShow powExact(3, 5).\nShow powExact(7, 0).\n",
        "1024\n243\n1",
    );
}

/// Modular exponentiation base^exp mod m (3-argument `pow`).
/// provenance — python: builtins.pow · go: math/big.Exp
#[test]
fn parity_math_pow_mod() {
    assert_all_tiers(
        "## Main\nShow powMod(2, 10, 1000).\nShow powMod(7, 256, 13).\n",
        "24\n9",
    );
}

/// Modular multiplicative inverse via the extended Euclidean algorithm.
/// provenance — go: math/big.ModInverse
#[test]
fn parity_math_mod_inverse() {
    assert_all_tiers(
        "## Main\nShow modInverse(3, 11).\nShow modInverse(10, 17).\n",
        "4\n12",
    );
}

/// Binomial coefficient n-choose-k (exact).
/// provenance — python: math.comb · go: math/big.Binomial
#[test]
fn parity_math_binomial() {
    assert_all_tiers(
        "## Main\nShow binomial(10, 3).\nShow binomial(5, 0).\nShow binomial(52, 5).\n",
        "120\n1\n2598960",
    );
}

/// Absolute value. Built-in call surface `abs(x)`.
/// provenance — c: stdlib.h.abs, math.h.fabs · python: builtins.abs · rust: i64.abs · go: math.Abs · java: java.lang.Math.abs
/// LOGOS surface: `abs(x)`
#[test]
fn parity_math_abs() {
    assert_all_tiers(
        "## Main\nShow abs(0 - 5).\nShow abs(5).\nShow abs(0).\n",
        "5\n5\n0",
    );
}

/// Constrain a value to an inclusive range. Auto-imported from math.lg.
/// provenance — rust: i64.clamp · java: java.lang.Math.clamp
/// LOGOS surface: `clamp(x, lo, hi)`
#[test]
fn parity_math_clamp() {
    assert_all_tiers(
        "## Main\nShow clamp(5, 1, 10).\nShow clamp(0, 1, 10).\nShow clamp(15, 1, 10).\n",
        "5\n1\n10",
    );
}

/// n! for a non-negative integer. Auto-imported from math.lg.
/// provenance — python: math.factorial
/// LOGOS surface: `factorial(n)`
#[test]
fn parity_math_factorial() {
    assert_all_tiers(
        "## Main\nShow factorial(5).\nShow factorial(0).\nShow factorial(1).\n",
        "120\n1\n1",
    );
}

/// Round down to the nearest integer. Built-in call surface `floor(x)`.
/// provenance — c: math.h.floor · python: math.floor · rust: f64.floor · go: math.Floor · java: java.lang.Math.floor
/// LOGOS surface: `floor(x)`
#[test]
fn parity_math_floor() {
    assert_all_tiers("## Main\nShow floor(3.7).\n", "3");
}

/// Greatest common divisor of two integers.
/// provenance — python: math.gcd · java: java.lang.Math.gcd
/// LOGOS surface: `gcd(a, b)` — auto-imported from the pure-LOGOS `math.lg` stdlib module.
/// LANDED: `gcd` is demand-loaded (no user definition) and agrees across every tier,
/// including Euclid over non-negative inputs and the abs-normalized negative case.
#[test]
fn parity_math_gcd() {
    assert_all_tiers(
        "## Main\nShow gcd(48, 18).\nShow gcd(18, 48).\nLet neg be 0 - 48.\nShow gcd(neg, 18).\n",
        "6\n6\n6",
    );
}

/// Larger of two values. Built-in call surface `max(a, b)`.
/// provenance — rust: i64.max · go: math.Max · java: java.lang.Math.max
/// LOGOS surface: `max(a, b)`
#[test]
fn parity_math_max() {
    assert_all_tiers(
        "## Main\nShow max(3, 7).\nShow max(7, 3).\nShow max(0 - 2, 0 - 9).\n",
        "7\n7\n-2",
    );
}

/// Smaller of two values. Built-in call surface `min(a, b)`.
/// provenance — rust: i64.min · go: math.Min · java: java.lang.Math.min
/// LOGOS surface: `min(a, b)`
#[test]
fn parity_math_min() {
    assert_all_tiers(
        "## Main\nShow min(3, 7).\nShow min(7, 3).\nShow min(0 - 2, 0 - 9).\n",
        "3\n3\n-9",
    );
}

/// x raised to the power y. Built-in call surface `pow(x, y)`.
/// provenance — c: math.h.pow · python: builtins.pow, math.pow · rust: f64.powf · go: math.Pow · java: java.lang.Math.pow
/// LOGOS surface: `pow(x, y)`
#[test]
fn parity_math_pow() {
    assert_all_tiers("## Main\nShow pow(2.0, 10.0).\n", "1024");
}

/// Sine (radians). Determinism-critical transcendental.
/// provenance — c: math.h.sin · python: math.sin · rust: f64.sin · go: math.Sin · java: java.lang.Math.sin
/// LOGOS surface: `the sine of x`
#[test]
fn parity_math_sin() {
    assert_all_tiers(
        "## Main
Show sin(0.0).
Show floor(sin(1.0) * 100.0).
",
        "0
84",
    );
}

/// Square root. Built-in call surface `sqrt(x)`.
/// provenance — c: math.h.sqrt · python: math.sqrt · rust: f64.sqrt · go: math.Sqrt · java: java.lang.Math.sqrt
/// LOGOS surface: `sqrt(x)`
#[test]
fn parity_math_sqrt() {
    assert_all_tiers("## Main\nShow sqrt(16.0).\n", "4");
}

/// Least common multiple. py: math.lcm
#[test]
fn parity_math_lcm() {
    assert_all_tiers("## Main\nShow lcm(4, 6).\nShow lcm(0, 5).\n", "12\n0");
}
/// Integer square root (floor). py: math.isqrt
#[test]
fn parity_math_isqrt() {
    assert_all_tiers(
        "## Main\nShow isqrt(16).\nShow isqrt(17).\nShow isqrt(1).\nShow isqrt(0).\n",
        "4\n4\n1\n0",
    );
}
/// Sign of an integer (-1/0/1). rust: i64.signum
#[test]
fn parity_math_sign() {
    assert_all_tiers(
        "## Main\nShow sign(0 - 5).\nShow sign(5).\nShow sign(0).\n",
        "-1\n1\n0",
    );
}
/// Cosine (radians, deterministic Taylor). c: math.h.cos · py: math.cos · rust: f64.cos
#[test]
fn parity_math_cos() {
    assert_all_tiers(
        "## Main\nShow cos(0.0).\nShow floor(cos(1.0) * 100.0).\n",
        "1\n54",
    );
}
/// Tangent (radians). c: math.h.tan · py: math.tan · rust: f64.tan
#[test]
fn parity_math_tan() {
    assert_all_tiers(
        "## Main\nShow tan(0.0).\nShow floor(tan(1.0) * 100.0).\n",
        "0\n155",
    );
}

/// e^x (deterministic Taylor). c: math.h.exp · py: math.exp · rust: f64.exp
#[test]
fn parity_math_exp() {
    assert_all_tiers(
        "## Main\nShow exp(0.0).\nShow floor(exp(1.0) * 1000.0).\n",
        "1\n2718",
    );
}
/// Euclidean hypotenuse. c: math.h.hypot · py: math.hypot · rust: f64.hypot
#[test]
fn parity_math_hypot() {
    assert_all_tiers("## Main\nShow hypot(3.0, 4.0).\n", "5");
}
/// Cube root (Newton). c: math.h.cbrt · py: math.cbrt · rust: f64.cbrt
#[test]
fn parity_math_cbrt() {
    assert_all_tiers("## Main\nShow cbrt(27.0).\nShow cbrt(0.0).\n", "3\n0");
}
/// Hyperbolic sine. c: math.h.sinh · py: math.sinh · rust: f64.sinh
#[test]
fn parity_math_sinh() {
    assert_all_tiers("## Main\nShow sinh(0.0).\n", "0");
}
/// Hyperbolic cosine. c: math.h.cosh · py: math.cosh · rust: f64.cosh
#[test]
fn parity_math_cosh() {
    assert_all_tiers("## Main\nShow cosh(0.0).\n", "1");
}
/// Hyperbolic tangent. c: math.h.tanh · py: math.tanh · rust: f64.tanh
#[test]
fn parity_math_tanh() {
    assert_all_tiers("## Main\nShow tanh(0.0).\n", "0");
}

/// Natural log (atanh series). c: math.h.log · py: math.log · rust: f64.ln
#[test]
fn parity_math_ln() {
    assert_all_tiers(
        "## Main\nShow ln(1.0).\nShow floor(ln(2.0) * 1000.0).\n",
        "0\n693",
    );
}
/// Base-2 log. c: math.h.log2 · py: math.log2 · rust: f64.log2
#[test]
fn parity_math_log2() {
    assert_all_tiers("## Main\nShow floor(log2(4.0) * 100.0).\n", "200");
}
/// Base-10 log. c: math.h.log10 · py: math.log10 · rust: f64.log10
#[test]
fn parity_math_log10() {
    assert_all_tiers("## Main\nShow floor(log10(2.0) * 1000.0).\n", "301");
}
/// Truncate toward zero. c: math.h.trunc · py: math.trunc · rust: f64.trunc
#[test]
fn parity_math_trunc() {
    assert_all_tiers(
        "## Main\nShow trunc(3.7).\nShow trunc(0.0 - 3.7).\n",
        "3\n-3",
    );
}
/// Radians to degrees. py: math.degrees · rust: f64.to_degrees
#[test]
fn parity_math_degrees() {
    assert_all_tiers("## Main\nShow floor(degrees(1.0)).\n", "57");
}
/// Degrees to radians. py: math.radians · rust: f64.to_radians
#[test]
fn parity_math_radians() {
    assert_all_tiers("## Main\nShow floor(radians(180.0) * 1000.0).\n", "3141");
}

/// Magnitude of x with sign of y. c: math.h.copysign · py: math.copysign · rust: f64.copysign
#[test]
fn parity_math_copysign() {
    assert_all_tiers(
        "## Main\nShow copysign(3.0, 0.0 - 1.0).\nShow copysign(0.0 - 3.0, 1.0).\n",
        "-3\n3",
    );
}
/// Smaller of two reals. c: math.h.fmin · go: math.Min
#[test]
fn parity_math_fmin() {
    assert_all_tiers("## Main\nShow fmin(3.0, 5.0).\n", "3");
}
/// Larger of two reals. c: math.h.fmax · go: math.Max
#[test]
fn parity_math_fmax() {
    assert_all_tiers("## Main\nShow fmax(3.0, 5.0).\n", "5");
}
/// e^x - 1. c: math.h.expm1 · py: math.expm1 · rust: f64.exp_m1
#[test]
fn parity_math_expm1() {
    assert_all_tiers("## Main\nShow expm1(0.0).\n", "0");
}
/// ln(1+x). c: math.h.log1p · py: math.log1p · rust: f64.ln_1p
#[test]
fn parity_math_log1p() {
    assert_all_tiers("## Main\nShow log1p(0.0).\n", "0");
}

/// Round up to the nearest integer (built-in `ceil`).
/// provenance — c: math.h.ceil · go: math.Ceil · python: math.ceil · java: java.lang.Math.ceil
#[test]
fn parity_math_ceil() {
    assert_all_tiers("## Main\nShow ceil(3.2).\nShow ceil(3.0).\n", "4\n3");
}

/// Round to the nearest integer (built-in `round`).
/// provenance — c: math.h.round · go: math.Round · python: builtins.round · java: java.lang.Math.round
#[test]
fn parity_math_round() {
    assert_all_tiers("## Main\nShow round(3.5).\nShow round(3.4).\n", "4\n3");
}

/// Arctangent (argument-reduction + Taylor). c: math.h.atan · go: math.Atan · py: math.atan
#[test]
fn parity_math_atan() {
    assert_all_tiers(
        "## Main\nShow floor(atan(1.0) * 1000.0).\nShow floor(atan(0.5) * 1000.0).\n",
        "785\n463",
    );
}

/// Arcsine (via atan). c: math.h.asin · go: math.Asin · py: math.asin
#[test]
fn parity_math_asin() {
    assert_all_tiers("## Main\nShow floor(asin(0.5) * 1000.0).\n", "523");
}

/// Arccosine (pi/2 - asin). c: math.h.acos · go: math.Acos · py: math.acos
#[test]
fn parity_math_acos() {
    assert_all_tiers("## Main\nShow floor(acos(0.5) * 1000.0).\n", "1047");
}

/// Two-argument arctangent with quadrant. c: math.h.atan2 · go: math.Atan2 · py: math.atan2
#[test]
fn parity_math_atan2() {
    assert_all_tiers(
        "## Main\nShow floor(atan2(1.0, 1.0) * 1000.0).\nShow floor(atan2(1.0, 0.0) * 1000.0).\n",
        "785\n1570",
    );
}

/// 2 raised to x. c: math.h.exp2 · go: math.Exp2 · py: math.exp2
#[test]
fn parity_math_exp2() {
    assert_all_tiers(
        "## Main\nShow floor(exp2(10.0) + 0.5).\nShow floor(exp2(0.0) + 0.5).\n",
        "1024\n1",
    );
}

/// x times 2^n. c: math.h.ldexp · go: math.Ldexp · py: math.ldexp
#[test]
fn parity_math_ldexp() {
    assert_all_tiers("## Main\nShow floor(ldexp(3.0, 4.0) + 0.5).\n", "48");
}

/// Inverse hyperbolic sine. c: math.h.asinh · go: math.Asinh · py: math.asinh
#[test]
fn parity_math_asinh() {
    assert_all_tiers("## Main\nShow floor(asinh(1.0) * 1000.0).\n", "881");
}

/// Inverse hyperbolic cosine. c: math.h.acosh · go: math.Acosh · py: math.acosh
#[test]
fn parity_math_acosh() {
    assert_all_tiers("## Main\nShow floor(acosh(2.0) * 1000.0).\n", "1316");
}

/// Inverse hyperbolic tangent. c: math.h.atanh · go: math.Atanh · py: math.atanh
#[test]
fn parity_math_atanh() {
    assert_all_tiers("## Main\nShow floor(atanh(0.5) * 1000.0).\n", "549");
}

/// IEEE float classification predicates (pure-LOGOS via `x != x` / `x - x`).
/// isnan: py math.isnan · c math.h.isnan · rust f64.is_nan · go math.IsNaN
#[test]
fn parity_math_isnan() {
    assert_all_tiers(
        "## Main\nLet big be 1.0e308.\nLet inf be big * big.\nLet nan be inf - inf.\nShow isNaN(nan).\nShow isNaN(1.5).\nShow isNaN(inf).\n",
        "true\nfalse\nfalse",
    );
}
/// isinf: py math.isinf · c math.h.isinf · rust f64.is_infinite · go math.IsInf
#[test]
fn parity_math_isinf() {
    assert_all_tiers(
        "## Main\nLet big be 1.0e308.\nLet inf be big * big.\nLet nan be inf - inf.\nShow isInf(inf).\nShow isInf(2.0).\nShow isInf(nan).\n",
        "true\nfalse\nfalse",
    );
}
/// isfinite: py math.isfinite · c math.h.isfinite · rust f64.is_finite
#[test]
fn parity_math_isfinite() {
    assert_all_tiers(
        "## Main\nLet big be 1.0e308.\nLet inf be big * big.\nLet nan be inf - inf.\nShow isFinite(3.5).\nShow isFinite(inf).\nShow isFinite(nan).\n",
        "true\nfalse\nfalse",
    );
}
