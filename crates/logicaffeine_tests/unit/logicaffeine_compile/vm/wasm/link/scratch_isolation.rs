//! Concurrent links own their intermediates until their output is read.
use super::*;
use std::os::unix::fs::PermissionsExt;

fn executable(path: &std::path::Path, source: &str) {
    std::fs::write(path, source).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

fn fixture(mode: &str) {
    let root = tempfile::Builder::new()
        .prefix("logos-link-isolation-")
        .tempdir_in("/tmp")
        .unwrap();
    let tools = root.path().join("tools");
    let linker = root.path().join("lib/rustlib/fixture-host/bin");
    std::fs::create_dir_all(&tools).unwrap();
    std::fs::create_dir_all(&linker).unwrap();
    executable(
        &tools.join("rustc"),
        r#"#!/bin/sh
set -eu
case "$1" in
  -vV) printf 'host: fixture-host\n' ;;
  --print) printf '%s\n' "$LOGOS_LINK_FIXTURE_ROOT" ;;
  *) exit 91 ;;
esac
"#,
    );
    executable(
        &linker.join("rust-lld"),
        r#"#!/usr/bin/python3
import os, pathlib, sys, time
root = pathlib.Path(os.environ["LOGOS_LINK_FIXTURE_ROOT"])
mode = os.environ["LOGOS_LINK_FIXTURE_MODE"]
args = sys.argv[1:]
source = pathlib.Path(next(x for x in args if x.endswith("obj0.o")))
out = pathlib.Path(args[args.index("-o") + 1])
if mode == "parallel":
    (root / ("ready-" + str(os.getpid()))).write_text(str(source))
    deadline = time.monotonic() + 20
    while len(list(root.glob("ready-*"))) != 2:
        if time.monotonic() > deadline:
            raise RuntimeError("second link never reached its linker")
        time.sleep(0.005)
payload = source.read_bytes()
out.write_bytes(payload)
if mode == "failure":
    sys.exit(37)
"#,
    );
    let output = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "vm::wasm::link::scratch_isolation::isolated_link_fixture_child",
            "--ignored",
            "--nocapture",
        ])
        .env(
            "PATH",
            format!(
                "{}:{}",
                tools.display(),
                std::env::var("PATH").unwrap_or_default()
            ),
        )
        .env("LOGOS_LINK_FIXTURE_ROOT", root.path())
        .env("LOGOS_LINK_FIXTURE_MODE", mode)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{mode} fixture failed:\n{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn a_single_link_returns_its_own_object() {
    fixture("single");
}

#[test]
fn simultaneous_links_in_one_scratch_root_keep_their_own_programs() {
    fixture("parallel");
}

#[test]
fn successful_links_release_only_their_owned_scratch() {
    fixture("cleanup");
}

#[test]
fn failed_links_release_only_their_owned_scratch() {
    fixture("failure");
}

#[test]
#[ignore = "subprocess fixture invoked by the four isolated parent controls"]
fn isolated_link_fixture_child() {
    let Ok(mode) = std::env::var("LOGOS_LINK_FIXTURE_MODE") else {
        return;
    };
    let root = PathBuf::from(std::env::var_os("LOGOS_LINK_FIXTURE_ROOT").unwrap());
    let scratch = root.join("shared");
    std::fs::create_dir(&scratch).unwrap();
    let sentinel = scratch.join("caller-owned.txt");
    std::fs::write(&sentinel, b"preserve caller data").unwrap();
    // Empty modules with different custom-section payloads are valid link inputs.
    let a = b"\0asm\x01\0\0\0\x00\x02\0A".to_vec();
    let b = b"\0asm\x01\0\0\0\x00\x02\0B".to_vec();
    if mode == "parallel" {
        std::thread::scope(|scope| {
            let one = scope.spawn(|| link_objects_with_rlibs(&[&a], &[], false, &scratch));
            let two = scope.spawn(|| link_objects_with_rlibs(&[&b], &[], false, &scratch));
            let first = one.join().unwrap().expect("first link succeeds");
            let second = two.join().unwrap().expect("second link succeeds");
            assert_eq!(first, a, "first caller must receive its own program");
            assert_eq!(second, b, "second caller must receive its own program");
        });
    } else {
        let got = link_objects_with_rlibs(&[&a], &[], false, &scratch);
        if mode == "failure" {
            assert!(got.is_err(), "linker failure must propagate");
        } else {
            assert_eq!(got.expect("single link succeeds"), a);
        }
    }
    assert_eq!(std::fs::read(&sentinel).unwrap(), b"preserve caller data");
    if mode == "cleanup" || mode == "failure" {
        let entries: Vec<_> = std::fs::read_dir(&scratch)
            .unwrap()
            .map(|e| e.unwrap().file_name())
            .collect();
        assert_eq!(
            entries,
            [std::ffi::OsString::from("caller-owned.txt")],
            "each invocation must release its own intermediates on every exit"
        );
    }
}
