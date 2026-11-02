//! A launched task that fails fails the run — on every tier, after the other tasks finish.
//!
//! The VM hands every task it launches the run's error sink: the first failure is kept, the rest of
//! the program still runs to quiescence, and the run reports the failure. The tree-walker gave only
//! the main task a sink, so a launched task's error vanished and the run succeeded. A failure that
//! disappears is how a launch passing its task the wrong number of arguments went unnoticed: the task
//! failed its arity check in silence and the program deadlocked or carried on without it.

use logicaffeine_compile::{
    interpret_for_ui_sync, run_treewalker_concurrent_seeded, run_vm_concurrent_seeded,
};

const FAILING_CHILD: &str = "## To bad (n: Int):\n    Show \"bad starts\".\n    Show 10 / n.\n\n## Main\nLaunch a task to bad with 0.\nShow \"main goes on\".\n";

#[test]
fn a_launched_tasks_failure_fails_the_run_on_the_tree_walker() {
    for seed in [0, 1, 7] {
        let run = run_treewalker_concurrent_seeded(FAILING_CHILD, seed);
        assert_eq!(
            run.lines,
            ["bad starts", "main goes on"],
            "the other tasks still run to quiescence"
        );
        let error = run
            .error
            .unwrap_or_else(|| panic!("seed {seed}: the launched task's failure vanished"));
        assert!(
            error.contains("Division by zero"),
            "the run reports the task's own error, got: {error}"
        );
    }
    let studio = interpret_for_ui_sync(FAILING_CHILD);
    assert!(
        studio.error.is_some_and(|e| e.contains("Division by zero")),
        "the Studio's run reports it too"
    );
}

#[test]
fn a_launched_tasks_failure_fails_the_run_on_the_vm() {
    for seed in [0, 1, 7] {
        let run = run_vm_concurrent_seeded(FAILING_CHILD, seed);
        assert_eq!(
            run.lines,
            ["bad starts", "main goes on"],
            "the other tasks still run to quiescence"
        );
        let error = run
            .error
            .unwrap_or_else(|| panic!("seed {seed}: the launched task's failure vanished"));
        assert!(
            error.contains("Division by zero"),
            "the run reports the task's own error, got: {error}"
        );
    }
}

#[test]
fn the_first_failure_is_the_one_reported() {
    let source = "## To bad (n: Int):\n    Show 10 / n.\n\n## To worse:\n    Fail with \"second\".\n\n## Main\n\
                  Launch a task to bad with 0.\nLaunch a task to worse.\nShow \"main\".\n";
    for (tier, run) in [
        ("tree-walker", run_treewalker_concurrent_seeded(source, 0)),
        ("vm", run_vm_concurrent_seeded(source, 0)),
    ] {
        let error = run
            .error
            .unwrap_or_else(|| panic!("{tier}: no failure reported"));
        assert!(
            error.contains("Division by zero"),
            "{tier}: the first task to fail is reported, got: {error}"
        );
    }
}
