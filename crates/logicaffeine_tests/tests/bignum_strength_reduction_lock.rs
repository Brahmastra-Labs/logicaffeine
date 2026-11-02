//! SOUNDNESS LOCK — constant-divisor strength reductions must NOT corrupt a bignum.
//!
//! LOGOS `Int` is arbitrary-precision: it transparently promotes to `BigInt` on overflow.
//! The division/modulo strength reductions — `MagicDivU` (`x / c`, `x % c` via the u64 magic
//! reciprocal), `DivPow2` (`x / 2^k` → shift), and the mod-pow2 mask (`x % 2^k` → `x & (2^k-1)`)
//! — are only sound when the dividend FITS A MACHINE WORD. A dividend built by a widening loop
//! leaves the abstract interval unbounded above (`lo = 0, hi = +∞`); proving `lo >= 0` alone is
//! NOT enough, because the value can be a `BigInt` at runtime. The gate must require a PROVEN
//! FINITE range, so these shapes keep plain `Div`/`Mod`/`FloorDiv` and stay bignum-exact.
//!
//! Regression guard for the bug where `s / 7` (with `s` looped up to `10^20`) fired `MagicDivU`
//! and panicked ("MagicDivU on a non-Int operand" in the VM; an `expect_i64` overflow in AOT).
//! This is the load-bearing correctness floor under arbitrary-precision math (e.g. `pi(digits)`).

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::{assert_all_tiers, assert_c_output, assert_compiled_equals_interpreted};

/// Build a program that widens `s` to `10^n` via a multiply loop (so its interval is
/// unbounded-above), then evaluates `expr` over that bignum.
fn looped(n: u32, expr: &str) -> String {
    format!(
        "## Main\nLet mutable s be 1.\nLet mutable i be 1.\nWhile i is at most {n}:\n    Set s to s * 10.\n    Set i to i + 1.\nShow {expr}.\n"
    )
}

// 10^20 = 100000000000000000000.
// 10^20 / 7   = 14285714285714285714  (rem 2)
// 10^20 % 7   = 2
// 10^20 / 2   = 50000000000000000000
// 10^20 % 8   = 0  ⟹ (10^20 + 3) % 8 = 3
const DIV7: &str = "s / 7";
const DIV7_VAL: &str = "14285714285714285714";
const MOD7: &str = "s mod 7";
const MOD7_VAL: &str = "2";
const DIV2: &str = "s / 2";
const DIV2_VAL: &str = "50000000000000000000";
const MOD8: &str = "(s + 3) mod 8";
const MOD8_VAL: &str = "3";

// --- tree-walker ≡ VM (the always-on differential) -------------------------------------------

#[test]
fn bignum_loop_div_const_all_tiers() {
    assert_all_tiers(&looped(20, DIV7), DIV7_VAL);
}
#[test]
fn bignum_loop_mod_const_all_tiers() {
    assert_all_tiers(&looped(20, MOD7), MOD7_VAL);
}
#[test]
fn bignum_loop_div_pow2_all_tiers() {
    assert_all_tiers(&looped(20, DIV2), DIV2_VAL);
}
#[test]
fn bignum_loop_mod_pow2_all_tiers() {
    assert_all_tiers(&looped(20, MOD8), MOD8_VAL);
}

// --- AOT-to-Rust ≡ interpreter ---------------------------------------------------------------

#[test]
fn bignum_loop_div_const_compiled() {
    assert_compiled_equals_interpreted(&looped(20, DIV7));
}
#[test]
fn bignum_loop_mod_const_compiled() {
    assert_compiled_equals_interpreted(&looped(20, MOD7));
}
#[test]
fn bignum_loop_div_pow2_compiled() {
    assert_compiled_equals_interpreted(&looped(20, DIV2));
}
#[test]
fn bignum_loop_mod_pow2_compiled() {
    assert_compiled_equals_interpreted(&looped(20, MOD8));
}

// --- AOT-to-C ---------------------------------------------------------------------------------

#[test]
fn bignum_loop_div_const_c() {
    assert_c_output(&looped(20, DIV7), DIV7_VAL);
}
#[test]
fn bignum_loop_mod_const_c() {
    assert_c_output(&looped(20, MOD7), MOD7_VAL);
}
#[test]
fn bignum_loop_div_pow2_c() {
    assert_c_output(&looped(20, DIV2), DIV2_VAL);
}
#[test]
fn bignum_loop_mod_pow2_c() {
    assert_c_output(&looped(20, MOD8), MOD8_VAL);
}
