//! A published crate cannot ship changed code under an empty CHANGELOG.
//!
//! `scripts/release/check-changelogs.py` compares each published crate's shipped code (`src/`, `build.rs`,
//! `assets/`, `stencils/`, `templates/`, `[features]`) with its last crates.io release and refuses a crate that
//! changed while its pending CHANGELOG section holds no bullet but the roll's "Synced to workspace version" line.
//! This lock holds `publish.yml` to running it before anything uploads, and runs the script itself on scratch
//! workspaces against an offline baseline (`CHECK_CHANGELOGS_BASELINE`): changed and silent is refused naming the
//! crate; changed and described passes; unchanged passes; a changed feature table counts as changed.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test changelog_release_gate_lock`

#![cfg(not(target_arch = "wasm32"))]

use std::path::{Path, PathBuf};
use std::process::Command;

const SCRIPT: &str = "scripts/release/check-changelogs.py";
const PUBLISH: &str = ".github/workflows/publish.yml";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

#[test]
fn publish_runs_the_changelog_gate_before_uploading() {
    let workflow = std::fs::read_to_string(repo_root().join(PUBLISH)).unwrap();
    let gate = workflow
        .find(SCRIPT)
        .unwrap_or_else(|| panic!("{PUBLISH} never runs {SCRIPT}"));
    let upload = workflow
        .find("cargo publish")
        .unwrap_or_else(|| panic!("{PUBLISH} has no `cargo publish`"));
    assert!(
        gate < upload,
        "{PUBLISH} runs {SCRIPT} after `cargo publish`; it must gate the upload"
    );
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A one-crate workspace at version 0.2.0 whose crate has `lib` and `features` as its shipped code and
/// `changelog` as its CHANGELOG, beside a baseline release of that crate holding `old_lib`/`old_features`.
fn scratch(
    lib: &str,
    features: &str,
    changelog: &str,
    old_lib: &str,
    old_features: &str,
) -> (tempdir::Dir, PathBuf) {
    let dir = tempdir::Dir::new();
    let ws = dir.path().join("ws");
    write(
        &ws.join("Cargo.toml"),
        "[workspace]\nmembers = [\"crates/*\"]\n\n[workspace.package]\nversion = \"0.2.0\"\n",
    );
    let krate = ws.join("crates/demo");
    write(
        &krate.join("Cargo.toml"),
        &format!(
            "[package]\nname = \"demo-crate\"\nversion.workspace = true\n\n[features]\n{features}"
        ),
    );
    write(&krate.join("src/lib.rs"), lib);
    write(&krate.join("CHANGELOG.md"), changelog);
    write(&ws.join("apps/.keep"), "");
    let base = dir.path().join("baseline/demo-crate");
    write(
        &base.join("Cargo.toml.orig"),
        &format!(
            "[package]\nname = \"demo-crate\"\nversion = \"0.1.0\"\n\n[features]\n{old_features}"
        ),
    );
    write(&base.join("src/lib.rs"), old_lib);
    let baseline = dir.path().join("baseline");
    (dir, baseline)
}

fn run(dir: &tempdir::Dir, baseline: &Path) -> (bool, String) {
    let out = Command::new("python3")
        .arg(repo_root().join(SCRIPT))
        .env("LOGOS_ROOT", dir.path().join("ws"))
        .env("CHECK_CHANGELOGS_BASELINE", baseline)
        .output()
        .expect("python3 runs the gate");
    (
        out.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        ),
    )
}

const EMPTY: &str = "# Changelog\n\n## [Unreleased]\n\n## [0.1.0] - 2026-01-01\n\n- first\n";
const SYNCED: &str = "# Changelog\n\n## [Unreleased]\n\n## [0.2.0] - 2026-02-01\n\nSynced to workspace version 0.2.0. See root CHANGELOG for full history.\n\n## [0.1.0] - 2026-01-01\n\n- first\n";
const SYNCED_BULLET: &str = "# Changelog\n\n## [Unreleased]\n\n- Synced to workspace version 0.2.0. See root CHANGELOG for full history.\n\n## [0.1.0] - 2026-01-01\n\n- first\n";
const DESCRIBED: &str = "# Changelog\n\n## [Unreleased]\n\n### Added\n\n- **`two` returns 2.** A new function.\n\n## [0.1.0] - 2026-01-01\n\n- first\n";

#[test]
fn changed_code_under_an_empty_changelog_is_refused_naming_the_crate() {
    let (dir, baseline) = scratch(
        "pub fn one() -> u8 { 1 }\npub fn two() -> u8 { 2 }\n",
        "",
        EMPTY,
        "pub fn one() -> u8 { 1 }\n",
        "",
    );
    let (ok, out) = run(&dir, &baseline);
    assert!(
        !ok,
        "a changed crate with an empty [Unreleased] must be refused:\n{out}"
    );
    assert!(
        out.contains("demo-crate") && out.contains("1 changed"),
        "the refusal names the crate and what changed:\n{out}"
    );
}

#[test]
fn the_rolls_synced_line_is_not_a_description() {
    for changelog in [SYNCED, SYNCED_BULLET] {
        let (dir, baseline) = scratch(
            "pub fn one() -> u8 { 1 }\npub fn two() -> u8 { 2 }\n",
            "",
            changelog,
            "pub fn one() -> u8 { 1 }\n",
            "",
        );
        let (ok, out) = run(&dir, &baseline);
        assert!(!ok, "\"Synced to workspace version\", as a line or a bullet, is not a description of changed code:\n{out}");
    }
}

#[test]
fn changed_code_with_a_bullet_passes() {
    let (dir, baseline) = scratch(
        "pub fn one() -> u8 { 1 }\npub fn two() -> u8 { 2 }\n",
        "",
        DESCRIBED,
        "pub fn one() -> u8 { 1 }\n",
        "",
    );
    let (ok, out) = run(&dir, &baseline);
    assert!(
        ok,
        "a changed crate whose [Unreleased] describes the change passes:\n{out}"
    );
}

#[test]
fn unchanged_code_needs_no_bullet() {
    let (dir, baseline) = scratch(
        "pub fn one() -> u8 { 1 }\n",
        "",
        EMPTY,
        "pub fn one() -> u8 { 1 }\n",
        "",
    );
    let (ok, out) = run(&dir, &baseline);
    assert!(ok, "an unchanged crate needs no CHANGELOG bullet:\n{out}");
}

#[test]
fn a_changed_feature_table_is_a_change() {
    let (dir, baseline) = scratch(
        "pub fn one() -> u8 { 1 }\n",
        "fast = []\n",
        EMPTY,
        "pub fn one() -> u8 { 1 }\n",
        "",
    );
    let (ok, out) = run(&dir, &baseline);
    assert!(
        !ok && out.contains("features changed"),
        "a new feature is a shipped change:\n{out}"
    );
}

mod tempdir {
    /// A directory under the system temp dir, removed on drop.
    pub struct Dir(std::path::PathBuf);

    impl Dir {
        pub fn new() -> Dir {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static N: AtomicUsize = AtomicUsize::new(0);
            let p = std::env::temp_dir().join(format!(
                "changelog-gate-{}-{}",
                std::process::id(),
                N.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&p).unwrap();
            Dir(p)
        }

        pub fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}
