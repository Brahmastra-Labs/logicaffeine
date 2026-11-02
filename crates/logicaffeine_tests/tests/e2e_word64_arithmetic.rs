//! WS2a — Word64 (unsigned, wrapping ring) scalar arithmetic in the hot loop.
//!
//! LOGOS has first-class unsigned machine words (`Word8/16/32/64`), refuting the
//! critique that "LOGOS actively removes Rust features (unsigned/sized types)
//! that support performance." A `Word64` iterative fib lowers to RAW wrapping u64
//! (`+` → `wrapping_add`, `%` → u64 `%`) with NO overflow guard — native C/Zig
//! speed, and a direct demonstration of the exact feature the critique says we
//! lack. These tests lock both the correctness and the guard-free shape.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(e2e_word64_arithmetic)'`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::assert_exact_output;
use common::compile_to_rust;

/// A fixed-trip `Word64` iterative fib. After 10 iterations `a = fib(10) = 55`
/// (the `% 1000000007` never fires at this size, so the answer is exact). Proves
/// the Word64 path compiles to valid Rust, runs, and computes the same value the
/// signed-`Int` version does — i.e. unsigned is a correct, drop-in alternative.
const WORD64_FIB_10: &str = r#"## Main
Let mutable a be word64(0).
Let mutable b be word64(1).
Let mutable i be word64(0).
While i is less than word64(10):
    Let temp be b.
    Set b to (a + b) % word64(1000000007).
    Set a to temp.
    Set i to i + word64(1).
Show a.
"#;

/// The signed-`Int` version of the same loop — the head-to-head oracle.
const INT_FIB_10: &str = r#"## Main
Let mutable a be 0.
Let mutable b be 1.
Let mutable i be 0.
While i is less than 10:
    Let temp be b.
    Set b to (a + b) % 1000000007.
    Set a to temp.
    Set i to i + 1.
Show a.
"#;

#[test]
fn word64_fib_matches_int_fib() {
    // Both the unsigned Word64 loop and the signed Int loop compute fib(10) = 55.
    assert_exact_output(INT_FIB_10, "55");
    assert_exact_output(WORD64_FIB_10, "55");
}

/// A larger trip count so the `% 1000000007` actually fires: fib(45) = 1134903170,
/// and 1134903170 mod (10^9+7) = 134903163. Confirms Word64 `%` computes the
/// truncated remainder correctly (identical to the Int version at this size).
#[test]
fn word64_fib_modulo_fires_correctly() {
    let word64_fib_45 = WORD64_FIB_10.replace("word64(10)", "word64(45)");
    let int_fib_45 = INT_FIB_10.replace("less than 10", "less than 45");
    assert_exact_output(&int_fib_45, "134903163");
    assert_exact_output(&word64_fib_45, "134903163");
}

/// The point of WS2a: Word64 arithmetic must lower to RAW wrapping/native ops —
/// no `logos_add_i64` (checked add → BigInt), no `logos_add_exact`, no
/// `checked_add`. The Word64 newtype's `Add`/`Rem` impls are `wrapping_add` / raw
/// `%`, so the bare `(a + b) % ...` we emit IS the native-speed form.
#[test]
fn word64_arithmetic_lowers_to_raw_wrapping() {
    let rust = compile_to_rust(WORD64_FIB_10).expect("Word64 fib should compile to Rust");
    assert!(
        !rust.contains("logos_add_i64")
            && !rust.contains("logos_add_exact")
            && !rust.contains("checked_add"),
        "Word64 arithmetic must be guard-free (raw wrapping), but a checked/exact \
         Int helper leaked in:\n{rust}"
    );
    // And it must actually be the raw binary op on the Word64 values.
    assert!(
        rust.contains("(a + b)"),
        "expected a raw `(a + b)` Word64 add in the hot loop:\n{rust}"
    );
}
