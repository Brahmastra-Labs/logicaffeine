//! Phase 4 (FINISH_INTERPRETER.md) — Send/escape analysis tests.
//!
//! The concurrency memory model is message-passing + CRDT: tasks have isolated
//! heaps; the only cross-task sharing is channels (move) and CRDT cells. This
//! first increment rejects `Simultaneously`/`Attempt all` blocks whose branches
//! share mutable state (a data race once they run in parallel).

use futures::executor::block_on;
use logicaffeine_compile::{interpret_for_ui, send_check_source};

#[test]
fn send_check_accepts_message_passing() {
    let src = "## To produce (ch: Int):\n\
        \x20   Send 1 into ch.\n\
        \n\
        ## Main\n\
        \x20   Let jobs be a Pipe of Int.\n\
        \x20   Launch a task to produce with jobs.\n\
        \x20   Receive x from jobs.\n\
        \x20   Show x.\n";
    let diags = send_check_source(src).expect("parse");
    assert!(diags.is_empty(), "a message-passing program is accepted, got {:?}", diags);
}

#[test]
fn send_check_allows_parallel_independent() {
    let src = "## Main\n\
        \x20   Simultaneously:\n\
        \x20       Let a be 100.\n\
        \x20       Let b be 200.\n\
        \x20   Show a.\n\
        \x20   Show b.\n";
    let diags = send_check_source(src).expect("parse");
    assert!(diags.is_empty(), "independent parallel branches are accepted, got {:?}", diags);
}

#[test]
fn send_check_allows_parallel_pipe_communication() {
    // Sharing a Pipe across parallel branches is the sanctioned CSP mechanism
    // (a channel is safe under concurrent access), not a data race — accepted.
    let src = "## Main\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Simultaneously:\n\
        \x20       Send 1 into ch.\n\
        \x20       Receive x from ch.\n\
        \x20   Show x.\n";
    let diags = send_check_source(src).expect("parse");
    assert!(diags.is_empty(), "pipe communication across branches is accepted, got {:?}", diags);
}

#[test]
fn send_check_allows_crdt_shared_cell() {
    // A CRDT (Shared) counter incremented from two parallel branches is the
    // sanctioned shared-state mechanism: the increments commute, so it is *not* a
    // data race. (CRDT mutations carry no plain-variable write, so the data-race
    // predicate correctly never flags them.)
    let src = "## Definition\n\
        A Counter is Shared and has:\n\
        \x20   points: ConvergentCount.\n\
        \n\
        ## Main\n\
        Let mutable c be a new Counter.\n\
        Simultaneously:\n\
        \x20   Increase c's points by 1.\n\
        \x20   Increase c's points by 2.\n\
        Show c's points.\n";
    let diags = send_check_source(src).expect("parse");
    assert!(diags.is_empty(), "CRDT shared cell across branches is accepted, got {:?}", diags);
}

#[test]
fn send_check_rejects_parallel_shared_mutable() {
    let src = "## Main\n\
        \x20   Let total be 0.\n\
        \x20   Simultaneously:\n\
        \x20       Set total to total + 1.\n\
        \x20       Set total to total + 2.\n";
    let diags = send_check_source(src).expect("parse");
    assert!(!diags.is_empty(), "parallel branches mutating a shared variable must be rejected");
    assert!(
        diags[0].message.contains("Pipe") || diags[0].message.contains("CRDT"),
        "diagnostic should suggest the fix, got: {}",
        diags[0].message
    );
}

#[test]
fn send_check_plain_program_is_clean() {
    let src = "## Main\n\
        \x20   Let x be 1 + 2.\n\
        \x20   Show x.\n";
    let diags = send_check_source(src).expect("parse");
    assert!(diags.is_empty());
}

// ---- run-path gate: the analysis must reject before execution ----------------

#[test]
fn run_path_rejects_shared_mutable_parallel() {
    // `interpret_for_ui` must refuse to run a racy program, surfacing the
    // diagnostic rather than executing the branches.
    let src = "## Main\n\
        \x20   Let total be 0.\n\
        \x20   Simultaneously:\n\
        \x20       Set total to total + 1.\n\
        \x20       Set total to total + 2.\n\
        \x20   Show total.\n";
    let result = block_on(interpret_for_ui(src));
    let err = result.error.expect("a racy parallel program must be rejected before running");
    assert!(err.contains("share mutable state"), "send-check message: {err}");
    assert!(result.lines.is_empty(), "nothing runs when rejected: {:?}", result.lines);
}

#[test]
fn run_path_rejects_shared_mutable_concurrent() {
    let src = "## Main\n\
        \x20   Let total be 0.\n\
        \x20   Attempt all of the following:\n\
        \x20       Set total to total + 1.\n\
        \x20       Set total to total + 2.\n\
        \x20   Show total.\n";
    let result = block_on(interpret_for_ui(src));
    let err = result.error.expect("a racy concurrent program must be rejected before running");
    assert!(err.contains("share mutable state"), "send-check message: {err}");
}

#[test]
fn run_path_accepts_independent_parallel() {
    // Distinct variables — no shared mutable state — runs to completion.
    let src = "## Main\n\
        \x20   Let a be 0.\n\
        \x20   Let b be 0.\n\
        \x20   Simultaneously:\n\
        \x20       Set a to 1.\n\
        \x20       Set b to 2.\n\
        \x20   Show a.\n\
        \x20   Show b.\n";
    let result = block_on(interpret_for_ui(src));
    assert!(result.error.is_none(), "independent branches must run: {:?}", result.error);
    assert_eq!(result.lines, vec!["1".to_string(), "2".to_string()]);
}

#[test]
fn run_path_accepts_parallel_pipe_communication() {
    // The run-path gate must let a pipe-communicating parallel block through.
    let src = "## Main\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Simultaneously:\n\
        \x20       Send 1 into ch.\n\
        \x20       Receive x from ch.\n\
        \x20   Show x.\n";
    let result = block_on(interpret_for_ui(src));
    assert!(result.error.is_none(), "pipe communication must be accepted: {:?}", result.error);
    assert_eq!(result.lines, vec!["1".to_string()]);
}

#[test]
fn run_path_accepts_message_passing() {
    // Channels move data between tasks — never a violation.
    let src = "## To produce (ch: Int):\n\
        \x20   Send 1 into ch.\n\
        \n\
        ## Main\n\
        \x20   Let jobs be a Pipe of Int.\n\
        \x20   Launch a task to produce with jobs.\n\
        \x20   Receive x from jobs.\n\
        \x20   Show x.\n";
    let result = block_on(interpret_for_ui(src));
    assert!(result.error.is_none(), "message passing must be accepted: {:?}", result.error);
    assert_eq!(result.lines, vec!["1".to_string()]);
}
