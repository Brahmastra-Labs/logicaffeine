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
