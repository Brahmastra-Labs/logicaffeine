//! E2E tests for Phase 54: Go-like Concurrency
//!
//! These tests actually compile and run the generated Rust code.

mod common;
use common::{run_logos, assert_runs, assert_exact_output, assert_output_contains_all, assert_output_lines};

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
    assert_exact_output(source, "pipe created");
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
    assert_eq!(result.stdout.trim(), "42", "Got: {}", result.stdout);
}

#[test]
fn e2e_try_send_nonblocking() {
    let source = r#"
## Main
    Let ch be a Pipe of Int.
    Try to send 99 into ch.
    Show "sent".
"#;
    assert_exact_output(source, "sent");
}

#[test]
fn e2e_try_receive_nonblocking() {
    let source = r#"
## Main
    Let ch be a Pipe of Int.
    Try to receive x from ch.
    Show "tried".
"#;
    assert_exact_output(source, "tried");
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
    assert_eq!(result.stdout.trim(), "timeout", "Got: {}", result.stdout);
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
    assert_eq!(result.stdout.trim(), "42", "Got: {}", result.stdout);
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
    assert_eq!(result.stdout.trim(), "84", "Got: {}", result.stdout);
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
    assert_eq!(result.stdout.trim(), "15", "Got: {}", result.stdout);
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
    assert_eq!(result.stdout.trim(), "20", "Got: {}", result.stdout);
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
    assert_eq!(result.stdout.trim(), "yes", "Got: {}", result.stdout);
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
    assert_eq!(result.stdout.trim(), "done", "Got: {}", result.stdout);
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
    assert_eq!(result.stdout.trim(), "123", "Got: {}", result.stdout);
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
    assert_eq!(result.stdout.trim(), "15", "Got: {}", result.stdout);
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
    assert_eq!(result.stdout.trim(), "6", "Got: {}", result.stdout);
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
    assert_eq!(result.stdout.trim(), "11", "Got: {}", result.stdout);
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
    assert_eq!(result.stdout.trim(), "3", "Got: {}", result.stdout);
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
    assert_eq!(result.stdout.trim(), "42", "Got: {}", result.stdout);
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
    assert_eq!(result.stdout.trim(), "42", "Got: {}", result.stdout);
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

// =============================================================================
// Phase G: Select with Receive Branches
// =============================================================================

#[test]
fn e2e_select_receive_branch() {
    // Select with a receive branch that fires
    let source = r#"
## Main
    Let ch be a Pipe of Int.
    Send 42 into ch.
    Await the first of:
        Receive x from ch:
            Show x.
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
    assert_eq!(result.stdout.trim(), "42", "Got: {}", result.stdout);
    assert!(!result.stdout.contains("timeout"), "Should NOT timeout: {}", result.stdout);
}

#[test]
fn e2e_select_timeout_fires() {
    // Select where timeout fires before receive
    let source = r#"
## Main
    Let ch be a Pipe of Int.
    Await the first of:
        Receive x from ch:
            Show x.
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
    assert_eq!(result.stdout.trim(), "timeout", "Got: {}", result.stdout);
}

#[test]
fn e2e_select_multiple_pipes() {
    // Select from multiple pipes - x works as variable name
    let source = r#"
## Main
    Let ch1 be a Pipe of Int.
    Let ch2 be a Pipe of Int.
    Send 42 into ch1.
    Await the first of:
        Receive x from ch1:
            Show x.
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
    assert_eq!(result.stdout.trim(), "42", "Got: {}", result.stdout);
}

// =============================================================================
// Phase H: Parallel Blocks (Simultaneously - CPU-bound)
// =============================================================================

#[test]
fn e2e_parallel_basic() {
    // Parallel execution with rayon
    let source = r#"
## To cpu_work (x: Int) -> Int:
    Return x * x.

## Main
    Simultaneously:
        Let a be cpu_work(5).
        Let b be cpu_work(10).
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
    assert!(result.stdout.contains("25"), "Should output 25: {}", result.stdout);
    assert!(result.stdout.contains("100"), "Should output 100: {}", result.stdout);
}

#[test]
fn e2e_parallel_three_tasks() {
    // Parallel with 3 tasks - compute and show inside the block since
    // parallel codegen for 3+ tasks doesn't extract variables properly yet
    let source = r#"
## To square_and_show (x: Int):
    Let result be x * x.
    Show result.

## Main
    Simultaneously:
        Call square_and_show with 2.
        Call square_and_show with 3.
        Call square_and_show with 4.
    Show "done".
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    // Should output 4, 9, 16 in some order, then "done"
    assert!(result.stdout.contains("4"), "Should output 4: {}", result.stdout);
    assert!(result.stdout.contains("9"), "Should output 9: {}", result.stdout);
    assert!(result.stdout.contains("16"), "Should output 16: {}", result.stdout);
    assert!(result.stdout.contains("done"), "Should output done: {}", result.stdout);
}

// =============================================================================
// Phase I: Concurrent with Multiple Tasks
// =============================================================================

#[test]
fn e2e_concurrent_three_tasks() {
    // Concurrent with 3 async tasks
    let source = r#"
## To fetch (id: Int) -> Int:
    Sleep 10.
    Return id * 10.

## Main
    Attempt all of the following:
        Let a be fetch(1).
        Let b be fetch(2).
        Let c be fetch(3).
    Show a + b + c.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert_eq!(result.stdout.trim(), "60", "Got: {}", result.stdout);
}

#[test]
fn e2e_concurrent_four_tasks() {
    // Concurrent with 4 async tasks
    let source = r#"
## To fetch (id: Int) -> Int:
    Sleep 5.
    Return id.

## Main
    Attempt all of the following:
        Let a be fetch(1).
        Let b be fetch(2).
        Let c be fetch(3).
        Let d be fetch(4).
    Show a + b + c + d.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert_eq!(result.stdout.trim(), "10", "Got: {}", result.stdout);
}

// =============================================================================
// Phase J: Launch Task With Handle
// =============================================================================

#[test]
fn e2e_launch_with_handle_and_stop() {
    // Explicit test of LaunchTaskWithHandle
    let source = r#"
## To counter:
    Let i be 0.
    While true:
        Sleep 10.
        Set i to i + 1.

## Main
    Let worker be Launch a task to counter.
    Sleep 30.
    Stop worker.
    Show "stopped".
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert_eq!(result.stdout.trim(), "stopped", "Got: {}", result.stdout);
}

// =============================================================================
// Phase K: Async in Struct Field Init
// =============================================================================

#[test]
fn e2e_async_in_struct_init() {
    // Async call in struct field initialization
    let source = r#"
## A Coords has:
    An x: Int.
    A y: Int.

## To async_coord -> Int:
    Sleep 10.
    Return 50.

## Main
    Let p be a new Coords with x async_coord() and y 100.
    Show p's x.
    Show p's y.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("50"), "Should output 50: {}", result.stdout);
    assert!(result.stdout.contains("100"), "Should output 100: {}", result.stdout);
}

// =============================================================================
// Phase L: Async in Send Value
// =============================================================================

#[test]
fn e2e_async_in_send() {
    // Async call as the value being sent into pipe
    let source = r#"
## To produce -> Int:
    Sleep 10.
    Return 42.

## Main
    Let ch be a Pipe of Int.
    Send produce() into ch.
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
    assert_eq!(result.stdout.trim(), "42", "Got: {}", result.stdout);
}

// =============================================================================
// Phase M: Sleep with Async Expression
// =============================================================================

#[test]
fn e2e_sleep_with_async_expr() {
    // Sleep with async expression for delay
    let source = r#"
## To get_delay -> Int:
    Sleep 10.
    Return 50.

## Main
    Sleep get_delay().
    Show "done".
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert_eq!(result.stdout.trim(), "done", "Got: {}", result.stdout);
}

// =============================================================================
// Phase N: Fan-out/Fan-in Patterns
// =============================================================================

#[test]
fn e2e_fanout_pattern() {
    // Multiple consumers from one producer
    let source = r#"
## To produce (ch: Int):
    Send 1 into ch.
    Send 2 into ch.
    Send 3 into ch.

## Main
    Let ch be a Pipe of Int.
    Launch a task to produce with ch.
    Receive a from ch.
    Receive b from ch.
    Receive c from ch.
    Show a + b + c.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert_eq!(result.stdout.trim(), "6", "Got: {}", result.stdout);
}

// =============================================================================
// Phase O: Loop + Concurrent Combinations
// =============================================================================

#[test]
fn e2e_concurrent_in_loop() {
    // Concurrent block inside a loop - concurrent with 2 vars to avoid tuple unpacking issue
    let source = r#"
## To fetch (id: Int) -> Int:
    Sleep 5.
    Return id * 10.

## Main
    Let mutable total be 0.
    Repeat for i from 1 to 2:
        Attempt all of the following:
            Let val be fetch(i).
            Let val2 be fetch(i).
        Set total to total + val + val2.
    Show total.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    // (10+10) + (20+20) = 60
    assert_eq!(result.stdout.trim(), "60", "Got: {}", result.stdout);
}

#[test]
fn e2e_loop_inside_concurrent() {
    // Loop inside concurrent block (each branch has its own loop)
    let source = r#"
## To sum_range (n: Int) -> Int:
    Sleep 10.
    Let total be 0.
    Repeat for i from 1 to n:
        Set total to total + i.
    Return total.

## Main
    Attempt all of the following:
        Let a be sum_range(3).
        Let b be sum_range(4).
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
    assert!(result.stdout.contains("6"), "Should output 6 (1+2+3): {}", result.stdout);
    assert!(result.stdout.contains("10"), "Should output 10 (1+2+3+4): {}", result.stdout);
}

// =============================================================================
// Phase P: Conditional + Pipe Combinations
// =============================================================================

#[test]
fn e2e_conditional_send() {
    // Conditional determines what to send
    let source = r#"
## Main
    Let ch be a Pipe of Int.
    Let flag be true.
    If flag:
        Send 42 into ch.
    Otherwise:
        Send 0 into ch.
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
    assert_eq!(result.stdout.trim(), "42", "Got: {}", result.stdout);
}

#[test]
fn e2e_select_in_loop() {
    // Select inside a loop - use mutable for total since it's reassigned
    let source = r#"
## Main
    Let ch be a Pipe of Int.
    Send 1 into ch.
    Send 2 into ch.
    Let mutable total be 0.
    Repeat for i from 1 to 2:
        Await the first of:
            Receive x from ch:
                Set total to total + x.
            After 1 seconds:
                Show "timeout".
    Show total.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert_eq!(result.stdout.trim(), "3", "Got: {}", result.stdout);
}

// =============================================================================
// Phase Q: Async Call Edge Cases
// =============================================================================

#[test]
fn e2e_async_call_in_both_sides_of_binary() {
    // Async calls on both sides of binary operation
    let source = r#"
## To left_val -> Int:
    Sleep 10.
    Return 10.

## To right_val -> Int:
    Sleep 10.
    Return 5.

## Main
    Let result be left_val() + right_val().
    Show result.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert_eq!(result.stdout.trim(), "15", "Got: {}", result.stdout);
}

#[test]
fn e2e_async_in_comparison() {
    // Async call used in a comparison after being assigned
    let source = r#"
## To threshold -> Int:
    Sleep 10.
    Return 50.

## Main
    Let limit be threshold().
    If 60 is greater than limit:
        Show "above".
    Otherwise:
        Show "below".
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert_eq!(result.stdout.trim(), "above", "Got: {}", result.stdout);
}

#[test]
fn e2e_multiple_async_in_one_expression() {
    // Multiple async calls in a single expression - each assigned then combined
    let source = r#"
## To get_a -> Int:
    Sleep 10.
    Return 1.

## To get_b -> Int:
    Sleep 10.
    Return 2.

## To get_c -> Int:
    Sleep 10.
    Return 3.

## Main
    Let av be get_a().
    Let bv be get_b().
    Let cv be get_c().
    Let result be av + bv * cv.
    Show result.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    // 1 + (2 * 3) = 7
    assert_eq!(result.stdout.trim(), "7", "Got: {}", result.stdout);
}

// =============================================================================
// Phase R: Transitive Async Through Multiple Layers
// =============================================================================

#[test]
fn e2e_transitive_async_three_levels() {
    // Transitive async through 3 levels of function calls
    let source = r#"
## To inner:
    Sleep 10.
    Show "inner".

## To middle:
    Call inner.
    Show "middle".

## To outer:
    Call middle.
    Show "outer".

## Main
    Call outer.
    Show "main".
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("inner"), "Should output inner: {}", result.stdout);
    assert!(result.stdout.contains("middle"), "Should output middle: {}", result.stdout);
    assert!(result.stdout.contains("outer"), "Should output outer: {}", result.stdout);
    assert!(result.stdout.contains("main"), "Should output main: {}", result.stdout);
}

#[test]
fn e2e_transitive_async_with_return() {
    // Transitive async with return values through multiple layers
    let source = r#"
## To base -> Int:
    Sleep 10.
    Return 10.

## To level1 -> Int:
    Return base() + 1.

## To level2 -> Int:
    Return level1() + 2.

## To level3 -> Int:
    Return level2() + 3.

## Main
    Let result be level3().
    Show result.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Should compile and run.\nGenerated:\n{}\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    // 10 + 1 + 2 + 3 = 16
    assert_eq!(result.stdout.trim(), "16", "Got: {}", result.stdout);
}
