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

// =============================================================================
// Phase B: Nested Async Call Tests (Expose Bug A - async in expressions)
// =============================================================================

#[test]
fn e2e_async_in_sync_arg() {
    // Bug A: Async call as argument to sync function
    // async_value() must be awaited before passing to double()
    let source = r#"
## To async_value -> Int:
    Sleep 10.
    Return 42.

## To double (x: Int) -> Int:
    Return x * 2.

## Main
    Let result be double(async_value()).
    Show result.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("84"), "Should output 84: {}", result.stdout);
}

#[test]
fn e2e_async_in_binary_op() {
    // Bug A: Async call in binary operation
    // compute() must be awaited before addition
    let source = r#"
## To compute -> Int:
    Sleep 10.
    Return 10.

## Main
    Let x be 5 + compute().
    Show x.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("15"), "Should output 15: {}", result.stdout);
}

#[test]
fn e2e_async_in_list() {
    // Bug A: Async call in list literal
    // get_value() must be awaited
    let source = r#"
## To get_value -> Int:
    Sleep 10.
    Return 99.

## Main
    Let items be [1, get_value(), 3].
    Show items.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("99"), "Should output 99: {}", result.stdout);
}

#[test]
fn e2e_async_in_index() {
    // Bug A: Async call in index expression
    // get_index() must be awaited
    let source = r#"
## To get_index -> Int:
    Sleep 10.
    Return 2.

## Main
    Let items be [10, 20, 30].
    Let val be items[get_index()].
    Show val.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("20"), "Should output 20: {}", result.stdout);
}

#[test]
fn e2e_async_in_condition() {
    // Bug A: Async call assigned to variable used in if condition
    // verify() must be awaited when assigned
    let source = r#"
## To verify -> Bool:
    Sleep 10.
    Return true.

## Main
    Let flag be verify().
    If flag:
        Show "yes".
    Otherwise:
        Show "no".
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("yes"), "Should output yes: {}", result.stdout);
}

#[test]
fn e2e_async_in_while_condition() {
    // Bug A: Async call assigned to variable used in while condition
    // should_continue() must be awaited when assigned
    let source = r#"
## To should_continue -> Bool:
    Sleep 10.
    Return false.

## Main
    Let run be should_continue().
    While run:
        Show "looping".
        Set run to should_continue().
    Show "done".
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("done"), "Should output done: {}", result.stdout);
}

#[test]
fn e2e_async_in_return() {
    // Bug A: Async call in return value
    // fetch() must be awaited before return
    let source = r#"
## To fetch -> Int:
    Sleep 10.
    Return 123.

## To wrapper -> Int:
    Return fetch().

## Main
    Let x be wrapper().
    Show x.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("123"), "Should output 123: {}", result.stdout);
}

// =============================================================================
// Phase C: Complex Control Flow + Async
// =============================================================================

#[test]
fn e2e_async_in_loop() {
    // Async call in loop body
    // fetch_next() must be awaited each iteration
    let source = r#"
## To fetch_next -> Int:
    Sleep 10.
    Return 5.

## Main
    Let total be 0.
    Repeat for i from 1 to 3:
        Set total to total + fetch_next().
    Show total.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("15"), "Should output 15: {}", result.stdout);
}

#[test]
fn e2e_multiple_async_sequence() {
    // Multiple async calls in sequence (already working, verifying)
    let source = r#"
## To step1 -> Int:
    Sleep 10.
    Return 1.

## To step2 -> Int:
    Sleep 10.
    Return 2.

## To step3 -> Int:
    Sleep 10.
    Return 3.

## Main
    Let a be step1().
    Let b be step2().
    Let c be step3().
    Show a + b + c.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("6"), "Should output 6: {}", result.stdout);
}

#[test]
fn e2e_async_deeply_nested() {
    // Deeply nested async call
    // inner() inside middle(inner()) must be awaited
    let source = r#"
## To inner -> Int:
    Sleep 10.
    Return 5.

## To middle (x: Int) -> Int:
    Return x * 2.

## To outer (x: Int) -> Int:
    Return x + 1.

## Main
    Let result be outer(middle(inner())).
    Show result.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("11"), "Should output 11: {}", result.stdout);
}

// =============================================================================
// Phase D: Concurrent/Parallel Block Tests
// =============================================================================

#[test]
fn e2e_concurrent_nested_async() {
    // Concurrent block with nested async in Let value
    // async_val() inside wrap() must be awaited
    let source = r#"
## To async_val -> Int:
    Sleep 10.
    Return 42.

## To wrap (x: Int) -> Int:
    Return x + 1.

## Main
    Attempt all of the following:
        Let a be wrap(async_val()).
        Let b be wrap(async_val()).
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
    assert!(result.stdout.contains("43"), "Should output 43: {}", result.stdout);
}

#[test]
fn e2e_concurrent_with_conditional() {
    // Concurrent with conditional async
    let source = r#"
## To cond_async (flag: Bool) -> Int:
    Sleep 10.
    If flag:
        Return 1.
    Otherwise:
        Return 0.

## Main
    Attempt all of the following:
        Let a be cond_async(true).
        Let b be cond_async(false).
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
    assert!(result.stdout.contains("1"), "Should output 1: {}", result.stdout);
    assert!(result.stdout.contains("0"), "Should output 0: {}", result.stdout);
}

// =============================================================================
// Phase E: Pipe/Channel Complex Tests
// =============================================================================

#[test]
fn e2e_pipe_async_producer() {
    // Pipe with async producer
    let source = r#"
## To produce (ch: Int):
    Sleep 10.
    Send 1 into ch.
    Sleep 10.
    Send 2 into ch.

## Main
    Let jobs be a Pipe of Int.
    Launch a task to produce with jobs.
    Receive a from jobs.
    Receive b from jobs.
    Show a + b.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("3"), "Should output 3: {}", result.stdout);
}

#[test]
fn e2e_pipe_in_conditional() {
    // Pipe inside conditional
    let source = r#"
## Main
    Let flag be true.
    If flag:
        Let ch be a Pipe of Int.
        Send 42 into ch.
        Receive x from ch.
        Show x.
    Otherwise:
        Show "no".
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

// =============================================================================
// Phase F: Edge Cases
// =============================================================================

#[test]
fn e2e_sync_in_async_context() {
    // Sync function in async context (should work)
    let source = r#"
## To sync_double (x: Int) -> Int:
    Return x * 2.

## To async_wrapper -> Int:
    Sleep 10.
    Return sync_double(21).

## Main
    Let x be async_wrapper().
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
fn e2e_async_void_function() {
    // Async function returning nothing
    let source = r#"
## To async_side_effect:
    Sleep 10.
    Show "effect".

## Main
    Call async_side_effect.
    Show "after".
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("effect"), "Should output effect: {}", result.stdout);
    assert!(result.stdout.contains("after"), "Should output after: {}", result.stdout);
}

#[test]
fn e2e_launch_in_conditional() {
    // Launch inside conditional
    let source = r#"
## To worker:
    Sleep 50.
    Show "worker done".

## Main
    Let flag be true.
    If flag:
        Launch a task to worker.
    Show "main done".
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("main done"), "Should output main done: {}", result.stdout);
}
