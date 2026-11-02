//! `Launch a task to f with a and b` hands `f` two arguments, and names `f` the way `Call` does.
//!
//! The launch statement read its arguments with the general expression parser, which takes `and` as
//! logical conjunction: `with ch and 42` became the single argument `ch and 42`, the task failed its
//! arity check, and a launched task's failure is not reported, so the program deadlocked or went on
//! without it. It also accepted only a few word classes as the task's name where `Call` accepts any
//! function the program declares. Both forms of the statement now read the target and the arguments
//! exactly as `Call` reads them.

use logicaffeine_compile::{compile_to_rust, interpret_for_ui_sync, run_vm_concurrent};

fn on_both_tiers(source: &str, expected: &[&str]) {
    let tw = interpret_for_ui_sync(source);
    assert_eq!(tw.error, None, "tree-walker failed:\n{source}");
    assert_eq!(tw.lines, expected, "tree-walker:\n{source}");
    let vm = run_vm_concurrent(source);
    assert_eq!(vm.error, None, "vm failed:\n{source}");
    assert_eq!(vm.lines, expected, "vm:\n{source}");
    compile_to_rust(source).unwrap_or_else(|e| panic!("AOT refused it: {e:?}\n{source}"));
}

#[test]
fn a_launch_passes_every_argument_it_names() {
    on_both_tiers(
        "## To report (name: Text, n: Int):\n    Show \"{name} {n}\".\n\n## Main\nLaunch a task to report with \"ada\" and 7.\nShow \"after\".\n",
        &["ada 7", "after"],
    );
}

#[test]
fn a_launch_passes_a_pipe_and_a_value() {
    on_both_tiers(
        "## To emit (ch: Int, n: Int):\n    Send n into ch.\n\n## Main\nLet ch be a Pipe of Int.\nLaunch a task to emit with ch and 42.\nReceive got from ch.\nShow got.\n",
        &["42"],
    );
}

#[test]
fn a_launch_passes_three_arguments_separated_by_commas_and_and() {
    on_both_tiers(
        "## To sum3 (a: Int, b: Int, c: Int):\n    Show a + b + c.\n\n## Main\nLaunch a task to sum3 with 1, 2 and 3.\n",
        &["6"],
    );
}

#[test]
fn a_launch_argument_may_itself_be_a_comparison() {
    on_both_tiers(
        "## To flag (big: Bool, n: Int):\n    Show \"{big} {n}\".\n\n## Main\nLet x be 9.\nLaunch a task to flag with x is greater than 5 and x.\n",
        &["true 9"],
    );
}

#[test]
fn a_launch_with_a_handle_passes_every_argument_too() {
    on_both_tiers(
        "## To report (name: Text, n: Int):\n    Show \"{name} {n}\".\n\n## Main\nLet h be Launch a task to report with \"ada\" and 7.\nAwait h.\nShow \"joined\".\n",
        &["ada 7", "joined"],
    );
}

#[test]
fn a_launch_names_any_function_the_program_declares() {
    // `later` is a temporal word, not a noun or verb — `Call later.` has always found it.
    on_both_tiers(
        "## To later:\n    Show \"later ran\".\n\n## Main\nLaunch a task to later.\nLet h be Launch a task to later.\nShow \"main\".\n",
        &["later ran", "later ran", "main"],
    );
}
