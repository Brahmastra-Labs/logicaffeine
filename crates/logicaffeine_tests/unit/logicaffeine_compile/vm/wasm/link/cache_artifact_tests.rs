//! The Cargo build graph, not file age or a directory census, defines the
//! dependency closure of the linked runtime. Toolchain stubs exercise the real
//! build helper without compiling WebAssembly or modifying process-global PATH.
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
printf '%s\n' "$@" > "$LOGOS_CACHE_FIXTURE_ARGS"
out=
while [ "$#" -gt 0 ]; do
    if [ "$1" = -o ]; then shift; out=$1; fi
    shift
done
test -n "$out"
printf 'fixture-runtime-object' > "$out"
"#,
    );
    let args = root.path().join("rustc-args");
    let result = Command::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "vm::wasm::link::cache_artifact_tests::artifact_build_fixture_child",
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
fn runtime_roots_come_from_the_successful_build_not_the_newest_rlib() {
    fixture("roots");
}

#[test]
fn linker_dependencies_exclude_stale_and_host_artifacts() {
    fixture("closure");
}

#[test]
fn a_primary_library_reported_outside_deps_remains_a_runtime_root() {
    fixture("primary");
}

#[test]
#[ignore = "subprocess fixture; invoked with isolated PATH by the two parent tests"]
fn artifact_build_fixture_child() {
    let Ok(mode) = std::env::var("LOGOS_CACHE_FIXTURE_MODE") else {
        return;
    };
    let scratch = std::env::temp_dir().join("runtime-scratch");
    let (object, libraries) = build_bigint_runtime(&scratch).expect("stub build succeeds");
    assert_eq!(object, b"fixture-runtime-object");
    match mode.as_str() {
        "roots" | "primary" => {
            let args =
                std::fs::read_to_string(std::env::var_os("LOGOS_CACHE_FIXTURE_ARGS").unwrap())
                    .unwrap();
            for name in ["logicaffeine_base", "logicaffeine_compile"] {
                let selected = args
                    .lines()
                    .find(|arg| arg.starts_with(&format!("{name}=")))
                    .expect("runtime extern root");
                assert!(selected.ends_with(&format!("lib{name}-current.rlib")),
                    "Cargo reported the current artifact; file age must not select a different build: {selected}");
                if mode == "primary" && name == "logicaffeine_compile" {
                    assert!(
                        !selected.contains("/deps/"),
                        "use Cargo's primary output path"
                    );
                    assert!(
                        libraries
                            .iter()
                            .any(|p| selected.ends_with(p.to_str().unwrap())),
                        "the selected primary root must be offered to the linker"
                    );
                }
            }
        }
        "closure" => {
            let mut names: Vec<_> = libraries
                .iter()
                .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
                .collect();
            names.sort();
            assert_eq!(
                names,
                [
                    "libdependency-current.rlib",
                    "liblogicaffeine_base-current.rlib",
                    "liblogicaffeine_compile-current.rlib",
                ],
                "the successful wasm build graph alone supplies linker dependencies"
            );
        }
        other => panic!("unknown artifact fixture mode {other}"),
    }
}
