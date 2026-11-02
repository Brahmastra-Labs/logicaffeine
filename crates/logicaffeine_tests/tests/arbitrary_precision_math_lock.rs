//! LOCK — arbitrary-precision math constants are EXACT on every tier.
//!
//! `math.lg` computes `pi(digits)`, `eDigits`, `tauDigits`, and `sqrtDigits(n, digits)` to any
//! number of decimal places in pure-LOGOS *integer* arithmetic (LOGOS `Int` promotes to `BigInt`),
//! so precision is bounded only by `digits` — never by an f64 ULP. This proves the digit strings
//! are byte-identical to the mathematical constants AND identical across tw ≡ VM ≡ AOT-Rust ≡ C
//! (the value is a widening-loop bignum + big/big division + big→text, exercised on every tier).
//!
//! Reference digits (20 fractional places) are the well-known expansions of π, e, τ, and √2.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

const PI20: &str = "3.14159265358979323846";
const E20: &str = "2.71828182845904523536";
const TAU20: &str = "6.28318530717958647692";
const SQRT2_20: &str = "1.41421356237309504880";
const PHI20: &str = "1.61803398874989484820";
const LN2_20: &str = "0.69314718055994530941";
const LN10_20: &str = "2.30258509299404568401";

fn show(call: &str) -> String {
    format!("## Main\nShow {call}.\n")
}

// --- tree-walker ≡ VM -------------------------------------------------------------------------

#[test]
fn pi_20_all_tiers() {
    assert_all_tiers(&show("pi(20)"), PI20);
}
#[test]
fn e_20_all_tiers() {
    assert_all_tiers(&show("eDigits(20)"), E20);
}
#[test]
fn tau_20_all_tiers() {
    assert_all_tiers(&show("tauDigits(20)"), TAU20);
}
#[test]
fn sqrt2_20_all_tiers() {
    assert_all_tiers(&show("sqrtDigits(2, 20)"), SQRT2_20);
}

/// √ of a perfect square is exact and integral (the fraction is all zeros).
#[test]
fn sqrt_perfect_square_all_tiers() {
    assert_all_tiers(&show("sqrtDigits(144, 10)"), "12.0000000000");
}

/// A longer run stays exact — π to 50 places, the boundary where a float-based method would have
/// died 34 digits ago.
#[test]
fn pi_50_all_tiers() {
    assert_all_tiers(
        &show("pi(50)"),
        "3.14159265358979323846264338327950288419716939937510",
    );
}

/// The golden ratio and ln 2 to arbitrary precision.
#[test]
fn phi_20_all_tiers() {
    assert_all_tiers(&show("phiDigits(20)"), PHI20);
}
#[test]
fn ln2_20_all_tiers() {
    assert_all_tiers(&show("ln2Digits(20)"), LN2_20);
}
/// Arbitrary-precision natural log of a positive integer (`x = m·2^e` reduction + atanh series).
#[test]
fn ln10_20_all_tiers() {
    assert_all_tiers(&show("lnDigits(10, 20)"), LN10_20);
}
/// `lnDigits(2, ·)` must equal `ln2Digits(·)` — the general path and the specialized constant agree.
#[test]
fn ln_of_2_equals_ln2_all_tiers() {
    assert_all_tiers(&show("lnDigits(2, 20)"), LN2_20);
}
/// `ln 1 = 0` exactly (the reduction and series degenerate cleanly).
#[test]
fn ln_of_1_is_zero_all_tiers() {
    assert_all_tiers(&show("lnDigits(1, 10)"), "0.0000000000");
}

// --- AOT-to-Rust ≡ interpreter ---------------------------------------------------------------

#[test]
fn pi_20_compiled() {
    assert_compiled_equals_interpreted(&show("pi(20)"));
}
#[test]
fn phi_20_compiled() {
    assert_compiled_equals_interpreted(&show("phiDigits(20)"));
}
#[test]
fn ln2_20_compiled() {
    assert_compiled_equals_interpreted(&show("ln2Digits(20)"));
}
#[test]
fn ln10_20_compiled() {
    assert_compiled_equals_interpreted(&show("lnDigits(10, 20)"));
}
#[test]
fn e_20_compiled() {
    assert_compiled_equals_interpreted(&show("eDigits(20)"));
}
#[test]
fn sqrt2_20_compiled() {
    assert_compiled_equals_interpreted(&show("sqrtDigits(2, 20)"));
}

// --- AOT-to-C ---------------------------------------------------------------------------------

#[test]
fn pi_20_c() {
    assert_c_output(&show("pi(20)"), PI20);
}
#[test]
fn e_20_c() {
    assert_c_output(&show("eDigits(20)"), E20);
}
#[test]
fn sqrt2_20_c() {
    assert_c_output(&show("sqrtDigits(2, 20)"), SQRT2_20);
}
#[test]
fn phi_20_c() {
    assert_c_output(&show("phiDigits(20)"), PHI20);
}
#[test]
fn ln2_20_c() {
    assert_c_output(&show("ln2Digits(20)"), LN2_20);
}
#[test]
fn ln10_20_c() {
    assert_c_output(&show("lnDigits(10, 20)"), LN10_20);
}
