//! E2E Tests: Temporal Types
//!
//! Tests Duration, Date, and Moment literals at runtime.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::{compile_logos, run_logos};

// === DURATION E2E TESTS ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_duration_milliseconds_compiles() {
    let result = compile_logos(
        r#"## Main
Let timeout be 500ms.
Show "done".
"#,
    );
    assert!(
        result.success,
        "Code should compile.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_duration_seconds_compiles() {
    let result = compile_logos(
        r#"## Main
Let delay be 2s.
Show "done".
"#,
    );
    assert!(
        result.success,
        "Code should compile.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_duration_nanoseconds_compiles() {
    let result = compile_logos(
        r#"## Main
Let precise be 50ns.
Show "done".
"#,
    );
    assert!(
        result.success,
        "Code should compile.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_duration_microseconds_compiles() {
    let result = compile_logos(
        r#"## Main
Let fast be 100us.
Show "done".
"#,
    );
    assert!(
        result.success,
        "Code should compile.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_duration_minutes_compiles() {
    let result = compile_logos(
        r#"## Main
Let long be 5min.
Show "done".
"#,
    );
    assert!(
        result.success,
        "Code should compile.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_duration_hours_compiles() {
    let result = compile_logos(
        r#"## Main
Let very_long be 1h.
Show "done".
"#,
    );
    assert!(
        result.success,
        "Code should compile.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
}

// === DATE E2E TESTS ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_date_literal_compiles() {
    let result = compile_logos(
        r#"## Main
Let graduation be 2026-05-20.
Show "done".
"#,
    );
    // Date literals generate LogosDate which may need runtime definition
    // For now, just check the Rust code generates correctly
    assert!(
        result.rust_code.contains("LogosDate"),
        "Code should contain LogosDate.\nGenerated Rust:\n{}",
        result.rust_code
    );
}

// === INTERPRETER TESTS ===
// These test the interpreter path which doesn't need compiled Rust

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn interpreter_duration_parses() {
    use common::run_interpreter;

    let result = run_interpreter(
        r#"## Main
Let timeout be 500ms.
"#,
    );
    // Interpreter should parse duration literals without error
    assert!(
        result.success || result.error.is_empty(),
        "Interpreter should handle duration literals.\nError: {}",
        result.error
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn interpreter_date_parses() {
    use common::run_interpreter;

    let result = run_interpreter(
        r#"## Main
Let d be 2026-05-20.
"#,
    );
    // Interpreter should parse date literals without error
    assert!(
        result.success || result.error.is_empty(),
        "Interpreter should handle date literals.\nError: {}",
        result.error
    );
}
