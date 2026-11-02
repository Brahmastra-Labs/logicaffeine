//! A Mode-B build replays the interpreter: at every seed, the compiled binary prints what the
//! tree-walker and the VM print.
//!
//! Mode B (`compile_program_full_deterministic`) runs a program's tasks, pipes, timers and selects on
//! `logicaffeine_system::replay`, the interpreter's scheduler ported (the port itself is locked
//! step for step by `replay_executor_port_lock`). This lock holds the whole path to that promise: the
//! lowering performs every operation where the interpreter performs it, so a launched task runs before
//! its parent resumes, values move through pipes in the same order, a select resolves to the same arm
//! and a same-tick timer tie breaks the same way — each at the seed `LOGOS_SEED` names.
//!
//! The corpus covers every scheduler operation the language has. The two reference tiers must agree on
//! each program first, so the target is unambiguous; the programs whose output depends on the seed must
//! show it, so the seed is proven to reach the binary. Every program is built as a binary of ONE cargo
//! package, so the dependency graph builds once.

use std::path::{Path, PathBuf};
use std::process::Command;

use logicaffeine_compile::{
    compile_program_full_deterministic, run_treewalker_concurrent_seeded, run_vm_concurrent_seeded,
};

const SEEDS: &[u64] = &[0, 1, 2, 3, 7, 42];

/// `(name, source, seed_sensitive)` — a seed-sensitive program must print more than one output
/// across `SEEDS`.
const CORPUS: &[(&str, &str, bool)] = &[
    (
        "studio_tasks",
        "## To worker:\n    Show \"worker done\".\n\n## To greet (name: Text):\n    Show \"Hello from {name}\".\n\n\
         ## Main\n\nLaunch a task to worker.\nShow \"main continues\".\n\nLaunch a task to greet with \"a task\".\n\n\
         Let handle be Launch a task to worker.\nStop handle.\nShow \"second worker stopped\".\n",
        false,
    ),
    (
        "launch_with_arguments",
        "## To report (name: Text, n: Int):\n    Show \"{name} {n}\".\n\n## Main\nLet who be \"ada\".\n\
         Launch a task to report with who and 7.\nShow \"after {who}\".\n",
        false,
    ),
    (
        "launch_in_a_loop",
        "## To shout (n: Int):\n    Show n * 10.\n\n## Main\nRepeat for i from 1 to 4:\n    Launch a task to shout with i.\n    Show i.\n",
        false,
    ),
    (
        "a_task_launches_a_task",
        "## To leaf (n: Int):\n    Show \"leaf {n}\".\n\n## To branch (n: Int):\n    Show \"branch {n}\".\n    \
         Launch a task to leaf with n + 1.\n    Show \"branch {n} done\".\n\n## Main\nLaunch a task to branch with 1.\nShow \"main\".\n",
        false,
    ),
    (
        "producer_consumer",
        "## To produce (ch: Int):\n    Send 1 into ch.\n    Show \"sent 1\".\n    Send 2 into ch.\n    Show \"sent 2\".\n    \
         Send 3 into ch.\n    Show \"sent 3\".\n\n## Main\nLet jobs be a Pipe of Int.\nLaunch a task to produce with jobs.\n\
         Receive a from jobs.\nShow a.\nReceive b from jobs.\nShow b.\nReceive c from jobs.\nShow c.\n",
        false,
    ),
    (
        "ping_pong",
        "## To pong (inbox: Int, outbox: Int):\n    Repeat for i from 1 to 3:\n        Receive v from inbox.\n        \
         Show \"pong {v}\".\n        Send v + 100 into outbox.\n\n## Main\nLet up be a Pipe of Int.\nLet down be a Pipe of Int.\n\
         Launch a task to pong with up and down.\nRepeat for i from 1 to 3:\n    Send i into up.\n    Receive r from down.\n    Show \"ping got {r}\".\n",
        false,
    ),
    (
        "text_through_a_pipe",
        "## To speak (ch: Text):\n    Send \"hello\" into ch.\n    Send \"world\" into ch.\n\n## Main\nLet words be a Pipe of Text.\n\
         Launch a task to speak with words.\nReceive first from words.\nReceive second from words.\nShow \"{first} {second}\".\n",
        false,
    ),
    (
        "a_record_through_a_pipe",
        "## A Point has:\n    An x: Int.\n    A y: Int.\n\n## To make (ch: Point):\n    Let p be a new Point with x 3 and y 4.\n    Send p into ch.\n\n\
         ## Main\nLet points be a Pipe of Point.\nLaunch a task to make with points.\nReceive p from points.\nShow p's x + p's y.\n",
        false,
    ),
    (
        "fan_in_order",
        "## To emit (ch: Int, n: Int):\n    Show \"emit {n}\".\n    Send n into ch.\n\n## Main\nLet ch be a Pipe of Int.\n\
         Launch a task to emit with ch and 10.\nLaunch a task to emit with ch and 20.\nLaunch a task to emit with ch and 30.\n\
         Receive first from ch.\nReceive second from ch.\nReceive third from ch.\nShow \"{first} {second} {third}\".\n",
        false,
    ),
    (
        "select_race",
        "## Main\nLet a be a Pipe of Int.\nLet b be a Pipe of Int.\nSend 1 into a.\nSend 2 into b.\nAwait the first of:\n    \
         Receive x from a:\n        Show \"a {x}\".\n    Receive y from b:\n        Show \"b {y}\".\n",
        true,
    ),
    (
        "select_race_three_times",
        "## Main\nLet a be a Pipe of Int.\nLet b be a Pipe of Int.\nRepeat for i from 1 to 3:\n    Send i into a.\n    \
         Send i * 10 into b.\nRepeat for i from 1 to 3:\n    Await the first of:\n        Receive x from a:\n            \
         Show \"a {x}\".\n        Receive y from b:\n            Show \"b {y}\".\n",
        true,
    ),
    (
        "select_waits_for_a_sender",
        "## To feed (ch: Int):\n    Sleep 3.\n    Send 5 into ch.\n\n## Main\nLet ch be a Pipe of Int.\nLaunch a task to feed with ch.\n\
         Await the first of:\n    Receive x from ch:\n        Show \"got {x}\".\n    After 10 seconds:\n        Show \"timed out\".\n",
        false,
    ),
    (
        "select_times_out",
        "## To feed (ch: Int):\n    Sleep 20 seconds.\n    Send 5 into ch.\n\n## Main\nLet ch be a Pipe of Int.\nLaunch a task to feed with ch.\n\
         Await the first of:\n    Receive x from ch:\n        Show \"got {x}\".\n    After 10 seconds:\n        Show \"timed out\".\n\
         Receive y from ch.\nShow \"then {y}\".\n",
        false,
    ),
    (
        "sleepers_wake_in_clock_order",
        "## To nap (ticks: Int, name: Text):\n    Sleep ticks.\n    Show name.\n\n## Main\nLaunch a task to nap with 30 and \"slow\".\n\
         Launch a task to nap with 10 and \"fast\".\nLaunch a task to nap with 20 and \"middle\".\nShow \"main\".\n",
        false,
    ),
    (
        "sleepers_on_the_same_tick",
        "## To nap (name: Text):\n    Sleep 5.\n    Show name.\n\n## Main\nLaunch a task to nap with \"one\".\n\
         Launch a task to nap with \"two\".\nLaunch a task to nap with \"three\".\nLaunch a task to nap with \"four\".\n",
        true,
    ),
    (
        "try_operations",
        "## Main\nLet ch be a Pipe of Int.\nTry to send 7 into ch.\nTry to receive y from ch.\nShow y.\nTry to receive z from ch.\nShow z.\n",
        false,
    ),
    (
        "stop_a_blocked_task",
        "## To idle (ch: Int):\n    Show \"waiting\".\n    Receive v from ch.\n    Show \"woke with {v}\".\n\n## Main\n\
         Let ch be a Pipe of Int.\nLet h be Launch a task to idle with ch.\nStop h.\nSend 1 into ch.\nShow \"stopped\".\n",
        false,
    ),
    (
        "await_a_task",
        "## To slow:\n    Sleep 4.\n    Show \"slow done\".\n\n## Main\nLet h be Launch a task to slow.\nShow \"awaiting\".\n\
         Await h.\nShow \"joined\".\n",
        false,
    ),
    (
        "await_into_a_result",
        "## To worker -> Int:\n    Return 5.\n\n## Main\nLet h be Launch a task to worker.\nAwait h into r.\nShow r.\n",
        false,
    ),
    (
        "close_keeps_what_was_sent",
        "## To fill (ch: Int):\n    Send 1 into ch.\n    Send 2 into ch.\n    Close ch.\n    Show \"closed\".\n\n## Main\n\
         Let ch be a Pipe of Int.\nLaunch a task to fill with ch.\nReceive x from ch.\nReceive y from ch.\nShow x + y.\n\
         Try to receive z from ch.\nShow z.\n",
        false,
    ),
    (
        "attempt_all_runs_in_order",
        "## To worker (n: Int):\n    Show \"worker {n}\".\n\n## Main\nLet ch be a Pipe of Int.\nAttempt all of the following:\n    \
         Launch a task to worker with 1.\n    Launch a task to worker with 2.\nShow \"after\".\n",
        false,
    ),
    (
        "a_failing_task_fails_the_run",
        "## To bad (n: Int):\n    Show \"bad starts\".\n    Show 10 / n.\n\n## Main\nLaunch a task to bad with 0.\nShow \"main goes on\".\n",
        false,
    ),
    (
        "select_names_one_pipe_twice",
        "## To feed (up: Int, down: Int):\n    Sleep 1.\n    Send 1 into up.\n    Send 2 into up.\n    Send 99 into down.\n\n\
         ## Main\nLet up be a Pipe of Int.\nLet down be a Pipe of Int.\nLaunch a task to feed with up and down.\n\
         Await the first of:\n    Receive x from up:\n        Show x.\n    Receive y from up:\n        Show y.\n\
         Receive z from down.\nShow z.\n",
        false,
    ),
    (
        "every_task_blocked_is_a_deadlock",
        "## Main\nLet ch be a Pipe of Int.\nShow \"before\".\nReceive x from ch.\nShow \"never\".\n",
        false,
    ),
];

struct Oracle {
    lines: Vec<String>,
    ok: bool,
}

/// The reference behaviour at `seed`: the tree-walker's, which the VM must match.
fn oracle(name: &str, source: &str, seed: u64) -> Oracle {
    let tw = run_treewalker_concurrent_seeded(source, seed);
    let vm = run_vm_concurrent_seeded(source, seed);
    assert_eq!(
        (&tw.lines, tw.error.is_none()),
        (&vm.lines, vm.error.is_none()),
        "{name} at seed {seed}: the tree-walker and the VM disagree (tw error {:?}, vm error {:?}), so the target is ambiguous",
        tw.error,
        vm.error
    );
    Oracle {
        lines: tw.lines,
        ok: tw.error.is_none(),
    }
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// Compile every program in Mode B and build them all as the binaries of one package.
fn build(programs: &[(&str, &str)]) -> std::collections::BTreeMap<String, PathBuf> {
    let root = workspace_root();
    let project = std::env::temp_dir().join(format!("logos_replay_corpus_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&project);
    std::fs::create_dir_all(project.join("src/bin")).expect("project dir");
    std::fs::write(project.join("src/lib.rs"), "").expect("lib.rs");
    let mut deps = std::collections::BTreeSet::new();
    for (name, source) in programs {
        let out = compile_program_full_deterministic(source)
            .unwrap_or_else(|e| panic!("{name}: Mode B refused the program: {e:?}"));
        for dep in &out.dependencies {
            deps.insert(format!("{} = \"{}\"\n", dep.name, dep.version));
        }
        std::fs::write(
            project.join("src/bin").join(format!("{name}.rs")),
            &out.rust_code,
        )
        .expect("write bin");
    }
    let manifest = format!(
        "[package]\nname = \"logos_replay_corpus\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n\
         [dependencies]\n\
         logicaffeine-data = {{ path = \"{root}/crates/logicaffeine_data\" }}\n\
         logicaffeine-system = {{ path = \"{root}/crates/logicaffeine_system\", features = [\"full\"] }}\n\
         tokio = {{ version = \"1\", features = [\"rt-multi-thread\", \"macros\"] }}\n\
         serde = {{ version = \"1\", features = [\"derive\"] }}\n\
         rayon = \"1\"\nmemchr = \"2\"\n{}",
        deps.into_iter().collect::<String>(),
        root = root.display(),
    );
    std::fs::write(project.join("Cargo.toml"), manifest).expect("Cargo.toml");
    std::fs::copy(root.join("Cargo.lock"), project.join("Cargo.lock")).expect("seed Cargo.lock");
    let target = std::env::temp_dir().join("logos_replay_corpus_target");
    let out = Command::new("cargo")
        .args(["build", "--quiet", "--offline", "--bins"])
        .current_dir(&project)
        .env("CARGO_TARGET_DIR", &target)
        .output()
        .expect("cargo runs");
    assert!(
        out.status.success(),
        "the Mode-B corpus does not build:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let _ = std::fs::remove_dir_all(&project);
    programs
        .iter()
        .map(|(name, _)| (name.to_string(), target.join("debug").join(name)))
        .collect()
}

#[test]
fn e2e_every_program_replays_the_interpreter_at_every_seed() {
    let programs: Vec<(&str, &str)> = CORPUS.iter().map(|(n, s, _)| (*n, *s)).collect();
    let bins = build(&programs);
    let mut failures = Vec::new();
    for (name, source, seed_sensitive) in CORPUS {
        let mut outputs = std::collections::BTreeSet::new();
        for &seed in SEEDS {
            let want = oracle(name, source, seed);
            outputs.insert(want.lines.join("\n"));
            let run = Command::new(&bins[*name])
                .env("LOGOS_SEED", seed.to_string())
                .output()
                .expect("the binary runs");
            let got: Vec<String> = String::from_utf8_lossy(&run.stdout)
                .lines()
                .map(str::to_string)
                .collect();
            if got != want.lines || run.status.success() != want.ok {
                failures.push(format!(
                    "{name} at seed {seed}:\n  interpreter ({}): {:?}\n  Mode B ({}): {:?}\n  stderr: {}",
                    if want.ok { "ok" } else { "failed" },
                    want.lines,
                    if run.status.success() { "ok" } else { "failed" },
                    got,
                    String::from_utf8_lossy(&run.stderr).trim()
                ));
            }
        }
        if *seed_sensitive {
            assert!(
                outputs.len() > 1,
                "{name} is meant to depend on the seed, but printed one output at every seed"
            );
        }
    }
    assert!(
        failures.is_empty(),
        "Mode B diverged from the interpreter:\n\n{}",
        failures.join("\n\n")
    );
}
