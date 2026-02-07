//! E2E Tests: Temporal Types
//!
//! Tests Duration, Date, and Moment literals at runtime.

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::run_logos;

// === DURATION E2E TESTS ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_duration_milliseconds_compiles() {
    let result = run_logos(
        r#"## Main
Let timeout be 500ms.
Show "done".
"#,
    );
    assert!(
        result.success,
        "Code should run.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert_eq!(result.stdout.trim(), "done", "Got: {}", result.stdout);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_duration_seconds_compiles() {
    let result = run_logos(
        r#"## Main
Let delay be 2s.
Show "done".
"#,
    );
    assert!(
        result.success,
        "Code should run.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert_eq!(result.stdout.trim(), "done", "Got: {}", result.stdout);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_duration_nanoseconds_compiles() {
    let result = run_logos(
        r#"## Main
Let precise be 50ns.
Show "done".
"#,
    );
    assert!(
        result.success,
        "Code should run.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert_eq!(result.stdout.trim(), "done", "Got: {}", result.stdout);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_duration_microseconds_compiles() {
    let result = run_logos(
        r#"## Main
Let fast be 100us.
Show "done".
"#,
    );
    assert!(
        result.success,
        "Code should run.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert_eq!(result.stdout.trim(), "done", "Got: {}", result.stdout);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_duration_minutes_compiles() {
    let result = run_logos(
        r#"## Main
Let long be 5min.
Show "done".
"#,
    );
    assert!(
        result.success,
        "Code should run.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert_eq!(result.stdout.trim(), "done", "Got: {}", result.stdout);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_duration_hours_compiles() {
    let result = run_logos(
        r#"## Main
Let very_long be 1h.
Show "done".
"#,
    );
    assert!(
        result.success,
        "Code should run.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert_eq!(result.stdout.trim(), "done", "Got: {}", result.stdout);
}

// === DATE E2E TESTS ===

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_date_literal_compiles() {
    let result = run_logos(
        r#"## Main
Let graduation be 2026-05-20.
Show "done".
"#,
    );
    assert!(
        result.success,
        "Date literal should run.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(
        result.rust_code.contains("LogosDate"),
        "Code should contain LogosDate.\nGenerated Rust:\n{}",
        result.rust_code
    );
    assert_eq!(result.stdout.trim(), "done", "Got: {}", result.stdout);
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
        result.success,
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
        result.success,
        "Interpreter should handle date literals.\nError: {}",
        result.error
    );
}
