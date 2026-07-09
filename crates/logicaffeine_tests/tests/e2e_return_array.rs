//! E2E Codegen Tests: fixed-size return-buffer scalarization (step 3b).
//!
//! A function whose return value is a `Seq` built by a CONSTANT count of pushes,
//! read-only and non-escaping except through the return, changes its return type
//! to a stack array `[T; N]` returned by value (zero heap) instead of a heap
//! `LogosSeq`. This fires ATOMICALLY with typing every call-site result variable
//! as `[T; N]`: a directly-bound-and-indexed result (form i) becomes a stack
//! array, and a seeded accumulator reassigned each iteration and then iterated
//! (form ii) rides the array end to end — the MD5 `md5Compress` → `h` shape
//! (borrowed state in, fixed 4-word state out, reassigned per block, iterated).

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::assert_exact_output;
use common::compile_to_rust;

/// Form (i): `pair` returns a fixed-2 buffer, bound to `p` and read by index.
/// The whole chain rides `[i64; 2]` — no heap `LogosSeq` for the result.
#[test]
fn return_buffer_bound_and_indexed_scalarizes() {
    let code = r#"## To pair (a: Int) and (b: Int) -> Seq of Int:
    Let mutable out be a new Seq of Int.
    Push a * 2 to out.
    Push b * 3 to out.
    Return out.

## Main
Let p be pair(5, 7).
Show (item 1 of p) + (item 2 of p).
"#;
    // 5*2 + 7*3 = 31.
    assert_exact_output(code, "31");
    let rust = compile_to_rust(code).unwrap();
    assert!(
        rust.contains("-> [i64; 2]"),
        "fixed-count return buffer should make the return type `[i64; 2]`, got:\n{}",
        rust
    );
}

/// Form (ii): `step` returns a fixed-2 buffer from a borrowed one; `run` seeds
/// `h`, reassigns it each iteration (`Set h to step(h)`), then sums it. The whole
/// chain must ride `[i64; 2]` — no per-step heap alloc — and still compute 36
/// (h=[10,20]→[11,22]→[12,24], sum 36).
#[test]
fn return_buffer_reassigned_and_iterated_scalarizes() {
    let code = r#"## To step (s: Seq of Int) -> Seq of Int:
    Let mutable out be a new Seq of Int.
    Push (item 1 of s) + 1 to out.
    Push (item 2 of s) + 2 to out.
    Return out.

## To run (n: Int) -> Int:
    Let mutable h be a new Seq of Int.
    Push 10 to h.
    Push 20 to h.
    Repeat for i from 1 to n:
        Set h to step(h).
    Let mutable total be 0.
    Repeat for x in h:
        Set total to total + x.
    Return total.

## Main
Show run(2).
"#;
    assert_exact_output(code, "36");
    let rust = compile_to_rust(code).unwrap();
    assert!(
        rust.contains("-> [i64; 2]"),
        "fixed-count return buffer should make the return type `[i64; 2]`, got:\n{}",
        rust
    );
    assert!(
        rust.contains("let mut h: [i64; 2]"),
        "the reassigned accumulator should become a stack array, got:\n{}",
        rust
    );
}

/// A VARIABLE-count return buffer can't become a fixed array — it stays a heap
/// `Seq`; output stays correct.
#[test]
fn variable_count_return_buffer_does_not_scalarize() {
    let code = r#"## To upto (n: Int) -> Seq of Int:
    Let mutable out be a new Seq of Int.
    Repeat for i from 1 to n:
        Push i * i to out.
    Return out.

## Main
Let s be upto(4).
Let mutable total be 0.
Repeat for x in s:
    Set total to total + x.
Show total.
"#;
    // 1+4+9+16 = 30.
    assert_exact_output(code, "30");
    let rust = compile_to_rust(code).unwrap();
    assert!(
        !rust.contains("-> [i64;"),
        "a variable-count return buffer must not become a fixed array, got:\n{}",
        rust
    );
}

/// A returned buffer that is PUSHED to by its caller is not a fixed array — it
/// stays a heap `Seq`; output stays correct.
#[test]
fn caller_mutated_result_does_not_scalarize() {
    let code = r#"## To seedPair (a: Int) -> Seq of Int:
    Let mutable out be a new Seq of Int.
    Push a to out.
    Push a + 1 to out.
    Return out.

## Main
Let mutable p be seedPair(10).
Push 99 to p.
Let mutable total be 0.
Repeat for x in p:
    Set total to total + x.
Show total.
"#;
    // 10 + 11 + 99 = 120.
    assert_exact_output(code, "120");
    let rust = compile_to_rust(code).unwrap();
    assert!(
        !rust.contains("-> [i64;"),
        "a caller-mutated result must not scalarize, got:\n{}",
        rust
    );
}
