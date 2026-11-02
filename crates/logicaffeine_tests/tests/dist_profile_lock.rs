//! The binaries we ship are built with the `dist` profile, and the profile stays what it claims.
//!
//! `release` is the everyday optimized build (benchmarks, local runs). What users download from a
//! GitHub release — `largo` and `logicaffeine-lsp` — is built with `[profile.dist]`: `release` plus
//! whole-program optimization across every crate (`lto = "fat"`, `codegen-units = 1`) and no symbol
//! table (`strip = "symbols"`). `panic` stays `unwind`, because production code recovers from panics
//! with `catch_unwind`; an `abort` would turn every such recovery into a crash.
//!
//! The full-suite gate builds its tests with `[profile.test-opt]`: the `test` profile, its checks on, with the
//! libraries the heavy tests spend their time in and every dependency at opt-level 2. A heavy test is as often
//! CPU-bound inside a dependency (the WebAssembly interpreter) as inside our own libraries.

use std::path::{Path, PathBuf};

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap()
}

#[test]
fn the_dist_profile_is_whole_program_stripped_and_unwinding() {
    let manifest: toml::Table = std::fs::read_to_string(root().join("Cargo.toml"))
        .unwrap()
        .parse()
        .unwrap();
    let dist = manifest
        .get("profile")
        .and_then(|p| p.get("dist"))
        .and_then(|d| d.as_table())
        .expect("the root Cargo.toml declares [profile.dist]");
    assert_eq!(
        dist.get("inherits").and_then(|v| v.as_str()),
        Some("release"),
        "dist inherits release"
    );
    assert_eq!(
        dist.get("lto").and_then(|v| v.as_str()),
        Some("fat"),
        "dist optimizes the whole program (lto = \"fat\")"
    );
    assert_eq!(
        dist.get("codegen-units").and_then(|v| v.as_integer()),
        Some(1),
        "dist uses one codegen unit"
    );
    assert_eq!(
        dist.get("strip").and_then(|v| v.as_str()),
        Some("symbols"),
        "dist ships no symbol table"
    );
    assert_ne!(
        dist.get("panic").and_then(|v| v.as_str()),
        Some("abort"),
        "dist must keep panic = unwind (catch_unwind recovers)"
    );
}

#[test]
fn the_release_builds_every_shipped_binary_with_dist() {
    let release = std::fs::read_to_string(root().join(".github/workflows/release.yml")).unwrap();
    let builds: Vec<&str> = release
        .lines()
        .filter(|l| {
            l.contains("cargo build -p logicaffeine-cli")
                || l.contains("cargo build -p logicaffeine-lsp")
        })
        .collect();
    assert!(
        builds.len() >= 3,
        "release.yml builds the LSP and both largo flavors ({} build lines found)",
        builds.len()
    );
    for line in &builds {
        assert!(
            line.contains("--profile dist") && !line.contains("--release"),
            "a shipped binary must be built with --profile dist: `{}`",
            line.trim()
        );
    }
    assert!(
        !release.contains("/release/largo") && !release.contains("/release/${{ matrix.binary }}"),
        "release.yml still ships a binary from target/<triple>/release"
    );
}

#[test]
fn the_gate_profile_optimizes_every_dependency_and_keeps_the_tests_checks() {
    let manifest: toml::Table = std::fs::read_to_string(root().join("Cargo.toml"))
        .unwrap()
        .parse()
        .unwrap();
    let gate = manifest
        .get("profile")
        .and_then(|p| p.get("test-opt"))
        .and_then(|d| d.as_table())
        .expect("the root Cargo.toml declares [profile.test-opt]");
    assert_eq!(
        gate.get("inherits").and_then(|v| v.as_str()),
        Some("test"),
        "test-opt inherits test, so the gate runs the tests as the test profile builds them"
    );
    let packages = gate
        .get("package")
        .and_then(|p| p.as_table())
        .expect("test-opt optimizes packages by name");
    assert_eq!(
        packages
            .get("*")
            .and_then(|p| p.get("opt-level"))
            .and_then(|v| v.as_integer()),
        Some(2),
        "test-opt builds every dependency at opt-level 2 ([profile.test-opt.package.\"*\"])"
    );
    let scopes = std::iter::once(("[profile.test-opt]".to_string(), gate)).chain(
        packages.iter().filter_map(|(name, table)| {
            table
                .as_table()
                .map(|t| (format!("[profile.test-opt.package.{name}]"), t))
        }),
    );
    for (scope, table) in scopes {
        for check in ["overflow-checks", "debug-assertions"] {
            assert_ne!(
                table.get(check).and_then(|v| v.as_bool()),
                Some(false),
                "{scope} turns off {check}: the gate must run the checks the tests are written against"
            );
        }
    }
}
