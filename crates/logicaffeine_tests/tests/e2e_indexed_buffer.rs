//! E2E Codegen Tests: indexed-write fixed-size buffer scalarization.
//!
//! A local `Seq` zero-initialized to a CONSTANT size by a counted loop, then
//! mutated ONLY by indexed writes (`Set item i of buf to …`) — never pushed
//! again, never escaping — is a fixed-size mutable buffer. The AOT lowers it to
//! a stack array `[T; N]` (`[0; N]`, indexed stores) instead of a heap `Vec`.
//! This is the memcpy-style buffer pattern (copy some, set the rest by index) —
//! the shape a streaming hash's 64-byte block wants, and the last fixed-buffer
//! shape the scalarizer didn't cover (scratch is push-built + read-only; this is
//! zero-init + indexed-written).

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::assert_exact_output;
use common::compile_to_rust;

/// A zero-init 8-slot buffer, the first 4 slots set by index from a borrowed
/// input, then summed. It must lower to `[i64; 8]` (no heap `Vec`) and compute
/// `(10+20+30+40)*2 = 200` (the other 4 slots stay 0).
#[test]
fn zero_init_then_indexed_write_scalarizes() {
    let code = r#"## To fill (xs: Seq of Int) -> Int:
    Let mutable buf be a new Seq of Int.
    Repeat for j from 1 to 8:
        Push 0 to buf.
    Repeat for j from 1 to 4:
        Set item j of buf to (item j of xs) * 2.
    Let mutable total be 0.
    Repeat for k from 1 to 8:
        Set total to total + item k of buf.
    Return total.

## Main
Let mutable xs be a new Seq of Int.
Push 10 to xs.
Push 20 to xs.
Push 30 to xs.
Push 40 to xs.
Show fill(xs).
"#;
    assert_exact_output(code, "200");
    let rust = compile_to_rust(code).unwrap();
    assert!(
        rust.contains("[i64; 8]"),
        "zero-init + indexed-write buffer should lower to `[i64; 8]`, got:\n{}",
        rust
    );
    assert!(
        !rust.contains("let mut buf : Vec") && !rust.contains("let mut buf: Vec"),
        "the buffer must not be a heap Vec, got:\n{}",
        rust
    );
}

/// A buffer PUSHED to after its zero-init fill is growable — not a fixed array.
/// It stays a heap `Seq`; output stays correct.
#[test]
fn repushed_buffer_does_not_scalarize() {
    let code = r#"## To grow (n: Int) -> Int:
    Let mutable buf be a new Seq of Int.
    Repeat for j from 1 to 4:
        Push 0 to buf.
    Set item 1 of buf to 5.
    Push 9 to buf.
    Let mutable total be 0.
    Repeat for k from 1 to 5:
        Set total to total + item k of buf.
    Return total.

## Main
Show grow(0).
"#;
    // slot1=5, slots2..4=0, pushed 9 → 5+0+0+0+9 = 14.
    assert_exact_output(code, "14");
    let rust = compile_to_rust(code).unwrap();
    assert!(
        !rust.contains("[i64; 4]"),
        "a re-pushed buffer must not become a fixed array, got:\n{}",
        rust
    );
}

/// A VARIABLE-count init can't size a stack array — stays a heap `Seq`.
#[test]
fn variable_init_buffer_does_not_scalarize() {
    let code = r#"## To dyn (n: Int) -> Int:
    Let mutable buf be a new Seq of Int.
    Repeat for j from 1 to n:
        Push 0 to buf.
    Set item 1 of buf to 7.
    Let mutable total be 0.
    Repeat for k from 1 to n:
        Set total to total + item k of buf.
    Return total.

## Main
Show dyn(3).
"#;
    // slot1=7, rest 0 → 7.
    assert_exact_output(code, "7");
    let rust = compile_to_rust(code).unwrap();
    assert!(
        !rust.contains("[i64;"),
        "a variable-count buffer must not become a fixed array, got:\n{}",
        rust
    );
}
