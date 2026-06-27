//! Phase 9 (FINISH_INTERPRETER.md) — concurrency runs in the browser (wasm32), headless.
//!
//! Drives the cooperative scheduler through `interpret_streaming` on the `wasm32` target:
//! the same slice-driven, macrotask-yielding loop the Studio uses. This proves the
//! concurrency path compiles and runs under wasm (not just native), with output identical
//! to the tree-walker. Runs under node via `wasm-bindgen-test-runner` — no browser needed,
//! no relay (networking variants live in `wasm_interp_net` / the relay scripts).
//!
//! Only builds on `wasm32`; inert on a normal `cargo test`.

#![cfg(target_arch = "wasm32")]

use std::cell::RefCell;
use std::rc::Rc;

use logicaffeine_compile::interpret_streaming;
use wasm_bindgen_test::*;

/// A 3-stage producer→pipe→consumer program runs concurrently in wasm and streams every
/// line. The drive loop advances the scheduler in slices and yields a macrotask between
/// them (so a real browser repaints); for this small program it completes promptly, and
/// the output must match exactly and in order.
#[wasm_bindgen_test]
async fn wasm_three_task_pipeline_streams() {
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
    let sink = Rc::new(RefCell::new(|_line: String| {}));
    let result = interpret_streaming(src, sink).await;
    assert!(result.error.is_none(), "unexpected error: {:?}", result.error);
    assert_eq!(
        result.lines,
        vec!["1".to_string(), "2".to_string(), "3".to_string()],
    );
}

/// A concurrent program that also sleeps must run on the scheduler under wasm without
/// blocking on a raw host timer — the sleep routes through a scheduler timer.
#[wasm_bindgen_test]
async fn wasm_concurrent_with_sleep_runs() {
    let src = "## To worker (ch: Int):\n\
        \x20   Sleep 1.\n\
        \x20   Send 42 into ch.\n\
        \n\
        ## Main\n\
        \x20   Let ch be a Pipe of Int.\n\
        \x20   Launch a task to worker with ch.\n\
        \x20   Receive x from ch.\n\
        \x20   Show x.\n";
    let sink = Rc::new(RefCell::new(|_line: String| {}));
    let result = interpret_streaming(src, sink).await;
    assert!(result.error.is_none(), "unexpected error: {:?}", result.error);
    assert_eq!(result.lines, vec!["42".to_string()]);
}
