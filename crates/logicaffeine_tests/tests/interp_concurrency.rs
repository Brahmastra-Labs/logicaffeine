//! Phase 3 (FINISH_INTERPRETER.md) — interpreted concurrency.
//!
//! These run Go-like concurrency programs on the *tree-walker* (via the
//! deterministic scheduler), which previously rejected them as "compiled mode
//! only". Output is asserted against the known result.

use futures::executor::block_on;
use logicaffeine_compile::interpret_for_ui;

#[test]
fn interp_launch_fire_and_forget() {
    let src = "## To worker:\n\
        \x20   Show \"worker ran\".\n\
        \n\
        ## Main\n\
        \x20   Launch a task to worker.\n\
        \x20   Show \"main ran\".\n";
    let result = block_on(interpret_for_ui(src));
    assert!(result.error.is_none(), "unexpected error: {:?}", result.error);
    let joined = result.lines.join("\n");
    assert!(joined.contains("worker ran"), "worker output missing: {:?}", result.lines);
    assert!(joined.contains("main ran"), "main output missing: {:?}", result.lines);
}

#[test]
fn interp_pipe_producer_consumer() {
    let src = "## To produce (ch: Int):\n\
        \x20   Send 1 into ch.\n\
        \x20   Send 2 into ch.\n\
        \n\
        ## Main\n\
        \x20   Let jobs be a Pipe of Int.\n\
        \x20   Launch a task to produce with jobs.\n\
        \x20   Receive first from jobs.\n\
        \x20   Receive second from jobs.\n\
        \x20   Show first.\n\
        \x20   Show second.\n";
    let result = block_on(interpret_for_ui(src));
    assert!(result.error.is_none(), "unexpected error: {:?}", result.error);
    assert_eq!(
        result.lines,
        vec!["1".to_string(), "2".to_string()],
        "producer/consumer should receive values in FIFO order"
    );
}

#[test]
fn interp_try_send_nonblocking() {
    let src = "## Main\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Try to send 99 into ch.\n\
        \x20   Show \"sent\".\n";
    let result = block_on(interpret_for_ui(src));
    assert!(result.error.is_none(), "unexpected error: {:?}", result.error);
    assert_eq!(result.lines, vec!["sent".to_string()], "try-send never blocks the sender");
}

#[test]
fn interp_try_receive_empty_is_nothing() {
    // A try-receive on an empty channel binds Nothing (never blocks).
    let src = "## Main\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Try to receive x from ch.\n\
        \x20   Show x.\n";
    let result = block_on(interpret_for_ui(src));
    assert!(result.error.is_none(), "unexpected error: {:?}", result.error);
    assert_eq!(result.lines, vec!["nothing".to_string()], "try-receive on empty yields Nothing");
}

#[test]
fn interp_select_timeout_only() {
    // No recv arm can ever be ready, so the timeout arm fires.
    let src = "## Main\n\
        \x20   Let p be a Pipe of Text.\n\
        \x20   Await the first of:\n\
        \x20       After 1 seconds:\n\
        \x20           Show \"timeout\".\n";
    let result = block_on(interpret_for_ui(src));
    assert!(result.error.is_none(), "unexpected error: {:?}", result.error);
    assert_eq!(result.lines, vec!["timeout".to_string()], "the timeout arm fires when no recv is ready");
}

#[test]
fn interp_select_receive_wins_over_timeout() {
    // A producer makes the recv arm ready before the timeout can fire, so the
    // received value is shown rather than the timeout.
    let src = "## To produce (ch: Int):\n\
        \x20   Send 7 into ch.\n\
        \n\
        ## Main\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Launch a task to produce with ch.\n\
        \x20   Await the first of:\n\
        \x20       Receive x from ch:\n\
        \x20           Show x.\n\
        \x20       After 1 seconds:\n\
        \x20           Show \"timeout\".\n";
    let result = block_on(interpret_for_ui(src));
    assert!(result.error.is_none(), "unexpected error: {:?}", result.error);
    assert_eq!(result.lines, vec!["7".to_string()], "the ready recv arm wins over the timeout");
}

#[test]
fn interp_stop_task_aborts() {
    let src = "## To infinite:\n\
        \x20   Let x be 0.\n\
        \x20   Show \"started\".\n\
        \n\
        ## Main\n\
        \x20   Let handle be Launch a task to infinite.\n\
        \x20   Stop handle.\n\
        \x20   Show \"stopped\".\n";
    let result = block_on(interpret_for_ui(src));
    assert!(result.error.is_none(), "unexpected error: {:?}", result.error);
    assert!(
        result.lines.contains(&"stopped".to_string()),
        "main reaches its Stop + Show: {:?}",
        result.lines
    );
}

#[test]
fn interp_concurrent_block_runs_all() {
    // `Attempt all of the following:` (Concurrent) runs every branch; each sends
    // into its own independent pipe so both arrive.
    let src = "## Main\n\
        \x20   Let cha be a Pipe of Int.\n\
        \x20   Let chb be a Pipe of Int.\n\
        \x20   Attempt all of the following:\n\
        \x20       Send 10 into cha.\n\
        \x20       Send 20 into chb.\n\
        \x20   Receive a from cha.\n\
        \x20   Receive b from chb.\n\
        \x20   Show a.\n\
        \x20   Show b.\n";
    let result = block_on(interpret_for_ui(src));
    assert!(result.error.is_none(), "unexpected error: {:?}", result.error);
    assert_eq!(result.lines, vec!["10".to_string(), "20".to_string()], "both concurrent branches ran");
}

#[test]
fn interp_parallel_block_runs_all() {
    // `Simultaneously:` (Parallel) runs every branch.
    let src = "## Main\n\
        \x20   Let cha be a Pipe of Int.\n\
        \x20   Let chb be a Pipe of Int.\n\
        \x20   Simultaneously:\n\
        \x20       Send 30 into cha.\n\
        \x20       Send 40 into chb.\n\
        \x20   Receive a from cha.\n\
        \x20   Receive b from chb.\n\
        \x20   Show a.\n\
        \x20   Show b.\n";
    let result = block_on(interpret_for_ui(src));
    assert!(result.error.is_none(), "unexpected error: {:?}", result.error);
    assert_eq!(result.lines, vec!["30".to_string(), "40".to_string()], "both parallel branches ran");
}

#[test]
fn interp_bounded_pipe_backpressure() {
    // Default channel capacity is 32; a 40-value producer must block when the
    // buffer fills and resume as the consumer drains — order preserved.
    let src = "## To produce (ch: Int):\n\
        \x20   Let i be 1.\n\
        \x20   While i is less than 41:\n\
        \x20       Send i into ch.\n\
        \x20       Set i to i + 1.\n\
        \n\
        ## Main\n\
        \x20   Let jobs be a Pipe of Int.\n\
        \x20   Launch a task to produce with jobs.\n\
        \x20   Let total be 0.\n\
        \x20   Let k be 0.\n\
        \x20   While k is less than 40:\n\
        \x20       Receive v from jobs.\n\
        \x20       Set total to total + v.\n\
        \x20       Set k to k + 1.\n\
        \x20   Show total.\n";
    let result = block_on(interpret_for_ui(src));
    assert!(result.error.is_none(), "unexpected error: {:?}", result.error);
    assert_eq!(result.lines, vec!["820".to_string()], "sum(1..40) delivered under backpressure");
}

#[test]
fn interp_deadlock_detected() {
    // A receive with no possible sender leaves every task blocked — a deadlock,
    // which the scheduler reports deterministically.
    let src = "## Main\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Receive x from ch.\n\
        \x20   Show x.\n";
    let result = block_on(interpret_for_ui(src));
    let err = result.error.expect("a no-sender receive must deadlock");
    assert!(err.contains("deadlock"), "deadlock is reported: {err}");
    assert!(result.lines.is_empty(), "the unreachable Show never ran: {:?}", result.lines);
}

#[test]
fn interp_seed_determinism() {
    // The seeded scheduler reproduces the same output on repeated runs.
    let src = "## To produce (ch: Int):\n\
        \x20   Send 1 into ch.\n\
        \x20   Send 2 into ch.\n\
        \x20   Send 3 into ch.\n\
        \n\
        ## Main\n\
        \x20   Let jobs be a Pipe of Int.\n\
        \x20   Launch a task to produce with jobs.\n\
        \x20   Receive a from jobs.\n\
        \x20   Receive b from jobs.\n\
        \x20   Receive c from jobs.\n\
        \x20   Show a.\n\
        \x20   Show b.\n\
        \x20   Show c.\n";
    let first = block_on(interpret_for_ui(src));
    let second = block_on(interpret_for_ui(src));
    assert!(first.error.is_none(), "unexpected error: {:?}", first.error);
    assert_eq!(first.lines, second.lines, "the same seed reproduces the same interleaving");
    assert_eq!(first.lines, vec!["1".to_string(), "2".to_string(), "3".to_string()], "FIFO order");
}
