//! A full run records what every test cost, test by test.
//!
//! In a full gate the box runs at a load of 50 to 90 on 32 threads, and a single-threaded heavy runs 4 to 19 times
//! slower than alone: a Z3 bit-blast 151 s alone took 1,227 s, a parity sweep 218 s took 1,071 s, one nvcc build 7.8 s
//! took 147 s. nextest counts every test as one thread, so the tests that fan out inside (a sweep's workers, a build's
//! jobs, a simulator's threads) crowd the box past its cores with nothing recording which. The full profile runs every
//! test through `scripts/test/test-ledger.sh`, which runs it under GNU time and appends one line to the run's ledger
//! (`$LOGOS_TEST_LEDGER`): when the test ran and what it cost in wall clock, CPU and memory, and how it ended.
//! `scripts/test/test-ledger-report.py` reads a run's load from it: the heaviest tests, the widest, the most crowded
//! stretch and who crowded it. The wrapper is transparent: the test's arguments, output, exit status and the signal
//! that ended it are the test's own, so nextest reports a failure, a crash or a timeout as it would unwrapped.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test test_ledger_lock`

// The wrapper reads its costs from GNU time and the tests signal process groups: Linux's.
#![cfg(target_os = "linux")]

use std::io::Write as _;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn wrapper() -> PathBuf {
    repo_root().join("scripts/test/test-ledger.sh")
}

/// The wrapper running `argv` as a test of run `run`, binary `binary`, named `name`, recording into `ledger` (none when
/// `None`).
fn wrapped(ledger: Option<&Path>, name: &str, argv: &[&str]) -> Command {
    let mut cmd = Command::new(wrapper());
    cmd.args(argv)
        .env("NEXTEST_RUN_ID", "run-1")
        .env("NEXTEST_BINARY_ID", "logicaffeine-tests::some_lock")
        .env("NEXTEST_TEST_NAME", name)
        .env_remove("LOGOS_TEST_LEDGER");
    if let Some(l) = ledger {
        cmd.env("LOGOS_TEST_LEDGER", l);
    }
    cmd
}

/// One ledger line, by field.
#[derive(Debug)]
struct Line {
    run: String,
    binary: String,
    test: String,
    start: f64,
    end: f64,
    wall: f64,
    user: Option<f64>,
    sys: Option<f64>,
    maxrss_kb: Option<u64>,
    outcome: String,
}

fn lines(ledger: &Path) -> Vec<Line> {
    let text = std::fs::read_to_string(ledger).unwrap_or_default();
    text.lines()
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            assert_eq!(f.len(), 10, "a ledger line has ten fields: {l:?}");
            let num = |s: &str| (s != "-").then(|| s.parse::<f64>().unwrap_or_else(|_| panic!("a number: {s:?} in {l:?}")));
            Line {
                run: f[0].into(),
                binary: f[1].into(),
                test: f[2].into(),
                start: f[3].parse().unwrap(),
                end: f[4].parse().unwrap(),
                wall: f[5].parse().unwrap(),
                user: num(f[6]),
                sys: num(f[7]),
                maxrss_kb: (f[8] != "-").then(|| f[8].parse().unwrap()),
                outcome: f[9].into(),
            }
        })
        .collect()
}

fn scratch() -> tempfile::TempDir {
    tempfile::tempdir().expect("a scratch directory")
}

#[test]
fn the_full_profile_runs_every_test_through_the_ledger() {
    let config: toml::Table = std::fs::read_to_string(repo_root().join(".config/nextest.toml"))
        .unwrap()
        .parse()
        .unwrap();
    let experimental: Vec<&str> = config
        .get("experimental")
        .and_then(|e| e.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
        .unwrap_or_default();
    assert!(experimental.contains(&"wrapper-scripts"), "nextest runs wrapper scripts only with the experiment on: {experimental:?}");
    let command = config
        .get("scripts")
        .and_then(|s| s.get("wrapper"))
        .and_then(|w| w.get("test-ledger"))
        .and_then(|l| l.get("command"))
        .expect("a wrapper script named test-ledger");
    assert_eq!(command.get("command-line").and_then(|c| c.as_str()), Some("scripts/test/test-ledger.sh"));
    assert_eq!(command.get("relative-to").and_then(|c| c.as_str()), Some("workspace-root"));
    let full: Vec<&toml::Table> = config
        .get("profile")
        .and_then(|p| p.get("full"))
        .and_then(|f| f.get("scripts"))
        .and_then(|s| s.as_array())
        .map(|a| a.iter().filter_map(|t| t.as_table()).collect())
        .unwrap_or_default();
    assert!(
        full.iter().any(|t| t.get("filter").and_then(|f| f.as_str()) == Some("all()")
            && t.get("run-wrapper").and_then(|w| w.as_str()) == Some("test-ledger")),
        "the full profile wraps every test in the ledger: {full:?}"
    );
    for (name, profile) in config.get("profile").and_then(|p| p.as_table()).unwrap() {
        if name != "full" {
            assert!(profile.get("scripts").is_none(), "only the full profile records a ledger; {name} runs its tests bare");
        }
    }
    let mode = std::fs::metadata(wrapper()).expect("the wrapper exists").permissions();
    assert!(std::os::unix::fs::PermissionsExt::mode(&mode) & 0o111 != 0, "nextest executes the wrapper directly");
}

#[test]
fn without_a_ledger_the_wrapper_only_runs_the_test() {
    let out = wrapped(None, "t", &["sh", "-c", "printf out; printf err >&2; exit 7"]).output().unwrap();
    assert_eq!(out.status.code(), Some(7), "the test's exit status");
    assert_eq!(out.stdout, b"out");
    assert_eq!(out.stderr, b"err");
}

#[test]
fn a_test_is_one_line_of_when_it_ran_and_what_it_cost() {
    let dir = scratch();
    let ledger = dir.path().join("run.ledger.tsv");
    let busy = "i=0; while [ $i -lt 400000 ]; do i=$((i+1)); done; printf out; printf err >&2; exit 3";
    let before = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
    let out = wrapped(Some(&ledger), "a_busy_test", &["sh", "-c", busy]).output().unwrap();
    let after = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs_f64();
    assert_eq!(out.status.code(), Some(3), "the test's exit status, recorded or not");
    assert_eq!((out.stdout.as_slice(), out.stderr.as_slice()), (&b"out"[..], &b"err"[..]), "the test's output, untouched");
    let l = lines(&ledger);
    assert_eq!(l.len(), 1, "one line a test: {l:?}");
    let l = &l[0];
    assert_eq!((l.run.as_str(), l.binary.as_str(), l.test.as_str()), ("run-1", "logicaffeine-tests::some_lock", "a_busy_test"));
    assert!(before <= l.start && l.start <= l.end && l.end <= after, "it ran within the call: {before} {l:?} {after}");
    assert!((l.wall - (l.end - l.start)).abs() < 0.05, "its wall clock is its span: {l:?}");
    let cpu = l.user.unwrap() + l.sys.unwrap();
    assert!(cpu > 0.05 && cpu <= l.wall + 0.05, "a loop of 400,000 steps costs CPU, at most its wall clock: {l:?}");
    assert!(l.maxrss_kb.unwrap() > 0, "its peak memory: {l:?}");
    assert_eq!(l.outcome, "exit 3");
}

#[test]
fn the_cpu_of_every_process_the_test_waits_for_is_its_own() {
    let dir = scratch();
    let ledger = dir.path().join("run.ledger.tsv");
    let spin = "i=0; while [ $i -lt 300000 ]; do i=$((i+1)); done";
    let four = format!("for k in 1 2 3 4; do sh -c '{spin}' & done; wait");
    let out = wrapped(Some(&ledger), "fans_out", &["sh", "-c", &four]).output().unwrap();
    assert!(out.status.success());
    let l = &lines(&ledger)[0];
    let cpu = l.user.unwrap() + l.sys.unwrap();
    let one = {
        let solo = dir.path().join("solo.tsv");
        wrapped(Some(&solo), "one", &["sh", "-c", spin]).output().unwrap();
        let s = &lines(&solo)[0];
        s.user.unwrap() + s.sys.unwrap()
    };
    assert!(cpu > 2.5 * one, "four workers cost about four times one ({cpu:.3} s against {one:.3} s): the ledger counts a test's children");
}

/// The kernel's time on the test's behalf is the test's cost too: a copy a byte at a time is mostly system calls.
#[test]
fn the_time_the_kernel_spends_for_the_test_is_its_own() {
    let dir = scratch();
    let ledger = dir.path().join("run.ledger.tsv");
    let copy = ["dd", "if=/dev/zero", "of=/dev/null", "bs=1", "count=3000000", "status=none"];
    let out = wrapped(Some(&ledger), "copies_bytewise", &copy).output().unwrap();
    assert!(out.status.success(), "{out:?}");
    let l = &lines(&ledger)[0];
    assert!(l.sys.unwrap() >= 0.1, "three million one-byte reads and writes are system time: {l:?}");
}

#[test]
fn a_test_killed_by_a_signal_ends_its_wrapper_by_the_same_signal() {
    let dir = scratch();
    let ledger = dir.path().join("run.ledger.tsv");
    let out = wrapped(Some(&ledger), "aborts", &["sh", "-c", "kill -ABRT $$"]).output().unwrap();
    assert_eq!(out.status.signal(), Some(libc::SIGABRT), "nextest sees the abort the test died of: {:?}", out.status);
    let out = wrapped(Some(&ledger), "exits_134", &["sh", "-c", "exit 134"]).output().unwrap();
    assert_eq!((out.status.code(), out.status.signal()), (Some(134), None), "an exit status of 134 is not a signal");
    let l = lines(&ledger);
    assert_eq!(l.iter().map(|l| (l.test.as_str(), l.outcome.as_str())).collect::<Vec<_>>(), [("aborts", "signal 6"), ("exits_134", "exit 134")]);
    let out = wrapped(None, "aborts_bare", &["sh", "-c", "kill -ABRT $$"]).output().unwrap();
    assert_eq!(out.status.signal(), Some(libc::SIGABRT), "and without a ledger");
}

/// nextest ends a test that overstays its budget by signalling the test's process group, which holds the wrapper and
/// everything under it.
#[test]
fn a_timeout_ends_the_wrapper_by_the_signal_nextest_sends_and_the_test_is_still_recorded() {
    let dir = scratch();
    let ledger = dir.path().join("run.ledger.tsv");
    let mut child = wrapped(Some(&ledger), "overstays", &["sleep", "60"])
        .process_group(0)
        .stdout(Stdio::null())
        .spawn()
        .unwrap();
    let pgid = child.id() as i32;
    let t0 = Instant::now();
    while !std::fs::read_dir("/proc").unwrap().flatten().any(|p| {
        std::fs::read_to_string(p.path().join("stat"))
            .ok()
            .and_then(|s| s.rsplit_once(") ").map(|(_, rest)| rest.split(' ').nth(2) == Some(&pgid.to_string()) && s.contains("(sleep)")))
            .unwrap_or(false)
    }) {
        assert!(t0.elapsed() < Duration::from_secs(20), "the test starts");
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(200));
    assert_eq!(unsafe { libc::kill(-pgid, libc::SIGTERM) }, 0);
    let status = child.wait().unwrap();
    assert!(t0.elapsed() < Duration::from_secs(20), "the test ended at the signal, not at its sleep's end");
    assert_eq!(status.signal(), Some(libc::SIGTERM), "nextest sees its own signal: {status:?}");
    let l = lines(&ledger);
    assert_eq!(l.len(), 1, "the test is recorded though it was ended: {l:?}");
    assert_eq!(l[0].outcome, "signal 15");
    assert!(l[0].wall >= 0.2, "its wall clock runs to the signal: {:?}", l[0]);
}

#[test]
fn the_arguments_reach_the_test_unchanged() {
    let dir = scratch();
    let ledger = dir.path().join("run.ledger.tsv");
    let args = ["a b", "", "--exact", "x\ty", "'quoted'", "$HOME", "*"];
    let mut argv = vec!["sh", "-c", "for a in \"$@\"; do printf '%s\\0' \"$a\"; done", "sh"];
    argv.extend(args);
    for ledger in [None, Some(ledger.as_path())] {
        let out = wrapped(ledger, "echoes", &argv).output().unwrap();
        let got: Vec<&[u8]> = out.stdout.split(|b| *b == 0).collect();
        let want: Vec<&[u8]> = args.iter().map(|a| a.as_bytes()).chain([&b""[..]]).collect();
        assert_eq!(got, want, "argv byte for byte, ledger {ledger:?}");
    }
}

#[test]
fn the_ledger_is_the_runs_alone_and_no_test_inside_it_writes_there() {
    let dir = scratch();
    let ledger = dir.path().join("run.ledger.tsv");
    let out = wrapped(Some(&ledger), "looks", &["sh", "-c", "printf '%s' \"${LOGOS_TEST_LEDGER-unset}\""]).output().unwrap();
    assert_eq!(out.stdout, b"unset", "a test inside the run sees no ledger, so a nested run records nothing into this one");
    assert_eq!(lines(&ledger).len(), 1);
}

#[test]
fn tests_ending_together_each_append_one_whole_line() {
    let dir = scratch();
    let ledger = dir.path().join("run.ledger.tsv");
    let names: Vec<String> = (0..48).map(|i| format!("{i:02}_{}", "n".repeat(900))).collect();
    let children: Vec<_> = names
        .iter()
        .map(|n| wrapped(Some(&ledger), n, &["true"]).stdout(Stdio::null()).spawn().unwrap())
        .collect();
    for mut c in children {
        assert!(c.wait().unwrap().success());
    }
    let mut got: Vec<String> = lines(&ledger).into_iter().map(|l| l.test).collect();
    got.sort();
    assert_eq!(got, names, "every test one line, none torn");
}

fn report(ledger: &str, args: &[&str]) -> Output {
    let dir = scratch();
    let path = dir.path().join("fixture.ledger.tsv");
    std::fs::File::create(&path).unwrap().write_all(ledger.as_bytes()).unwrap();
    let out = Command::new("python3")
        .arg(repo_root().join("scripts/test/test-ledger-report.py"))
        .arg(&path)
        .args(args)
        .output()
        .expect("python3 runs the report");
    assert!(out.status.success(), "the report runs: {}", String::from_utf8_lossy(&out.stderr));
    out
}

/// A ledger of one run, by hand: four tests over 0..1200 s, and a stray line of a nested run.
const FIXTURE: &str = "run_id\tbinary_id\ttest\tstart\tend\twall_s\tuser_s\tsys_s\tmaxrss_kb\toutcome
r\tlt::wide_sweep\tsweeps\t1000.000000\t1600.000000\t600.000\t2300.000\t100.000\t4194304\texit 0
r\tlt::lone_heavy\tbit_blasts\t1600.000000\t2200.000000\t600.000\t590.000\t10.000\t1048576\texit 0
r\tlt::quick\tchecks\t1100.000000\t1101.000000\t1.000\t0.500\t0.100\t2048\texit 0
r\tlt::runaway\tgrows\t1200.000000\t1260.000000\t60.000\t-\t-\t-\tsignal 15
nested\tlt::inner\tinside\t1000.000000\t1001.000000\t1.000\t50.000\t0.000\t1024\texit 0
";

#[test]
fn the_report_reads_the_runs_cost_from_the_run_alone() {
    let text = String::from_utf8(report(FIXTURE, &[]).stdout).unwrap();
    assert!(text.contains("4 tests"), "the nested run's line is not this run's: {text}");
    assert!(text.contains("CPU 0.83 h"), "2,400 + 600 + 0.6 s of CPU: {text}");
    assert!(!text.contains("inside"), "{text}");
}

#[test]
fn the_report_names_the_heaviest_and_the_widest_tests() {
    let text = String::from_utf8(report(FIXTURE, &["--top", "2"]).stdout).unwrap();
    let heaviest = text.split("heaviest").nth(1).expect("a heaviest section");
    let first = heaviest.lines().find(|l| l.contains("lt::")).unwrap();
    assert!(first.contains("lt::wide_sweep::sweeps") && first.contains("2400") && first.contains("4.0"), "CPU 2,400 s at 4.0 cores: {first}");
    let widest = text.split("widest").nth(1).expect("a widest section");
    let first = widest.lines().find(|l| l.contains("lt::")).unwrap();
    assert!(first.contains("wide_sweep") && first.contains("4.0"), "{first}");
    assert!(!widest.split("crowded").next().unwrap().contains("lone_heavy"), "a test at one core is not wide: {widest}");
    assert!(text.contains("lt::runaway::grows") && text.contains("signal 15"), "a test ended by a signal is listed with how it ended: {text}");
}

#[test]
fn the_report_finds_the_most_crowded_stretch_and_who_crowded_it() {
    let text = String::from_utf8(report(FIXTURE, &["--window", "600"]).stdout).unwrap();
    let crowded = text.split("crowded").nth(1).expect("a crowded section");
    let head = crowded.lines().next().unwrap();
    assert!(head.contains("4.0 cores") && head.contains("+0:00:00") && head.contains("+0:10:00"), "the sweep's 600 s at 4 cores, from the run's start: {head}");
    let first = crowded.lines().find(|l| l.contains("lt::")).unwrap();
    assert!(first.contains("wide_sweep"), "{first}");
}

/// The run's widest tests, as the overrides that tell nextest their width: a test that keeps `n` cores busy
/// (`threads-required = n`, its CPU over its wall clock, rounded) takes `n` of the run's threads, so the run's load
/// stays near its threads instead of nextest counting each such test as one. Only tests at least `--widths` cores wide
/// and `--min-cpu` seconds of CPU are declared; a test's filter names its binary and itself exactly.
#[test]
fn the_report_writes_the_widest_tests_as_nextest_overrides() {
    let ledger = "run_id\tbinary_id\ttest\tstart\tend\twall_s\tuser_s\tsys_s\tmaxrss_kb\toutcome
r\tlogicaffeine-tests::wide_lock\tbuilds_in_parallel\t1000\t1300\t300.000\t1700.000\t100.000\t1024\texit 0
r\tlogicaffeine-tests::wide_lock\tbuilds_briefly\t1000\t1010\t10.000\t70.000\t0.000\t1024\texit 0
r\tlogicaffeine-compile\tcodegen::tests::fans_out\t1000\t1100\t100.000\t900.000\t50.000\t1024\texit 0
r\tlogicaffeine-tests::narrow_lock\tone_core\t1000\t1600\t600.000\t590.000\t5.000\t1024\texit 0
r\tlogicaffeine-tests::huge_lock\tsaturates\t1000\t1100\t100.000\t9000.000\t0.000\t1024\texit 0
";
    let out = String::from_utf8(report(ledger, &["--widths", "4", "--min-cpu", "120", "--max-threads", "24"]).stdout).unwrap();
    let blocks: Vec<&str> = out.split("[[profile.full.overrides]]").skip(1).collect();
    assert_eq!(blocks.len(), 3, "three tests are at least four cores wide with two minutes of CPU:\n{out}");
    let toml: toml::Table = out.parse().unwrap_or_else(|e| panic!("the overrides are TOML: {e}\n{out}"));
    let rows: Vec<(String, i64)> = toml["profile"]["full"]["overrides"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| (o["filter"].as_str().unwrap().to_string(), o["threads-required"].as_integer().unwrap()))
        .collect();
    assert_eq!(
        rows,
        [
            ("binary_id(logicaffeine-compile) & test(=codegen::tests::fans_out)".to_string(), 10),
            ("binary_id(logicaffeine-tests::huge_lock) & test(=saturates)".to_string(), 24),
            ("binary_id(logicaffeine-tests::wide_lock) & test(=builds_in_parallel)".to_string(), 6),
        ],
        "each test's filter is exact, its width its CPU over its wall rounded, never more than the run's threads"
    );
}

/// The widths the full profile declares are a ledger's, and every one names a test that exists: a renamed test would
/// leave its override matching nothing, and its width undeclared.
#[test]
fn every_declared_width_names_a_test_that_exists() {
    let text = std::fs::read_to_string(repo_root().join(".config/nextest.toml")).unwrap();
    let begin = text.find("# BEGIN test widths").expect("the full profile's measured widths");
    let end = text.find("# END test widths").expect("its end marker");
    assert!(begin < end);
    let config: toml::Table = text.parse().unwrap();
    let overrides = config["profile"]["full"]["overrides"].as_array().unwrap();
    let block: toml::Table = text[begin..end].parse().expect("the widths block is TOML on its own");
    let declared = block["profile"]["full"]["overrides"].as_array().expect("the block declares full-profile overrides");
    assert!(declared.len() >= 10, "the widths block declares the widest tests: {}", declared.len());
    let last_hand_written = text[..begin].matches("[[profile.full.overrides]]").count();
    assert_eq!(overrides.len(), last_hand_written + declared.len(), "the widths come after every hand-written override");
    let tests_dir = repo_root().join("crates/logicaffeine_tests/tests");
    for o in declared {
        let filter = o["filter"].as_str().unwrap();
        let n = o["threads-required"].as_integer().expect("a width is a number of threads");
        assert!((2..=24).contains(&n), "{filter}: {n} threads");
        assert_eq!(o.as_table().map(|t| t.len()), Some(2), "{filter}: a width override sets its width alone");
        let (bin, test) = filter
            .strip_prefix("binary_id(")
            .and_then(|r| r.split_once(") & test(="))
            .and_then(|(b, t)| t.strip_suffix(')').map(|t| (b, t)))
            .unwrap_or_else(|| panic!("an exact filter: {filter}"));
        let leaf = test.rsplit("::").next().unwrap();
        let src = match bin.split_once("::") {
            Some(("logicaffeine-tests", b)) => std::fs::read_to_string(tests_dir.join(format!("{b}.rs")))
                .or_else(|_| std::fs::read_to_string(tests_dir.join(b).join("main.rs")))
                .unwrap_or_else(|_| panic!("{filter}: no test binary {b}")),
            _ => {
                let dir = repo_root().join("crates/logicaffeine_tests/unit");
                let mut all = String::new();
                for e in walk(&dir) {
                    all.push_str(&std::fs::read_to_string(e).unwrap_or_default());
                }
                all
            }
        };
        assert!(src.contains(&format!("fn {leaf}(")), "{filter}: no test {leaf} in {bin}");
    }
}

fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else if p.extension().is_some_and(|x| x == "rs") {
            out.push(p);
        }
    }
    out
}

#[test]
fn the_fast_script_records_a_ledger_beside_its_log_and_reports_it() {
    let script = std::fs::read_to_string(repo_root().join("scripts/run-all-tests-fast.sh")).unwrap();
    let export = script.find("export LOGOS_TEST_LEDGER=").expect("the fast script names the ledger");
    let pass1 = script.find("\ncargo nextest run --workspace").expect("pass 1, the run itself");
    let unset = script.find("unset LOGOS_TEST_LEDGER").expect("the ledger closes after pass 1");
    assert!(export < pass1 && pass1 < unset, "the ledger is open for pass 1 alone");
    assert!(script.contains("LEDGER_FILE=\"${LOG_FILE%.log}.ledger.tsv\""), "the ledger sits beside the run's log");
    assert!(script.contains("scripts/test/test-ledger-report.py") || script.contains("test-ledger-report.py"), "the run reports its load");
    assert!(script.contains(" ledger   : "), "the summary names the ledger");
}
