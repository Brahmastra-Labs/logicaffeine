//! E2E Tests: Closures & First-Class Functions
//!
//! Tests that closures can be created, captured, passed as arguments,
//! returned from functions, and called — through both the interpreter
//! and the full codegen pipeline.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::{assert_interpreter_output, assert_exact_output};

// =============================================================================
// A: Basic Closure Creation & Call (interpreter)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closure_identity() {
    assert_interpreter_output(
        r#"## Main
Let f be (x: Int) -> x.
Show f(42).
"#,
        "42",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closure_arithmetic() {
    assert_interpreter_output(
        r#"## Main
Let doubler be (n: Int) -> n * 2.
Show doubler(5).
"#,
        "10",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closure_two_params() {
    assert_interpreter_output(
        r#"## Main
Let add be (a: Int, b: Int) -> a + b.
Show add(3, 7).
"#,
        "10",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closure_no_params() {
    assert_interpreter_output(
        r#"## Main
Let greet be () -> "hello".
Show greet().
"#,
        "hello",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closure_boolean() {
    assert_interpreter_output(
        r#"## Main
Let isPositive be (n: Int) -> n > 0.
Show isPositive(5).
Show isPositive(0 - 3).
"#,
        "true\nfalse",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closure_text() {
    assert_interpreter_output(
        r#"## Main
Let echo be (s: Text) -> s.
Show echo("world").
"#,
        "world",
    );
}

// =============================================================================
// B: Block Closures (interpreter)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closure_block_body() {
    assert_interpreter_output(
        r#"## Main
Let process be (n: Int) ->:
    Let doubled be n * 2.
    Return doubled + 1.
Show process(5).
"#,
        "11",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closure_block_with_if() {
    assert_interpreter_output(
        r#"## Main
Let clamp be (n: Int) ->:
    If n > 100:
        Return 100.
    If n < 0:
        Return 0.
    Return n.
Show clamp(150).
Show clamp(0 - 10).
Show clamp(50).
"#,
        "100\n0\n50",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closure_block_with_local_var() {
    assert_interpreter_output(
        r#"## Main
Let compute be (a: Int, b: Int) ->:
    Let sum be a + b.
    Let product be a * b.
    Return sum + product.
Show compute(3, 4).
"#,
        "19",
    );
}

// =============================================================================
// C: Variable Capture (interpreter)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closure_capture_int() {
    assert_interpreter_output(
        r#"## Main
Let offset be 10.
Let addOffset be (n: Int) -> n + offset.
Show addOffset(5).
"#,
        "15",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closure_capture_text() {
    assert_interpreter_output(
        r#"## Main
Let prefix be "hello ".
Let greet be (name: Text) -> prefix combined with name.
Show greet("world").
"#,
        "hello world",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closure_capture_multiple() {
    assert_interpreter_output(
        r#"## Main
Let base be 100.
Let scale be 3.
Let transform be (n: Int) -> base + n * scale.
Show transform(5).
"#,
        "115",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closure_capture_is_snapshot() {
    assert_interpreter_output(
        r#"## Main
Let x be 10.
Let getX be () -> x.
Set x to 999.
Show getX().
"#,
        "10",
    );
}

// =============================================================================
// D: Higher-Order Functions (e2e — needs codegen)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closure_pass_to_function() {
    assert_exact_output(
        r#"## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:
    Return f(x).

## Main
Show apply((n: Int) -> n * 2, 5).
"#,
        "10",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closure_return_from_function() {
    assert_exact_output(
        r#"## To makeAdder (n: Int) -> fn(Int) -> Int:
    Return (x: Int) -> x + n.

## Main
Let add5 be makeAdder(5).
Show add5(10).
"#,
        "15",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closure_inline_argument() {
    assert_exact_output(
        r#"## To applyTwice (f: fn(Int) -> Int) and (x: Int) -> Int:
    Return f(f(x)).

## Main
Show applyTwice((n: Int) -> n + 1, 10).
"#,
        "12",
    );
}

// =============================================================================
// E: Function Type Annotations (e2e)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closure_fn_type_param() {
    assert_exact_output(
        r#"## To transform (f: fn(Int) -> Int) and (x: Int) -> Int:
    Return f(x).

## Main
Let triple be (n: Int) -> n * 3.
Show transform(triple, 7).
"#,
        "21",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closure_fn_type_return() {
    assert_exact_output(
        r#"## To makeMultiplier (factor: Int) -> fn(Int) -> Int:
    Return (x: Int) -> x * factor.

## Main
Let times4 be makeMultiplier(4).
Show times4(8).
"#,
        "32",
    );
}

// =============================================================================
// F: Advanced (interpreter)
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closure_nested() {
    assert_interpreter_output(
        r#"## Main
Let makeAdder be (a: Int) ->:
    Return (b: Int) -> a + b.
Let add3 be makeAdder(3).
Show add3(7).
"#,
        "10",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closure_multiple_calls() {
    assert_interpreter_output(
        r#"## Main
Let square be (n: Int) -> n * n.
Show square(2).
Show square(5).
Show square(10).
"#,
        "4\n25\n100",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closure_call_named_function() {
    assert_interpreter_output(
        r#"## To double (n: Int) -> Int:
    Return n * 2.

## Main
Let process be (x: Int) -> double(x) + 1.
Show process(5).
"#,
        "11",
    );
}

// =============================================================================
// G: Additional robust tests
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closure_as_let_binding_then_call() {
    assert_interpreter_output(
        r#"## Main
Let negate be (n: Int) -> 0 - n.
Let result be negate(42).
Show result.
"#,
        "-42",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closure_capture_in_block() {
    assert_interpreter_output(
        r#"## Main
Let multiplier be 5.
Let scale be (n: Int) ->:
    Let result be n * multiplier.
    Return result.
Show scale(6).
"#,
        "30",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closure_zero_param_block() {
    assert_interpreter_output(
        r#"## Main
Let greeting be () ->:
    Let msg be "hi there".
    Return msg.
Show greeting().
"#,
        "hi there",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closure_expression_returns_closure() {
    assert_exact_output(
        r#"## To makeGreeter (greeting: Text) -> fn(Text) -> Text:
    Return (name: Text) -> greeting combined with " " combined with name.

## Main
Let hello be makeGreeter("Hello").
Show hello("Alice").
"#,
        "Hello Alice",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closure_two_closures_same_capture() {
    assert_interpreter_output(
        r#"## Main
Let base be 10.
Let addBase be (n: Int) -> n + base.
Let mulBase be (n: Int) -> n * base.
Show addBase(5).
Show mulBase(5).
"#,
        "15\n50",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closure_codegen_basic() {
    assert_exact_output(
        r#"## Main
Let doubler be (n: Int) -> n * 2.
Show doubler(7).
"#,
        "14",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closure_codegen_capture() {
    assert_exact_output(
        r#"## Main
Let offset be 100.
Let addOffset be (n: Int) -> n + offset.
Show addOffset(23).
"#,
        "123",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closure_codegen_block() {
    assert_exact_output(
        r#"## Main
Let process be (n: Int) ->:
    Let doubled be n * 2.
    Return doubled + 1.
Show process(10).
"#,
        "21",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closure_codegen_snapshot() {
    assert_exact_output(
        r#"## Main
Let x be 5.
Let getX be () -> x.
Set x to 999.
Show getX().
"#,
        "5",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_closure_pass_closure_variable() {
    assert_exact_output(
        r#"## To apply (f: fn(Int) -> Int) and (x: Int) -> Int:
    Return f(x).

## Main
Let square be (n: Int) -> n * n.
Show apply(square, 6).
"#,
        "36",
    );
}
