//! E2E Codegen Tests: scratch-buffer scalarization.
//!
//! A local `Seq` built by a CONSTANT-count push loop, thereafter only read
//! (`item _ of buf` / `length of buf` / a `&[T]` borrow-arg) and never escaping
//! (not returned, not stored in a field, not aliased), is a fixed-size scratch
//! buffer. The AOT lowers it to a stack array `[T; N]` built by
//! `::std::array::from_fn` — zero heap, direct indexing — instead of a `Vec`
//! rebuilt (and reallocated) on every entry. This is the escape-analysis
//! sibling of the constant-table pass (which handles constant VALUES; this
//! handles constant COUNT with per-iteration values).
//!
//! The tests pin BOTH correctness (byte-exact output, opt on vs off) AND that
//! the optimization actually fires (generated-code shape) AND that it declines
//! on every escape/variable-size shape (the buffer stays a heap `Seq`, still
//! correct).

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::assert_exact_output;
use common::compile_to_rust;

/// A fixed-count buffer filled from a borrowed slice, then summed by index, is a
/// scratch buffer: it never leaves `sumBuf`. It must lower to `[i64; 4]` via
/// `from_fn` (no `Vec` build) and still compute `(10+20+30+40)*2 = 200`.
#[test]
fn scratch_buffer_fixed_count_readonly_scalarizes() {
    let code = r#"## To sumBuf (xs: Seq of Int) -> Int:
    Let mutable buf be a new Seq of Int.
    Repeat for j from 0 to 3:
        Push (item (j + 1) of xs) * 2 to buf.
    Let mutable total be 0.
    Repeat for k from 1 to 4:
        Set total to total + item k of buf.
    Return total.

## Main
Let mutable xs be a new Seq of Int.
Push 10 to xs.
Push 20 to xs.
Push 30 to xs.
Push 40 to xs.
Show sumBuf(xs).
"#;
    assert_exact_output(code, "200");
    let rust = compile_to_rust(code).unwrap();
    assert!(
        rust.contains("[i64; 4]") && rust.contains("from_fn"),
        "scratch buffer should lower to a `[i64; 4]` built by from_fn, got:\n{}",
        rust
    );
    assert!(
        !rust.contains("let mut buf : Vec") && !rust.contains("let mut buf: Vec"),
        "scratch buffer must not be a heap Vec, got:\n{}",
        rust
    );
}

/// A buffer passed by `&[T]` borrow to a helper (never indexed locally) is still
/// a non-escaping scratch buffer — the borrow reads it, it does not escape. It
/// must scalarize and pass as `&buf` (array coerces to `&[T]`).
#[test]
fn scratch_buffer_borrowed_into_helper_scalarizes() {
    let code = r#"## To total4 (v: Seq of Int) -> Int:
    Let mutable s be 0.
    Repeat for i from 1 to 4:
        Set s to s + item i of v.
    Return s.

## To doubleSum (xs: Seq of Int) -> Int:
    Let mutable buf be a new Seq of Int.
    Repeat for j from 0 to 3:
        Push (item (j + 1) of xs) * 2 to buf.
    Return total4(buf).

## Main
Let mutable xs be a new Seq of Int.
Push 10 to xs.
Push 20 to xs.
Push 30 to xs.
Push 40 to xs.
Show doubleSum(xs).
"#;
    assert_exact_output(code, "200");
    let rust = compile_to_rust(code).unwrap();
    assert!(
        rust.contains("[i64; 4]") && rust.contains("from_fn"),
        "borrowed scratch buffer should scalarize to `[i64; 4]`, got:\n{}",
        rust
    );
    assert!(
        rust.contains("total4(&buf)"),
        "scalarized array should pass as `&buf` (coerces to &[i64]), got:\n{}",
        rust
    );
}

/// A buffer that ESCAPES by being returned must NOT scalarize — a stack array
/// can't outlive the frame. It stays a heap `Seq`; output stays correct.
#[test]
fn returned_buffer_does_not_scalarize() {
    let code = r#"## To buildBuf (n: Int) -> Seq of Int:
    Let mutable buf be a new Seq of Int.
    Repeat for j from 0 to 3:
        Push (j + 1) * n to buf.
    Return buf.

## Main
Let r be buildBuf(5).
Show item 2 of r.
"#;
    // item 2 (1-based) = (1+1)*5 = 10.
    assert_exact_output(code, "10");
    let rust = compile_to_rust(code).unwrap();
    assert!(
        !rust.contains("from_fn"),
        "a returned buffer must not scalarize to a stack array, got:\n{}",
        rust
    );
}

/// A VARIABLE trip count can't size a stack array — the buffer stays a heap
/// `Seq`; output stays correct.
#[test]
fn variable_count_buffer_does_not_scalarize() {
    let code = r#"## To sumVar (xs: Seq of Int) and (n: Int) -> Int:
    Let mutable buf be a new Seq of Int.
    Repeat for j from 1 to n:
        Push item j of xs to buf.
    Let mutable total be 0.
    Repeat for k from 1 to n:
        Set total to total + item k of buf.
    Return total.

## Main
Let mutable xs be a new Seq of Int.
Push 3 to xs.
Push 4 to xs.
Push 5 to xs.
Show sumVar(xs, 3).
"#;
    assert_exact_output(code, "12");
    let rust = compile_to_rust(code).unwrap();
    assert!(
        !rust.contains("from_fn"),
        "a variable-count buffer must not scalarize to a stack array, got:\n{}",
        rust
    );
}

/// A buffer mutated AFTER its fill loop (a later `Set index`/`Push`) is not a
/// write-once table — it must NOT scalarize (or must stay sound). Stays a heap
/// `Seq`; output stays correct.
#[test]
fn post_mutated_buffer_does_not_scalarize() {
    let code = r#"## To tweak (xs: Seq of Int) -> Int:
    Let mutable buf be a new Seq of Int.
    Repeat for j from 0 to 3:
        Push item (j + 1) of xs to buf.
    Push 99 to buf.
    Let mutable total be 0.
    Repeat for k from 1 to 5:
        Set total to total + item k of buf.
    Return total.

## Main
Let mutable xs be a new Seq of Int.
Push 1 to xs.
Push 2 to xs.
Push 3 to xs.
Push 4 to xs.
Show tweak(xs).
"#;
    // 1+2+3+4 + 99 = 109.
    assert_exact_output(code, "109");
    let rust = compile_to_rust(code).unwrap();
    assert!(
        !rust.contains("from_fn"),
        "a post-mutated buffer must not scalarize, got:\n{}",
        rust
    );
}
