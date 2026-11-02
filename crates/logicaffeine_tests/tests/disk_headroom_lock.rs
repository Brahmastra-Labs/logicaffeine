//! A full test run is refused before it builds anything when a disk it writes to is short of room.
//!
//! A full run writes tens of gigabytes of test scratch into its `TMPDIR` (the cargo projects, e2e
//! caches and per-process lane directories the tests build) and rebuilds into its target directory.
//! A run that fills its disk fails every test after the first `No space left on device` for a reason
//! that is not the test's own, and fails every other build on that disk with it, so the result cannot
//! be read. `scripts/test/disk-headroom.sh <GB> <dir>…` refuses when the filesystem holding any
//! directory has fewer than `<GB>` gigabytes free, and both full runners call it on their `TMPDIR` and
//! their target directory before their first cargo command.

use std::path::{Path, PathBuf};
use std::process::Command;

/// The free space, in gigabytes, a full run needs on each disk it writes to.
const FLOOR_GB: &str = "150";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn headroom(floor: &str, dirs: &[&Path]) -> (bool, String) {
    let out = Command::new("bash")
        .arg(repo_root().join("scripts/test/disk-headroom.sh"))
        .arg(floor)
        .args(dirs)
        .output()
        .expect("bash runs the headroom check");
    (out.status.success(), String::from_utf8_lossy(&out.stderr).into_owned())
}

#[test]
fn the_headroom_check_refuses_a_floor_no_disk_meets_and_names_the_directory() {
    let tmp = std::env::temp_dir();
    let (ok, err) = headroom("1000000000", &[&tmp]);
    assert!(!ok, "no disk has an exabyte free, so the check must refuse");
    assert!(err.contains(&tmp.display().to_string()), "the refusal names the directory: {err}");
    assert!(err.contains("GB free"), "the refusal says how much room there is: {err}");
}

#[test]
fn the_headroom_check_passes_a_floor_every_disk_meets() {
    let tmp = std::env::temp_dir();
    let (ok, err) = headroom("0", &[&tmp, &repo_root()]);
    assert!(ok, "every disk has zero gigabytes free or more: {err}");
}

#[test]
fn a_directory_not_made_yet_is_measured_on_the_disk_it_will_be_made_on() {
    let unborn = std::env::temp_dir().join("disk-headroom-lock-not-made").join("deeper");
    assert!(!unborn.exists());
    let (ok, err) = headroom("0", &[&unborn]);
    assert!(ok, "a target directory a run will create is measured where it will be created: {err}");
    let (refused, _) = headroom("1000000000", &[&unborn]);
    assert!(!refused);
}

/// Both full runners call the check on their scratch and their target directory, at the floor, before
/// any cargo command they run.
#[test]
fn both_full_runners_check_headroom_before_their_first_cargo_command() {
    for runner in ["run-all-tests-fast.sh", "run-all-tests.sh"] {
        let src = std::fs::read_to_string(repo_root().join("scripts").join(runner)).expect("the runner");
        let call = format!("scripts/test/disk-headroom.sh\" {FLOOR_GB} \"${{TMPDIR:-/tmp}}\" \"${{CARGO_TARGET_DIR:-$REPO_ROOT/target}}\"");
        let at = src
            .find(&call)
            .unwrap_or_else(|| panic!("{runner} does not check {FLOOR_GB} GB of headroom on its TMPDIR and target: {call}"));
        let first_cargo = src
            .lines()
            .scan(0usize, |offset, line| {
                let here = *offset;
                *offset += line.len() + 1;
                Some((here, line.trim_start()))
            })
            .find(|(_, line)| !line.starts_with('#') && !line.starts_with("echo") && (line.starts_with("cargo test") || line.starts_with("cargo nextest") || line.starts_with("cargo build")))
            .map(|(offset, _)| offset)
            .unwrap_or_else(|| panic!("{runner} runs no cargo command"));
        assert!(at < first_cargo, "{runner} checks headroom only after it starts cargo");
    }
}
