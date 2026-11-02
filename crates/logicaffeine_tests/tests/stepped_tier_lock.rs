//! The stepped tier carries every value the C tier carries, across every scheduling point.
//!
//! The stepped tier lowers a concurrent program's tasks into step functions over heap frames
//! (`codegen_c::stepped`): the ordinary C lowering under a `switch` on the task's resume point, every binding
//! a frame field. So a task keeps records, sequences, maps, text and Reals across a wait; calls ordinary
//! helpers; calls a function that waits from inside an expression or a loop's condition (the call runs first,
//! in evaluation order, into a cell); recurses through waits, directly and through another function that
//! waits; waits inside a select arm; and a runtime failure — a failing statement, or a zero divisor inside
//! the arithmetic — fails its task while every other task finishes, as on the interpreter.
//!
//! Each program runs on the tree-walker for the reference, pinned per row at seed 0, then on the stepped tier
//! and on host C at every seed.

mod common;

use std::path::PathBuf;
use std::process::Command;

use logicaffeine_compile::compile::{compile_to_c, compile_to_c_stepped};
use logicaffeine_compile::run_treewalker_concurrent_seeded;

const SEEDS: &[u64] = &[0, 1, 2, 3];

#[derive(Debug, PartialEq)]
struct Outcome {
    lines: Vec<String>,
    failed: bool,
}

fn reference(src: &str, seed: u64) -> Outcome {
    let tw = run_treewalker_concurrent_seeded(src, seed);
    Outcome {
        lines: tw.lines,
        failed: tw.error.is_some(),
    }
}

fn build(tier: &str, name: &str, c: Result<String, String>) -> Result<PathBuf, String> {
    let c = c?;
    let dir = std::env::temp_dir().join(format!(
        "logos_stepped_{}_{tier}_{name}",
        std::process::id()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let (file, bin) = (dir.join("prog.c"), dir.join("prog"));
    std::fs::write(&file, &c).unwrap();
    let out = Command::new("gcc")
        .args(["-O1", "-o"])
        .arg(&bin)
        .arg(&file)
        .arg("-lm")
        .output()
        .unwrap();
    if !out.status.success() {
        return Err(format!(
            "does not build as C:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(bin)
}

fn run(bin: &std::path::Path, seed: u64) -> Outcome {
    run_with(bin, seed, &[])
}

fn run_with(bin: &std::path::Path, seed: u64, env: &[(&str, &str)]) -> Outcome {
    let run = Command::new(bin)
        .env("LOGOS_SEED", seed.to_string())
        .envs(env.iter().copied())
        .output()
        .expect("the binary runs");
    Outcome {
        lines: String::from_utf8_lossy(&run.stdout)
            .lines()
            .map(str::to_string)
            .collect(),
        failed: !run.status.success(),
    }
}

/// `(name, source, the reference's lines at seed 0, whether it fails)`.
const CORPUS: &[(&str, &str, &[&str], bool)] = &[
    (
        "a_task_builds_a_record_from_what_it_received",
        "## A Point has:\n    an x, which is Int.\n    a y, which is Int.\n\n## To worker (ch: Int):\n    \
         Receive n from ch.\n    Let p be a new Point with x n and y n * 2.\n    Show p's x + p's y.\n\n\
         ## Main\nLet ch be a Pipe of Int.\nLaunch a task to worker with ch.\nSend 5 into ch.\n",
        &["15"],
        false,
    ),
    (
        "a_sequence_grows_across_waits",
        "## To collect (ch: Int):\n    Let mutable xs be a new Seq of Int.\n    Repeat for i from 1 to 3:\n        \
         Receive v from ch.\n        Push v to xs.\n    Show xs.\n\n## Main\nLet ch be a Pipe of Int.\n\
         Launch a task to collect with ch.\nRepeat for i from 1 to 3:\n    Send i * 10 into ch.\n",
        &["[10, 20, 30]"],
        false,
    ),
    (
        "text_grows_across_waits",
        "## To join (ch: Text):\n    Let mutable s be \"\".\n    Repeat for i from 1 to 3:\n        Receive w from ch.\n        \
         Set s to s + w + \",\".\n    Show s.\n\n## Main\nLet ch be a Pipe of Text.\nLaunch a task to join with ch.\n\
         Send \"a\" into ch.\nSend \"b\" into ch.\nSend \"c\" into ch.\n",
        &["a,b,c,"],
        false,
    ),
    (
        "a_map_counts_across_waits",
        "## To tally (ch: Text):\n    Let mutable m be a new Map of Text to Int.\n    Repeat for i from 1 to 3:\n        \
         Receive w from ch.\n        If m contains w:\n            Set item w of m to item w of m + 1.\n        Otherwise:\n            \
         Set item w of m to 1.\n    Show item \"a\" of m.\n    Show item \"b\" of m.\n\n## Main\nLet ch be a Pipe of Text.\n\
         Launch a task to tally with ch.\nSend \"a\" into ch.\nSend \"b\" into ch.\nSend \"a\" into ch.\n",
        &["2", "1"],
        false,
    ),
    (
        "reals_add_up_across_waits",
        "## To sum (ch: Real):\n    Let mutable total be 0.0.\n    Repeat for i from 1 to 3:\n        Receive x from ch.\n        \
         Set total to total + x.\n    Show total / 2.0.\n\n## Main\nLet ch be a Pipe of Real.\nLaunch a task to sum with ch.\n\
         Send 1.5 into ch.\nSend 2.25 into ch.\nSend 0.25 into ch.\n",
        &["2"],
        false,
    ),
    (
        "a_task_calls_an_ordinary_helper",
        "## To square (n: Int) -> Int:\n    Return n * n.\n\n## To worker (ch: Int):\n    Receive v from ch.\n    \
         Show square(v) + 1.\n\n## Main\nLet ch be a Pipe of Int.\nLaunch a task to worker with ch.\nSend 7 into ch.\n",
        &["50"],
        false,
    ),
    (
        "calls_that_wait_run_left_to_right_inside_an_expression",
        "## To get (ch: Int) -> Int:\n    Receive x from ch.\n    Return x.\n\n## To feed (ch: Int):\n    \
         Send 10 into ch.\n    Send 3 into ch.\n\n## Main\nLet ch be a Pipe of Int.\nLaunch a task to feed with ch.\n\
         Show get(ch) - get(ch).\n",
        &["7"],
        false,
    ),
    (
        "a_loop_condition_that_waits_is_read_every_pass",
        "## To get (ch: Int) -> Int:\n    Receive x from ch.\n    Return x.\n\n## To feed (ch: Int):\n    \
         Send 2 into ch.\n    Send 1 into ch.\n    Send 0 into ch.\n\n## Main\nLet ch be a Pipe of Int.\n\
         Launch a task to feed with ch.\nWhile get(ch) is greater than 0:\n    Show \"tick\".\nShow \"done\".\n",
        &["tick", "tick", "done"],
        false,
    ),
    (
        "a_break_leaves_a_loop_whose_condition_waits",
        "## To get (ch: Int) -> Int:\n    Receive x from ch.\n    Return x.\n\n## To feed (ch: Int):\n    \
         Send 5 into ch.\n\n## Main\nLet ch be a Pipe of Int.\nLaunch a task to feed with ch.\n\
         While get(ch) is greater than 0:\n    Show \"tick\".\n    Break.\nShow \"done\".\n",
        &["tick", "done"],
        false,
    ),
    (
        "a_return_in_a_zone_of_a_function_that_waits_leaves_the_zone",
        "## To f (ch: Int) -> Int:\n    Receive n from ch.\n    Inside a zone called \"Z\":\n        Show n.\n        \
         Return 5.\n    Return 7.\n\n## To feed (ch: Int):\n    Send 3 into ch.\n\n## Main\nLet ch be a Pipe of Int.\n\
         Launch a task to feed with ch.\nShow f(ch).\n",
        &["3", "7"],
        false,
    ),
    (
        "a_text_a_function_that_waits_grows_is_its_own",
        "## To grow (s: Text, ch: Int) -> Text:\n    Receive n from ch.\n    Set s to s + \"!\".\n    Return s.\n\n\
         ## To feed (ch: Int):\n    Send 1 into ch.\n\n## Main\nLet ch be a Pipe of Int.\nLet t be \"hi\".\n\
         Launch a task to feed with ch.\nShow grow(t, ch).\nShow t.\n",
        &["hi!", "hi"],
        false,
    ),
    (
        "a_text_condition_that_waits_is_true_while_it_holds_a_character",
        "## To get (ch: Text) -> Text:\n    Receive x from ch.\n    Return x.\n\n## To feed (ch: Text):\n    \
         Send \"a\" into ch.\n    Send \"\" into ch.\n\n## Main\nLet ch be a Pipe of Text.\n\
         Launch a task to feed with ch.\nWhile get(ch):\n    Show \"tick\".\nShow \"done\".\n",
        &["tick", "done"],
        false,
    ),
    (
        "a_function_that_waits_recurses",
        "## To drain (ch: Int) -> Int:\n    Receive x from ch.\n    If x equals 0:\n        Return 0.\n    \
         Return x + drain(ch).\n\n## To feed (ch: Int):\n    Send 3 into ch.\n    Send 4 into ch.\n    Send 0 into ch.\n\n\
         ## Main\nLet ch be a Pipe of Int.\nLaunch a task to feed with ch.\nShow drain(ch).\n",
        &["7"],
        false,
    ),
    (
        "two_functions_that_wait_recurse_through_each_other",
        "## To ping (ch: Int):\n    Receive x from ch.\n    If x is greater than 0:\n        Show x.\n        pong(ch).\n\n\
         ## To pong (ch: Int):\n    Receive y from ch.\n    If y is greater than 0:\n        Show y.\n        ping(ch).\n\n\
         ## Main\nLet ch be a Pipe of Int.\nLaunch a task to ping with ch.\nSend 2 into ch.\nSend 1 into ch.\nSend 0 into ch.\n",
        &["2", "1"],
        false,
    ),
    (
        "a_select_arm_waits_inside_its_body",
        "## To feed (a: Int, b: Int):\n    Send 1 into a.\n    Send 41 into b.\n\n## Main\nLet a be a Pipe of Int.\n\
         Let b be a Pipe of Int.\nLaunch a task to feed with a and b.\nAwait the first of:\n    Receive x from a:\n        \
         Receive y from b.\n        Show x + y.\n    After 5 seconds:\n        Show 0.\n",
        &["42"],
        false,
    ),
    (
        "a_runtime_failure_fails_its_task_and_the_others_finish",
        "## To bad (ch: Int):\n    Receive d from ch.\n    Show 10 / d.\n\n## To good:\n    Show \"good\".\n\n\
         ## Main\nLet ch be a Pipe of Int.\nLaunch a task to bad with ch.\nLaunch a task to good.\nSend 0 into ch.\n\
         Show \"main\".\n",
        &["good", "main"],
        true,
    ),
    (
        "a_real_division_by_zero_fails_its_task",
        "## To bad (ch: Real):\n    Receive d from ch.\n    Show 1.5 / d.\n\n## Main\nLet ch be a Pipe of Real.\n\
         Launch a task to bad with ch.\nSend 0.0 into ch.\nShow \"main\".\n",
        &["main"],
        true,
    ),
    (
        "a_remainder_by_zero_fails_its_task",
        "## To bad (ch: Int):\n    Receive d from ch.\n    Show 7 % d.\n\n## Main\nLet ch be a Pipe of Int.\n\
         Launch a task to bad with ch.\nSend 0 into ch.\nShow \"main\".\n",
        &["main"],
        true,
    ),
    (
        "a_floor_division_by_zero_fails_its_task",
        "## To bad (ch: Int):\n    Receive d from ch.\n    Show 7 // d.\n\n## Main\nLet ch be a Pipe of Int.\n\
         Launch a task to bad with ch.\nSend 0 into ch.\nShow \"main\".\n",
        &["main"],
        true,
    ),
    (
        "a_function_that_waits_returns_an_integer_past_a_word",
        "## To drain (ch: Int) -> Int:\n    Receive x from ch.\n    If x equals 0:\n        Return 0.\n    \
         Return x + drain(ch).\n\n## To feed (ch: Int):\n    Send 4611686018427387904 into ch.\n    \
         Send 4611686018427387904 into ch.\n    Send 4611686018427387904 into ch.\n    Send 0 into ch.\n\n\
         ## Main\nLet ch be a Pipe of Int.\nLaunch a task to feed with ch.\nShow drain(ch).\n",
        &["13835058055282163712"],
        false,
    ),
    (
        "a_function_that_waits_takes_an_integer_past_a_word",
        "## To total (ch: Int, acc: Int) -> Int:\n    Receive x from ch.\n    If x equals 0:\n        Return acc.\n    \
         Return total(ch, acc + x).\n\n## To feed (ch: Int):\n    Send 4611686018427387904 into ch.\n    \
         Send 4611686018427387904 into ch.\n    Send 4611686018427387904 into ch.\n    Send 0 into ch.\n\n\
         ## Main\nLet ch be a Pipe of Int.\nLaunch a task to feed with ch.\nLet mutable start be 4611686018427387904.\n\
         Set start to start + start.\nShow total(ch, start).\n",
        &["23058430092136939520"],
        false,
    ),
    (
        "a_task_is_launched_with_an_integer_past_a_word",
        "## To report (n: Int):\n    Show n.\n\n## Main\nLet mutable big be 4611686018427387904.\n\
         Set big to big + big.\nSet big to big + big.\nLaunch a task to report with big.\nShow \"main\".\n",
        &["18446744073709551616", "main"],
        false,
    ),
];

#[test]
fn the_reference_shows_what_each_row_pins() {
    for (name, src, lines, fails) in CORPUS {
        let want = Outcome {
            lines: lines.iter().map(|l| l.to_string()).collect(),
            failed: *fails,
        };
        assert_eq!(
            reference(src, 0),
            want,
            "{name}: the tree-walker no longer shows what the row pins"
        );
    }
}

fn diverging_on(tier: &str, lower: impl Fn(&str) -> Result<String, String>) -> Vec<String> {
    let mut diverged = Vec::new();
    for (name, src, _, _) in CORPUS {
        let bin = match build(tier, name, lower(src)) {
            Ok(bin) => bin,
            Err(why) => {
                diverged.push(format!("{name}: {why}"));
                continue;
            }
        };
        for &seed in SEEDS {
            let (want, got) = (reference(src, seed), run(&bin, seed));
            if got != want {
                diverged.push(format!(
                    "{name} at seed {seed}:\n  interpreter {want:?}\n  {tier:<11} {got:?}"
                ));
                break;
            }
        }
    }
    diverged
}

#[test]
fn the_stepped_tier_runs_every_program_as_the_interpreter_does() {
    if !common::require_tool("gcc") {
        return;
    }
    let diverged = diverging_on("stepped", |src| {
        compile_to_c_stepped(src).map_err(|e| format!("does not lower: {e:?}"))
    });
    assert!(
        diverged.is_empty(),
        "{} of {} programs diverge on the stepped tier:\n\n{}",
        diverged.len(),
        CORPUS.len(),
        diverged.join("\n\n")
    );
}

#[test]
fn host_c_runs_every_program_as_the_interpreter_does() {
    if !common::require_tool("gcc") {
        return;
    }
    let diverged = diverging_on("host C", |src| {
        compile_to_c(src).map_err(|e| format!("does not lower: {e:?}"))
    });
    assert!(
        diverged.is_empty(),
        "{} of {} programs diverge on host C:\n\n{}",
        diverged.len(),
        CORPUS.len(),
        diverged.join("\n\n")
    );
}

/// The rows whose integers outgrow a word, on both tiers, collecting before every allocation: an integer past a word
/// parked in a task's argument or in a waiting call's argument or result cell is held through the frame that holds the
/// cell, so every collection the run makes leaves it whole.
#[test]
fn integers_past_a_word_survive_a_collection_at_every_allocation() {
    if !common::require_tool("gcc") {
        return;
    }
    let rows: Vec<_> = CORPUS
        .iter()
        .filter(|(name, ..)| name.ends_with("_past_a_word"))
        .collect();
    assert_eq!(rows.len(), 3, "the rows whose integers outgrow a word");
    let mut diverged = Vec::new();
    for (name, src, _, _) in rows {
        let tiers = [
            (
                "stepped",
                compile_to_c_stepped(src).map_err(|e| format!("does not lower: {e:?}")),
            ),
            (
                "host C",
                compile_to_c(src).map_err(|e| format!("does not lower: {e:?}")),
            ),
        ];
        for (tier, c) in tiers {
            let bin = match build(tier, name, c) {
                Ok(bin) => bin,
                Err(why) => {
                    diverged.push(format!("{name} on {tier}: {why}"));
                    continue;
                }
            };
            let (want, got) = (
                reference(src, 0),
                run_with(&bin, 0, &[("LOGOS_GC_STRESS", "1")]),
            );
            if got != want {
                diverged.push(format!(
                    "{name} on {tier}, collecting at every allocation:\n  interpreter {want:?}\n  {tier:<11} {got:?}"
                ));
            }
        }
    }
    assert!(diverged.is_empty(), "{}", diverged.join("\n\n"));
}
