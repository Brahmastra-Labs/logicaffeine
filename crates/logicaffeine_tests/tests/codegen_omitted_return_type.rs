//! Omitted function return types must be INFERRED into a valid Rust signature.
//!
//! A `## To f (…):` with no `-> T` annotation but a value-returning body used to
//! reach AOT codegen with `-> _` (rustc E0121) or a `()`-typed merged function
//! (rustc E0308) whenever the return couldn't be inferred by the codegen-local
//! heuristic — specifically for a struct FIELD return, a CALL to another
//! omitted-return function, a user-struct return, and mutual recursion. The
//! type checker (`check_program`) already infers all of these via
//! Hindley-Milner; codegen must consume that inference. These lock it end to end
//! (real rustc compile + run), single-file, so they are unambiguously a codegen
//! contract independent of the module system.

#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::fs;

use tempfile::tempdir;

use logicaffeine_compile::compile::compile_project;

/// Compile a single-file program through the real AOT path and run it under
/// rustc; return `(compiled_and_ran, trimmed_stdout, stderr, rust)`.
fn compile_and_run(src: &str) -> (bool, String, String, String) {
    let dir = tempdir().unwrap();
    let main = dir.path().join("main.lg");
    fs::write(&main, src).unwrap();
    let output = compile_project(&main).expect("program should lower to Rust");
    let r = common::run_rust_project(&output.rust_code, &output.dependencies, &[]);
    (
        r.success,
        r.stdout.trim().to_string(),
        r.stderr,
        output.rust_code,
    )
}

/// A struct FIELD return with an omitted type infers the field's type.
#[test]
fn omitted_return_field_access() {
    let (ok, out, err, rust) = compile_and_run(
        "# App\n\n## A Point has:\n    An x: Int.\n    A y: Int.\n\n\
         ## To originX (p: Point):\n    Return p's x.\n\n\
         ## Main\nLet p be a new Point with x 9 and y 4.\nShow originX(p).\n",
    );
    assert!(
        !rust.contains("-> _"),
        "return type must not be the placeholder `_`.\n{rust}"
    );
    assert!(
        ok,
        "field-return fn must compile+run.\nstderr: {err}\n{rust}"
    );
    assert_eq!(out, "9");
}

/// A return that is a CALL to another omitted-return function infers through it.
#[test]
fn omitted_return_call_through() {
    let (ok, out, err, rust) = compile_and_run(
        "# App\n\n## To baseVal (n: Int):\n    Return n + 40.\n\n\
         ## To wrap (n: Int):\n    Return baseVal(n).\n\n\
         ## Main\nShow wrap(2).\n",
    );
    assert!(
        !rust.contains("-> _"),
        "return type must not be the placeholder `_`.\n{rust}"
    );
    assert!(
        ok,
        "call-through fn must compile+run.\nstderr: {err}\n{rust}"
    );
    assert_eq!(out, "42");
}

/// A function that RETURNS a user struct (omitted type) infers the struct type,
/// not `_`.
#[test]
fn omitted_return_user_struct() {
    let (ok, out, err, rust) = compile_and_run(
        "# App\n\n## A Point has:\n    An x: Int.\n    A y: Int.\n\n\
         ## To origin ():\n    Return a new Point with x 7 and y 0.\n\n\
         ## Main\nLet p be origin().\nShow p's x.\n",
    );
    assert!(
        !rust.contains("-> _"),
        "struct-return type must resolve, not `_`.\n{rust}"
    );
    assert!(
        ok,
        "struct-return fn must compile+run.\nstderr: {err}\n{rust}"
    );
    assert_eq!(out, "7");
}

/// Mutual recursion with omitted return types must not emit a `()`-returning
/// merged function that then returns a value.
#[test]
fn omitted_return_mutual_recursion() {
    let (ok, out, err, rust) = compile_and_run(
        "# App\n\n## To pingpong (n: Int):\n    If n is less than 1:\n        Return 0.\n    Return pong(n - 1).\n\n\
         ## To pong (n: Int):\n    If n is less than 1:\n        Return 1.\n    Return pingpong(n - 1).\n\n\
         ## Main\nShow pingpong(4).\n",
    );
    assert!(
        ok,
        "mutually-recursive fns must compile+run.\nstderr: {err}\n{rust}"
    );
    assert_eq!(out, "0");
}
