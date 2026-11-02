//! ★ LOCK — the tree-walker recurses as deep as the bytecode VM, whatever thread calls it. ★
//!
//! Every engine enforces one call-depth limit (`semantics::MAX_CALL_DEPTH`); past it a program gets
//! a catchable error, never a host crash. The bytecode VM keeps its frames on the heap, so it reaches
//! that depth on any thread. The tree-walker keeps its frames on the Rust stack — a LOGOS call costs
//! it a chain of statement → expression → call frames — so on an ordinary 2 MiB thread it overflowed
//! near fifty calls, a process abort with no diagnostic.
//!
//! A tree-walker SESSION therefore owns its stack: `interpreter::on_session_stack` runs the whole
//! session — the sealed world a test enters, the program, the page it drives, every snapshot — on one
//! dedicated thread with room for the full depth, so every thread-local the session touches stays on
//! the thread that set it. These tests call each host entry from an ORDINARY 2 MiB thread, the floor a
//! library caller can count on, and demand the VM's depth from it.

use logicaffeine_compile::interpreter::{on_session_stack, Interpreter, RuntimeValue};
use logicaffeine_compile::page_host::LoweredCalls;
use logicaffeine_compile::semantics::{CALL_DEPTH_ERR, MAX_CALL_DEPTH};
use logicaffeine_compile::testrun::{run_tests, TestConfig, Verdict};

/// Run `f` on a thread with the ORDINARY 2 MiB stack.
fn on_an_ordinary_thread<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    std::thread::Builder::new()
        .stack_size(2 * 1024 * 1024)
        .spawn(f)
        .expect("spawn")
        .join()
        .expect("the engine must not abort or panic on an ordinary thread")
}

/// The deepest recursion every engine promises: `down(n)` is `n + 1` live calls, and the outermost
/// caller's own frame is the one more that reaching `MAX_CALL_DEPTH` itself would take.
fn deepest() -> usize {
    MAX_CALL_DEPTH - 2
}

const DOWN: &str = "## To down (n: Int) -> Int:\n    If n is at most 0:\n        Return 0.\n    Return down(n - 1) - 1.\n";

fn deep_main(depth: usize) -> String {
    format!("{DOWN}\n## Main\n    Show down({depth}).\n")
}

fn deep_test(depth: usize) -> String {
    format!("{DOWN}\n## Test: it recurses all the way down\nExpect down({depth}) is 0 - {depth}.\n")
}

#[test]
fn the_vm_reaches_the_depth_this_lock_demands() {
    // The yardstick: the VM's own depth for the very same program, on the ordinary thread.
    let depth = deepest();
    let out = on_an_ordinary_thread(move || {
        logicaffeine_compile::compile::run_strict_vm(&deep_main(depth))
    });
    assert_eq!(
        out.error, None,
        "the VM recurses {depth} calls deep on an ordinary thread"
    );
    assert_eq!(out.output.trim(), format!("-{depth}"));
}

#[test]
fn a_test_recurses_as_deep_as_the_vm_from_an_ordinary_thread() {
    let depth = deepest();
    let report =
        on_an_ordinary_thread(move || run_tests(&deep_test(depth), &TestConfig::default()));
    let report = report.expect("the program parses");
    assert_eq!(report.outcomes.len(), 1);
    let outcome = &report.outcomes[0];
    assert_eq!(
        outcome.verdict,
        Verdict::Passed,
        "`largo test`'s tree-walker recurses {depth} calls deep: {:?}",
        outcome.failures
    );
}

#[test]
fn past_the_limit_a_test_reports_the_depth_error_rather_than_aborting() {
    let depth = MAX_CALL_DEPTH + 100;
    let report =
        on_an_ordinary_thread(move || run_tests(&deep_test(depth), &TestConfig::default()));
    let outcome = &report.expect("the program parses").outcomes[0];
    assert_eq!(outcome.verdict, Verdict::Failed);
    assert!(
        outcome.failures.iter().any(|f| f.contains(CALL_DEPTH_ERR)),
        "the failure is the documented depth-limit error: {:?}",
        outcome.failures
    );
}

#[test]
fn a_host_call_into_a_page_recurses_as_deep_as_the_vm() {
    // The LoweredHere shape: the page's main runs, then the host calls into it — every call on the
    // session's own stack.
    let depth = deepest();
    let answer = on_an_ordinary_thread(move || {
        on_session_stack(|| {
            let src = format!("{DOWN}\n## Main\nLet ready be true.\n");
            logicaffeine_compile::ui_bridge::with_parsed_program(&src, |parsed, interner| {
                let (stmts, types, _) = parsed.expect("parses");
                let mut interp = Interpreter::new(interner).with_type_registry(types);
                interp.run_page_main(stmts).expect("the page's main runs");
                match interp.call("down", vec![RuntimeValue::Int(depth as i64)]) {
                    Ok(RuntimeValue::Int(n)) => Ok(n),
                    Ok(other) => Err(format!("answered a {}", other.type_name())),
                    Err(e) => Err(e),
                }
            })
        })
    });
    assert_eq!(answer, Ok(-(depth as i64)));
}

#[test]
fn a_session_runs_on_one_thread_from_start_to_finish() {
    // Every thread-local a session writes must be there when the same session reads it: the whole
    // closure runs on one thread, and a session inside a session does not move to another one.
    let (outer, inner, nested_again) = on_an_ordinary_thread(|| {
        on_session_stack(|| {
            let outer = std::thread::current().id();
            let inner = on_session_stack(|| std::thread::current().id());
            let nested_again =
                on_session_stack(|| on_session_stack(|| std::thread::current().id()));
            (outer, inner, nested_again)
        })
    });
    assert_eq!(
        outer, inner,
        "a nested session stays on the session's own thread"
    );
    assert_eq!(outer, nested_again);
}

#[test]
fn a_session_does_not_run_on_its_callers_thread() {
    let (caller, session) = on_an_ordinary_thread(|| {
        let caller = std::thread::current().id();
        (caller, on_session_stack(|| std::thread::current().id()))
    });
    assert_ne!(
        caller, session,
        "an ordinary caller's thread is never deep enough to host one"
    );
}

#[test]
fn a_panic_in_a_session_reaches_its_caller_unchanged() {
    let caught = std::panic::catch_unwind(|| on_session_stack(|| panic!("inside the session")));
    let payload = caught.expect_err("the panic crosses back to the caller");
    let message = payload
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| payload.downcast_ref::<String>().cloned())
        .unwrap_or_default();
    assert_eq!(message, "inside the session");
}
