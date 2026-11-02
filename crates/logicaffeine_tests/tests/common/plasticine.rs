//! The public Plasticine toolchain (Spatial → PIR → plastiroute → Tungsten, built by
//! `scripts/plasticine/`) as a test-time CAPABILITY: where its tree is, whether a gate can run from
//! this process, and how to run one app through `gate.sh` in the container and read the chain's answer.
//!
//! `tool_available("plasticine-toolchain")` routes here, so a chain-gated lock skips with the honest
//! `LOGOS_TOOL_SKIP:` marker when the tree is not fetched+built or docker is not reachable, and fails loud
//! under `LOGOS_REQUIRE_TOOLS=plasticine-toolchain`.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::process::Command;

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("the workspace root resolves from the test crate")
}

/// The toolchain tree (`run-in-container.sh` mounts it at `/work`); `LOGOS_PLASTICINE_TOOLCHAIN` overrides,
/// exactly as the scripts read it.
pub fn toolchain_dir() -> PathBuf {
    std::env::var_os("LOGOS_PLASTICINE_TOOLCHAIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| repo_root().join("target/plasticine-toolchain"))
}

/// Spatial's `pirTest` regression apps — the directory an emitted `Logos_<name>.scala` goes into so that
/// `gate.sh Logos_<name>` finds it.
pub fn regression_dir() -> PathBuf {
    toolchain_dir().join("spatial/pir/regression")
}

pub fn run_in_container() -> PathBuf {
    repo_root().join("scripts/plasticine/run-in-container.sh")
}

/// How docker answers from this process: directly, or only through `sg docker` (the group membership the
/// box grants at login but not to every spawned shell). `None` when neither works.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Docker {
    Direct,
    ViaSg,
}

pub fn docker() -> Option<Docker> {
    let ok =
        |o: std::io::Result<std::process::Output>| o.map(|o| o.status.success()).unwrap_or(false);
    if ok(Command::new("docker").arg("info").output()) {
        return Some(Docker::Direct);
    }
    if ok(Command::new("sg")
        .args(["docker", "-c", "docker info"])
        .output())
    {
        return Some(Docker::ViaSg);
    }
    None
}

/// The fetched+built tree is present and docker is reachable. The three files checked are the ones a gate
/// run reads first: Spatial's build, the regression dir the app is written into, and the runner script.
pub fn ready() -> bool {
    toolchain_dir().join("spatial/build.sbt").is_file()
        && regression_dir().is_dir()
        && run_in_container().is_file()
        && docker().is_some()
}

/// What one `gate.sh <app>` run said.
pub struct GateRun {
    pub app: String,
    pub status: Option<i32>,
    pub log: String,
    pub log_path: PathBuf,
}

impl GateRun {
    /// Tungsten's `Simulation complete at cycle N` — the LAST one printed (the hybrid-network run follows the
    /// ideal-network run when both are simulated).
    pub fn tungsten_cycles(&self) -> Option<u64> {
        self.log
            .lines()
            .filter_map(|l| l.split("Simulation complete at cycle ").nth(1))
            .filter_map(|r| {
                r.trim()
                    .split(|c: char| !c.is_ascii_digit())
                    .next()?
                    .parse()
                    .ok()
            })
            .last()
    }

    pub fn passed(&self) -> bool {
        self.log.contains(&format!("PASS: true ({})", self.app))
    }

    pub fn tail(&self, n: usize) -> String {
        let lines: Vec<&str> = self.log.lines().collect();
        lines[lines.len().saturating_sub(n)..].join("\n")
    }
}

fn shell_quote(p: &Path) -> String {
    format!("'{}'", p.to_string_lossy().replace('\'', "'\\''"))
}

/// Run `gate.sh <app>` through `run-in-container.sh` (building the image if needed), capture everything the
/// chain printed, and keep the log under `target/plasticine-gate-logs/<app>.log` for the ledger and for a
/// failure's post-mortem.
pub fn run_gate(app: &str) -> GateRun {
    run_script("gate.sh", app, "log")
}

/// Run `sim.sh <app>` the same way — Spatial's own Scala backend, the app compiled and simulated functionally with
/// the host's check, no fabric mapping: the gate for an app too large for PIR to hold as one JVM class. The log is
/// kept under `target/plasticine-gate-logs/<app>.sim.log`.
pub fn run_sim(app: &str) -> GateRun {
    run_script("sim.sh", app, "sim.log")
}

fn run_script(script: &str, app: &str, suffix: &str) -> GateRun {
    let mode = docker().expect("docker is reachable (checked by `ready`)");
    let cmd = format!("{} bash /{script} {app}", shell_quote(&run_in_container()));
    let output = match mode {
        Docker::Direct => Command::new("bash")
            .arg("-c")
            .arg(&cmd)
            .current_dir(repo_root())
            .output(),
        Docker::ViaSg => Command::new("sg")
            .args(["docker", "-c", &cmd])
            .current_dir(repo_root())
            .output(),
    }
    .unwrap_or_else(|e| panic!("spawning {script} for {app}: {e}"));
    let mut log = String::from_utf8_lossy(&output.stdout).into_owned();
    if !output.stderr.is_empty() {
        log.push_str("\n=== stderr\n");
        log.push_str(&String::from_utf8_lossy(&output.stderr));
    }
    let dir = repo_root().join("target/plasticine-gate-logs");
    std::fs::create_dir_all(&dir).expect("the gate log dir");
    let log_path = dir.join(format!("{app}.{suffix}"));
    std::fs::write(&log_path, &log).expect("write the gate log");
    // ★ AN APP THAT DOES NOT COMPILE LEAVES THE REGRESSION DIRECTORY. `pirTest` compiles every app in it on every
    // run, so one that fails to compile fails every later gate and simulation on the box, whatever app they were
    // for. It is moved beside its log, where the failure's post-mortem still has it.
    if log.contains("Compilation failed") {
        for ext in ["scala", "lg"] {
            let placed = regression_dir().join(format!("{app}.{ext}"));
            if placed.is_file() {
                let kept = dir.join(format!("{app}.uncompiled.{ext}"));
                std::fs::rename(&placed, &kept)
                    .or_else(|_| {
                        std::fs::copy(&placed, &kept).and_then(|_| std::fs::remove_file(&placed))
                    })
                    .expect("move the uncompiled app out of the regression dir");
            }
        }
    }
    GateRun {
        app: app.to_string(),
        status: output.status.code(),
        log,
        log_path,
    }
}
