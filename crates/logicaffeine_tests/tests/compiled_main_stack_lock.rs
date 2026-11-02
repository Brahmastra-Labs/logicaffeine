//! A compiled program runs its `Main` on the thread that started it.
//!
//! The program needs a deep stack (`logicaffeine_system::stack::MAIN_STACK_BYTES`), and it used to get one by
//! starting a thread with that stack and waiting for it. A second thread, even one only waiting, puts every `malloc`
//! and `free` of the process on the C library's locked, atomic path: binary_trees ran 0.55x as fast as the same code
//! on one thread. `Main` now runs on a stack mapped for it, switched to on the starting thread, and the process stays
//! single-threaded. The stack is as deep as before, a run past its end reports the sentence every tier reports for it,
//! and a runtime failure ends the program as before.

#![cfg(not(target_arch = "wasm32"))]

mod common;
use common::{compile_logos, compile_to_rust};
use std::time::{Duration, Instant};

const DEADLINE: Duration = Duration::from_secs(60);

/// The compiled `source` run with `args`, killed at [`DEADLINE`]: (exit code, stdout, stderr).
fn run(source: &str, args: &[&str]) -> (Option<i32>, String, String) {
    let built = compile_logos(source);
    assert!(built.success, "the program builds:\n{}", built.stderr);
    let mut child = std::process::Command::new(&built.binary_path)
        .args(args)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let started = Instant::now();
    while child.try_wait().unwrap().is_none() {
        if started.elapsed() > DEADLINE {
            child.kill().ok();
            panic!("the program was still running after {DEADLINE:?}");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let out = child.wait_with_output().unwrap();
    (
        out.status.code(),
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
    )
}

/// The text of the generated `fn main`.
fn main_fn(rust: &str) -> &str {
    let at = rust.find("fn main() {").expect("a `fn main`");
    let len = rust[at..].find("\n}\n").expect("the end of `fn main`");
    &rust[at..at + len]
}

#[test]
fn main_switches_to_its_own_stack_on_the_starting_thread() {
    let rust = compile_to_rust("## Main\nShow 1.\n").unwrap();
    let main = main_fn(&rust);
    assert!(
        main.contains("logicaffeine_system::stack::run_main(_logos_main);"),
        "`Main` runs through the stack switch:\n{main}"
    );
    assert!(!main.contains("std::thread"), "`main` starts no thread:\n{main}");
}

/// Runs about a second: long enough to look at the process while it runs.
const BUSY: &str = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Let mutable xs be a new Seq of Int.
Let mutable i be 0.
While i is less than n:
    Push (i * 7919) % 1000 to xs.
    Set i to i + 1.
Let mutable total be 0.
Let mutable round be 0.
While round is less than 40:
    Set i to 1.
    While i is at most n:
        Set total to (total + item i of xs * round) % 1000000007.
        Set i to i + 1.
    Set round to round + 1.
Show total.
"#;

#[cfg(target_os = "linux")]
#[test]
fn a_compiled_program_runs_on_one_thread() {
    let built = compile_logos(BUSY);
    assert!(built.success, "the program builds:\n{}", built.stderr);
    let mut child = std::process::Command::new(&built.binary_path)
        .arg("2000000")
        .stdout(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let status = format!("/proc/{}/status", child.id());
    let started = Instant::now();
    let mut samples = Vec::new();
    while child.try_wait().unwrap().is_none() && started.elapsed() < DEADLINE {
        if let Ok(text) = std::fs::read_to_string(&status) {
            if let Some(n) = text.lines().find_map(|l| l.strip_prefix("Threads:")) {
                samples.push(n.trim().parse::<u32>().unwrap());
            }
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    child.kill().ok();
    child.wait().ok();
    assert!(samples.len() >= 3, "the program ran long enough to be looked at: {samples:?}");
    assert!(
        samples.iter().all(|&n| n == 1),
        "a running compiled program has one thread; a second puts every malloc on the locked path: {samples:?}"
    );
}

/// `walk(n)` recurses `n` calls deep, each keeping a value for after its call returns.
const WALK: &str = r#"## To native args () -> Seq of Text
## To native parseInt (s: Text) -> Int

## To walk (n: Int) -> Int:
    If n equals 0:
        Return 0.
    Let below be walk(n - 1).
    Return (below * 31 + n) % 1000003.

## Main
Let arguments be args().
Let n be parseInt(item 2 of arguments).
Show "walking".
Show walk(n).
"#;

#[test]
fn deep_recursion_has_the_stack_it_always_had() {
    // 60 000 calls hold about 47 MiB in this build: six times a default main thread's stack.
    let (code, out, err) = run(WALK, &["60000"]);
    assert_eq!((code, out.as_str()), (Some(0), "walking\n162380\n"), "stderr: {err}");
}

#[test]
fn running_past_the_end_of_the_stack_reports_the_call_depth_sentence() {
    let (code, out, err) = run(WALK, &["100000000"]);
    assert_eq!(out, "walking\n", "what the program showed before it ran out of stack is written out");
    assert_eq!(err, format!("{}\n", logicaffeine_data::failure::CALL_DEPTH_EXCEEDED));
    assert_eq!(code, Some(1), "a run past the end of the stack ends like any runtime failure");
}

#[test]
fn a_runtime_failure_reports_once_and_ends_with_status_one() {
    let source = "## Main\nLet xs be [1, 2, 3].\nShow \"before\".\nShow item 7 of xs.\n";
    let (code, out, err) = run(source, &[]);
    assert_eq!(out, "before\n");
    assert_eq!(err, format!("{}\n", logicaffeine_data::failure::index_out_of_bounds(7)));
    assert_eq!(code, Some(1));
}
