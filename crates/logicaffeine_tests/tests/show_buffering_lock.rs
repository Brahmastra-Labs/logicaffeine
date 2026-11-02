//! ★ LOCK — a program that shows a line at a time does not pay a kernel crossing per line. ★
//!
//! A write to stdout is a system call with a fixed cost whatever its size (the CPU performance-engineering
//! benchmark `11-syscall-cost`). `std::io::Stdout` is line-buffered whatever it is connected to, so the compiled
//! Rust made one `write(2)` per `Show`; the C tier's stdio buffers when stdout is a pipe or a file. Every tier now
//! buffers too: `logicaffeine_system::io` holds 64 KiB and the interpreted run writes its lines in one stream.
//!
//! Buffering may never move a byte. The output before a failure is complete and comes before the failure's reason;
//! a prompt reaches stdout before the program reads its answer; a program's own Rust (`Escape to Rust:`) writes in
//! its place; and a terminal still sees each line as it is shown. Counted with `strace`, observed under a pipe and
//! under a pseudo-terminal (`script`).

#![cfg(target_os = "linux")]

#[path = "tier_parity/mod.rs"]
mod tier_parity;

mod common;
use common::require_tool;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use tier_parity::rust_batch::{self, Profile};
use tier_parity::Program;

const LINES: usize = 100_000;

fn many_lines() -> String {
    format!("## Main\nRepeat for i from 1 to {LINES}:\n    Show i.\n")
}

const FAILS_AFTER_SHOWING: &str =
    "## Main\nShow 1.\nShow 2.\nShow 3.\nLet zero be 0.\nShow 10 / zero.\n";

const PROMPTS: &str =
    "## Main\nShow \"name?\".\nRead name from the console.\nShow \"hello \" + name.\n";

const ESCAPES: &str = "## Main\nShow \"a\".\nEscape to Rust:\n    println!(\"b\");\nShow \"c\".\n";

fn scratch(tag: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("logos-show-buffering-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    dir
}

/// Build every program once, both shipped Rust profiles, and hand back each executable.
fn binaries(programs: &[(&str, String)]) -> std::collections::BTreeMap<(String, Profile), PathBuf> {
    let batch: Vec<Program> = programs
        .iter()
        .map(|(id, src)| Program::new(*id, src.clone()))
        .collect();
    let built = rust_batch::build(&batch, 4)
        .unwrap_or_else(|why| panic!("the Rust batch build could not start: {why}"));
    let mut out = std::collections::BTreeMap::new();
    for (id, _) in programs {
        for profile in [Profile::Debug, Profile::Release] {
            let bin = built
                .binary(id, profile)
                .unwrap_or_else(|why| panic!("{id} ({profile:?}) has no binary: {why}"));
            out.insert((id.to_string(), profile), bin);
        }
    }
    out
}

/// A run under `strace`: its stdout and stderr, whether it succeeded, and the system calls it made on stdout,
/// stderr and stdin, in order.
struct Traced {
    stdout: String,
    stderr: String,
    success: bool,
    calls: Vec<String>,
}

fn traced(cmd: &[&str], stdin: &str, dir: &Path, tag: &str) -> Traced {
    let trace = dir.join(format!("{tag}.strace"));
    let mut child = Command::new("strace")
        .args([
            "-f",
            "-qq",
            "-e",
            "trace=write,read",
            "-e",
            "signal=none",
            "-o",
        ])
        .arg(&trace)
        .args(cmd)
        .current_dir(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("strace preflighted present but failed to spawn");
    use std::io::Write as _;
    child
        .stdin
        .take()
        .expect("piped stdin")
        .write_all(stdin.as_bytes())
        .expect("feed stdin");
    let out = child.wait_with_output().expect("the traced run ends");
    let calls = std::fs::read_to_string(&trace)
        .expect("the trace")
        .lines()
        .map(|l| {
            l.split_once(' ')
                .map_or(l, |(_, call)| call)
                .trim()
                .to_string()
        })
        .filter(|c| {
            c.starts_with("write(1,") || c.starts_with("write(2,") || c.starts_with("read(0,")
        })
        .collect();
    Traced {
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
        success: out.status.success(),
        calls,
    }
}

fn stdout_writes(t: &Traced) -> usize {
    t.calls.iter().filter(|c| c.starts_with("write(1,")).count()
}

/// The fewest writes `bytes` of output could take at 64 KiB a write, and a few more for the tail and the flushes
/// at exit.
fn budget(bytes: usize) -> usize {
    bytes.div_ceil(64 * 1024) + 3
}

fn expected_lines() -> String {
    (1..=LINES).map(|i| format!("{i}\n")).collect()
}

#[test]
fn a_compiled_program_writes_many_lines_in_a_handful_of_writes() {
    if !require_tool("strace") {
        return;
    }
    let bins = binaries(&[("many", many_lines())]);
    let want = expected_lines();
    for profile in [Profile::Debug, Profile::Release] {
        let bin = &bins[&("many".to_string(), profile)];
        let dir = scratch(&format!("many-{profile:?}"));
        let t = traced(&[bin.to_str().unwrap()], "", &dir, "many");
        assert!(t.success, "{profile:?}: the program runs:\n{}", t.stderr);
        assert_eq!(t.stdout, want, "{profile:?}: every line, in order");
        assert!(
            stdout_writes(&t) <= budget(want.len()),
            "{profile:?}: {LINES} lines ({} bytes) took {} write(2) calls on stdout; at most {} may",
            want.len(),
            stdout_writes(&t),
            budget(want.len())
        );
    }
}

#[test]
fn an_interpreted_run_writes_many_lines_in_a_handful_of_writes() {
    if !require_tool("strace") {
        return;
    }
    let dir = scratch("interpret");
    tier_parity::project(&dir, &many_lines());
    let largo = tier_parity::largo();
    let t = traced(
        &[largo.to_str().unwrap(), "run", "--interpret"],
        "",
        &dir,
        "interpret",
    );
    assert!(t.success, "the interpreted run succeeds:\n{}", t.stderr);
    let want = expected_lines();
    assert_eq!(t.stdout, want, "every line, in order");
    assert!(
        stdout_writes(&t) <= budget(want.len()),
        "{LINES} interpreted lines ({} bytes) took {} write(2) calls on stdout; at most {} may",
        want.len(),
        stdout_writes(&t),
        budget(want.len())
    );
}

#[test]
fn the_output_before_a_failure_is_complete_and_comes_first() {
    if !require_tool("strace") {
        return;
    }
    let bins = binaries(&[("fails", FAILS_AFTER_SHOWING.to_string())]);
    for profile in [Profile::Debug, Profile::Release] {
        let bin = &bins[&("fails".to_string(), profile)];
        let dir = scratch(&format!("fails-{profile:?}"));
        let t = traced(&[bin.to_str().unwrap()], "", &dir, "fails");
        assert!(!t.success, "{profile:?}: the program fails");
        assert_eq!(
            t.stdout, "1\n2\n3\n",
            "{profile:?}: everything shown before the failure is written"
        );
        assert!(
            t.stderr.contains("Division by zero"),
            "{profile:?}: the failure's reason:\n{}",
            t.stderr
        );
        let first_err = t
            .calls
            .iter()
            .position(|c| c.starts_with("write(2,"))
            .expect("the reason is written to stderr");
        let last_out = t
            .calls
            .iter()
            .rposition(|c| c.starts_with("write(1,"))
            .expect("the lines are written to stdout");
        assert!(
            last_out < first_err,
            "{profile:?}: stdout is written before the failure's reason:\n{:#?}",
            t.calls
        );
    }
}

#[test]
fn a_prompt_is_written_before_the_answer_is_read() {
    if !require_tool("strace") {
        return;
    }
    let bins = binaries(&[("prompts", PROMPTS.to_string())]);
    for profile in [Profile::Debug, Profile::Release] {
        let bin = &bins[&("prompts".to_string(), profile)];
        let dir = scratch(&format!("prompts-{profile:?}"));
        let t = traced(&[bin.to_str().unwrap()], "ada\n", &dir, "prompts");
        assert!(t.success, "{profile:?}: the program runs:\n{}", t.stderr);
        assert_eq!(
            t.stdout, "name?\nhello ada\n",
            "{profile:?}: the prompt and the greeting"
        );
        let prompt = t
            .calls
            .iter()
            .position(|c| c.starts_with("write(1,") && c.contains("name?"))
            .expect("the prompt is written");
        let read = t
            .calls
            .iter()
            .position(|c| c.starts_with("read(0,"))
            .expect("stdin is read");
        assert!(
            prompt < read,
            "{profile:?}: the prompt reaches stdout before the answer is read:\n{:#?}",
            t.calls
        );
    }
}

#[test]
fn a_programs_own_rust_writes_in_its_place() {
    let bins = binaries(&[("escapes", ESCAPES.to_string())]);
    for profile in [Profile::Debug, Profile::Release] {
        let bin = &bins[&("escapes".to_string(), profile)];
        let out = Command::new(bin)
            .stdin(Stdio::null())
            .output()
            .expect("the program runs");
        assert!(out.status.success(), "{profile:?}: the program runs");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            "a\nb\nc\n",
            "{profile:?}: the escape's line sits between the shown lines"
        );
    }
}

#[test]
fn a_terminal_sees_each_line_as_it_is_shown() {
    if !require_tool("strace") || !require_tool("script") {
        return;
    }
    let bins = binaries(&[("three", "## Main\nShow 1.\nShow 2.\nShow 3.\n".to_string())]);
    let bin = &bins[&("three".to_string(), Profile::Release)];
    let dir = scratch("terminal");
    let trace = dir.join("terminal.strace");
    let inner = format!(
        "strace -f -qq -e trace=write -e signal=none -o {} {}",
        trace.display(),
        bin.display()
    );
    let out = Command::new("script")
        .args(["-qec", &inner, "/dev/null"])
        .current_dir(&dir)
        .stdin(Stdio::null())
        .output()
        .expect("script runs");
    assert!(
        out.status.success(),
        "the program runs under a pseudo-terminal:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let writes = std::fs::read_to_string(&trace)
        .expect("the trace")
        .lines()
        .filter(|l| l.contains("write(1,"))
        .count();
    assert_eq!(writes, 3, "a terminal is written a line at a time");
}
