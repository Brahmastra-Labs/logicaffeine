//! Phase 1a (FINISH_INTERPRETER.md) — `TaskState` split canary.
//!
//! Extracting the per-task execution state (env, call depth, TCO scratch) into a
//! `TaskState` sub-struct is a structural refactor that must not change any
//! observable behavior. The full suite is the real proof; this is a focused
//! smoke test that the single-task path still produces correct output.

use futures::executor::block_on;
use logicaffeine_compile::interpret_for_ui;

#[test]
fn interp_single_task_state_roundtrip() {
    let src = "## Main\n\
        \x20   Let x be 21.\n\
        \x20   Show x + x.\n";
    let result = block_on(interpret_for_ui(src));
    assert!(result.error.is_none(), "unexpected error: {:?}", result.error);
    assert!(
        result.lines.iter().any(|l| l.contains("42")),
        "expected output to contain 42, got {:?}",
        result.lines
    );
}

#[test]
fn interp_task_state_recursion_still_works() {
    // Exercises call_depth + TCO scratch (now in TaskState).
    let src = "## To fact (n: Int) -> Int:\n\
        \x20   If n equals 0:\n\
        \x20       Return 1.\n\
        \x20   Return n * fact(n - 1).\n\
        \n\
        ## Main\n\
        \x20   Show fact(5).\n";
    let result = block_on(interpret_for_ui(src));
    assert!(result.error.is_none(), "unexpected error: {:?}", result.error);
    assert!(
        result.lines.iter().any(|l| l.contains("120")),
        "expected 5! = 120, got {:?}",
        result.lines
    );
}
