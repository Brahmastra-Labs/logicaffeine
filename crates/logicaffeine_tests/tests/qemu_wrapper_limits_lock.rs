//! The emulator wrapper composes its cap with its caller's limits without adding guest diagnostics.
#![cfg(target_os = "linux")]

use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::atomic::{AtomicUsize, Ordering};

struct Fixture(PathBuf);

impl Fixture {
    fn new() -> Self {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!(
            "logos_qemu_wrapper_{}_{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&dir).expect("create wrapper fixture");
        let emulator = dir.join("emulator");
        std::fs::write(
            &emulator,
            r#"#!/bin/bash
printf 'soft=%s hard=%s\n' "$(ulimit -Sv)" "$(ulimit -Hv)"
if [ "$#" -gt 0 ]; then printf '%s\n' "$1" >&2; exit "$2"; fi
"#,
        )
        .unwrap();
        std::fs::set_permissions(&emulator, std::fs::Permissions::from_mode(0o755)).unwrap();
        let repo = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
        let source =
            std::fs::read_to_string(repo.join("scripts/test/toolwrap/qemu-riscv64")).unwrap();
        let binding = "REAL=/usr/bin/qemu-riscv64";
        assert_eq!(source.matches(binding).count(), 1, "one emulator binding");
        // Only the external emulator is replaced; every wrapper instruction runs unchanged.
        std::fs::write(
            dir.join("wrapper"),
            source.replace(binding, &format!("REAL='{}'", emulator.display())),
        )
        .unwrap();
        Self(dir)
    }

    fn run(&self, soft: u64, hard: u64, mb: u64, args: &[&str]) -> Output {
        Command::new("bash")
            .args([
                "-c",
                "ulimit -Sv \"$1\" && ulimit -Hv \"$2\" && shift 2 && exec bash \"$@\"",
                "fixture",
            ])
            .arg(soft.to_string())
            .arg(hard.to_string())
            .arg(self.0.join("wrapper"))
            .args(args)
            .env("LOGICAFFEINE_QEMU_MEM_MB", mb.to_string())
            .output()
            .expect("run wrapper")
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        std::fs::remove_dir_all(&self.0).unwrap();
    }
}

fn assert_limits(out: Output, soft: u64, hard: u64) {
    assert!(out.status.success(), "wrapper failed: {out:?}");
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        format!("soft={soft} hard={hard}\n")
    );
    assert!(
        out.stderr.is_empty(),
        "wrapper contaminated guest stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn inherited_stricter_hard_limit_is_sufficient_without_a_warning() {
    assert_limits(
        Fixture::new().run(524288, 524288, 8192, &[]),
        524288,
        524288,
    );
}

#[test]
fn inherited_stricter_soft_limit_is_preserved() {
    assert_limits(
        Fixture::new().run(524288, 1048576, 8192, &[]),
        524288,
        1048576,
    );
}

#[test]
fn requested_stricter_limit_is_applied() {
    assert_limits(Fixture::new().run(524288, 524288, 256, &[]), 262144, 262144);
}

#[test]
fn disabling_the_wrapper_cap_preserves_inherited_limits() {
    assert_limits(Fixture::new().run(524288, 1048576, 0, &[]), 524288, 1048576);
}

#[test]
fn guest_diagnostics_and_exit_status_are_preserved_exactly() {
    let out = Fixture::new().run(524288, 524288, 8192, &["guest failure", "42"]);
    assert_eq!(out.status.code(), Some(42));
    assert_eq!(out.stdout, b"soft=524288 hard=524288\n");
    assert_eq!(out.stderr, b"guest failure\n");
}

#[test]
fn an_inherited_soft_cap_is_not_raised_even_when_the_hard_cap_allows_it() {
    assert_limits(
        Fixture::new().run(524288, 16777216, 8192, &[]),
        524288,
        16777216,
    );
}
