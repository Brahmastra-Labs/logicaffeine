//! Loop-invariant libdivide (O9) — `x % n` / `x / n` by a runtime-but-loop-
//! invariant divisor `n` becomes a precomputed magic multiply.
//!
//! Both rustc and gcc leave a runtime-invariant divisor as a real hardware
//! `div`/`idiv` (~20–40 cycles); neither synthesizes the magic-multiply. So
//! hoisting `LogosDivU64::new(n)` into the loop's region and rewriting each
//! in-loop `% n` to `.rem(x)` is a strict win over the C baseline on
//! division-hot loops — graph_bfs's `% n` adjacency build runs `5n` of them.
//!
//! Soundness is gated three ways and verified by `assert_exact_output` + the
//! corpus differential gate: the divisor is an immutable scalar, every rewritten
//! `% n` sits inside a loop whose execution implies `n >= 1`, and the dividend
//! is proven `>= 0` so the `i64`→`u64` reinterpretation is value-preserving.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::assert_exact_output;
use logicaffeine_compile::compile::compile_to_rust;

/// A loop-invariant divisor `n` driving the loop bound, with a non-negative
/// dividend (`i * 7 + 3`, `i >= 0`). The modulo must lower to the precomputed
/// helper, not a hardware `%`.
const MOD_LOOP: &str = r#"## To native parseInt (s: Text) -> Int
## Main
Let n be parseInt("10").
Let mutable total be 0.
Let mutable i be 0.
While i is less than n:
    Let r be (i * 7 + 3) % n.
    Set total to total + r.
    Set i to i + 1.
Show total.
"#;

#[test]
fn loop_invariant_modulo_lowers_to_libdivide() {
    let rust = compile_to_rust(MOD_LOOP).unwrap();
    assert!(
        rust.contains("LogosDivU64::new("),
        "the loop-invariant divisor must precompute a magic multiply once \
         (`let __lcdiv_n = LogosDivU64::new((n) as u64);`). Got:\n{}",
        rust
    );
    assert!(
        rust.contains(".rem("),
        "the in-loop `% n` must lower to `__lcdiv_n.rem(..)`, not a hardware `%`. \
         Got:\n{}",
        rust
    );
    assert!(
        !rust.contains(") % n)") && !rust.contains(") % (n))"),
        "the raw hardware modulo by the loop-invariant `n` must be gone. Got:\n{}",
        rust
    );
}

/// The rewrite must be exactly value-preserving. For n = 10, r = (i*7+3) % 10
/// over i in 0..10 sums to 3+0+7+4+1+8+5+2+9+6 = 45.
#[test]
fn loop_invariant_modulo_is_value_preserving() {
    assert_exact_output(MOD_LOOP, "45");
}

/// A NON-invariant divisor (the divisor changes each iteration) must NOT be
/// rewritten — the magic can only be precomputed for a fixed divisor.
const VARYING_DIVISOR: &str = r#"## To native parseInt (s: Text) -> Int
## Main
Let n be parseInt("10").
Let mutable total be 0.
Let mutable i be 1.
While i is less than n:
    Let r be (n * 7 + 3) % i.
    Set total to total + r.
    Set i to i + 1.
Show total.
"#;

#[test]
fn varying_divisor_is_not_rewritten() {
    let rust = compile_to_rust(VARYING_DIVISOR).unwrap();
    assert!(
        !rust.contains("LogosDivU64"),
        "a divisor that changes each iteration must keep the hardware `%`. Got:\n{}",
        rust
    );
}

/// The dividend's proof needs its LOWER bound alone. `i - 1`, over a counter from 1 that the loop bounds only by a
/// runtime `n`, is `[0, +inf)` to the oracle: finite below, unbounded above, and `>= 0` is all the `i64`→`u64`
/// reinterpretation asks.
const SHIFTED_DIVIDEND: &str = r#"## To native parseInt (s: Text) -> Int
## Main
Let n be parseInt("10").
Let mutable total be 0.
Let mutable i be 1.
While i is less than n:
    Let r be (i - 1) % n.
    Set total to total + r.
    Set i to i + 1.
Show total.
"#;

#[test]
fn a_dividend_proven_nonnegative_by_its_lower_bound_alone_lowers_to_libdivide() {
    let rust = compile_to_rust(SHIFTED_DIVIDEND).unwrap();
    assert!(
        rust.contains("LogosDivU64::new(") && rust.contains(".rem("),
        "`i - 1` is `>= 0` with no upper bound, which is enough for the magic multiply. Got:\n{}",
        rust
    );
}

/// `(i - 1) % 10` over `i` in `1..10` is `0 + 1 + … + 8 = 36`.
#[test]
fn a_dividend_proven_nonnegative_by_its_lower_bound_alone_is_value_preserving() {
    assert_exact_output(SHIFTED_DIVIDEND, "36");
}

/// A dividend that can be negative keeps the hardware `%`: `i - 7` from `i = 1` is `-6`, which the unsigned
/// reinterpretation would read as `2^64 - 6`.
const SIGNED_DIVIDEND: &str = r#"## To native parseInt (s: Text) -> Int
## Main
Let n be parseInt("10").
Let mutable total be 0.
Let mutable i be 1.
While i is less than n:
    Let r be (i - 7) % n.
    Set total to total + r.
    Set i to i + 1.
Show total.
"#;

#[test]
fn a_dividend_that_can_be_negative_keeps_the_hardware_modulo() {
    let rust = compile_to_rust(SIGNED_DIVIDEND).unwrap();
    assert!(
        !rust.contains("LogosDivU64"),
        "`i - 7` is `-6` on the first pass, so the divisor keeps the hardware `%`. Got:\n{}",
        rust
    );
}

/// Truncated remainders: `(i - 7) % 10` over `i` in `1..10` is `-6 - 5 - 4 - 3 - 2 - 1 + 0 + 1 + 2 = -18`.
#[test]
fn a_dividend_that_can_be_negative_keeps_truncated_remainder_semantics() {
    assert_exact_output(SIGNED_DIVIDEND, "-18");
}

/// `while c < n` proves `n >= 1` only for a counter proven `>= 0`: from `c = -3`, `n` may be `-2` when the body runs.
/// The dividend `i * i` is never negative, but the divisor is not proven positive, so it keeps the hardware `%`.
const NEGATIVE_COUNTER: &str = r#"## To native parseInt (s: Text) -> Int
## Main
Let n be parseInt("10").
Let mutable total be 0.
Let mutable i be -3.
While i is less than n:
    Let r be (i * i) % n.
    Set total to total + r.
    Set i to i + 1.
Show total.
"#;

#[test]
fn a_counter_that_starts_negative_proves_no_divisor_positive() {
    let rust = compile_to_rust(NEGATIVE_COUNTER).unwrap();
    assert!(
        !rust.contains("LogosDivU64"),
        "a counter from -3 bounds `n` below by -2, not 1, so the divisor keeps the hardware `%`. Got:\n{}",
        rust
    );
}

/// `(i * i) % 10` over `i` in `-3..10` is `9 + 4 + 1 + 0 + 1 + 4 + 9 + 6 + 5 + 6 + 9 + 4 + 1 = 59`.
#[test]
fn a_counter_that_starts_negative_keeps_its_remainders() {
    assert_exact_output(NEGATIVE_COUNTER, "59");
}
