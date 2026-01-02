//! E2E tests for Phase 54: Go-like Concurrency
//!
//! These tests actually compile and run the generated Rust code.

mod common;
use common::{run_logos, assert_runs, assert_output};

// =============================================================================
// Launch Task E2E Tests
// =============================================================================

#[test]
fn e2e_launch_simple_task() {
    // Fire-and-forget task that prints a value
    let source = r#"
## To worker:
    Show "worker done".

## Main
    Launch a task to worker.
    Show "main continues".
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    // Main should print immediately (doesn't wait for task)
    assert!(result.stdout.contains("main continues"), "Main should continue: {}", result.stdout);
}

#[test]
fn e2e_launch_task_with_arg() {
    let source = r#"
## To greet (name: Text):
    Show name.

## Main
    Launch a task to greet with "Hello from task".
    Show "launched".
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("launched"), "Should launch: {}", result.stdout);
}

// =============================================================================
// Pipe E2E Tests
// =============================================================================

#[test]
fn e2e_pipe_create() {
    // Just create a pipe - should compile and run
    let source = r#"
## Main
    Let ch be a Pipe of Int.
    Show "pipe created".
"#;
    assert_output(source, "pipe created");
}

#[test]
fn e2e_pipe_send_receive() {
    // Send and receive on same task (will block in practice, but tests codegen)
    let source = r#"
## Main
    Let ch be a Pipe of Int.
    Send 42 into ch.
    Receive x from ch.
    Show x.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("42"), "Should receive 42: {}", result.stdout);
}

#[test]
fn e2e_try_send_nonblocking() {
    let source = r#"
## Main
    Let ch be a Pipe of Int.
    Try to send 99 into ch.
    Show "sent".
"#;
    assert_output(source, "sent");
}

#[test]
fn e2e_try_receive_nonblocking() {
    let source = r#"
## Main
    Let ch be a Pipe of Int.
    Try to receive x from ch.
    Show "tried".
"#;
    assert_output(source, "tried");
}

// =============================================================================
// Select E2E Tests
// =============================================================================

#[test]
fn e2e_select_timeout_only() {
    // Select with just a timeout - should trigger after specified time
    let source = r#"
## Main
    Let p be a Pipe of Text.
    Await the first of:
        After 1 seconds:
            Show "timeout".
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("timeout"), "Should timeout: {}", result.stdout);
}

// =============================================================================
// Stop Task E2E Tests
// =============================================================================

#[test]
fn e2e_stop_task() {
    let source = r#"
## To infinite:
    Let x be 0.
    Show "started".

## Main
    Let handle be Launch a task to infinite.
    Stop handle.
    Show "stopped".
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("stopped"), "Should stop: {}", result.stdout);
}

// =============================================================================
// Integration E2E Tests
// =============================================================================

#[test]
fn e2e_producer_consumer() {
    let source = r#"
## To produce (ch: Int):
    Send 1 into ch.
    Send 2 into ch.

## Main
    Let jobs be a Pipe of Int.
    Launch a task to produce with jobs.
    Receive first from jobs.
    Receive second from jobs.
    Show first.
    Show second.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("1"), "Should show 1: {}", result.stdout);
    assert!(result.stdout.contains("2"), "Should show 2: {}", result.stdout);
}
