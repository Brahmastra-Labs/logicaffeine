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
//! - Async coloring: a function that reaches a sleeping function from any expression or nested
//!   block is itself async on the compiled tier, a loop that awaits keeps the collection it walks,
//!   and a cycle of calls through a sleeping function runs; each prints what the tree-walker and the
//!   scheduler-driven VM print

#[cfg(not(target_arch = "wasm32"))]
mod common;

#[cfg(not(target_arch = "wasm32"))]
use common::{assert_exact_output, assert_output_contains_all, assert_output_lines};

// =============================================================================
// Category A: Async + Temporal
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_async_returns_duration() {
    assert_exact_output(
        r#"## To async_delay -> Duration:
    Sleep 10.
    Return 100ms.

## Main
    Let d be async_delay().
    Show "ok".
"#,
        "ok",
    );
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
    assert_exact_output(source, "done");
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
    assert_exact_output(source, "30");
}

// =============================================================================
// Category B: Async + CRDTs
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_async_function_with_crdt_struct() {
    assert_exact_output(
        r#"## A Score is Shared and has:
    a points, which is a Tally.

## To init_score -> Score:
    Sleep 10.
    Return a new Score.

## Main
    Let mutable s be init_score().
    Increase s's points by 100.
    Show s's points.
"#,
        "100",
    );
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
    assert_exact_output(source, "50");
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
    assert_exact_output(source, "30");
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
    assert_exact_output(source, "42");
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
    assert_exact_output(source, "0");
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
    assert_exact_output(source, "123");
}

// =============================================================================
// Category D: Async + Structs
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_async_struct_field_init() {
    assert_output_lines(
        r#"## A Point has:
    An x: Int.
    A y: Int.

## To async_coord -> Int:
    Sleep 10.
    Return 50.

## Main
    Let p be a new Point with x async_coord() and y 100.
    Show p's x.
    Show p's y.
"#,
        &["50", "100"],
    );
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
    assert_exact_output(source, "100");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_launch_task_with_struct() {
    assert_output_contains_all(
        r#"## A Settings has:
    A level: Int.

## To worker (cfg: Settings):
    Sleep 10.
    Show cfg's level.

## Main
    Let settings be a new Settings with level 99.
    Launch a task to worker with settings.
    Sleep 50.
    Show "done".
"#,
        &["99", "done"],
    );
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
    assert_exact_output(source, "12");
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
    assert_exact_output(source, "6");
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
    assert_exact_output(source, "21");
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
    assert_exact_output(source, "42");
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
    assert_exact_output(source, "100");
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
    assert_exact_output(source, "2");
}

// =============================================================================
// Category H: Async + Refinement Types
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_async_returning_refined_value() {
    assert_exact_output(
        r#"## To get_positive -> Int:
    Sleep 10.
    Return 42.

## Main
    Let x: Int where x is greater than 0 be get_positive().
    Show x.
"#,
        "42",
    );
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
    assert_exact_output(source, "10");
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
    assert_exact_output(source, "42");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_async_in_function_with_zone() {
    assert_exact_output(
        r#"## To process_in_zone -> Int:
    Sleep 10.
    Inside a zone called "Work":
        Let x be 42.
    Return 42.

## Main
    Let result be process_in_zone().
    Show result.
"#,
        "42",
    );
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
    assert_exact_output(source, "42");
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
    assert_exact_output(source, "3");
}

// =============================================================================
// Category K: Parallel + Features
// =============================================================================

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_parallel_with_structs() {
    assert_output_lines(
        r#"## A Data has:
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
"#,
        &["10", "20"],
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_parallel_with_collections() {
    assert_output_lines(
        r#"## To sum (items: Seq of Int) -> Int:
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
"#,
        &["6", "15"],
    );
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
    assert_exact_output(source, "5");
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
    assert_exact_output(source, "10");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_concurrent_with_different_return_types() {
    assert_output_lines(
        r#"## To get_int -> Int:
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
"#,
        &["42", "hello"],
    );
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
    assert_exact_output(source, "99");
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_concurrent_struct_creation() {
    assert_output_lines(
        r#"## A Point has:
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
"#,
        &["10", "20"],
    );
}

// =============================================================================
// Async coloring: every expression and nested block reaches the callee
// =============================================================================

/// The declarations every coloring program starts from: a sleeping `nap` and `naps`, a record and a variant
/// to build around them.
#[cfg(not(target_arch = "wasm32"))]
const SLEEPERS: &str = r#"## A Box has:
    a v: Int.

## A Wrap is one of:
    A W with n Int.

## To nap (n: Int) -> Int:
    Sleep 1.
    Return n.

## To naps (n: Int) -> Seq of Int:
    Sleep 1.
    Return [n, n + 1, n + 2].
"#;

/// `body` after [`SLEEPERS`] prints `expected` on the tree-walker, on the VM under its scheduler and on the compiled
/// tier, whose function `f` awaits the sleeper it reaches only through the position under test.
#[cfg(not(target_arch = "wasm32"))]
fn assert_colored(body: &str, expected: &str) {
    let source = format!("{SLEEPERS}\n{body}");
    for (tier, run) in [
        ("the tree-walker", logicaffeine_compile::compile::tw_outcome(&source)),
        ("the scheduled VM", logicaffeine_compile::compile::vm_outcome_concurrent(&source)),
    ] {
        assert_eq!(run.error, None, "{tier} refused:\n{source}");
        assert_eq!(run.output.trim(), expected, "{tier}:\n{source}");
    }
    assert_exact_output(&source, expected);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_sleeping_call_in_a_record_field_colors_the_caller() {
    assert_colored(
        "## To f (n: Int) -> Box:\n    Return a new Box with v nap(n).\n\n## Main\nShow f(4)'s v.\n",
        "4",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_sleeping_call_in_a_variant_field_colors_the_caller() {
    assert_colored(
        "## To f (n: Int) -> Wrap:\n    Return a new W with n nap(n).\n\n## Main\nInspect f(3):\n    When W (k):\n        Show k.\n",
        "3",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_sleeping_call_under_not_colors_the_caller() {
    assert_colored(
        "## To f (n: Int) -> Bool:\n    Return not (nap(n) equals 0).\n\n## Main\nShow f(5).\n",
        "true",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_sleeping_call_set_into_a_field_colors_the_caller() {
    assert_colored(
        "## To f (n: Int) -> Int:\n    Let b be a new Box with v 1.\n    Set b's v to nap(n).\n    Return b's v.\n\n## Main\nShow f(7).\n",
        "7",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_sleeping_calls_pushed_together_color_the_caller() {
    assert_colored(
        "## To f (n: Int) -> Seq of Int:\n    Let mutable xs be a new Seq of Int.\n    Push nap(n), nap(n + 1) to xs.\n    Return xs.\n\n## Main\nShow f(2).\n",
        "[2, 3]",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_sleeping_call_in_a_reversible_block_colors_the_caller() {
    assert_colored(
        "## To f (n: Int) -> Int:\n    Let mutable x be 1.\n    Reversibly:\n        Set x to x + nap(n).\n    Return x.\n\n## Main\nShow f(4).\n",
        "5",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_sleep_in_a_reversible_block_colors_the_function() {
    assert_colored(
        "## To f (n: Int) -> Int:\n    Reversibly:\n        Sleep 1.\n    Return n.\n\n## Main\nShow f(4).\n",
        "4",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_sleeping_call_wrapped_in_some_colors_the_caller() {
    assert_colored(
        "## To f (n: Int) -> Option of Int:\n    Return some nap(n).\n\n## Main\nShow f(6).\n",
        "6",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_sleeping_call_as_a_range_bound_colors_the_caller() {
    assert_colored(
        "## To f (n: Int) -> Int:\n    Let mutable t be 0.\n    Repeat for i from 1 to nap(n):\n        Set t to t + i.\n    Return t.\n\n## Main\nShow f(4).\n",
        "10",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_sleeping_call_under_length_colors_the_caller() {
    assert_colored(
        "## To f (n: Int) -> Int:\n    Return length of naps(n).\n\n## Main\nShow f(1).\n",
        "3",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_sleeping_call_under_contains_colors_the_caller() {
    assert_colored(
        "## To f (n: Int) -> Bool:\n    Return naps(n) contains 2.\n\n## Main\nShow f(1).\n",
        "true",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_sleeping_call_as_a_slice_bound_colors_the_caller() {
    assert_colored(
        "## To f (n: Int) -> Int:\n    Let xs be [5, 6, 7, 8].\n    Return length of (items 2 through (nap(n)) of xs).\n\n## Main\nShow f(3).\n",
        "2",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_sleeping_call_under_copy_colors_the_caller() {
    assert_colored(
        "## To f (n: Int) -> Seq of Int:\n    Return copy of naps(n).\n\n## Main\nShow f(1).\n",
        "[1, 2, 3]",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_sleeping_call_added_to_a_set_colors_the_caller() {
    assert_colored(
        "## To f (n: Int) -> Int:\n    Let mutable s be a new Set of Int.\n    Add nap(n) to s.\n    Add nap(n + 1) to s.\n    Return length of s.\n\n## Main\nShow f(1).\n",
        "2",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_sleeping_call_removed_from_a_set_colors_the_caller() {
    assert_colored(
        "## To f (n: Int) -> Int:\n    Let mutable s be a new Set of Int.\n    Add 1 to s.\n    Add 2 to s.\n    Remove nap(n) from s.\n    Return length of s.\n\n## Main\nShow f(1).\n",
        "1",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_sleeping_call_in_a_set_union_colors_the_caller() {
    assert_colored(
        "## To f (n: Int) -> Int:\n    Let mutable s be a new Set of Int.\n    Add n to s.\n    Let mutable t be a new Set of Int.\n    Add nap(n + 1) to t.\n    Return length of (s union t).\n\n## Main\nShow f(1).\n",
        "2",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_sleeping_call_as_a_capacity_colors_the_caller() {
    assert_colored(
        "## To f (n: Int) -> Int:\n    Let mutable xs be a new Seq of Int with capacity nap(n).\n    Push n to xs.\n    Return length of xs.\n\n## Main\nShow f(8).\n",
        "1",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_sleeping_call_in_a_failure_reason_colors_the_caller() {
    assert_colored(
        "## To f (n: Int) -> Int:\n    If n equals 0:\n        Fail with \"bad {nap(n)}\".\n    Return n.\n\n## Main\nShow f(2).\n",
        "2",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_sleeping_call_in_a_list_and_a_text_colors_the_caller() {
    assert_colored(
        "## To f (n: Int) -> Seq of Int:\n    Return [nap(n), nap(n + 1)].\n\n## To g (n: Int) -> Text:\n    Return \"got {nap(n)}\".\n\n## Main\nShow f(1).\nShow g(2).\n",
        "[1, 2]\ngot 2",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_sleeping_call_in_a_loop_condition_colors_the_caller() {
    assert_colored(
        "## To f (n: Int) -> Int:\n    Let mutable i be 0.\n    While i is less than nap(n):\n        Set i to i + 1.\n    Return i.\n\n## Main\nShow f(3).\n",
        "3",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_coloring_reaches_a_caller_through_every_new_position() {
    assert_colored(
        "## To inner (n: Int) -> Box:\n    Return a new Box with v nap(n).\n\n## To middle (n: Int) -> Bool:\n    Return not (inner(n)'s v equals 0).\n\n## To outer (n: Int) -> Seq of Bool:\n    Return copy of [middle(n), middle(0)].\n\n## Main\nShow outer(2).\n",
        "[true, false]",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_a_loop_that_awaits_keeps_the_map_it_walks() {
    assert_colored(
        "## To f (m: Map of Text to Int) -> Int:\n    Let mutable t be 0.\n    Repeat for (k, v) in m:\n        Set t to t + nap(v).\n    Return t + length of m.\n\n## Main\nLet mutable m be a new Map of Text to Int.\nSet item \"a\" of m to 5.\nSet item \"b\" of m to 7.\nShow f(m).\nShow length of m.\n",
        "14\n2",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_a_loop_that_awaits_keeps_the_list_it_walks() {
    assert_colored(
        "## To f (n: Int) -> Int:\n    Let xs be [n, n + 1, n + 2].\n    Let mutable t be 0.\n    Repeat for x in xs:\n        Set t to t + nap(x).\n    Repeat for y in xs:\n        Set t to t + y.\n    Return t + length of xs.\n\n## Main\nShow f(1).\n",
        "15",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_a_loop_that_awaits_walks_the_characters_of_a_text() {
    assert_colored(
        "## To f (n: Int) -> Int:\n    Let s be \"{n}:{n}\".\n    Let mutable t be 0.\n    Repeat for c in s:\n        If c equals \":\":\n            Set t to t + nap(10).\n    Return t + length of s.\n\n## Main\nShow f(42).\n",
        "15",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_a_sleeping_function_calls_itself() {
    assert_colored(
        "## To f (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    Sleep 1.\n    Return f(n - 1) + 1.\n\n## Main\nShow f(3).\n",
        "3",
    );
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn e2e_two_functions_call_each_other_through_a_sleep() {
    assert_colored(
        "## To ping (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    Return pong(n - 1) + 1.\n\n## To pong (n: Int) -> Int:\n    Return ping(n) + nap(0).\n\n## Main\nShow ping(4).\nShow pong(2).\n",
        "4\n2",
    );
}
