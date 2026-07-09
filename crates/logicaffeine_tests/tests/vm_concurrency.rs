//! Phase 5b (work/FINISH_INTERPRETER.md) — concurrency on the bytecode VM tier.
//!
//! These run Go-like concurrency programs through `run_vm_concurrent`, which
//! compiles to the T10 opcodes and drives the resumable VM (`run_until_block`)
//! under the deterministic scheduler via a `VmTask` — the VM analog of the
//! tree-walker's scheduler path. Output is asserted against the known result and
//! must match what `interp_concurrency.rs` asserts for the same program.

use logicaffeine_compile::run_vm_concurrent;

#[test]
fn vm_pipe_producer_consumer() {
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
    let result = run_vm_concurrent(src);
    assert!(result.error.is_none(), "unexpected error: {:?}", result.error);
    assert_eq!(
        result.lines,
        vec!["1".to_string(), "2".to_string()],
        "producer/consumer should receive values in FIFO order on the VM tier"
    );
}

#[test]
fn vm_launch_fire_and_forget() {
    let src = "## To worker:\n\
        \x20   Show \"worker ran\".\n\
        \n\
        ## Main\n\
        \x20   Launch a task to worker.\n\
        \x20   Show \"main ran\".\n";
    let result = run_vm_concurrent(src);
    assert!(result.error.is_none(), "unexpected error: {:?}", result.error);
    let joined = result.lines.join("\n");
    assert!(joined.contains("worker ran"), "worker output missing: {:?}", result.lines);
    assert!(joined.contains("main ran"), "main output missing: {:?}", result.lines);
}

#[test]
fn vm_try_receive_empty_is_nothing() {
    // Non-blocking receive on an empty channel binds Nothing (never blocks) on
    // the VM's ChanTryRecv op.
    let src = "## Main\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Try to receive x from ch.\n\
        \x20   Show x.\n";
    let result = run_vm_concurrent(src);
    assert!(result.error.is_none(), "unexpected error: {:?}", result.error);
    assert_eq!(result.lines, vec!["nothing".to_string()], "try-receive on empty yields Nothing");
}

#[test]
fn vm_stop_task_aborts() {
    let src = "## To infinite:\n\
        \x20   Let x be 0.\n\
        \x20   Show \"started\".\n\
        \n\
        ## Main\n\
        \x20   Let handle be Launch a task to infinite.\n\
        \x20   Stop handle.\n\
        \x20   Show \"stopped\".\n";
    let result = run_vm_concurrent(src);
    assert!(result.error.is_none(), "unexpected error: {:?}", result.error);
    assert!(
        result.lines.contains(&"stopped".to_string()),
        "main reaches its Stop + Show on the VM: {:?}",
        result.lines
    );
}

#[test]
fn vm_hot_loop_inside_task_tiers_and_is_correct() {
    // A hot integer loop INSIDE a spawned task must run at full tier (it
    // JIT-compiles exactly like the main program) and produce the right result.
    // The concurrency op (Send) is JIT-ineligible, so the tiered region is
    // yield-free and the task only suspends on the bytecode path. (Phase 6 seam.)
    let src = "## To compute (ch: Int):\n\
        \x20   Let total be 0.\n\
        \x20   Let i be 0.\n\
        \x20   While i is less than 100000:\n\
        \x20       Set total to total + i.\n\
        \x20       Set i to i + 1.\n\
        \x20   Send total into ch.\n\
        \n\
        ## Main\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Launch a task to compute with ch.\n\
        \x20   Receive result from ch.\n\
        \x20   Show result.\n";
    let result = run_vm_concurrent(src);
    assert!(result.error.is_none(), "unexpected error: {:?}", result.error);
    assert_eq!(
        result.lines,
        vec!["4999950000".to_string()],
        "the hot loop inside the task computed sum(0..99999) correctly under tiering"
    );
}

#[test]
fn vm_loop_containing_channel_op_runs_correctly() {
    // A loop whose body contains a concurrency op (ChanSend / ChanRecv) is NOT
    // integer-only, so the JIT region detector never selects it — it runs on the
    // bytecode path, suspending at each op. Correct output proves the op was
    // never miscompiled into a tiered region (Phase 6 deny-by-construction).
    let src = "## To produce (ch: Int):\n\
        \x20   Let i be 1.\n\
        \x20   While i is less than 6:\n\
        \x20       Send i into ch.\n\
        \x20       Set i to i + 1.\n\
        \n\
        ## Main\n\
        \x20   Let jobs be a Pipe of Int.\n\
        \x20   Launch a task to produce with jobs.\n\
        \x20   Let total be 0.\n\
        \x20   Let k be 0.\n\
        \x20   While k is less than 5:\n\
        \x20       Receive v from jobs.\n\
        \x20       Set total to total + v.\n\
        \x20       Set k to k + 1.\n\
        \x20   Show total.\n";
    let result = run_vm_concurrent(src);
    assert!(result.error.is_none(), "unexpected error: {:?}", result.error);
    assert_eq!(result.lines, vec!["15".to_string()], "summed 1..5 received in a loop");
}

#[test]
fn vm_select_timeout_only() {
    // No recv arm can be ready, so the VM's Select fires the timeout arm.
    let src = "## Main\n\
        \x20   Let p be a Pipe of Text.\n\
        \x20   Await the first of:\n\
        \x20       After 1 seconds:\n\
        \x20           Show \"timeout\".\n";
    let result = run_vm_concurrent(src);
    assert!(result.error.is_none(), "unexpected error: {:?}", result.error);
    assert_eq!(result.lines, vec!["timeout".to_string()], "the timeout arm fires on the VM");
}

#[test]
fn vm_select_receive_wins_over_timeout() {
    // A producer makes the recv arm ready before the timeout — the received
    // value is shown via the winning branch on the VM.
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
    let result = run_vm_concurrent(src);
    assert!(result.error.is_none(), "unexpected error: {:?}", result.error);
    assert_eq!(result.lines, vec!["7".to_string()], "the ready recv arm wins on the VM");
}

#[test]
fn vm_concurrent_block_runs_all() {
    // `Attempt all of the following:` (Concurrent) sequentializes on the VM and
    // runs every branch.
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
    let result = run_vm_concurrent(src);
    assert!(result.error.is_none(), "unexpected error: {:?}", result.error);
    assert_eq!(result.lines, vec!["10".to_string(), "20".to_string()], "both concurrent branches ran");
}

#[test]
fn vm_parallel_block_runs_all() {
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
    let result = run_vm_concurrent(src);
    assert!(result.error.is_none(), "unexpected error: {:?}", result.error);
    assert_eq!(result.lines, vec!["30".to_string(), "40".to_string()], "both parallel branches ran");
}

#[test]
fn vm_bounded_pipe_backpressure() {
    // The default channel capacity is 32; sending 40 values forces the producer
    // to block when the buffer is full and resume as the consumer drains — the
    // sum proves all 40 arrived in FIFO order under backpressure.
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
    let result = run_vm_concurrent(src);
    assert!(result.error.is_none(), "unexpected error: {:?}", result.error);
    assert_eq!(result.lines, vec!["820".to_string()], "sum(1..40) delivered under backpressure");
}

#[test]
fn vm_deadlock_detected() {
    // A receive with no possible sender leaves every task blocked — the VM
    // scheduler reports the deadlock deterministically.
    let src = "## Main\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Receive x from ch.\n\
        \x20   Show x.\n";
    let result = run_vm_concurrent(src);
    let err = result.error.expect("a no-sender receive must deadlock on the VM");
    assert!(err.contains("deadlock"), "deadlock is reported: {err}");
    assert!(result.lines.is_empty(), "the unreachable Show never ran: {:?}", result.lines);
}

/// A `Sleep` inside a VM task must route through the scheduler's virtual timer, not block
/// the worker on a real host sleep (which also errors outright on wasm). With a 2000-unit
/// sleep, a virtual timer completes instantly; a real `std::thread::sleep` would take ~2s.
/// The generous threshold proves the sleep is virtual without being timing-flaky.
#[test]
fn vm_task_sleep_uses_virtual_timer() {
    let src = "## To worker (ch: Int):\n\
        \x20   Sleep 2000.\n\
        \x20   Send 42 into ch.\n\
        \n\
        ## Main\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Launch a task to worker with ch.\n\
        \x20   Receive x from ch.\n\
        \x20   Show x.\n";
    let start = std::time::Instant::now();
    let result = run_vm_concurrent(src);
    let elapsed = start.elapsed();
    assert!(result.error.is_none(), "unexpected error: {:?}", result.error);
    assert_eq!(result.lines, vec!["42".to_string()]);
    assert!(
        elapsed < std::time::Duration::from_millis(500),
        "VM task Sleep must use the virtual timer (instant), took {elapsed:?} — \
         it is blocking on a real host sleep instead of yielding to the scheduler"
    );
}
