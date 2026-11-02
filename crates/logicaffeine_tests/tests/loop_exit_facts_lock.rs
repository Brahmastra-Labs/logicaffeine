//! ★ LOCK — what the optimizer knows after a loop is what holds after it. ★
//!
//! The range oracle (`Opt::Oracle`, `optimize::abstract_interp`) folds a branch it proves can only go one way, and
//! after a loop it may assume only what every way out of that loop establishes: a variable the loop writes holds
//! whatever its last iteration left, and the loop ended because its condition failed, unless a `Break` of its own
//! can end it with the condition still holding. A counter read after `While i is less than n:` is not its value
//! before the loop, an exit by `Break` is not an exit by the condition, and a bound past the counter's start is not
//! an empty range.
//!
//! Each row runs on the tree-walker over the program as written (the reference), the VM over the raw compile and
//! over the run path's optimized program, `largo run --interpret`'s entry on each of its engines (tree-walker, VM,
//! VM with the JIT), and both C tiers built as a release C build is, and every one prints the literal expected
//! bytes. The same programs are semantic probes (`tier_parity::semantic`, family `loop_exit/`), so the generated
//! Rust under both shipped profiles holds them too.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test loop_exit_facts_lock`

#![cfg(not(target_arch = "wasm32"))]

mod common;

use logicaffeine_compile::compile::{
    compile_to_c, compile_to_c_via_bytecode, compile_to_program_optimized, program_outcome,
    tw_lines, vm_outcome,
};
use logicaffeine_compile::ui_bridge::{interpret_for_ui_sync_with_args, set_engine, Engine};

/// Lines as a process writes them: each ended by a newline.
fn written(lines: &[&str]) -> String {
    lines.iter().map(|line| format!("{line}\n")).collect()
}

/// A C program built as a release C build is, run: what it printed, and its standard error when it failed.
fn run_c(
    tier: &str,
    c: Result<String, logicaffeine_compile::ParseError>,
    src: &str,
) -> (String, Option<String>) {
    let c = c.unwrap_or_else(|e| panic!("the {tier} generator refused ({e:?}):\n{src}"));
    let dir = tempfile::tempdir().expect("temp dir");
    let (c_path, bin) = (dir.path().join("main.c"), dir.path().join("main"));
    std::fs::write(&c_path, &c).expect("write the C");
    let built = common::build_c_release(&c_path, &bin).expect("run the C compiler");
    assert!(
        built.status.success(),
        "the {tier} does not compile:\n{}\n{src}",
        String::from_utf8_lossy(&built.stderr)
    );
    let run = std::process::Command::new(&bin)
        .output()
        .expect("run the C program");
    (
        String::from_utf8_lossy(&run.stdout).into_owned(),
        (!run.status.success()).then(|| String::from_utf8_lossy(&run.stderr).into_owned()),
    )
}

/// `src` prints exactly `expected` on every tier.
fn prints(src: &str, expected: &[&str]) {
    let want = written(expected);

    let tw = tw_lines(src);
    assert_eq!(tw.error, None, "the tree-walker failed:\n{src}");
    assert_eq!(
        written(&tw.lines.iter().map(String::as_str).collect::<Vec<_>>()),
        want,
        "the tree-walker's answer:\n{src}"
    );

    let vm = vm_outcome(src);
    assert_eq!(vm.error, None, "the VM failed:\n{src}");
    assert_eq!(
        vm.output, want,
        "the VM's answer over the raw compile:\n{src}"
    );

    let program = compile_to_program_optimized(src)
        .unwrap_or_else(|e| panic!("the run path does not compile it ({e:?}):\n{src}"));
    let optimized = program_outcome(&program);
    assert_eq!(
        optimized.error, None,
        "the VM failed over the optimized program:\n{src}"
    );
    assert_eq!(
        optimized.output, want,
        "the VM's answer over the run path's optimized program:\n{src}"
    );

    // The native tier, installed as `largo` installs it at startup, on the platforms it installs it on.
    #[cfg(all(target_arch = "x86_64", not(target_os = "windows")))]
    logicaffeine_jit::install();
    let engines: &[Engine] = if cfg!(all(target_arch = "x86_64", not(target_os = "windows"))) {
        &[Engine::TreeWalker, Engine::Vm, Engine::Jit]
    } else {
        &[Engine::TreeWalker, Engine::Vm]
    };
    for &engine in engines {
        set_engine(engine);
        let run = interpret_for_ui_sync_with_args(src, &["t".to_string()]);
        set_engine(Engine::Auto);
        assert_eq!(
            run.error, None,
            "`largo run --interpret` on {engine:?} failed:\n{src}"
        );
        assert_eq!(
            written(&run.lines.iter().map(String::as_str).collect::<Vec<_>>()),
            want,
            "`largo run --interpret`'s answer on {engine:?}:\n{src}"
        );
    }

    for (tier, c) in [
        ("AST C", compile_to_c(src)),
        ("bytecode C", compile_to_c_via_bytecode(src)),
    ] {
        let (output, failure) = run_c(tier, c, src);
        assert_eq!(failure, None, "the {tier} program failed:\n{src}\n{output}");
        assert_eq!(output, want, "the {tier} program's answer:\n{src}");
    }
}

/// A counter a loop advances to a bound the program reads at run time is not, after the loop, the value it started
/// at: the branch that tests for the start runs only when the loop never did.
#[test]
fn a_counter_after_a_loop_to_a_run_time_bound_is_not_its_starting_value() {
    prints(
        "## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt(\"7\").
Let mutable i be 0.
While i is less than n:
    Set i to i + 1.
If i equals 0:
    Show \"never ran\".
Otherwise:
    Show \"ran\".
Show i.
",
        &["ran", "7"],
    );
}

/// The same loop with a bound of zero never runs, and the branch that tests for the start does.
#[test]
fn a_loop_that_never_runs_leaves_its_counter_where_it_started() {
    prints(
        "## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt(\"0\").
Let mutable i be 0.
While i is less than n:
    Set i to i + 1.
If i equals 0:
    Show \"never ran\".
Otherwise:
    Show \"ran\".
Show i.
",
        &["never ran", "0"],
    );
}

/// After `While i is less than 10:` the counter is at least 10, so a test for less than 10 never holds, and a test
/// for exactly 10 is not decided by the bound alone.
#[test]
fn a_loop_to_a_literal_bound_leaves_its_counter_at_the_bound() {
    prints(
        "## Main
Let mutable i be 0.
While i is less than 10:
    Set i to i + 1.
If i equals 10:
    Show \"ten\".
Otherwise:
    Show \"not ten\".
If i is less than 10:
    Show \"below\".
Show i.
",
        &["ten", "10"],
    );
}

/// A counter that starts past its bound skips the loop and keeps its start.
#[test]
fn a_counter_that_starts_past_its_bound_keeps_its_start() {
    prints(
        "## Main
Let mutable i be 12.
While i is less than 10:
    Set i to i + 1.
If i equals 12:
    Show \"skipped\".
Otherwise:
    Show \"ran\".
Show i.
",
        &["skipped", "12"],
    );
}

/// A `Break` ends a loop whose condition still holds: nothing after it may assume the condition failed.
#[test]
fn a_loop_left_by_break_promises_nothing_about_its_condition() {
    prints(
        "## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt(\"7\").
Let mutable i be 0.
While i is less than 100:
    If i equals n:
        Break.
    Set i to i + 1.
If i is at least 100:
    Show \"ran out\".
Otherwise:
    Show \"broke early\".
Show i.
",
        &["broke early", "7"],
    );
}

/// A value the last iteration wrote before it broke out is the value after the loop, though the rest of the body
/// would have overwritten it.
#[test]
fn a_loop_left_by_break_keeps_what_its_last_iteration_wrote() {
    prints(
        "## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt(\"3\").
Let mutable x be 0.
Let mutable i be 0.
While i is less than 10:
    Set x to 5.
    If i equals n:
        Break.
    Set x to 7.
    Set i to i + 1.
If x equals 7:
    Show \"seven\".
Otherwise:
    Show \"not seven\".
Show x.
Show i.
",
        &["not seven", "5", "3"],
    );
}

/// A `Break` in a counted loop ends it early too.
#[test]
fn a_counted_loop_left_by_break_stops_where_it_broke() {
    prints(
        "## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt(\"4\").
Let mutable steps be 0.
Repeat for k from 1 to 100:
    Set steps to steps + 1.
    If k equals n:
        Break.
If steps is at least 100:
    Show \"ran to the end\".
Otherwise:
    Show \"stopped at {steps}\".
",
        &["stopped at 4"],
    );
}

/// A `Break` inside a nested loop ends only that loop: the outer one still ends on its condition.
#[test]
fn a_break_in_a_nested_loop_ends_only_that_loop() {
    prints(
        "## To native parseInt (s: Text) -> Int

## Main
Let n be parseInt(\"5\").
Let mutable i be 0.
Let mutable inner be 0.
While i is less than n:
    Let mutable j be 0.
    While j is less than 100:
        If j equals 2:
            Break.
        Set j to j + 1.
    Set inner to inner + j.
    Set i to i + 1.
If i is less than n:
    Show \"outer broke\".
Otherwise:
    Show \"outer ran out\".
Show inner.
",
        &["outer ran out", "10"],
    );
}

/// The counter of a loop inside a function, against a bound the function is handed.
#[test]
fn a_counter_after_a_loop_to_a_parameter_is_not_its_starting_value() {
    prints(
        "## To steps (n: Int) -> Text:
    Let mutable i be 0.
    While i is less than n:
        Set i to i + 1.
    If i equals 0:
        Return \"none\".
    Return \"some\".

## Main
Show steps(3).
Show steps(0).
",
        &["some", "none"],
    );
}
