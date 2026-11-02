//! A full test run is refused while a heavy non-cargo job runs on the box.
//!
//! Place-and-route, synthesis and emulation saturate cores for hours, and a baseline taken beside one
//! fails load-sensitive tests that pass in isolation: the result cannot be told apart from a real
//! regression. `scripts/test/heavy-jobs.sh` names every such job and exits 1 when there is one, and both
//! full runners call it before their first cargo command.
//!
//! Each test here runs `sleep` through a symlink named for a tool, so the kernel records the tool's name for
//! it exactly as it does for the real tool, and asks the check about that process by its pid: a real heavy
//! job running elsewhere on the box (a silicon lock's yosys inside this very suite) changes no verdict.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test heavy_jobs_lock`

// The check and these tests read what the kernel records under /proc and the cgroup v2 hierarchy: Linux's.
#![cfg(target_os = "linux")]

use std::path::{Path, PathBuf};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

/// The check's verdict and its report: `(refused, stdout)`.
fn heavy_jobs() -> (bool, String) {
    let out = Command::new("bash")
        .arg(repo_root().join("scripts/test/heavy-jobs.sh"))
        .output()
        .expect("bash runs the heavy-job check");
    (!out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned())
}

/// A process running under `name`: `sleep` executed through a symlink called `name`, so the kernel records
/// `name` as its executable. A symlink rather than a copy: a file this process has just written can still be
/// open for writing in a child another test thread forked meanwhile, and executing it then fails with
/// `ETXTBSY`. Killed when dropped.
struct Impostor {
    child: Child,
    _dir: tempfile::TempDir,
}

impl Impostor {
    fn named(name: &str) -> Impostor {
        let dir = tempfile::tempdir().expect("a scratch directory");
        let exe = dir.path().join(name);
        std::os::unix::fs::symlink(sleep_binary(), &exe).expect("link sleep under the tool's name");
        let child = Command::new(&exe).arg0("sleep").arg("120").spawn().expect("run the impostor");
        wait_until_named(child.id(), name);
        Impostor { child, _dir: dir }
    }

    fn pid(&self) -> u32 {
        self.child.id()
    }
}

impl Drop for Impostor {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn sleep_binary() -> &'static Path {
    ["/usr/bin/sleep", "/bin/sleep"]
        .into_iter()
        .map(Path::new)
        .find(|p| p.exists())
        .expect("coreutils sleep")
}

/// The kernel records at most 15 characters of an executable's name.
fn recorded_name(name: &str) -> &str {
    &name[..name.len().min(15)]
}

fn wait_until_named(pid: u32, name: &str) {
    let comm = format!("/proc/{pid}/comm");
    for _ in 0..200 {
        if std::fs::read_to_string(&comm).map(|c| c.trim() == recorded_name(name)).unwrap_or(false) {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    panic!("pid {pid} never ran as {name}");
}

fn names_pid(report: &str, pid: u32) -> bool {
    report.lines().any(|line| line.contains(&format!("(pid {pid})")))
}

/// Every executable of every heavy tool family, as each installs itself.
const HEAVY: &[&str] = &[
    "ours_flow",
    "yosys",
    "yosys-abc",
    "nextpnr",
    "nextpnr-xilinx",
    "nextpnr-ice40",
    "nextpnr-ecp5",
    "nextpnr-nexus",
    "nextpnr-himbaechel",
    "nextpnr-generic",
    "vivado",
    "quartus_sh",
    "quartus_syn",
    "quartus_fit",
    "quartus_sta",
    "openFPGALoader",
    "xc7frames2bit",
    "qemu-riscv64",
    "qemu-aarch64",
    "openroad",
    "magic",
    "klayout",
    "netgen",
    "TritonRoute",
];

#[test]
fn every_heavy_tool_is_named_and_refuses_the_run() {
    for name in HEAVY {
        let job = Impostor::named(name);
        let (refused, report) = heavy_jobs();
        assert!(refused, "{name} is running and the check let the run start:\n{report}");
        assert!(
            names_pid(&report, job.pid()),
            "the check refused but did not name {name} (pid {}):\n{report}",
            job.pid()
        );
        assert!(
            report.contains(recorded_name(name)),
            "the report does not say which tool is running ({name}):\n{report}"
        );
    }
}

/// nextpnr installs one executable per architecture and never one called `nextpnr`; the xc7 routes ran as
/// `nextpnr-xilinx` beside a gate that matched only the bare name.
#[test]
fn an_architecture_specific_nextpnr_is_a_heavy_job() {
    let job = Impostor::named("nextpnr-xilinx");
    let (refused, report) = heavy_jobs();
    assert!(refused && names_pid(&report, job.pid()), "nextpnr-xilinx ran unseen:\n{report}");
}

/// Quartus runs one executable per stage of a compile; a FIM build's fitter ran as `quartus_fit` beside a
/// gate that knew no Quartus name.
#[test]
fn a_quartus_stage_is_a_heavy_job() {
    let job = Impostor::named("quartus_fit");
    let (refused, report) = heavy_jobs();
    assert!(refused && names_pid(&report, job.pid()), "quartus_fit ran unseen:\n{report}");
}

/// A process whose command line merely mentions a tool is not that tool: matching command lines made a
/// runner refuse against its own watcher shell.
#[test]
fn a_command_line_that_mentions_a_tool_is_not_the_tool() {
    let mut shell = Command::new("bash")
        .args(["-c", "read -r -t 120 _; : yosys nextpnr-xilinx quartus_fit openroad qemu-riscv64"])
        .stdin(std::process::Stdio::piped())
        .spawn()
        .expect("run a shell that mentions the tools");
    wait_until_named(shell.id(), "bash");
    let (_, report) = heavy_jobs();
    let named = names_pid(&report, shell.id());
    let _ = shell.kill();
    let _ = shell.wait();
    assert!(!named, "the check named a shell that only mentions the tools:\n{report}");
}

#[test]
fn an_ordinary_process_is_not_a_heavy_job() {
    let job = Impostor::named("logos-idle");
    let (_, report) = heavy_jobs();
    assert!(!names_pid(&report, job.pid()), "the check named an ordinary process:\n{report}");
}

/// Both full runners ask the check before any cargo command they run, and refuse on its verdict.
#[test]
fn both_full_runners_check_for_heavy_jobs_before_their_first_cargo_command() {
    for runner in ["run-all-tests-fast.sh", "run-all-tests.sh"] {
        let src = std::fs::read_to_string(repo_root().join("scripts").join(runner)).expect("the runner");
        let call = "scripts/test/heavy-jobs.sh\"";
        let at = src
            .find(call)
            .unwrap_or_else(|| panic!("{runner} does not ask scripts/test/heavy-jobs.sh before it runs"));
        let first_cargo = src
            .lines()
            .scan(0usize, |offset, line| {
                let here = *offset;
                *offset += line.len() + 1;
                Some((here, line.trim_start()))
            })
            .find(|(_, line)| {
                !line.starts_with('#')
                    && !line.starts_with("echo")
                    && (line.starts_with("cargo test") || line.starts_with("cargo nextest") || line.starts_with("cargo build"))
            })
            .map(|(offset, _)| offset)
            .unwrap_or_else(|| panic!("{runner} runs no cargo command"));
        assert!(at < first_cargo, "{runner} checks for heavy jobs only after it starts cargo");
    }
}
