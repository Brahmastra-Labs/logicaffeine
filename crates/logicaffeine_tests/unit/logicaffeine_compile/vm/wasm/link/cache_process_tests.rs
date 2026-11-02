//! Runtime build serialization, retry, and per-call scratch ownership.
use super::*;
use std::os::unix::fs::PermissionsExt;

fn executable(path: &std::path::Path, source: &str) {
    std::fs::write(path, source).expect("write toolchain fixture");
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
        .expect("executable fixture");
}

fn fixture(mode: &str) {
    let root = tempfile::Builder::new()
        .prefix("logos-cache-artifacts-")
        .tempdir_in("/tmp")
        .unwrap();
    let tools = root.path().join("tools");
    let temp = root.path().join("temp");
    std::fs::create_dir(&tools).unwrap();
    std::fs::create_dir(&temp).unwrap();
    let linker = root.path().join("lib/rustlib/fixture-host/bin");
    std::fs::create_dir_all(&linker).unwrap();
    std::fs::create_dir_all(root.path().join("lib/rustlib/wasm32-unknown-unknown/lib")).unwrap();
    executable(
        &linker.join("rust-lld"),
        r#"#!/usr/bin/python3
import hashlib, pathlib, sys, os
failure = pathlib.Path(os.environ["LOGOS_CACHE_FIXTURE_ROOT"]) / "fail-next-link"
if failure.exists():
    failure.unlink()
    sys.exit(1)
args = sys.argv[1:]
archives = [pathlib.Path(a) for a in args if a.endswith(".rlib")]
assert len(archives) == 3, "the complete target archive set must reach the linker"
payload = b"".join(p.read_bytes() for p in archives)
payload += pathlib.Path(next(a for a in args if a.endswith("obj0.o"))).read_bytes()
out = pathlib.Path(args[args.index("-o") + 1])
out.write_bytes(b"\x00asm\x01\x00\x00\x00\x00\x21\x00" + hashlib.sha256(payload).digest())
"#,
    );

    executable(
        &tools.join("cargo"),
        r#"#!/usr/bin/python3
import json, os, pathlib, sys
args = sys.argv[1:]
target = pathlib.Path(args[args.index("--target-dir") + 1])
root = pathlib.Path(os.environ["LOGOS_CACHE_FIXTURE_ROOT"])
mode = os.environ["LOGOS_CACHE_FIXTURE_MODE"]
pathlib.Path(os.environ["LOGOS_CACHE_FIXTURE_TARGET"]).write_text(str(target))
with pathlib.Path(os.environ["LOGOS_CACHE_FIXTURE_BUILDS"]).open("a") as f:
    f.write("build\n")
if mode == "retry" and not (root / "first-failed").exists():
    (root / "first-failed").touch()
    print(json.dumps({"reason":"build-finished","success":False}))
    sys.exit(1)
if mode == "worker-b" and not (root / "release-a").exists():
    sys.exit("second Cargo build overlapped the first runtime compilation")
deps = target / "wasm32-unknown-unknown/debug/deps"
deps.mkdir(parents=True, exist_ok=True)
(target / "debug/deps").mkdir(parents=True, exist_ok=True)
for name in ["logicaffeine_base", "logicaffeine_compile", "dependency"]:
    path = deps / ("lib" + name + "-current.rlib")
    path.write_bytes(("current-" + name).encode())
    print(json.dumps({"reason":"compiler-artifact","target":{"name":name},"filenames":[str(path)]}))
print(json.dumps({"reason":"build-finished","success":True}))
"#,
    );
    executable(
        &tools.join("rustc"),
        r#"#!/usr/bin/python3
import os, pathlib, sys, time
args = sys.argv[1:]
root = pathlib.Path(os.environ["LOGOS_CACHE_FIXTURE_ROOT"])
if args[0] == "-vV":
    print("host: fixture-host")
    sys.exit(0)
if args[0] == "--print":
    print(root)
    sys.exit(0)
if os.environ["LOGOS_CACHE_FIXTURE_MODE"] == "worker-a":
    (root / "rustc-a-ready").touch()
    deadline = time.monotonic() + 15
    while not (root / "release-a").exists():
        assert time.monotonic() < deadline, "fixture barrier timed out waiting for release-a"
        time.sleep(.01)
pathlib.Path(os.environ["LOGOS_CACHE_FIXTURE_ARGS"]).write_text("\n".join(args))
pathlib.Path(args[args.index("-o") + 1]).write_bytes(b"\x00asm\x01\x00\x00\x00")
"#,
    );
    let args = root.path().join("rustc-args");
    let result = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "vm::wasm::link::cache_process_tests::process_fixture_child",
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
        .env("TMPDIR", &temp)
        .env("LOGOS_CACHE_FIXTURE_MODE", mode)
        .env("LOGOS_CACHE_FIXTURE_ARGS", &args)
        .env(
            "LOGOS_CACHE_FIXTURE_TARGET",
            root.path().join("cargo-target"),
        )
        .env("LOGOS_CACHE_FIXTURE_BUILDS", root.path().join("builds"))
        .env("LOGOS_CACHE_FIXTURE_ROOT", root.path())
        .output()
        .expect("run isolated fixture process");
    assert!(
        result.status.success(),
        "artifact fixture {mode} failed:\n{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
}

#[test]
fn successful_cached_links_release_their_temporary_files() {
    fixture("cleanup");
}

#[test]
fn a_failed_link_releases_temporary_files_and_keeps_the_cache_usable() {
    fixture("cleanup-failure");
}

#[test]
fn a_failed_runtime_build_can_be_retried() {
    fixture("retry");
}

#[test]
fn concurrent_processes_serialize_the_build_through_runtime_compilation() {
    fixture("concurrent");
}

fn wait_for(path: &std::path::Path) {
    let start = std::time::Instant::now();
    while !path.exists() {
        assert!(
            start.elapsed() < std::time::Duration::from_secs(10),
            "fixture barrier timed out: {}",
            path.display()
        );
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
#[ignore = "subprocess fixture; invoked with isolated PATH by the process parent tests"]
fn process_fixture_child() {
    let Ok(mode) = std::env::var("LOGOS_CACHE_FIXTURE_MODE") else {
        return;
    };
    let root = PathBuf::from(std::env::var_os("LOGOS_CACHE_FIXTURE_ROOT").unwrap());
    let program = b"\0asm\x01\0\0\0\x00\x02\0P";
    if mode == "concurrent" {
        let spawn = |mode| {
            Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "vm::wasm::link::cache_process_tests::process_fixture_child",
                    "--ignored",
                    "--nocapture",
                ])
                .env("LOGOS_CACHE_FIXTURE_MODE", mode)
                .stdout(std::process::Stdio::piped())
                .stderr(std::process::Stdio::piped())
                .spawn()
                .expect("spawn cache worker")
        };
        let first = spawn("worker-a");
        wait_for(&root.join("rustc-a-ready"));
        let second = spawn("worker-b");
        wait_for(&root.join("worker-b-ready"));
        // Give an unprotected second build time to enter Cargo while A's
        // runtime compile is paused. The deliberate overlap mutant must fail.
        std::thread::sleep(std::time::Duration::from_secs(1));
        std::fs::write(root.join("release-a"), b"").unwrap();
        for (name, worker) in [("first", first), ("second", second)] {
            let result = worker.wait_with_output().unwrap();
            assert!(
                result.status.success(),
                "{name} worker failed:\n{}\n{}",
                String::from_utf8_lossy(&result.stdout),
                String::from_utf8_lossy(&result.stderr)
            );
        }
        return;
    }
    if mode == "worker-b" {
        std::fs::write(root.join("worker-b-ready"), b"").unwrap();
    }
    if mode == "retry" {
        assert!(
            link_relocatable_bigint(program).is_err(),
            "injected initial Cargo failure must surface"
        );
    }
    let before = link_relocatable_bigint(program).expect("complete runtime link succeeds");
    if mode.starts_with("worker-") {
        return;
    }
    if mode == "cleanup-failure" {
        std::fs::write(root.join("fail-next-link"), b"").unwrap();
        assert!(
            link_relocatable_bigint(program).is_err(),
            "injected linker failure must surface"
        );
    }
    let after = link_relocatable_bigint(program).expect("cache remains reusable");
    assert_eq!(before, after, "reused runtime preserves exact linked bytes");
    let builds =
        std::fs::read_to_string(std::env::var_os("LOGOS_CACHE_FIXTURE_BUILDS").unwrap()).unwrap();
    assert_eq!(builds.lines().count(), if mode == "retry" { 2 } else { 1 });
    if mode != "retry" {
        let target = PathBuf::from(
            std::fs::read_to_string(std::env::var_os("LOGOS_CACHE_FIXTURE_TARGET").unwrap())
                .unwrap(),
        );
        let mut leftovers = Vec::new();
        for entry in std::fs::read_dir(std::env::temp_dir()).unwrap() {
            let path = entry.unwrap().path();
            // The reusable Cargo cache and its persistent synchronization file
            // are intentional; program objects and copied archives are per call.
            if path != target && path != target.with_extension("lock") {
                leftovers.push(path);
            }
        }
        assert!(
            leftovers.is_empty(),
            "per-link runtime scratch must be reclaimed: {leftovers:?}"
        );
    }
}
