//! ★ LOCK — process.lg's `runProcess` runs another program, BOUNDED, on every engine. ★
//!
//! A LOGOS program that drives a compiler, a proof checker or an exporter has to start it, wait for it
//! and read what it said. `runProcess` is that: `program` + `args` (no shell — an argument is passed
//! exactly as written), a working directory, an environment laid over the inherited one, a deadline
//! and an output cap, both mandatory. It answers a `ProcessResult` record built in LOGOS from the
//! native `runProcessParts`, so every engine shares one implementation
//! (`logicaffeine_system::process::run_parts`) and one record.
//!
//! The promises, each locked here:
//! * the exit code, both streams and the two flags come back exactly, on the tree-walker, the VM,
//!   the optimizing VM, the native VM tier and the compiled Rust binary;
//! * no shell: a space, a `$` or a `*` inside an argument is that character;
//! * a deadline stops the run AND every process it started (the run owns a process group), and the
//!   call returns promptly — a sleeping grandchild does not keep it waiting;
//! * a flood of output is cut at `maxOutput` bytes and drained, so the child neither blocks nor is
//!   killed for writing; `truncated` says it happened;
//! * standard input is empty, so a program that reads it ends at once instead of hanging;
//! * an impossible run (no such program, no such directory, a malformed environment entry, a bound
//!   out of range) is an `Err`, a run that happened and failed is an `Ok` that says so;
//! * a program's own `runProcess` wins over the library's (declarer-wins);
//! * a sealed `## Test` starts no processes unless it grants `This test may use the real processes.`
//!   and runs under `--live`.
//!
//! YOU DO NOT GET TO WEAKEN THIS LOCK TO MAKE IT PASS. A divergence is a defect in an engine.
#![cfg(unix)]
#![allow(dead_code)]
mod common;
use common::{assert_all_tiers, assert_compiled_equals_interpreted_eq};
use logicaffeine_compile::compile::{compile_to_rust, tw_outcome, vm_outcome};
use std::time::{Duration, Instant};

/// A program that runs `program` with `args` and shows every field of the record, one per line —
/// the shape every behavioural test below prints.
fn shows_run(program: &str, args: &[&str], cwd: &str, env: &[(&str, &str)], timeout_ms: i64, max_output: i64) -> String {
    let mut src = String::from("## Main\nLet mutable args be a new Seq of Text.\n");
    for a in args {
        src.push_str(&format!("Push {} to args.\n", logos_text(a)));
    }
    src.push_str("Let mutable env be a new Map of Text to Text.\n");
    for (k, v) in env {
        src.push_str(&format!("Set env at {} to {}.\n", logos_text(k), logos_text(v)));
    }
    src.push_str(&format!(
        "Inspect runProcess({}, args, {}, env, {timeout_ms}, {max_output}):\n\
         \x20   When Ok (r):\n\
         \x20       Show \"exit \" + r's exitCode.\n\
         \x20       Show \"out [\" + r's stdout + \"]\".\n\
         \x20       Show \"err [\" + r's stderr + \"]\".\n\
         \x20       Show \"timedOut \" + r's timedOut.\n\
         \x20       Show \"truncated \" + r's truncated.\n\
         \x20   When Err (e):\n\
         \x20       Show \"refused: \" + e.\n",
        logos_text(program),
        logos_text(cwd),
    ));
    src
}

/// A LOGOS text literal for `s`: quotes and backslashes escaped, braces doubled (a brace opens an
/// interpolation hole in a text literal).
fn logos_text(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '{' => out.push_str("{{"),
            '}' => out.push_str("}}"),
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn tw_and_vm(src: &str) -> Vec<(&'static str, Option<String>, String)> {
    let tw = tw_outcome(src);
    let vm = vm_outcome(src);
    vec![("tree-walker", tw.error, tw.output), ("vm", vm.error, vm.output)]
}

/// Each interpreting engine, run ONE AT A TIME: a test that inspects what a run left behind on the
/// machine (a pid file, a process) must look before the next engine runs the program again.
fn each_engine() -> [(&'static str, fn(&str) -> logicaffeine_compile::compile::RunOutcome); 2] {
    [("tree-walker", tw_outcome), ("vm", vm_outcome)]
}

// ── what a run answers ────────────────────────────────────────────────────────────────────────────

#[test]
fn echo_answers_its_output_and_a_clean_exit_on_every_interpreting_tier() {
    assert_all_tiers(
        &shows_run("/bin/echo", &["hello", "world"], "", &[], 5_000, 65_536),
        "exit 0\nout [hello world\n]\nerr []\ntimedOut false\ntruncated false",
    );
}

#[test]
fn a_failing_program_is_an_ok_carrying_its_code_and_its_stderr() {
    assert_all_tiers(
        &shows_run("/bin/sh", &["-c", "echo partial; echo oops >&2; exit 3"], "", &[], 5_000, 65_536),
        "exit 3\nout [partial\n]\nerr [oops\n]\ntimedOut false\ntruncated false",
    );
}

#[test]
fn a_program_killed_by_a_signal_answers_128_plus_the_signal() {
    assert_all_tiers(
        &shows_run("/bin/sh", &["-c", "kill -TERM $$"], "", &[], 5_000, 65_536),
        "exit 143\nout []\nerr []\ntimedOut false\ntruncated false",
    );
}

#[test]
fn arguments_reach_the_program_verbatim_with_no_shell_in_between() {
    // A shell would split "a b", expand "$HOME" and glob "*"; runProcess hands each over as written.
    assert_all_tiers(
        &shows_run("/bin/echo", &["a b", "$HOME", "*", "'q'", "\"dq\""], "", &[], 5_000, 65_536),
        "exit 0\nout [a b $HOME * 'q' \"dq\"\n]\nerr []\ntimedOut false\ntruncated false",
    );
}

#[test]
fn a_bare_program_name_is_found_on_path() {
    assert_all_tiers(
        &shows_run("echo", &["found"], "", &[], 5_000, 65_536),
        "exit 0\nout [found\n]\nerr []\ntimedOut false\ntruncated false",
    );
}

#[test]
fn standard_input_is_empty_so_a_reader_ends_at_once() {
    let started = Instant::now();
    assert_all_tiers(
        &shows_run("/bin/cat", &[], "", &[], 20_000, 65_536),
        "exit 0\nout []\nerr []\ntimedOut false\ntruncated false",
    );
    assert!(
        started.elapsed() < Duration::from_secs(15),
        "cat must see end-of-input immediately, not wait for the deadline"
    );
}

#[test]
fn the_working_directory_is_the_one_named() {
    assert_all_tiers(
        &shows_run("/bin/pwd", &[], "/", &[], 5_000, 65_536),
        "exit 0\nout [/\n]\nerr []\ntimedOut false\ntruncated false",
    );
}

#[test]
fn the_environment_is_laid_over_the_inherited_one() {
    // The named variable is set, and the inherited PATH is still there beside it.
    assert_all_tiers(
        &shows_run(
            "/bin/sh",
            &["-c", "echo \"$LOGOS_PROCESS_LOCK:${PATH:+inherited}\""],
            "",
            &[("LOGOS_PROCESS_LOCK", "laid over")],
            5_000,
            65_536,
        ),
        "exit 0\nout [laid over:inherited\n]\nerr []\ntimedOut false\ntruncated false",
    );
}

#[test]
fn output_that_is_not_utf8_is_decoded_with_replacement() {
    assert_all_tiers(
        &shows_run("/usr/bin/printf", &["a\\377b"], "", &[], 5_000, 65_536),
        "exit 0\nout [a\u{FFFD}b]\nerr []\ntimedOut false\ntruncated false",
    );
}

// ── the bounds ────────────────────────────────────────────────────────────────────────────────────

#[test]
fn a_flood_of_output_is_cut_at_the_cap_and_the_program_still_finishes() {
    // 200 000 bytes into a 1000-byte cap: the stream is drained (the child never blocks on a full
    // pipe), the first 1000 bytes are kept, and the program's own exit code survives.
    let src = "## Main\n\
        Let env be a new Map of Text to Text.\n\
        Inspect runProcess(\"/bin/sh\", [\"-c\", \"head -c 200000 /dev/zero | tr '\\\\000' a; echo done >&2\"], \"\", env, 10000, 1000):\n\
        \x20   When Ok (r):\n\
        \x20       Show \"exit \" + r's exitCode.\n\
        \x20       Show length of r's stdout.\n\
        \x20       Show \"err [\" + r's stderr + \"]\".\n\
        \x20       Show \"timedOut \" + r's timedOut.\n\
        \x20       Show \"truncated \" + r's truncated.\n\
        \x20   When Err (e):\n\
        \x20       Show \"refused: \" + e.\n";
    assert_all_tiers(src, "exit 0\n1000\nerr [done\n]\ntimedOut false\ntruncated true");
}

#[test]
fn a_deadline_stops_the_run_and_answers_promptly() {
    let started = Instant::now();
    assert_all_tiers(
        &shows_run("/bin/sleep", &["30"], "", &[], 300, 65_536),
        "exit 137\nout []\nerr []\ntimedOut true\ntruncated false",
    );
    assert!(
        started.elapsed() < Duration::from_secs(20),
        "every tier must return near its 300 ms deadline, not after the 30 s sleep (took {:?})",
        started.elapsed()
    );
}

#[test]
fn nothing_a_run_started_outlives_it() {
    // The shell backgrounds a grandchild that would sleep for a minute, records its pid, and then
    // waits. The deadline stops the shell; the grandchild must be gone too, because the run owns
    // its whole process group. A grandchild that survived would also hold the pipes open and keep
    // the call waiting — so the prompt return is part of the same promise.
    let dir = tempfile::tempdir().expect("a scratch dir");
    let pid_file = dir.path().join("grandchild.pid");
    let script = format!("sleep 60 & echo $! > {}; wait", pid_file.display());
    let src = shows_run("/bin/sh", &["-c", &script], "", &[], 500, 65_536);
    for (engine, run) in each_engine() {
        let ran = run(&src);
        let (error, output) = (ran.error, ran.output);
        assert_eq!(error, None, "{engine}");
        assert!(output.contains("timedOut true"), "{engine}: {output}");
        let pid: i32 = std::fs::read_to_string(&pid_file)
            .expect("the grandchild recorded its pid")
            .trim()
            .parse()
            .expect("a pid");
        let gone = (0..50).any(|_| {
            let alive = unsafe { libc::kill(pid, 0) } == 0;
            if alive {
                std::thread::sleep(Duration::from_millis(20));
            }
            !alive
        });
        assert!(gone, "{engine}: grandchild {pid} outlived the run");
        let _ = std::fs::remove_file(&pid_file);
    }
}

#[test]
fn a_child_that_exits_leaves_no_background_process_behind() {
    // The leader exits at once but leaves a sleeper in its group holding stdout: the call must not
    // wait the minute for it, and the sleeper must not survive the run.
    let dir = tempfile::tempdir().expect("a scratch dir");
    let pid_file = dir.path().join("straggler.pid");
    let script = format!("sleep 60 & echo $! > {}; echo leader-done", pid_file.display());
    let started = Instant::now();
    let src = shows_run("/bin/sh", &["-c", &script], "", &[], 5_000, 65_536);
    for (engine, run) in each_engine() {
        let ran = run(&src);
        let (error, output) = (ran.error, ran.output);
        assert_eq!(error, None, "{engine}");
        assert!(output.contains("out [leader-done\n]"), "{engine}: {output}");
        assert!(output.contains("exit 0"), "{engine}: {output}");
        let pid: i32 = std::fs::read_to_string(&pid_file).expect("pid").trim().parse().expect("a pid");
        let gone = (0..50).any(|_| {
            let alive = unsafe { libc::kill(pid, 0) } == 0;
            if alive {
                std::thread::sleep(Duration::from_millis(20));
            }
            !alive
        });
        assert!(gone, "{engine}: straggler {pid} outlived its leader's run");
        let _ = std::fs::remove_file(&pid_file);
    }
    assert!(started.elapsed() < Duration::from_secs(30), "took {:?}", started.elapsed());
}

// ── what is refused ───────────────────────────────────────────────────────────────────────────────

#[test]
fn a_program_that_cannot_start_is_an_err_naming_it() {
    let src = shows_run("/nonexistent/logos-no-such-program", &[], "", &[], 5_000, 65_536);
    for (engine, error, output) in tw_and_vm(&src) {
        assert_eq!(error, None, "{engine}");
        assert!(
            output.trim().starts_with("refused: runProcess: cannot start '/nonexistent/logos-no-such-program'"),
            "{engine}: {output}"
        );
    }
}

#[test]
fn a_directory_that_does_not_exist_is_an_err_naming_it() {
    let src = shows_run("/bin/pwd", &[], "/nonexistent/logos-no-such-dir", &[], 5_000, 65_536);
    for (engine, error, output) in tw_and_vm(&src) {
        assert_eq!(error, None, "{engine}");
        assert_eq!(
            output.trim(),
            "refused: runProcess: the working directory '/nonexistent/logos-no-such-dir' is not a directory",
            "{engine}"
        );
    }
}

#[test]
fn bounds_out_of_range_are_refused_before_anything_starts() {
    let cases: &[(i64, i64, &str)] = &[
        (0, 1024, "refused: runProcess: timeoutMs must be between 1 and 86400000, got 0"),
        (-5, 1024, "refused: runProcess: timeoutMs must be between 1 and 86400000, got -5"),
        (86_400_001, 1024, "refused: runProcess: timeoutMs must be between 1 and 86400000, got 86400001"),
        (1000, 0, "refused: runProcess: maxOutput must be between 1 and 1073741824, got 0"),
        (1000, 1_073_741_825, "refused: runProcess: maxOutput must be between 1 and 1073741824, got 1073741825"),
    ];
    for (timeout, cap, expected) in cases {
        let src = shows_run("/bin/echo", &["never"], "", &[], *timeout, *cap);
        for (engine, error, output) in tw_and_vm(&src) {
            assert_eq!(error, None, "{engine}");
            assert_eq!(output.trim(), *expected, "{engine}");
        }
    }
}

#[test]
fn a_malformed_environment_entry_is_refused() {
    // The raw native takes NAME=value entries; a name that is empty, or one holding `=` or NUL, is
    // not a variable a program could read back.
    for (entry, expected) in [
        ("=v", "refused: runProcess: an environment entry needs a name before '=', got \"=v\""),
        ("NOEQUALS", "refused: runProcess: an environment entry is NAME=value, got \"NOEQUALS\""),
    ] {
        let src = format!(
            "## Main\nInspect runProcessParts(\"/bin/echo\", [\"x\"], \"\", [{}], 1000, 1024):\n\
             \x20   When Ok (parts):\n\
             \x20       Show \"ran\".\n\
             \x20   When Err (e):\n\
             \x20       Show \"refused: \" + e.\n",
            logos_text(entry)
        );
        for (engine, error, output) in tw_and_vm(&src) {
            assert_eq!(error, None, "{engine}");
            assert_eq!(output.trim(), expected, "{engine}");
        }
    }
}

#[test]
fn an_argument_holding_nul_is_refused() {
    let src = "## Main\nLet env be a new Map of Text to Text.\n\
        Inspect runProcess(\"/bin/echo\", [\"a\" + chr(0) + \"b\"], \"\", env, 1000, 1024):\n\
        \x20   When Ok (r):\n\
        \x20       Show \"ran\".\n\
        \x20   When Err (e):\n\
        \x20       Show \"refused: \" + e.\n";
    for (engine, error, output) in tw_and_vm(src) {
        assert_eq!(error, None, "{engine}");
        assert_eq!(output.trim(), "refused: runProcess: argument 1 holds a NUL byte", "{engine}");
    }
}

// ── the raw native ────────────────────────────────────────────────────────────────────────────────

#[test]
fn the_raw_native_answers_the_five_parts_in_order() {
    let src = "## Main\n\
        Inspect runProcessParts(\"/bin/sh\", [\"-c\", \"printf out; printf err >&2; exit 4\"], \"\", [\"UNUSED=1\"], 5000, 1024):\n\
        \x20   When Ok (parts):\n\
        \x20       Repeat for p in parts:\n\
        \x20           Show \"[\" + p + \"]\".\n\
        \x20   When Err (e):\n\
        \x20       Show \"refused: \" + e.\n";
    assert_all_tiers(src, "[4]\n[out]\n[err]\n[false]\n[false]");
}

// ── declarer wins ─────────────────────────────────────────────────────────────────────────────────

#[test]
fn a_program_that_defines_run_process_keeps_its_own() {
    let src = "## To runProcess (program: Text) -> Text:\n    Return \"mine: \" + program.\n\n## Main\nShow runProcess(\"/bin/echo\").\n";
    assert_all_tiers(src, "mine: /bin/echo");
}

// ── the compiled tier ─────────────────────────────────────────────────────────────────────────────

#[test]
fn the_compiled_binary_runs_processes_exactly_as_the_interpreters_do() {
    // One program covering the shapes above, built to Rust and run, compared byte-for-byte with the
    // interpreter: the record, a failure, the cap, the deadline and a refusal.
    let src = "## To report (r: Result of ProcessResult and Text):\n\
        \x20   Inspect r:\n\
        \x20       When Ok (p):\n\
        \x20           Show \"exit \" + p's exitCode + \" out [\" + p's stdout + \"] err [\" + p's stderr + \"] t \" + p's timedOut + \" c \" + p's truncated.\n\
        \x20       When Err (e):\n\
        \x20           Show \"refused: \" + e.\n\
        \n\
        ## Main\n\
        Let mutable env be a new Map of Text to Text.\n\
        Set env at \"LOGOS_PROCESS_LOCK\" to \"aot\".\n\
        report(runProcess(\"/bin/echo\", [\"a b\", \"$HOME\"], \"\", env, 5000, 65536)).\n\
        report(runProcess(\"/bin/sh\", [\"-c\", \"echo $LOGOS_PROCESS_LOCK; echo e >&2; exit 2\"], \"/\", env, 5000, 65536)).\n\
        report(runProcess(\"/bin/sh\", [\"-c\", \"head -c 5000 /dev/zero | tr '\\\\000' z\"], \"\", env, 5000, 10)).\n\
        report(runProcess(\"/bin/sleep\", [\"30\"], \"\", env, 300, 65536)).\n\
        report(runProcess(\"/bin/echo\", [\"x\"], \"\", env, 0, 65536)).\n";
    assert_compiled_equals_interpreted_eq(
        src,
        "exit 0 out [a b $HOME\n] err [] t false c false\n\
         exit 2 out [aot\n] err [e\n] t false c false\n\
         exit 0 out [zzzzzzzzzz] err [] t false c true\n\
         exit 137 out [] err [] t true c false\n\
         refused: runProcess: timeoutMs must be between 1 and 86400000, got 0",
    );
}

#[test]
fn the_rust_backend_links_the_native_to_the_system_crate() {
    let src = "## Main\nLet env be a new Map of Text to Text.\nLet r be runProcess(\"/bin/true\", [], \"\", env, 1000, 1024).\n";
    let rust = compile_to_rust(src).expect("the program compiles to Rust");
    assert!(
        rust.contains("logicaffeine_system::process::run_parts("),
        "the native is the system crate's run_parts:\n{rust}"
    );
}

// ── the sealed test world ─────────────────────────────────────────────────────────────────────────

#[test]
fn a_sealed_test_starts_no_processes() {
    let src = "## Test: a sealed test cannot start a process\n\
        \x20   Let env be a new Map of Text to Text.\n\
        \x20   Let r be runProcess(\"/bin/echo\", [\"hi\"], \"\", env, 1000, 1024).\n\
        \x20   Inspect r:\n\
        \x20       When Ok (p):\n\
        \x20           Expect that \"started\" is equal to \"refused\".\n\
        \x20       When Err (e):\n\
        \x20           Expect that e is equal to \"runProcess: a sealed test starts no processes — grant `This test may use the real processes.` and run under `largo test --live`\".\n";
    let report = logicaffeine_compile::testrun::run_tests(src, &Default::default()).expect("the tests run");
    assert!(report.all_passed(), "{}", logicaffeine_compile::testrun::render_report(&report));
}

#[test]
fn a_test_granted_real_processes_runs_them_under_live_and_is_skipped_otherwise() {
    let src = "## Test: a live test starts a real process\n\
        \x20   This test may use the real processes.\n\
        \x20   Let env be a new Map of Text to Text.\n\
        \x20   Let r be runProcess(\"/bin/echo\", [\"live\"], \"\", env, 5000, 1024).\n\
        \x20   Inspect r:\n\
        \x20       When Ok (p):\n\
        \x20           Expect that p's stdout is equal to \"live\\n\".\n\
        \x20       When Err (e):\n\
        \x20           Expect that e is equal to \"a real process\".\n";
    let hermetic = logicaffeine_compile::testrun::run_tests(src, &Default::default()).expect("the tests run");
    let rendered = logicaffeine_compile::testrun::render_report(&hermetic);
    assert!(rendered.contains("largo test --live"), "skipped loudly without --live:\n{rendered}");
    let live = logicaffeine_compile::testrun::TestConfig { live: true, ..Default::default() };
    let report = logicaffeine_compile::testrun::run_tests(src, &live).expect("the tests run");
    assert!(report.all_passed(), "{}", logicaffeine_compile::testrun::render_report(&report));
}
