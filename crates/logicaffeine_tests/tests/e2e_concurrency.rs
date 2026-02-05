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

// =============================================================================
// Async Function Call E2E Tests (Bug 1 & 6 Verification)
// =============================================================================

#[test]
fn e2e_async_function_expression_call() {
    // Test Bug 6: Let x be async_func() should await and get the result, not a Future
    let source = r#"
## To sleeper -> Int:
    Sleep 10.
    Return 42.

## Main
    Let x be sleeper().
    Show x.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("42"), "Should output 42: {}", result.stdout);
}

#[test]
fn e2e_async_function_statement_call() {
    // Test Bug 1: Call async_func should await
    let source = r#"
## To async_printer:
    Sleep 10.
    Show "async done".

## Main
    Call async_printer.
    Show "main done".
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    // async_printer should complete before main_done because we await it
    assert!(result.stdout.contains("async done"), "Should output async done: {}", result.stdout);
    assert!(result.stdout.contains("main done"), "Should output main done: {}", result.stdout);
}

#[test]
fn e2e_transitive_async() {
    // Test Bug 2: Wrapper function that calls async function should also be async
    let source = r#"
## To helper:
    Sleep 50.
    Show "helper done".

## To wrapper:
    Call helper.
    Show "wrapper done".

## Main
    Call wrapper.
    Show "main done".
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    // All three should print in order
    assert!(result.stdout.contains("helper done"), "Should output helper done: {}", result.stdout);
    assert!(result.stdout.contains("wrapper done"), "Should output wrapper done: {}", result.stdout);
    assert!(result.stdout.contains("main done"), "Should output main done: {}", result.stdout);
}

#[test]
fn e2e_concurrent_with_sync_function() {
    // Test Bug 3: Sync functions in concurrent block should NOT get .await
    let source = r#"
## To sync_double (x: Int) -> Int:
    Return x * 2.

## Main
    Attempt all of the following:
        Let a be sync_double(5).
        Let b be sync_double(10).
    Show a.
    Show b.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("10"), "Should output 10: {}", result.stdout);
    assert!(result.stdout.contains("20"), "Should output 20: {}", result.stdout);
}

#[test]
fn e2e_concurrent_with_async_function() {
    // Test: Async functions in concurrent block SHOULD get .await
    let source = r#"
## To delayed_value (x: Int) -> Int:
    Sleep 10.
    Return x.

## Main
    Attempt all of the following:
        Let a be delayed_value(5).
        Let b be delayed_value(10).
    Show a.
    Show b.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("5"), "Should output 5: {}", result.stdout);
    assert!(result.stdout.contains("10"), "Should output 10: {}", result.stdout);
}
