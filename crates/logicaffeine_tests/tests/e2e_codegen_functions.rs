//! E2E Codegen Tests: Functions
//!
//! Mirrors e2e_functions.rs but compiles through the Rust codegen pipeline.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::assert_exact_output;

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_function_single_param() {
    assert_exact_output(
        r#"## To double (x: Int):
    Return x * 2.

## Main
Let result be double(5).
Show result.
"#,
        "10",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_function_two_params() {
    assert_exact_output(
        r#"## To add (a: Int) and (b: Int):
    Return a + b.

## Main
Show add(3, 7).
"#,
        "10",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_recursive_factorial() {
    assert_exact_output(
        r#"## To factorial (n: Int):
    If n is less than 2:
        Return 1.
    Return n * factorial(n - 1).

## Main
Show factorial(5).
"#,
        "120",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_function_no_params() {
    assert_exact_output(
        r#"## To greet -> Text:
    Return "Hello".

## Main
Show greet().
"#,
        "Hello",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_function_three_params() {
    assert_exact_output(
        r#"## To add3 (a: Int) and (b: Int) and (c: Int) -> Int:
    Return a + b + c.

## Main
Show add3(1, 2, 3).
"#,
        "6",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_function_returns_bool() {
    assert_exact_output(
        r#"## To isEven (n: Int) -> Bool:
    Return n / 2 * 2 equals n.

## Main
Show isEven(4).
"#,
        "true",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_function_returns_text() {
    assert_exact_output(
        r#"## To greeting (name: Text) -> Text:
    Return name.

## Main
Show greeting("World").
"#,
        "World",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_function_returns_seq() {
    assert_exact_output(
        r#"## To makeList -> Seq of Int:
    Return [1, 2, 3].

## Main
Show makeList().
"#,
        "[1, 2, 3]",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_function_with_loop() {
    assert_exact_output(
        r#"## To sumTo (n: Int) -> Int:
    Let sum be 0.
    Let i be 1.
    While i is at most n:
        Set sum to sum + i.
        Set i to i + 1.
    Return sum.

## Main
Show sumTo(5).
"#,
        "15",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_function_with_if() {
    assert_exact_output(
        r#"## To abs (n: Int) -> Int:
    If n is less than 0:
        Return 0 - n.
    Return n.

## Main
Show abs(0 - 5).
"#,
        "5",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_function_multiple_calls() {
    assert_exact_output(
        r#"## To double (x: Int) -> Int:
    Return x * 2.

## Main
Let a be double(3).
Let b be double(5).
Let c be double(7).
Show a + b + c.
"#,
        "30",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_codegen_nested_function_calls() {
    assert_exact_output(
        r#"## To double (x: Int) -> Int:
    Return x * 2.

## Main
Show double(double(5)).
"#,
        "20",
    );
}
