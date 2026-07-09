//! E2E Codegen Tests: fixed-array borrow-parameter propagation.
//!
//! When EVERY call site passes a given `Seq of T` borrow parameter a fixed-size
//! `[T; N]` stack array of the SAME N, the parameter is emitted as `&[T; N]`
//! instead of `&[T]`. LLVM then knows the length, so a constant-index read
//! (`item 3 of v`) drops its bounds check. Purely a type refinement — the caller
//! passes `&arr` either way (an array coerces to a slice), so it is value-safe.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use common::assert_exact_output;
use common::compile_to_rust;

/// `sumFixed` reads a borrowed 4-slot buffer; its only caller passes a fixed
/// `[i64; 4]`. The parameter must become `&[i64; 4]` and still compute 100.
#[test]
fn borrow_param_all_fixed_callers_becomes_array_ref() {
    let code = r#"## To sumFixed (v: Seq of Int) -> Int:
    Let mutable s be 0.
    Repeat for i from 1 to 4:
        Set s to s + item i of v.
    Return s.

## Main
Let mutable a be a new Seq of Int.
Push 10 to a.
Push 20 to a.
Push 30 to a.
Push 40 to a.
Show sumFixed(a).
"#;
    assert_exact_output(code, "100");
    let rust = compile_to_rust(code).unwrap();
    assert!(
        rust.contains("v: &[i64; 4]"),
        "a borrow param passed only fixed [i64; 4] arrays should be `&[i64; 4]`, got:\n{}",
        rust
    );
}

/// A borrow param whose callers pass a VARIABLE-length collection (a runtime Vec)
/// stays `&[T]`. Output stays correct.
#[test]
fn borrow_param_variable_caller_stays_slice() {
    let code = r#"## To total (v: Seq of Int) -> Int:
    Let mutable s be 0.
    Repeat for x in v:
        Set s to s + x.
    Return s.

## Main
Let mutable a be a new Seq of Int.
Repeat for i from 1 to 5:
    Push i to a.
Show total(a).
"#;
    assert_exact_output(code, "15");
    let rust = compile_to_rust(code).unwrap();
    assert!(
        rust.contains("v: &[i64]") && !rust.contains("v: &[i64;"),
        "a borrow param passed a runtime-length collection must stay `&[i64]`, got:\n{}",
        rust
    );
}
