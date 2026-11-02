//! Every C-family tier schedules a concurrent program as the interpreter does, at every seed.
//!
//! The tree-walker and the VM both run on `logicaffeine_runtime`'s scheduler; the C family runs on its
//! port, `codegen_c::SCHED_CORE` (held to the runtime step for step by `c_scheduler_port_lock`). This
//! lock holds the whole path: each tier's lowering performs every operation where the interpreter
//! performs it — a launched task runs before its parent resumes, a `Sleep` parks on the logical clock,
//! creating a pipe is a scheduling point, a default pipe holds thirty-two values, a select resolves to
//! the arm the seed picks — and a run fails exactly when the interpreter's does: a task left waiting
//! forever is a deadlock, and a failing task fails the run while every other task still finishes (on a GPU
//! thread, a failure inside a runtime helper traps the whole run instead — `CUDA_TRAPS`).
//!
//! Each program runs on the two reference tiers first (they must agree, so the target is unambiguous),
//! then on each C-family tier at every seed, which it reads from `LOGOS_SEED`. The seed-sensitive rows
//! must show more than one outcome across the seeds, so the seed is proven to reach the binary.

mod common;

use std::path::PathBuf;
use std::process::Command;

use logicaffeine_compile::codegen_riscv::RISCV_RUNTIME_C;
use logicaffeine_compile::compile::{
    compile_to_c, compile_to_c_stepped, compile_to_c_via_bytecode, compile_to_cuda,
    compile_to_riscv,
};
use logicaffeine_compile::{run_treewalker_concurrent_seeded, run_vm_concurrent_seeded};

/// The seeds every program runs at; the first is the one each row pins.
const SEEDS: &[u64] = &[0, 1, 2, 3, 7, 42];

/// A corpus program: its source, whether its outcome depends on the seed, and its outcome at seed 0 on
/// the interpreter — what the name says it shows, pinned, so a row that stops showing it goes red.
struct Row {
    name: &'static str,
    src: &'static str,
    seed_sensitive: bool,
    at_seed_0: &'static [&'static str],
    fails: bool,
}

const CORPUS: &[Row] = &[
    Row {
        name: "a_sleeping_task_wakes_after_its_parent",
        src: "## To nap:\n    Sleep 5.\n    Show \"napped\".\n\n## Main\nLaunch a task to nap.\nShow \"main\".\n",
        seed_sensitive: false,
        at_seed_0: &["main", "napped"],
        fails: false,
    },
    Row {
        name: "a_task_launches_one_declared_after_it",
        src: "## To first:\n    Show \"first\".\n    Launch a task to second.\n    Show \"first done\".\n\n\
         ## To second:\n    Show \"second\".\n\n## Main\nLaunch a task to first.\nShow \"main\".\n",
        seed_sensitive: false,
        at_seed_0: &["first", "main", "second", "first done"],
        fails: false,
    },
    Row {
        name: "a_select_naming_one_pipe_twice_leaves_no_registration_behind",
        src: "## To relay (start: Int, up: Int, down: Int):\n    Receive go from start.\n    Send 1 into up.\n    Send 2 into up.\n    \
         Send 99 into down.\n\n## To kick (start: Int):\n    Send 0 into start.\n\n## Main\nLet start be a Pipe of Int.\n\
         Let up be a Pipe of Int.\nLet down be a Pipe of Int.\nLaunch a task to relay with start, up and down.\n\
         Launch a task to kick with start.\nAwait the first of:\n    Receive x from up:\n        Show x.\n    \
         Receive y from up:\n        Show y.\nReceive z from down.\nShow z.\n",
        seed_sensitive: false,
        at_seed_0: &["1", "99"],
        fails: false,
    },
    Row {
        name: "a_select_waits_on_two_pipes_at_once",
        src: "## To relay (start: Int, left: Int, right: Int):\n    Receive go from start.\n    Send 5 into right.\n    \
         Send 6 into left.\n\n## To kick (start: Int):\n    Send 0 into start.\n\n## To other (left: Int):\n    \
         Receive w from left.\n    Show \"other got {w}\".\n\n## Main\nLet start be a Pipe of Int.\nLet left be a Pipe of Int.\n\
         Let right be a Pipe of Int.\nLaunch a task to relay with start, left and right.\nLaunch a task to kick with start.\n\
         Await the first of:\n    Receive x from left:\n        Show \"left {x}\".\n    Receive y from right:\n        \
         Show \"right {y}\".\nShow \"main done\".\n",
        seed_sensitive: false,
        at_seed_0: &["right 5", "main done"],
        fails: false,
    },
    Row {
        name: "closing_a_pipe_resolves_a_waiting_select",
        src: "## To watch (ch: Int):\n    Await the first of:\n        Receive x from ch:\n            Show \"received\".\n        \
         After 50 seconds:\n            Show \"timed out\".\n    Show \"watch done\".\n\n## Main\nLet ch be a Pipe of Int.\n\
         Launch a task to watch with ch.\nClose ch.\nShow \"closed\".\n",
        seed_sensitive: false,
        at_seed_0: &["received", "watch done", "closed"],
        fails: false,
    },
    Row {
        name: "a_receiver_resumes_before_the_sender_it_frees",
        src: "## To produce (ch: Int):\n    Repeat for i from 1 to 34:\n        Send i into ch.\n    Show \"produced\".\n\n\
         ## To consume (ch: Int):\n    Receive a from ch.\n    Show \"consumed {a}\".\n\n## Main\nLet ch be a Pipe of Int.\n\
         Launch a task to produce with ch.\nLaunch a task to consume with ch.\nShow \"main\".\nReceive b from ch.\nShow \"main got {b}\".\n",
        seed_sensitive: false,
        at_seed_0: &["main", "consumed 1", "main got 2", "produced"],
        fails: false,
    },
    Row {
        name: "a_select_race_breaks_by_the_seed",
        src: "## Main\nLet a be a Pipe of Int.\nLet b be a Pipe of Int.\nSend 1 into a.\nSend 2 into b.\nAwait the first of:\n    \
         Receive x from a:\n        Show \"a {x}\".\n    Receive y from b:\n        Show \"b {y}\".\n",
        seed_sensitive: true,
        at_seed_0: &["b 2"],
        fails: false,
    },
    Row {
        name: "same_tick_sleepers_wake_in_the_seeds_order",
        src: "## To nap (n: Int):\n    Sleep 5.\n    Show n.\n\n## Main\nLaunch a task to nap with 1.\nLaunch a task to nap with 2.\n\
         Launch a task to nap with 3.\nLaunch a task to nap with 4.\n",
        seed_sensitive: true,
        at_seed_0: &["4", "1", "3", "2"],
        fails: false,
    },
    Row {
        name: "sleeping_no_time_is_no_scheduling_point",
        src: "## To later:\n    Sleep 0.\n    Show \"later\".\n\n## To now:\n    Show \"now\".\n\n## Main\nLaunch a task to later.\n\
         Launch a task to now.\nShow \"main\".\n",
        seed_sensitive: false,
        at_seed_0: &["later", "now", "main"],
        fails: false,
    },
    Row {
        name: "an_awaited_task_answers_nothing",
        src: "## To worker -> Int:\n    Return 5.\n\n## Main\nLet h be Launch a task to worker.\nAwait h into r.\nShow r.\n",
        seed_sensitive: false,
        at_seed_0: &["nothing"],
        fails: false,
    },
    Row {
        name: "a_default_pipe_holds_thirty_two_values",
        src: "## To produce (ch: Int):\n    Repeat for i from 1 to 40:\n        Send i into ch.\n        Show i.\n\n## Main\n\
         Let ch be a Pipe of Int.\nLaunch a task to produce with ch.\nSleep 1.\nShow \"main\".\n",
        seed_sensitive: false,
        at_seed_0: &[
            "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16", "17",
            "18", "19", "20", "21", "22", "23", "24", "25", "26", "27", "28", "29", "30", "31", "32",
            "main",
        ],
        fails: true,
    },
    Row {
        name: "creating_a_pipe_lets_a_ready_task_run",
        src: "## To make:\n    Let p be a Pipe of Int.\n    Show \"made\".\n\n## Main\nLaunch a task to make.\nShow \"main\".\n",
        seed_sensitive: false,
        at_seed_0: &["main", "made"],
        fails: false,
    },
    Row {
        name: "a_task_left_waiting_is_a_deadlock",
        src: "## To wait (ch: Int):\n    Receive x from ch.\n    Show \"never\".\n\n## Main\nLet ch be a Pipe of Int.\n\
         Launch a task to wait with ch.\nShow \"main done\".\n",
        seed_sensitive: false,
        at_seed_0: &["main done"],
        fails: true,
    },
    Row {
        name: "a_failing_task_fails_the_run_and_the_others_finish",
        src: "## To fail:\n    Show \"failing\".\n    Require that 1 equals 2.\n    Show \"never\".\n\n## To finish:\n    \
         Show \"finishing\".\n\n## Main\nLaunch a task to fail.\nLaunch a task to finish.\nShow \"main\".\n",
        seed_sensitive: false,
        at_seed_0: &["failing", "finishing", "main"],
        fails: true,
    },
    Row {
        name: "a_zero_divisor_fails_its_task_and_the_others_finish",
        src: "## To bad (ch: Int):\n    Receive d from ch.\n    Show \"dividing\".\n    Show 10 / d.\n    Show \"never\".\n\n\
         ## To finish:\n    Show \"finishing\".\n\n## Main\nLet ch be a Pipe of Int.\nLaunch a task to bad with ch.\n\
         Launch a task to finish.\nSend 0 into ch.\nShow \"main\".\n",
        seed_sensitive: false,
        at_seed_0: &["finishing", "dividing", "main"],
        fails: true,
    },
    Row {
        name: "a_closed_drained_pipe_reads_nothing",
        src: "## Main\nLet ch be a Pipe of Int.\nSend 5 into ch.\nClose ch.\nReceive a from ch.\nShow a.\nReceive b from ch.\nShow b.\n",
        seed_sensitive: false,
        at_seed_0: &["5", "nothing"],
        fails: false,
    },
    Row {
        name: "a_closed_drained_pipe_compares_as_nothing",
        src: "## Main\nLet ch be a Pipe of Int.\nClose ch.\nReceive a from ch.\nIf a is equal to nothing:\n    Show \"drained\".\n\
         Otherwise:\n    Show \"a value\".\nIf a is not equal to nothing:\n    Show \"present\".\nOtherwise:\n    Show \"absent\".\n",
        seed_sensitive: false,
        at_seed_0: &["drained", "absent"],
        fails: false,
    },
    Row {
        name: "a_value_read_from_a_pipe_is_not_nothing",
        src: "## Main\nLet ch be a Pipe of Int.\nSend 0 into ch.\nReceive a from ch.\nIf a is equal to nothing:\n    Show \"nothing\".\n\
         Otherwise:\n    Show \"got {a}\".\n",
        seed_sensitive: false,
        at_seed_0: &["got 0"],
        fails: false,
    },
    Row {
        name: "a_select_arm_reads_nothing_from_a_closed_pipe",
        src: "## Main\nLet ch be a Pipe of Int.\nClose ch.\nAwait the first of:\n    Receive x from ch:\n        Show x.\n        \
         Show \"read {x}\".\n",
        seed_sensitive: false,
        at_seed_0: &["nothing", "read nothing"],
        fails: false,
    },
    Row {
        name: "arithmetic_on_nothing_fails_the_run",
        src: "## Main\nLet ch be a Pipe of Int.\nClose ch.\nReceive a from ch.\nShow \"before\".\nShow a + 1.\nShow \"after\".\n",
        seed_sensitive: false,
        at_seed_0: &["before"],
        fails: true,
    },
    Row {
        name: "a_copy_of_nothing_is_nothing",
        src: "## Main\nLet ch be a Pipe of Int.\nClose ch.\nReceive a from ch.\nLet b be a.\nShow b.\nSet b to 7.\nShow b.\n",
        seed_sensitive: false,
        at_seed_0: &["nothing", "7"],
        fails: false,
    },
    Row {
        name: "a_task_receives_a_real_argument_whole",
        src: "## To half (x: Real):\n    Show x / 2.0.\n\n## Main\nLaunch a task to half with 3.5.\nShow \"main\".\n",
        seed_sensitive: false,
        at_seed_0: &["1.75", "main"],
        fails: false,
    },
    Row {
        name: "a_task_takes_five_arguments",
        src: "## To total (a: Int, b: Int, c: Int, d: Int, e: Int):\n    Show a + b + c + d + e.\n\n## Main\n\
         Launch a task to total with 1, 2, 3, 4 and 5.\nShow \"main\".\n",
        seed_sensitive: false,
        at_seed_0: &["15", "main"],
        fails: false,
    },
    Row {
        name: "two_tasks_looping_at_once_keep_their_own_places",
        src: "## To count (tag: Int):\n    Repeat for i from 1 to 3:\n        Show \"{tag}: {i}\".\n        Sleep 1.\n\n## Main\n\
             Launch a task to count with 1.\nLaunch a task to count with 2.\nRepeat for k from 1 to 2:\n    Show \"main {k}\".\n    \
             Sleep 1.\n",
        seed_sensitive: true,
        at_seed_0: &["1: 1", "2: 1", "main 1", "2: 2", "1: 2", "main 2", "1: 3", "2: 3"],
        fails: false,
    },
    Row {
        name: "producer_consumer",
        src: "## To produce (ch: Int):\n    Send 1 into ch.\n    Show \"sent 1\".\n    Send 2 into ch.\n    Show \"sent 2\".\n    \
         Send 3 into ch.\n    Show \"sent 3\".\n\n## Main\nLet jobs be a Pipe of Int.\nLaunch a task to produce with jobs.\n\
         Receive a from jobs.\nShow a.\nReceive b from jobs.\nShow b.\nReceive c from jobs.\nShow c.\n",
        seed_sensitive: false,
        at_seed_0: &["sent 1", "1", "sent 2", "2", "sent 3", "3"],
        fails: false,
    },
    Row {
        name: "ping_pong",
        src: "## To pong (inbox: Int, outbox: Int):\n    Repeat for i from 1 to 3:\n        Receive v from inbox.\n        \
         Show \"pong {v}\".\n        Send v + 100 into outbox.\n\n## Main\nLet up be a Pipe of Int.\nLet down be a Pipe of Int.\n\
         Launch a task to pong with up and down.\nRepeat for i from 1 to 3:\n    Send i into up.\n    Receive r from down.\n    \
         Show \"ping got {r}\".\n",
        seed_sensitive: false,
        at_seed_0: &["pong 1", "ping got 101", "pong 2", "ping got 102", "pong 3", "ping got 103"],
        fails: false,
    },
    Row {
        name: "fan_in_order",
        src: "## To emit (ch: Int, n: Int):\n    Show \"emit {n}\".\n    Send n into ch.\n\n## Main\nLet ch be a Pipe of Int.\n\
         Launch a task to emit with ch and 10.\nLaunch a task to emit with ch and 20.\nLaunch a task to emit with ch and 30.\n\
         Receive first from ch.\nReceive second from ch.\nReceive third from ch.\nShow \"{first} {second} {third}\".\n",
        seed_sensitive: false,
        at_seed_0: &["emit 10", "emit 20", "emit 30", "10 20 30"],
        fails: false,
    },
    Row {
        name: "a_select_race_three_times",
        src: "## Main\nLet a be a Pipe of Int.\nLet b be a Pipe of Int.\nRepeat for i from 1 to 3:\n    Send i into a.\n    \
         Send i * 10 into b.\nRepeat for i from 1 to 3:\n    Await the first of:\n        Receive x from a:\n            \
         Show \"a {x}\".\n        Receive y from b:\n            Show \"b {y}\".\n",
        seed_sensitive: true,
        at_seed_0: &["b 10", "a 1", "b 20"],
        fails: false,
    },
    Row {
        name: "a_select_times_out_on_the_logical_clock",
        src: "## To feed (ch: Int):\n    Sleep 20 seconds.\n    Send 5 into ch.\n\n## Main\nLet ch be a Pipe of Int.\n\
         Launch a task to feed with ch.\nAwait the first of:\n    Receive x from ch:\n        Show \"got {x}\".\n    \
         After 10 seconds:\n        Show \"timed out\".\nReceive y from ch.\nShow \"then {y}\".\n",
        seed_sensitive: false,
        at_seed_0: &["timed out", "then 5"],
        fails: false,
    },
    Row {
        name: "stopping_a_blocked_sender_drops_its_value",
        src: "## To give (ch: Int):\n    Repeat for i from 1 to 40:\n        Send i into ch.\n    Show \"gave all\".\n\n## Main\n\
         Let ch be a Pipe of Int.\nLet h be Launch a task to give with ch.\nSleep 1.\nStop h.\nRepeat for i from 1 to 33:\n    \
         Try to receive x from ch.\n    Show x.\n",
        seed_sensitive: false,
        at_seed_0: &[
            "1", "2", "3", "4", "5", "6", "7", "8", "9", "10", "11", "12", "13", "14", "15", "16", "17",
            "18", "19", "20", "21", "22", "23", "24", "25", "26", "27", "28", "29", "30", "31", "32",
            "nothing",
        ],
        fails: false,
    },
    Row {
        name: "a_pipe_of_real_carries_its_fraction",
        src: "## To consume (ch: Real):\n    Receive x from ch.\n    Show x.\n\n## Main\nLet ch be a Pipe of Real.\n\
         Launch a task to consume with ch.\nSend 1.5 into ch.\n",
        seed_sensitive: false,
        at_seed_0: &["1.5"],
        fails: false,
    },
    Row {
        name: "a_received_real_does_real_arithmetic",
        src: "## To consume (ch: Real):\n    Receive x from ch.\n    Show x * 2.0.\n    Show x + 1.\n\n## Main\n\
         Let ch be a Pipe of Real.\nLaunch a task to consume with ch.\nSend 1.5 into ch.\n",
        seed_sensitive: false,
        at_seed_0: &["3", "2.5"],
        fails: false,
    },
    Row {
        name: "a_pipe_of_text_carries_its_text",
        src: "## To consume (ch: Text):\n    Receive x from ch.\n    Show x.\n\n## Main\nLet ch be a Pipe of Text.\n\
         Launch a task to consume with ch.\nSend \"hello\" into ch.\n",
        seed_sensitive: false,
        at_seed_0: &["hello"],
        fails: false,
    },
    Row {
        name: "a_pipe_of_bool_carries_its_truth",
        src: "## To consume (ch: Bool):\n    Receive x from ch.\n    Show x.\n\n## Main\nLet ch be a Pipe of Bool.\n\
         Launch a task to consume with ch.\nSend true into ch.\n",
        seed_sensitive: false,
        at_seed_0: &["true"],
        fails: false,
    },
    Row {
        name: "a_closed_pipe_of_real_reads_its_value_then_nothing",
        src: "## Main\nLet ch be a Pipe of Real.\nSend 2.5 into ch.\nClose ch.\nReceive a from ch.\nReceive b from ch.\n\
         Show a.\nShow b.\n",
        seed_sensitive: false,
        at_seed_0: &["2.5", "nothing"],
        fails: false,
    },
    Row {
        name: "a_select_arm_reads_text",
        src: "## Main\nLet ch be a Pipe of Text.\nSend \"hi\" into ch.\nAwait the first of:\n    Receive t from ch:\n        \
         Show t.\n    After 5 seconds:\n        Show \"late\".\n",
        seed_sensitive: false,
        at_seed_0: &["hi"],
        fails: false,
    },
];

/// What a run printed and whether it failed.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct Outcome {
    lines: Vec<String>,
    failed: bool,
}

fn oracle(name: &str, src: &str, seed: u64) -> Outcome {
    let tw = run_treewalker_concurrent_seeded(src, seed);
    let vm = run_vm_concurrent_seeded(src, seed);
    let tw = Outcome {
        lines: tw.lines,
        failed: tw.error.is_some(),
    };
    let vm = Outcome {
        lines: vm.lines,
        failed: vm.error.is_some(),
    };
    assert_eq!(tw, vm, "{name}: the tree-walker and the VM disagree at seed {seed} — the target is ambiguous:\n{src}");
    tw
}

fn scratch(tier: &str, name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("logos_cfam_{}_{tier}_{name}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn outcome_of(run: std::process::Output) -> Outcome {
    Outcome {
        lines: String::from_utf8_lossy(&run.stdout)
            .lines()
            .map(str::to_string)
            .collect(),
        failed: !run.status.success(),
    }
}

/// The program built for host C, run at `seed`.
fn run_c(bin: &std::path::Path, seed: u64) -> Outcome {
    outcome_of(
        Command::new(bin)
            .env("LOGOS_SEED", seed.to_string())
            .output()
            .expect("the binary runs"),
    )
}

/// The program built for RISC-V, run under `qemu-riscv64` at `seed`.
fn run_riscv(bin: &std::path::Path, seed: u64) -> Outcome {
    outcome_of(
        Command::new("qemu-riscv64")
            .arg(bin)
            .env("LOGOS_SEED", seed.to_string())
            .output()
            .expect("qemu-riscv64 runs the binary"),
    )
}

fn build_c(name: &str, src: &str) -> Result<PathBuf, String> {
    let c = compile_to_c(src).map_err(|e| format!("does not lower to C: {e:?}"))?;
    let dir = scratch("c", name);
    let file = dir.join("prog.c");
    std::fs::write(&file, &c).unwrap();
    let bin = dir.join("prog");
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

fn build_riscv(name: &str, src: &str) -> Result<PathBuf, String> {
    let asm = compile_to_riscv(src).map_err(|e| format!("does not lower to RISC-V: {e:?}"))?;
    let dir = scratch("riscv", name);
    let (main, runtime, bin) = (dir.join("main.s"), dir.join("logos_rt.c"), dir.join("prog"));
    std::fs::write(&main, &asm).unwrap();
    std::fs::write(&runtime, RISCV_RUNTIME_C).unwrap();
    let out = Command::new("riscv64-linux-gnu-gcc")
        .args(["-O2", "-static", "-o"])
        .arg(&bin)
        .arg(&main)
        .arg(&runtime)
        .arg("-lm")
        .output()
        .unwrap();
    if !out.status.success() {
        return Err(format!(
            "does not build for RISC-V:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(bin)
}

/// The program as C with its tasks as step functions over frames — the stepped tier.
fn build_stepped(name: &str, src: &str) -> Result<PathBuf, String> {
    let c = compile_to_c_stepped(src)
        .map_err(|e| format!("does not lower to the stepped tier: {e:?}"))?;
    let dir = scratch("stepped", name);
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

/// The program through the bytecode walker, whose concurrency lowers to the stepped tier.
fn build_walker(name: &str, src: &str) -> Result<PathBuf, String> {
    let c = compile_to_c_via_bytecode(src)
        .map_err(|e| format!("does not lower through the bytecode walker: {e:?}"))?;
    let dir = scratch("walker", name);
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

/// The program for the GPU, built with `nvcc` for the RTX 3060 class (`sm_86`).
fn build_cuda(name: &str, src: &str) -> Result<PathBuf, String> {
    let artifact = compile_to_cuda(src).map_err(|e| format!("does not lower to CUDA: {e:?}"))?;
    let dir = scratch("cuda", name);
    let (file, bin) = (dir.join("prog.cu"), dir.join("prog"));
    std::fs::write(&file, &artifact.kernel_cu).unwrap();
    let out = Command::new("nvcc")
        .args(["-O2", "-arch=sm_86", "-fmad=false", "-o"])
        .arg(&bin)
        .arg(&file)
        .output()
        .unwrap();
    if !out.status.success() {
        return Err(format!(
            "does not build with nvcc:\n{}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(bin)
}

/// The corpus programs whose failure happens inside a runtime helper, which on a GPU thread traps the whole run
/// — there is no escape to unwind to. The run fails, and what it showed is what the interpreter showed up to
/// the failure; the lines the other tasks would still have shown are lost. A listed program must keep exactly
/// that outcome — the day it conforms in full, it leaves this list.
const CUDA_TRAPS: &[&str] = &["a_zero_divisor_fails_its_task_and_the_others_finish"];

/// Every corpus program on one tier at every seed, holding those whose failure `traps` the run to the trap's
/// outcome; the divergences, named.
fn diverging_on(
    tier: &str,
    traps: &[&str],
    build: impl Fn(&str, &str) -> Result<PathBuf, String>,
    run: impl Fn(&std::path::Path, u64) -> Outcome,
) -> Vec<String> {
    let mut diverged = Vec::new();
    for name in traps {
        if !CORPUS.iter().any(|row| row.name == *name) {
            diverged.push(format!(
                "{name}: listed for {tier}, but no corpus row has that name"
            ));
        }
    }
    for row in CORPUS {
        let bin = match build(row.name, row.src) {
            Ok(bin) => bin,
            Err(why) => {
                diverged.push(format!("{}: {why}", row.name));
                continue;
            }
        };
        let traps_here = traps.contains(&row.name);
        for &seed in SEEDS {
            let want = oracle(row.name, row.src, seed);
            let got = run(&bin, seed);
            let kept = if traps_here {
                got.failed
                    && want.failed
                    && got.lines.len() < want.lines.len()
                    && want.lines.starts_with(&got.lines)
            } else {
                got == want
            };
            if !kept {
                let what = if traps_here && got == want {
                    "now conforms in full — take it off the trap list".to_string()
                } else if traps_here {
                    "listed as trapping the run, but it does not show a failed prefix of the interpreter's lines".to_string()
                } else {
                    "diverges".to_string()
                };
                diverged.push(format!(
                    "{} at seed {seed} {what}:\n  interpreter {want:?}\n  {tier:<11} {got:?}",
                    row.name
                ));
                break;
            }
        }
    }
    diverged
}

#[test]
fn the_reference_tiers_agree_on_the_corpus_and_the_seed_reaches_them() {
    for row in CORPUS {
        let outcomes: Vec<Outcome> = SEEDS
            .iter()
            .map(|s| oracle(row.name, row.src, *s))
            .collect();
        let pinned = Outcome {
            lines: row.at_seed_0.iter().map(|l| l.to_string()).collect(),
            failed: row.fails,
        };
        assert_eq!(
            outcomes[0], pinned,
            "{}: the interpreter no longer shows what the row pins at seed 0",
            row.name
        );
        let distinct = outcomes
            .iter()
            .collect::<std::collections::BTreeSet<_>>()
            .len();
        if row.seed_sensitive {
            assert!(
                distinct > 1,
                "{} must show more than one outcome across the seeds",
                row.name
            );
        } else {
            assert_eq!(
                distinct, 1,
                "{} is marked seed-insensitive, but its outcome changes with the seed",
                row.name
            );
        }
    }
}

#[test]
fn every_row_is_named_once() {
    let names: std::collections::BTreeSet<&str> = CORPUS.iter().map(|row| row.name).collect();
    assert_eq!(names.len(), CORPUS.len(), "two corpus rows share a name");
}

#[test]
fn host_c_schedules_every_program_as_the_interpreter_does() {
    if !common::require_tool("gcc") {
        return;
    }
    let diverged = diverging_on("host C", &[], build_c, run_c);
    assert!(
        diverged.is_empty(),
        "{} of {} programs diverge on host C:\n\n{}",
        diverged.len(),
        CORPUS.len(),
        diverged.join("\n\n")
    );
}

#[test]
fn riscv_schedules_every_program_as_the_interpreter_does() {
    if !common::require_tool("riscv64-linux-gnu-gcc") || !common::require_tool("qemu-riscv64") {
        return;
    }
    let diverged = diverging_on("RISC-V", &[], build_riscv, run_riscv);
    assert!(
        diverged.is_empty(),
        "{} of {} programs diverge on RISC-V:\n\n{}",
        diverged.len(),
        CORPUS.len(),
        diverged.join("\n\n")
    );
}

#[test]
fn the_stepped_tier_schedules_every_program_as_the_interpreter_does() {
    if !common::require_tool("gcc") {
        return;
    }
    let diverged = diverging_on("stepped", &[], build_stepped, run_c);
    assert!(
        diverged.is_empty(),
        "{} of {} programs diverge on the stepped tier:\n\n{}",
        diverged.len(),
        CORPUS.len(),
        diverged.join("\n\n")
    );
}

#[test]
fn the_bytecode_walker_schedules_every_program_as_the_interpreter_does() {
    if !common::require_tool("gcc") {
        return;
    }
    let diverged = diverging_on("walker", &[], build_walker, run_c);
    assert!(
        diverged.is_empty(),
        "{} of {} programs diverge through the bytecode walker:\n\n{}",
        diverged.len(),
        CORPUS.len(),
        diverged.join("\n\n")
    );
}

#[test]
fn cuda_schedules_every_program_as_the_interpreter_does() {
    if !common::require_tool("nvcc") || !common::require_gpu() {
        return;
    }
    let diverged = diverging_on("CUDA", CUDA_TRAPS, build_cuda, run_c);
    assert!(
        diverged.is_empty(),
        "{} of {} programs diverge on CUDA:\n\n{}",
        diverged.len(),
        CORPUS.len(),
        diverged.join("\n\n")
    );
}
