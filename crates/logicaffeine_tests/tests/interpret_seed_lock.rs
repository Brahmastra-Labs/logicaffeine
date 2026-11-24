//! `largo run --interpret` SCHEDULES A CONCURRENT PROGRAM AT THE SEED IT IS GIVEN, ON EVERY ENGINE.
//!
//! A concurrent program is a deterministic function of its source and its seed: the tree-walker and
//! the VM both run it on `logicaffeine_runtime`'s scheduler, and `run_treewalker_concurrent_seeded` /
//! `run_vm_concurrent_seeded` are what it prints at each seed (`c_family_scheduler_conformance_lock`
//! holds the two to each other and every C-family tier to them). The command line is the way a person
//! reaches that function, so it must reach the same one: `--seed S` and `LOGOS_SEED=S` each run the
//! program at seed `S` on the default engine, the tree-walker, the VM and the JIT, and print exactly
//! what the library prints at `S` — the same lines, failing exactly when it fails. A run that names no
//! seed is scheduled at seed 0. An engine that ignored the seed would print one schedule for every
//! seed, which is why the seed-sensitive programs here must show more than one outcome across them.
//!
//! The scheduler serves tasks, pipes, selects and the clock, not the host. A concurrent program that
//! writes a file needs the reactor only the tree-walker's async executor drives, so the VM and the JIT
//! refuse it in words that say so, and the default engine runs it.
#![cfg(not(target_arch = "wasm32"))]

mod cli_common;

use cli_common::{largo, scaffold};
use logicaffeine_compile::{run_treewalker_concurrent_seeded, run_vm_concurrent_seeded};
use std::path::Path;

/// The seeds every program runs at.
const SEEDS: &[u64] = &[0, 1, 2, 3, 7, 42];

/// Every engine `largo run --interpret` offers, as the arguments that select it (the first is the
/// default, which names none).
const ENGINES: &[&[&str]] = &[&[], &["--engine", "tree"], &["--engine", "vm"], &["--engine", "jit"]];

/// A concurrent program and whether what it prints depends on the seed. Some wait on the logical
/// clock (`Sleep`), which the run path treats as asynchronous; some only touch pipes and tasks; some
/// fail, so the seed must decide a failure as faithfully as it decides output.
const CORPUS: &[(&str, &str, bool)] = &[
    (
        "same_tick_sleepers_wake_in_the_seeds_order",
        "## To nap (n: Int):\n    Sleep 5.\n    Show n.\n\n## Main\nLaunch a task to nap with 1.\n\
         Launch a task to nap with 2.\nLaunch a task to nap with 3.\nLaunch a task to nap with 4.\n",
        true,
    ),
    (
        "two_tasks_looping_at_once_keep_their_own_places",
        "## To count (tag: Int):\n    Repeat for i from 1 to 3:\n        Show \"{tag}: {i}\".\n        Sleep 1.\n\n\
         ## Main\nLaunch a task to count with 1.\nLaunch a task to count with 2.\nRepeat for k from 1 to 2:\n    \
         Show \"main {k}\".\n    Sleep 1.\n",
        true,
    ),
    (
        "a_select_race_breaks_by_the_seed",
        "## Main\nLet a be a Pipe of Int.\nLet b be a Pipe of Int.\nSend 1 into a.\nSend 2 into b.\n\
         Await the first of:\n    Receive x from a:\n        Show \"a {x}\".\n    Receive y from b:\n        \
         Show \"b {y}\".\n",
        true,
    ),
    (
        "a_receiver_resumes_before_the_sender_it_frees",
        "## To produce (ch: Int):\n    Repeat for i from 1 to 34:\n        Send i into ch.\n    Show \"produced\".\n\n\
         ## To consume (ch: Int):\n    Receive a from ch.\n    Show \"consumed {a}\".\n\n## Main\n\
         Let ch be a Pipe of Int.\nLaunch a task to produce with ch.\nLaunch a task to consume with ch.\n\
         Show \"main\".\nReceive b from ch.\nShow \"main got {b}\".\n",
        false,
    ),
    (
        "a_task_left_waiting_is_a_deadlock",
        "## To wait (ch: Int):\n    Receive x from ch.\n    Show \"never\".\n\n## Main\nLet ch be a Pipe of Int.\n\
         Launch a task to wait with ch.\nShow \"main done\".\n",
        false,
    ),
    (
        "a_failing_task_fails_the_run_and_the_others_finish",
        "## To fail:\n    Show \"failing\".\n    Require that 1 equals 2.\n    Show \"never\".\n\n## To finish:\n    \
         Show \"finishing\".\n\n## Main\nLaunch a task to fail.\nLaunch a task to finish.\nShow \"main\".\n",
        false,
    ),
];

/// What a run printed and whether it failed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Outcome {
    lines: Vec<String>,
    failed: bool,
}

/// What the library prints at `seed`: the tree-walker's outcome, which the VM's must equal.
fn library(name: &str, src: &str, seed: u64) -> Outcome {
    let tw = run_treewalker_concurrent_seeded(src, seed);
    let vm = run_vm_concurrent_seeded(src, seed);
    let tw = Outcome { lines: tw.lines, failed: tw.error.is_some() };
    let vm = Outcome { lines: vm.lines, failed: vm.error.is_some() };
    assert_eq!(tw, vm, "{name}: the library's tree-walker and VM disagree at seed {seed}:\n{src}");
    tw
}

/// How a run is told its seed.
#[derive(Clone, Copy, Debug)]
enum Told {
    Flag(u64),
    Env(u64),
    Nothing,
}

fn run(dir: &Path, engine: &[&str], told: Told) -> Outcome {
    let mut cmd = largo();
    cmd.current_dir(dir).args(["run", "--interpret"]).args(engine).env_remove("LOGOS_SEED");
    match told {
        Told::Flag(seed) => {
            cmd.args(["--seed", &seed.to_string()]);
        }
        Told::Env(seed) => {
            cmd.env("LOGOS_SEED", seed.to_string());
        }
        Told::Nothing => {}
    }
    let out = cmd.output().expect("largo should spawn");
    Outcome {
        lines: String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect(),
        failed: !out.status.success(),
    }
}

fn project(root: &Path, name: &str, src: &str) -> std::path::PathBuf {
    let dir = root.join(name);
    scaffold(&dir, name);
    std::fs::write(dir.join("src/main.lg"), src).unwrap();
    dir
}

/// Every corpus program on one engine, at every seed, told by flag, by environment, and not at all.
fn engine_schedules_at_the_seed_it_is_given(engine: &[&str]) {
    let root = tempfile::tempdir().unwrap();
    let mut wrong = Vec::new();
    for (name, src, seed_sensitive) in CORPUS {
        let dir = project(root.path(), name, src);
        let at: Vec<Outcome> = SEEDS.iter().map(|&seed| library(name, src, seed)).collect();
        if *seed_sensitive {
            let mut distinct = at.clone();
            distinct.sort();
            distinct.dedup();
            assert!(distinct.len() > 1, "{name} is listed as seed-sensitive but prints one outcome at every seed");
        }
        for (&seed, expected) in SEEDS.iter().zip(&at) {
            for told in [Told::Flag(seed), Told::Env(seed)] {
                let got = run(&dir, engine, told);
                if &got != expected {
                    wrong.push(format!(
                        "`largo run --interpret {}` told {told:?} on `{name}`:\n  expected {expected:?}\n  got      {got:?}",
                        engine.join(" ")
                    ));
                }
            }
        }
        let got = run(&dir, engine, Told::Nothing);
        if got != at[0] {
            wrong.push(format!(
                "`largo run --interpret {}` told no seed on `{name}`: expected seed 0's {:?}, got {got:?}",
                engine.join(" "),
                at[0]
            ));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n\n"));
}

#[test]
fn the_default_engine_schedules_at_the_seed_it_is_given() {
    engine_schedules_at_the_seed_it_is_given(ENGINES[0]);
}

#[test]
fn the_tree_walker_schedules_at_the_seed_it_is_given() {
    engine_schedules_at_the_seed_it_is_given(ENGINES[1]);
}

#[test]
fn the_vm_schedules_at_the_seed_it_is_given() {
    engine_schedules_at_the_seed_it_is_given(ENGINES[2]);
}

#[test]
fn the_jit_schedules_at_the_seed_it_is_given() {
    engine_schedules_at_the_seed_it_is_given(ENGINES[3]);
}

/// A concurrent program that reaches the host: a task writes a file.
const REACHES_THE_HOST: &str = "## To save:\n    Write \"saved\" to file \"out.txt\".\n    Show \"wrote\".\n\n\
     ## Main\nLaunch a task to save.\nShow \"main\".\n";

#[test]
fn the_vm_and_the_jit_refuse_a_program_that_reaches_the_host_and_say_why() {
    let root = tempfile::tempdir().unwrap();
    let dir = project(root.path(), "reaches_the_host", REACHES_THE_HOST);
    for engine in &ENGINES[2..] {
        let out = largo()
            .current_dir(&dir)
            .args(["run", "--interpret"])
            .args(*engine)
            .env_remove("LOGOS_SEED")
            .output()
            .expect("largo should spawn");
        let said = cli_common::strip_ansi(&cli_common::stderr(&out));
        assert!(
            !out.status.success()
                && out.stdout.is_empty()
                && said.contains("not the host")
                && said.contains("--engine tree"),
            "`largo run --interpret {}` must refuse a program that writes a file, naming the host and the \
             engine that runs it; exit {:?}, stdout {:?}, stderr:\n{said}",
            engine.join(" "),
            out.status.code(),
            String::from_utf8_lossy(&out.stdout)
        );
    }
    let out = largo()
        .current_dir(&dir)
        .args(["run", "--interpret"])
        .env_remove("LOGOS_SEED")
        .output()
        .expect("largo should spawn");
    let mut shown: Vec<String> =
        String::from_utf8_lossy(&out.stdout).lines().map(str::to_string).collect();
    shown.sort();
    assert!(
        out.status.success() && shown == ["main", "wrote"],
        "the default engine runs it: exit {:?}, stdout {shown:?}, stderr:\n{}",
        out.status.code(),
        cli_common::stderr(&out)
    );
}
