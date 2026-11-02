//! ════════════════════════════════════════════════════════════════════════════════════════════════
//! qemu_wrapper_lock — the memory-capped `qemu-riscv64` wrapper is a ceiling, and it is silent when the
//! ceiling already holds.
//!
//! `scripts/test/toolwrap/qemu-riscv64` caps the emulator's address space (qemu-user maps a guest's
//! allocations into its own host process, and an unbounded guest once took the whole box down), and the
//! full-suite runners put it ahead of `/usr/bin` on `PATH`. The parity harness runs every RISC-V program
//! under a stricter cap of its own (`prlimit --as`, 512 MiB), and a differential compares the program's
//! stderr byte for byte with the VM's. So the wrapper must keep an inherited cap that is already at or
//! under its own, and say nothing: raising it is impossible, and a warning would land in the program's
//! stderr and turn every RISC-V run into a divergence.
//!
//! The limit is read from the guest itself: a static RV64 program prints `/proc/self/limits`, which
//! qemu-user serves from the emulator's own process.
//!
//!   cargo nextest run -p logicaffeine-tests --test qemu_wrapper_lock
//! ════════════════════════════════════════════════════════════════════════════════════════════════

#![cfg(target_os = "linux")]

mod common;

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

fn wrapper() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../scripts/test/toolwrap/qemu-riscv64")
}

fn tools() -> bool {
    ["qemu-riscv64", "riscv64-linux-gnu-gcc", "prlimit"]
        .iter()
        .all(|t| common::require_tool(t))
}

/// A static RV64 program that prints the emulator process's address-space limit line, built in a directory
/// of the test's own.
fn limits_probe(test: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("logos-qemu-wrapper-{test}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a scratch directory");
    let src = dir.join("limits.c");
    std::fs::write(
        &src,
        "#include <stdio.h>\n#include <string.h>\nint main(void) {\n  FILE *f = fopen(\"/proc/self/limits\", \"r\");\n  char line[256];\n  if (!f) return 3;\n  while (fgets(line, sizeof line, f)) if (strncmp(line, \"Max address space\", 17) == 0) fputs(line, stdout);\n  return 0;\n}\n",
    )
    .expect("write the probe");
    let bin = dir.join("limits");
    let built = Command::new("riscv64-linux-gnu-gcc")
        .args(["-static", "-O1", "-o"])
        .arg(&bin)
        .arg(&src)
        .output()
        .expect("riscv64-linux-gnu-gcc runs");
    assert!(
        built.status.success(),
        "the probe builds:\n{}",
        String::from_utf8_lossy(&built.stderr)
    );
    bin
}

/// The wrapper running `probe` under an inherited address-space limit of `inherited_cap` bytes (or none),
/// with `env` set.
fn run(probe: &Path, inherited_cap: Option<&str>, env: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new("prlimit");
    cmd.arg(format!("--as={}", inherited_cap.unwrap_or("unlimited")))
        .arg("--")
        .arg(wrapper());
    cmd.arg(probe)
        .env_remove("LOGICAFFEINE_QEMU_MEM_MB")
        .stdin(Stdio::null());
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.output().expect("the wrapper runs")
}

/// The soft address-space limit the guest saw, in bytes, or `None` for unlimited.
fn soft_limit(out: &Output) -> Option<u64> {
    let text = String::from_utf8_lossy(&out.stdout);
    let line = text
        .lines()
        .find(|l| l.starts_with("Max address space"))
        .unwrap_or_else(|| panic!("the probe printed no limit: {text:?}"));
    let soft = line["Max address space".len()..]
        .split_whitespace()
        .next()
        .expect("a soft limit");
    (soft != "unlimited").then(|| {
        soft.parse()
            .unwrap_or_else(|_| panic!("a numeric limit in {line:?}"))
    })
}

#[test]
fn a_stricter_inherited_cap_is_kept_and_nothing_is_said() {
    if !tools() {
        return;
    }
    let probe = limits_probe("stricter");
    let out = run(&probe, Some("536870912"), &[]);
    assert!(out.status.success(), "the probe runs under a 512 MiB cap");
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        "",
        "the wrapper wrote into the program's stderr under a cap stricter than its own"
    );
    assert_eq!(
        soft_limit(&out),
        Some(512 << 20),
        "the inherited 512 MiB cap is kept"
    );
}

#[test]
fn an_uncapped_run_is_capped_at_the_default() {
    if !tools() {
        return;
    }
    let probe = limits_probe("uncapped");
    let out = run(&probe, None, &[]);
    assert!(out.status.success(), "the probe runs");
    assert_eq!(String::from_utf8_lossy(&out.stderr), "");
    assert_eq!(
        soft_limit(&out),
        Some(8192 << 20),
        "an unlimited emulator is capped at the 8192 MB default"
    );
}

#[test]
fn the_override_sets_the_cap_and_zero_lifts_it() {
    if !tools() {
        return;
    }
    let probe = limits_probe("override");
    let out = run(&probe, None, &[("LOGICAFFEINE_QEMU_MEM_MB", "2048")]);
    assert_eq!(
        (
            String::from_utf8_lossy(&out.stderr).into_owned(),
            soft_limit(&out)
        ),
        (String::new(), Some(2048 << 20))
    );
    let out = run(
        &probe,
        Some("1073741824"),
        &[("LOGICAFFEINE_QEMU_MEM_MB", "4096")],
    );
    assert_eq!(
        (
            String::from_utf8_lossy(&out.stderr).into_owned(),
            soft_limit(&out)
        ),
        (String::new(), Some(1 << 30)),
        "an override above an inherited cap keeps the inherited one"
    );
    let out = run(&probe, None, &[("LOGICAFFEINE_QEMU_MEM_MB", "0")]);
    assert_eq!(soft_limit(&out), None, "zero leaves the emulator uncapped");
}
