//! Phase 9 (work/FINISH_INTERPRETER.md) — the browser cooperative drive loop.
//!
//! Concurrent programs in the Studio run on the deterministic scheduler driven in *slices*
//! that yield a macrotask between them (so Dioxus repaints instead of freezing), emitting a
//! `SchedSnapshot` to an observer for the Tasks/Channels strip. These native tests prove the
//! slice-driven path produces the same output as run-to-quiescence, and that the observer
//! fires for a program that outlives one slice.

use std::cell::RefCell;
use std::rc::Rc;

use futures::executor::block_on;
use logicaffeine_compile::{
    interpret_streaming, interpret_streaming_with_vfs_observer, ObserverCallback,
};

/// A no-op streaming sink — these tests assert on `result.lines`, not the live callback.
fn sink() -> Rc<RefCell<impl FnMut(String)>> {
    Rc::new(RefCell::new(|_line: String| {}))
}

#[test]
fn streaming_producer_consumer_output_correct() {
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
    let result = block_on(interpret_streaming(src, sink()));
    assert!(result.error.is_none(), "unexpected error: {:?}", result.error);
    assert_eq!(result.lines, vec!["1".to_string(), "2".to_string()]);
}

#[test]
fn streaming_observer_fires_on_long_program() {
    // A producer/consumer that runs well past one slice budget (default pipe capacity forces
    // backpressure interleaving), so `run_slice` returns `None` at least once and the
    // observer is invoked with a live snapshot.
    let src = "## To produce (ch: Int):\n\
        \x20   Let i be 200.\n\
        \x20   While i is greater than 0:\n\
        \x20       Send i into ch.\n\
        \x20       Set i to i - 1.\n\
        \n\
        ## Main\n\
        \x20   Let jobs be a Pipe of Int.\n\
        \x20   Launch a task to produce with jobs.\n\
        \x20   Let total be 0.\n\
        \x20   Let j be 200.\n\
        \x20   While j is greater than 0:\n\
        \x20       Receive x from jobs.\n\
        \x20       Set total to total + x.\n\
        \x20       Set j to j - 1.\n\
        \x20   Show total.\n";

    let count = Rc::new(RefCell::new(0usize));
    let c = count.clone();
    let observer: ObserverCallback = Rc::new(RefCell::new(move |_snap| {
        *c.borrow_mut() += 1;
    }));

    let result = block_on(interpret_streaming_with_vfs_observer(src, sink(), None, observer));
    assert!(result.error.is_none(), "unexpected error: {:?}", result.error);
    assert_eq!(result.lines, vec!["20100".to_string()], "sum of 1..=200");
    assert!(
        *count.borrow() >= 1,
        "the observer must fire at least once for a program that outlives one slice"
    );
}

/// A concurrent program that *also* sleeps must run on the scheduler without panicking.
/// Previously the async `Sleep` blocked on a raw host timer, suspending the task with no
/// scheduler request — the cooperative driver then panicked (and under `block_on` there is
/// no tokio reactor at all). The sleep must route through a scheduler timer instead.
#[test]
fn streaming_concurrent_with_sleep_runs() {
    let src = "## To worker (ch: Int):\n\
        \x20   Sleep 1.\n\
        \x20   Send 42 into ch.\n\
        \n\
        ## Main\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Launch a task to worker with ch.\n\
        \x20   Receive x from ch.\n\
        \x20   Show x.\n";
    let result = block_on(interpret_streaming(src, sink()));
    assert!(result.error.is_none(), "unexpected error: {:?}", result.error);
    assert_eq!(result.lines, vec!["42".to_string()]);
}
