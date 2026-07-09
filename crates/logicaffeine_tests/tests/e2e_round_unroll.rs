//! E2E Codegen Tests: crypto round-loop unrolling.
//!
//! A small, constant-trip loop whose induction variable feeds a rotate
//! shift-amount (`rotl(x, item ((i % 4) + 1) of r)`) is fully unrolled — even at
//! the TOP LEVEL of a function, where the ordinary unroller defers to LLVM. The
//! payoff is the reason hand-written MD5/SHA fully unroll: with `i` a literal the
//! shift amount folds to a constant, so each data-dependent rotate becomes a
//! single `rol` instruction instead of a runtime table load + variable-count
//! rotate. Unrolling is pure sequencing, so output is byte-identical.
//!
//! Both programs feed the kernel a RUNTIME-varying seed (the outer loop counter)
//! so the whole-program const-folder can't collapse the call — the round loop
//! survives to codegen, where we inspect its shape.
//!
//! The negative test locks that the gate is the ROTATE, not mere array indexing:
//! a top-level loop that indexes a fixed array but never rotates by `i` stays
//! rolled (LLVM's job), so we never over-fire and bloat ordinary code.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::assert_compiled_equals_interpreted;
use common::compile_to_rust;

/// A top-level 4-trip loop that rotates by an `i`-indexed table entry unrolls
/// into four straight-line `rotl` calls with no surviving `for i` loop.
#[test]
fn top_level_crypto_round_loop_unrolls() {
    let code = r#"## To mix (r: Seq of Int) and (seed: Word32) -> Word32:
    Let mutable acc be seed.
    Repeat for i from 0 to 3:
        Set acc to acc + rotl(acc, item ((i % 4) + 1) of r).
    Return acc.

## Main
Let mutable r be a new Seq of Int.
Push 7 to r.
Push 12 to r.
Push 17 to r.
Push 22 to r.
Let mutable acc be 0.
Repeat for j from 1 to 50:
    Set acc to (acc + intOfWord32(mix(r, word32(j)))) % 1000000007.
Show acc.
"#;
    assert_compiled_equals_interpreted(code);
    let rust = compile_to_rust(code).unwrap();
    let n = rust.matches("rotl(").count();
    assert!(
        n >= 4,
        "the 4-trip round loop should unroll into >=4 straight-line rotl calls, got {}:\n{}",
        n,
        rust
    );
    assert!(
        !rust.contains("for i in"),
        "the unrolled round loop should leave no `for i` loop, got:\n{}",
        rust
    );
}

/// A top-level loop that indexes a fixed array but does NOT rotate by `i` stays
/// rolled — the crypto gate is the rotate, not array indexing, so ordinary loops
/// are left for LLVM and never bloated.
#[test]
fn top_level_non_rotating_loop_stays_rolled() {
    let code = r#"## To pick (r: Seq of Int) and (seed: Int) -> Int:
    Let mutable s be seed.
    Repeat for i from 0 to 3:
        Set s to s + item (i + 1) of r.
    Return s.

## Main
Let mutable r be a new Seq of Int.
Push 1 to r.
Push 2 to r.
Push 3 to r.
Push 4 to r.
Let mutable acc be 0.
Repeat for j from 1 to 50:
    Set acc to (acc + pick(r, j)) % 1000000007.
Show acc.
"#;
    assert_compiled_equals_interpreted(code);
    let rust = compile_to_rust(code).unwrap();
    assert!(
        rust.contains("for i in"),
        "a top-level non-rotating loop should stay rolled (LLVM's job), got:\n{}",
        rust
    );
}
