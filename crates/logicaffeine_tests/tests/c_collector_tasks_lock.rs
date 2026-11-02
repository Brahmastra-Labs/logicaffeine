//! A concurrent program's memory is collected, and a suspended task keeps what it holds.
//!
//! The C tier runs tasks two ways: each on a stack of its own (`compile_to_c`, the `ucontext` tier) and as step
//! functions over heap frames (`compile_to_c_stepped`). A task that holds a value only in its own frame and is then
//! suspended — waiting on a pipe while other tasks allocate — must find that value intact when it resumes: the
//! collector scans every live task's stack and frame, not only the running one. Under `LOGOS_GC_STRESS=1` it collects
//! before every allocation, so a missed root frees the value and the run differs from the VM's. Each program is also
//! built with AddressSanitizer: nothing it allocates outlives the run.
#![cfg(not(target_arch = "wasm32"))]

mod common;

use std::path::Path;
use std::process::Command;

/// A worker that builds a list, waits on a pipe while Main allocates, then shows the list.
const SUSPENDED_HOLDER: &str = "## To worker (ch: Int):\n    Let xs be [randomInt(1, 1), 2, 3, 4, 5].\n    Receive go from ch.\n    Show length of xs.\n    Show xs.\n\n## Main\n    Let ch be a Pipe of Int.\n    Launch a task to worker with ch.\n    Let mutable total be 0.\n    Repeat for i from 1 to 200:\n        Let ys be [i, i, i].\n        Set total to total + length of ys.\n    Send 1 into ch.\n    Show total.\n";

fn vm_stdout(src: &str) -> String {
    let r = logicaffeine_compile::compile::vm_outcome_concurrent(src);
    assert!(r.error.is_none(), "the VM fails the program: {:?}", r.error);
    assert_eq!(
        r.output.lines().collect::<Vec<_>>(),
        ["5", "[1, 2, 3, 4, 5]", "600"],
        "the reference scheduler preserves the declared program result"
    );
    if r.output.is_empty() {
        String::new()
    } else {
        r.output.lines().map(|l| format!("{l}\n")).collect()
    }
}

fn build(c: &str, dir: &Path, flags: &[&str]) -> std::path::PathBuf {
    std::fs::create_dir_all(dir).expect("scratch dir");
    std::fs::write(dir.join("prog.c"), c).expect("write C");
    let out = Command::new("gcc")
        .args(flags)
        .args(["-o", "prog", "prog.c", "-lm"])
        .current_dir(dir)
        .output()
        .expect("gcc runs");
    assert!(
        out.status.success(),
        "gcc failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    dir.join("prog")
}

fn check(tier: &str, c: &str, workers: usize) -> Vec<String> {
    let mut problems = Vec::new();
    if !c.contains("/* LC_GC_COLLECTOR_BEGIN */") {
        problems.push(format!("[{tier}] the program's C does not carry the collector, so nothing it allocates is reclaimed"));
        return problems;
    }
    let c = format!("#define LOGOS_SCHED_WORKERS {workers}\n{c}");
    let c = c.as_str();
    let want = vm_stdout(SUSPENDED_HOLDER);
    let dir = std::env::temp_dir().join(format!(
        "logos_collector_tasks_{}_{tier}",
        std::process::id()
    ));
    let bin = build(c, &dir.join("plain"), &["-O2", "-pthread"]);
    let stressed = Command::new(&bin)
        .env("LOGOS_GC_STRESS", "1")
        .output()
        .expect("the program runs");
    if !stressed.status.success() || String::from_utf8_lossy(&stressed.stdout) != want {
        problems.push(format!(
            "[{tier}] with a collection at every allocation the run shows {:?} (status {:?}), the VM {want:?}",
            String::from_utf8_lossy(&stressed.stdout),
            stressed.status.code()
        ));
    }
    let asan = build(
        c,
        &dir.join("asan"),
        &[
            "-O1",
            "-g",
            "-fsanitize=address",
            "-fno-omit-frame-pointer",
            "-pthread",
        ],
    );
    let run = Command::new(&asan)
        .env("ASAN_OPTIONS", "detect_leaks=1:exitcode=99")
        .output()
        .expect("the program runs");
    let report: Vec<String> = String::from_utf8_lossy(&run.stderr)
        .lines()
        .filter(|l| {
            l.contains("ERROR: LeakSanitizer")
                || l.contains("ERROR: AddressSanitizer")
                || l.starts_with("SUMMARY: ")
        })
        .map(str::to_string)
        .collect();
    if !run.status.success() || String::from_utf8_lossy(&run.stdout) != want {
        problems.push(format!(
            "[{tier}] sanitizer run: status {:?}, output {:?}, VM {want:?}; stderr {}",
            run.status.code(),
            String::from_utf8_lossy(&run.stdout),
            String::from_utf8_lossy(&run.stderr)
        ));
    }
    if !report.is_empty() {
        problems.push(format!("[{tier}] {}", report.join(" | ")));
    }
    let _ = std::fs::remove_dir_all(&dir);
    problems
}

#[test]
fn a_suspended_coroutine_keeps_what_it_holds_through_every_collection() {
    if !common::require_tool("gcc") {
        return;
    }
    let c = logicaffeine_compile::compile::compile_to_c(SUSPENDED_HOLDER)
        .expect("the coroutine tier lowers the program");
    let problems = check("coroutines", &c, 1);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn a_suspended_stepped_task_keeps_what_it_holds_through_every_collection() {
    if !common::require_tool("gcc") {
        return;
    }
    let c = logicaffeine_compile::compile::compile_to_c_stepped(SUSPENDED_HOLDER)
        .expect("the stepped tier lowers the program");
    let problems = check("stepped", &c, 1);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

#[test]
fn stepped_workers_collect_from_the_running_workers_stack() {
    if !common::require_tool("gcc") {
        return;
    }
    let c = logicaffeine_compile::compile::compile_to_c_stepped(SUSPENDED_HOLDER)
        .expect("the stepped tier lowers the program");
    let problems = check("stepped_workers", &c, 4);
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}
