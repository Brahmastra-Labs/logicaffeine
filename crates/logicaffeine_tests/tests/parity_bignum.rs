//! Stdlib-parity north-star corpus - domain: bignum
//!
//! LOGOS `Int` is arbitrary-precision (transparently promotes to BigInt on overflow, on every
//! tier), so the whole big-integer surface — Go's math/big, Python's unbounded int, Java's
//! BigInteger — is a NATIVE primitive: ordinary `+ - * / mod` on `Int` ARE big.Add/Sub/Mul/…, no
//! library needed. Values here are COMPUTED (the lexer caps literals at u64), which is how you make
//! a bignum anyway.
#![allow(unused_imports, dead_code)]

mod common;
use common::assert_all_tiers;

/// Exact product past i64 — the `*` operator is big.Mul. go: math/big.Mul
#[test]
fn parity_bignum_mul() {
    assert_all_tiers(
        "## Main\nShow 10000000000 * 10000000000.\n",
        "100000000000000000000",
    );
}
/// Addition on a big value. go: math/big.Add
#[test]
fn parity_bignum_add() {
    assert_all_tiers(
        "## Main\nLet a be 10000000000 * 10000000000.\nShow a + 123.\n",
        "100000000000000000123",
    );
}
/// Subtraction on a big value. go: math/big.Sub
#[test]
fn parity_bignum_sub() {
    assert_all_tiers(
        "## Main\nLet a be 10000000000 * 10000000000.\nShow a - 1.\n",
        "99999999999999999999",
    );
}
/// Exact division of a big value. go: math/big.Quo / Div
#[test]
fn parity_bignum_div() {
    assert_all_tiers(
        "## Main\nLet a be 10000000000 * 10000000000.\nShow a / 100000000.\n",
        "1000000000000",
    );
}
/// Modulo of a big value. go: math/big.Mod / Rem
#[test]
fn parity_bignum_mod() {
    assert_all_tiers(
        "## Main\nLet a be 10000000000 * 10000000000.\nShow (a + 7) mod 10.\n",
        "7",
    );
}
/// Negation of a big value. go: math/big.Neg
#[test]
fn parity_bignum_neg() {
    assert_all_tiers(
        "## Main\nLet a be 10000000000 * 10000000000.\nShow 0 - a.\n",
        "-100000000000000000000",
    );
}
