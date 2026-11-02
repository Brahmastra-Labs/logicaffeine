//! Returned runtime archives must remain paired with their object across later
//! Cargo clean and rebuild operations. Each fixture owns an isolated process.
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
import hashlib, pathlib, sys
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
        r#"#!/bin/sh
set -eu
target=
while [ "$#" -gt 0 ]; do
    if [ "$1" = --target-dir ]; then shift; target=$1; fi
    shift
done
test -n "$target"
printf '%s' "$target" > "$LOGOS_CACHE_FIXTURE_TARGET"
printf 'build\n' >> "$LOGOS_CACHE_FIXTURE_BUILDS"
deps="$target/wasm32-unknown-unknown/debug/deps"
host="$target/debug/deps"
mkdir -p "$deps" "$host"
for name in logicaffeine_base logicaffeine_compile dependency; do
    artifact_dir="$deps"
    if [ "$LOGOS_CACHE_FIXTURE_MODE" = primary ] && [ "$name" = logicaffeine_compile ]; then
        artifact_dir="$target/wasm32-unknown-unknown/debug"
    fi
    printf 'current-%s' "$name" > "$artifact_dir/lib$name-current.rlib"
    printf '{"reason":"compiler-artifact","package_id":"path+file:///fixture/%s#0.11.0","manifest_path":"/fixture/%s/Cargo.toml","target":{"kind":["lib"],"crate_types":["lib"],"name":"%s","src_path":"/fixture/%s/src/lib.rs","edition":"2021","doc":true,"doctest":true,"test":true},"profile":{"opt_level":"0","debuginfo":2,"debug_assertions":true,"overflow_checks":true,"test":false},"features":[],"filenames":["%s/lib%s-current.rlib"],"executable":null,"fresh":false}\n' "$name" "$name" "$name" "$name" "$artifact_dir" "$name"
done
for name in logicaffeine_base logicaffeine_compile orphan; do
    printf 'stale-%s' "$name" > "$deps/lib$name-stale.rlib"
    touch -t 203801010000 "$deps/lib$name-stale.rlib"
done
printf 'host-only' > "$host/libhost_only-current.rlib"
printf '{"reason":"compiler-artifact","package_id":"path+file:///fixture/host_only#0.11.0","manifest_path":"/fixture/host_only/Cargo.toml","target":{"kind":["lib"],"crate_types":["lib"],"name":"host_only","src_path":"/fixture/host_only/src/lib.rs","edition":"2021","doc":true,"doctest":true,"test":true},"profile":{"opt_level":"0","debuginfo":2,"debug_assertions":true,"overflow_checks":true,"test":false},"features":[],"filenames":["%s/libhost_only-current.rlib"],"executable":null,"fresh":false}\n' "$host"
printf '{"reason":"build-finished","success":true}\n'
"#,
    );
    executable(
        &tools.join("rustc"),
        r#"#!/bin/sh
set -eu
case "$1" in
  -vV) printf 'host: fixture-host\n'; exit 0 ;;
  --print) printf '%s\n' "$LOGOS_CACHE_FIXTURE_ROOT"; exit 0 ;;
esac
printf '%s\n' "$@" > "$LOGOS_CACHE_FIXTURE_ARGS"
out=
while [ "$#" -gt 0 ]; do
    if [ "$1" = -o ]; then shift; out=$1; fi
    shift
done
test -n "$out"
printf '\000asm\001\000\000\000' > "$out"
"#,
    );
    let args = root.path().join("rustc-args");
    let result = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "vm::wasm::link::cache_snapshot_tests::artifact_build_fixture_child",
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
fn repeated_cached_links_reuse_one_complete_runtime() {
    fixture("unchanged");
}

#[test]
fn cached_runtime_archives_survive_a_later_cargo_clean() {
    fixture("removed");
}

#[test]
fn cached_runtime_archives_keep_their_bytes_after_a_later_rebuild() {
    fixture("rewritten");
}

#[test]
#[ignore = "subprocess fixture; invoked with isolated PATH by the three snapshot parent tests"]
fn artifact_build_fixture_child() {
    let Ok(mode) = std::env::var("LOGOS_CACHE_FIXTURE_MODE") else {
        return;
    };
    let program = b"\0asm\x01\0\0\0\x00\x02\0P";
    let before = link_relocatable_bigint(program).expect("the first complete link succeeds");
    let shared = PathBuf::from(
        std::fs::read_to_string(std::env::var_os("LOGOS_CACHE_FIXTURE_TARGET").unwrap()).unwrap(),
    );
    match mode.as_str() {
        "removed" => std::fs::remove_dir_all(&shared).unwrap(),
        "rewritten" => {
            for entry in
                std::fs::read_dir(shared.join("wasm32-unknown-unknown/debug/deps")).unwrap()
            {
                let path = entry.unwrap().path();
                if path.extension().is_some_and(|ext| ext == "rlib") {
                    std::fs::write(path, b"a different later build").unwrap();
                }
            }
        }
        "unchanged" => {}
        other => panic!("unknown snapshot fixture mode {other}"),
    }
    let after = link_relocatable_bigint(program)
        .expect("cached link must survive later Cargo cleanup or rebuild");
    assert_eq!(
        after, before,
        "a cached link must retain exactly its original runtime dependency bytes"
    );
    let builds =
        std::fs::read_to_string(std::env::var_os("LOGOS_CACHE_FIXTURE_BUILDS").unwrap()).unwrap();
    assert_eq!(
        builds.lines().count(),
        1,
        "the process cache should reuse the completed runtime"
    );
}
