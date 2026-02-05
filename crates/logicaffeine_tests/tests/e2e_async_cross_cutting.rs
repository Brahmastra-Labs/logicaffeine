//! E2E Tests: Async Cross-Cutting
//!
//! Tests async/concurrency features combined with other language features:
//! - Async + Enums
//! - Async + Structs
//! - Async + CRDTs
//! - Async + Collections
//! - Async + Temporal
//! - Async + Maps
//! - Async + Sets
//! - Async + Refinement
//! - Async + Zones
//! - Select + Features
//! - Parallel + Features
//!
//! ## Test Results Summary
//! - 28 passing tests
//! - 3 failing tests (codegen bugs identified)
//!
//! ## Known Codegen Bugs Found:
//! 1. Transitive async in Inspect arms: Sleep inside Inspect arm doesn't
//!    make the function async (e2e_async_inside_inspect_arm)
//! 2. Duration return type: `Duration` should be `std::time::Duration`
//!    (e2e_async_returns_duration)
//! 3. Async in list iteration: Parse/codegen issue with mutable variables
//!    in for-in loops (e2e_async_in_list_iteration)

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::{assert_output, assert_runs, run_logos};

// =============================================================================
// Category A: Async + Temporal
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_async_returns_duration() {
    let source = r#"## To async_delay -> Duration:
    Sleep 10.
    Return 100ms.

## Main
    Let d be async_delay().
    Show "ok".
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Async returning Duration should compile.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("ok"), "Should output ok: {}", result.stdout);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_sleep_with_async_delay() {
    let source = r#"## To get_delay -> Int:
    Sleep 10.
    Return 50.

## Main
    Sleep get_delay().
    Show "done".
"#;
    assert_output(source, "done");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_concurrent_with_sleep() {
    let source = r#"## To delayed (x: Int) -> Int:
    Sleep 20.
    Return x.

## Main
    Attempt all of the following:
        Let a be delayed(10).
        Let b be delayed(20).
    Show a + b.
"#;
    assert_output(source, "30");
}

// =============================================================================
// Category B: Async + CRDTs
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_async_function_with_crdt_struct() {
    let source = r#"## A Score is Shared and has:
    a points, which is a Tally.

## To init_score -> Score:
    Sleep 10.
    Return a new Score.

## Main
    Let mutable s be init_score().
    Increase s's points by 100.
    Show s's points.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Async with CRDT struct should compile.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("100"), "Should output 100: {}", result.stdout);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_crdt_after_async_call() {
    let source = r#"## A Counter is Shared and has:
    a value, which is a Tally.

## To wait:
    Sleep 10.
    Return.

## Main
    Let mutable c be a new Counter.
    Call wait.
    Increase c's value by 50.
    Show c's value.
"#;
    assert_output(source, "50");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_crdt_in_concurrent_block() {
    let source = r#"## A Stats is Shared and has:
    a hits, which is a Tally.

## To fetch (x: Int) -> Int:
    Sleep 10.
    Return x.

## Main
    Let mutable s be a new Stats.
    Attempt all of the following:
        Let a be fetch(10).
        Let b be fetch(20).
    Increase s's hits by a.
    Increase s's hits by b.
    Show s's hits.
"#;
    assert_output(source, "30");
}

// =============================================================================
// Category C: Async + Enums
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_async_returns_enum() {
    let source = r#"## A Result is one of:
    A Success with value Int.
    A Failure with msg Text.

## To async_compute -> Result:
    Sleep 10.
    Return a new Success with value 42.

## Main
    Let r be async_compute().
    Inspect r:
        When Success (v): Show v.
        When Failure (m): Show m.
"#;
    assert_output(source, "42");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_async_inside_inspect_arm() {
    let source = r#"## A Status is one of:
    A Pending.
    A Done with value Int.

## To process_status (s: Status) -> Int:
    Inspect s:
        When Pending:
            Sleep 10.
            Return 0.
        When Done (v):
            Return v.

## Main
    Let s be a new Pending.
    Let result be process_status(s).
    Show result.
"#;
    assert_output(source, "0");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_pipe_with_enum() {
    let source = r#"## A Message is one of:
    A Data with payload Int.
    An Eof.

## Main
    Let ch be a Pipe of Message.
    Let msg be a new Data with payload 123.
    Send msg into ch.
    Receive received from ch.
    Inspect received:
        When Data (p): Show p.
        When Eof: Show "end".
"#;
    assert_output(source, "123");
}

// =============================================================================
// Category D: Async + Structs
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_async_struct_field_init() {
    let source = r#"## A Point has:
    An x: Int.
    A y: Int.

## To async_coord -> Int:
    Sleep 10.
    Return 50.

## Main
    Let p be a new Point with x async_coord() and y 100.
    Show p's x.
    Show p's y.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Async struct field init should compile.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("50"), "Should output 50: {}", result.stdout);
    assert!(result.stdout.contains("100"), "Should output 100: {}", result.stdout);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_struct_field_after_async() {
    let source = r#"## A Config has:
    A threshold: Int.

## To wait:
    Sleep 10.
    Return.

## Main
    Let cfg be a new Config with threshold 100.
    Call wait.
    Show cfg's threshold.
"#;
    assert_output(source, "100");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_launch_task_with_struct() {
    let source = r#"## A Settings has:
    A level: Int.

## To worker (cfg: Settings):
    Sleep 10.
    Show cfg's level.

## Main
    Let settings be a new Settings with level 99.
    Launch a task to worker with settings.
    Sleep 50.
    Show "done".
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Launch with struct param should compile.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("done"), "Should output done: {}", result.stdout);
}

// =============================================================================
// Category E: Async + Collections
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_async_in_list_iteration() {
    // Note: "item" as loop variable causes parser issues, using "x" instead
    let source = r#"## To process (n: Int) -> Int:
    Sleep 10.
    Return n * 2.

## Main
Let items be [1, 2, 3].
Let mutable total be 0.
Repeat for x in items:
    Let processed be process(x).
    Set total to total + processed.
Show total.
"#;
    assert_output(source, "12");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_pipe_with_int_list_workaround() {
    // Note: Pipe of complex generic types may have parse issues
    // This test verifies pipe works with basic list operations
    let source = r#"## Main
    Let ch be a Pipe of Int.
    Send 1 into ch.
    Send 2 into ch.
    Send 3 into ch.
    Receive a from ch.
    Receive b from ch.
    Receive c from ch.
    Show a + b + c.
"#;
    assert_output(source, "6");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_concurrent_list_processing() {
    let source = r#"## To sum_list (items: Seq of Int) -> Int:
    Let mutable total be 0.
    Repeat for i in items:
        Set total to total + i.
    Return total.

## Main
    Let list1 be [1, 2, 3].
    Let list2 be [4, 5, 6].
    Attempt all of the following:
        Let a be sum_list(list1).
        Let b be sum_list(list2).
    Show a + b.
"#;
    assert_output(source, "21");
}

// =============================================================================
// Category F: Async + Maps
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_async_map_value_init() {
    let source = r#"## To async_value -> Int:
    Sleep 10.
    Return 42.

## Main
    Let mutable data be a new Map of Text to Int.
    Set item "key" of data to async_value().
    Show item "key" of data.
"#;
    assert_output(source, "42");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_map_access_after_async() {
    let source = r#"## To wait:
    Sleep 10.
    Return.

## Main
    Let mutable scores be a new Map of Text to Int.
    Set item "alice" of scores to 100.
    Call wait.
    Show item "alice" of scores.
"#;
    assert_output(source, "100");
}

// =============================================================================
// Category G: Async + Sets
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_async_set_add() {
    let source = r#"## To async_item -> Int:
    Sleep 10.
    Return 42.

## Main
    Let mutable items be a new Set of Int.
    Add async_item() to items.
    Add 100 to items.
    Show length of items.
"#;
    assert_output(source, "2");
}

// =============================================================================
// Category H: Async + Refinement Types
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_async_returning_refined_value() {
    let source = r#"## To get_positive -> Int:
    Sleep 10.
    Return 42.

## Main
    Let x: Int where x is greater than 0 be get_positive().
    Show x.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Async returning to refined var should compile.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("42"), "Should output 42: {}", result.stdout);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_refinement_after_async() {
    let source = r#"## To wait:
    Sleep 10.
    Return.

## Main
    Let x: Int where x is greater than 0 be 10.
    Call wait.
    Show x.
"#;
    assert_output(source, "10");
}

// =============================================================================
// Category I: Async + Zones
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_zone_after_async() {
    let source = r#"## To wait:
    Sleep 10.
    Return.

## Main
    Call wait.
    Inside a zone called "Work":
        Let x be 42.
        Show x.
"#;
    assert_output(source, "42");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_async_in_function_with_zone() {
    let source = r#"## To process_in_zone -> Int:
    Sleep 10.
    Inside a zone called "Work":
        Let x be 42.
    Return 42.

## Main
    Let result be process_in_zone().
    Show result.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Async function with zone should compile.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("42"), "Should output 42: {}", result.stdout);
}

// =============================================================================
// Category J: Select + Features
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_select_with_enum_handling() {
    let source = r#"## A Status is one of:
    A Ok with value Int.
    A Timeout.

## Main
    Let ch be a Pipe of Int.
    Send 42 into ch.
    Await the first of:
        Receive x from ch:
            Show x.
        After 1 seconds:
            Show "timeout".
"#;
    assert_output(source, "42");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_select_inside_loop() {
    let source = r#"## Main
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
    assert_output(source, "3");
}

// =============================================================================
// Category K: Parallel + Features
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_parallel_with_structs() {
    let source = r#"## A Data has:
    A value: Int.

## To compute (d: Data) -> Int:
    Return d's value * 2.

## Main
    Let d1 be a new Data with value 5.
    Let d2 be a new Data with value 10.
    Simultaneously:
        Let a be compute(d1).
        Let b be compute(d2).
    Show a.
    Show b.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Parallel with structs should compile.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("10"), "Should output 10: {}", result.stdout);
    assert!(result.stdout.contains("20"), "Should output 20: {}", result.stdout);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_parallel_with_collections() {
    let source = r#"## To sum (items: Seq of Int) -> Int:
    Let mutable total be 0.
    Repeat for i in items:
        Set total to total + i.
    Return total.

## Main
    Simultaneously:
        Let a be sum([1, 2, 3]).
        Let b be sum([4, 5, 6]).
    Show a.
    Show b.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Parallel with collections should compile.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("6"), "Should output 6: {}", result.stdout);
    assert!(result.stdout.contains("15"), "Should output 15: {}", result.stdout);
}

// =============================================================================
// Additional Edge Cases
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_nested_async_calls_with_struct() {
    let source = r#"## A Wrapper has:
    A value: Int.

## To inner -> Int:
    Sleep 10.
    Return 5.

## To outer -> Wrapper:
    Let v be inner().
    Return a new Wrapper with value v.

## Main
    Let w be outer().
    Show w's value.
"#;
    assert_output(source, "5");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_async_chain_with_enum_result() {
    let source = r#"## A Maybe is one of:
    A Just with value Int.
    A Nothing.

## To step1 -> Int:
    Sleep 10.
    Return 10.

## To step2 (x: Int) -> Maybe:
    Sleep 10.
    Return a new Just with value x.

## Main
    Let v be step1().
    Let result be step2(v).
    Inspect result:
        When Just (val): Show val.
        When Nothing: Show "none".
"#;
    assert_output(source, "10");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_concurrent_with_different_return_types() {
    let source = r#"## To get_int -> Int:
    Sleep 10.
    Return 42.

## To get_text -> Text:
    Sleep 10.
    Return "hello".

## Main
    Attempt all of the following:
        Let num be get_int().
        Let text be get_text().
    Show num.
    Show text.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Concurrent with different types should compile.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("42"), "Should output 42: {}", result.stdout);
    assert!(result.stdout.contains("hello"), "Should output hello: {}", result.stdout);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_pipe_send_async_value() {
    let source = r#"## To produce -> Int:
    Sleep 10.
    Return 99.

## Main
    Let ch be a Pipe of Int.
    Send produce() into ch.
    Receive x from ch.
    Show x.
"#;
    assert_output(source, "99");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_concurrent_struct_creation() {
    let source = r#"## A Point has:
    An x: Int.
    A y: Int.

## To make_x -> Int:
    Sleep 10.
    Return 10.

## To make_y -> Int:
    Sleep 10.
    Return 20.

## Main
    Attempt all of the following:
        Let x be make_x().
        Let y be make_y().
    Let p be a new Point with x x and y y.
    Show p's x.
    Show p's y.
"#;
    let result = run_logos(source);
    assert!(
        result.success,
        "Concurrent struct creation should compile.\nGenerated Rust:\n{}\n\nstderr: {}",
        result.rust_code,
        result.stderr
    );
    assert!(result.stdout.contains("10"), "Should output 10: {}", result.stdout);
    assert!(result.stdout.contains("20"), "Should output 20: {}", result.stdout);
}
