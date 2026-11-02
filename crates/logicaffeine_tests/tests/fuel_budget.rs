//! Engine fuel: the deterministic step budget behind `largo run --max-steps`
//! and the MCP `logos_run` `step_budget`.
//!
//! One `FuelHandles` cell installed via `interpreter::install_fuel(n)` bounds
//! the WHOLE run — the straight-line VM path and the tree-walker (including
//! every spawned task, which shares the same cell). The contract:
//!
//!   * an unbudgeted run is byte-identical to a budgeted run that finishes
//!     under budget (fuel never perturbs semantics);
//!   * a budgeted run that exhausts its fuel STOPS, with a structured
//!     `exhausted` flag (never string-sniffing, never a panic, never a hang);
//!   * the partial output of an exhausted run is a prefix of the fuller run.
//!
//! Every test drives the real dispatcher (`interpret_for_ui_sync_with_args`)
//! on a watchdog thread so a fuel regression reports as a failure, not a
//! wedged suite.

use std::sync::mpsc;
use std::time::Duration;

use logicaffeine_compile::interpreter::{clear_fuel, install_fuel};
use logicaffeine_compile::ui_bridge::{interpret_for_ui_sync_with_args, InterpreterResult};

/// Run on a watchdog thread; panic if the engine wedges.
fn run_guarded(
    source: &'static str,
    fuel: Option<u64>,
) -> (InterpreterResult, Option<(bool, u64)>) {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let handles = fuel.map(install_fuel);
        let result = interpret_for_ui_sync_with_args(source, &[]);
        let report = handles.map(|h| (h.exhausted.get(), h.remaining.get()));
        clear_fuel();
        let _ = tx.send((result, report));
    });
    rx.recv_timeout(Duration::from_secs(60))
        .expect("the engine must stop within its budget — a hang here is the fuel regression")
}

const VM_INFINITE: &str = "# Main\n\n## Main\n\nLet mutable x be 1.\nWhile x is greater than 0:\n    Set x to x plus 1.\nShow x.\n";

const TASK_INFINITE: &str = "\
# Main

## To spin:
    Let mutable x be 1.
    While x is greater than 0:
        Set x to x plus 1.

## Main

Launch a task to spin.
Show \"started\".
";

const PRINTING_LOOP: &str = "# Main\n\n## Main\n\nLet mutable i be 1.\nWhile i is greater than 0:\n    Show i.\n    Set i to i plus 1.\n";

const HELLO: &str = "# Main\n\n## Main\n\nShow \"Hello, world!\".\n";

#[test]
fn vm_infinite_loop_stops_within_budget() {
    // Non-async straight-line program: the sync dispatcher's VM+JIT path.
    let (result, report) = run_guarded(VM_INFINITE, Some(200_000));
    let (exhausted, remaining) = report.unwrap();
    assert!(exhausted, "the fuel flag is the structured signal");
    assert_eq!(remaining, 0, "an exhausted budget reads zero");
    let error = result.error.expect("an exhausted run reports an error");
    assert!(
        error.contains("step budget"),
        "the error names the budget: {error}"
    );
}

#[test]
fn concurrent_program_budget_spans_all_tasks() {
    // The runaway is inside a SPAWNED task; the scheduler path runs the
    // tree-walker, and every task interpreter shares one fuel cell.
    let (result, report) = run_guarded(TASK_INFINITE, Some(200_000));
    let (exhausted, _) = report.unwrap();
    assert!(
        exhausted,
        "one budget spans the whole cohort: {:?}",
        result.error
    );
}

#[test]
fn budget_not_hit_is_byte_identical() {
    let (unbudgeted, _) = run_guarded(HELLO, None);
    let (budgeted, report) = run_guarded(HELLO, Some(u64::MAX / 2));
    assert_eq!(
        unbudgeted.lines, budgeted.lines,
        "fuel never perturbs output"
    );
    assert_eq!(
        unbudgeted.error, budgeted.error,
        "fuel never perturbs errors"
    );
    let (exhausted, _) = report.unwrap();
    assert!(!exhausted, "a completed run never reads exhausted");
}

#[test]
fn budget_exceeded_output_is_prefix_of_full_run() {
    let (small, small_report) = run_guarded(PRINTING_LOOP, Some(10_000));
    let (large, _) = run_guarded(PRINTING_LOOP, Some(40_000));
    assert!(small_report.unwrap().0, "the small budget must exhaust");
    assert!(
        !small.lines.is_empty(),
        "partial output survives an exhausted budget"
    );
    assert!(
        small.lines.len() < large.lines.len(),
        "a bigger budget gets further"
    );
    assert_eq!(
        small.lines[..],
        large.lines[..small.lines.len()],
        "the exhausted run's output is a prefix of the fuller run"
    );
}

#[test]
fn budget_exceeded_is_structured_not_panic() {
    let (result, report) = run_guarded(VM_INFINITE, Some(50_000));
    let (exhausted, remaining) = report.unwrap();
    assert!(exhausted);
    assert_eq!(remaining, 0);
    assert!(result.error.is_some(), "structured outcome, never a panic");
}

const COUNTED: &str = "# Main\n\n## Main\n\nLet mutable s be 0.\nRepeat for i from 1 to 200:\n    Set s to s plus i.\nShow s.\n";

#[test]
fn a_finished_vm_run_reports_exactly_the_steps_it_took() {
    // What a finished run leaves in the cell is the budget less what it spent, so `--max-steps` and the
    // MCP step budget can say how much of a budget a run used, not only whether it ran out.
    let budget = 10_000_000;
    let (result, report) = run_guarded(COUNTED, Some(budget));
    assert_eq!(result.error, None);
    let (exhausted, remaining) = report.unwrap();
    assert!(!exhausted, "the run finishes");
    let used = budget - remaining;
    assert!(used > 200, "two hundred iterations take more than two hundred steps ({used})");
    let (_, exact) = run_guarded(COUNTED, Some(used));
    assert!(!exact.unwrap().0, "a budget of exactly the steps reported finishes");
    let (_, short) = run_guarded(COUNTED, Some(used - 1));
    assert!(short.unwrap().0, "one step less runs out");
}
