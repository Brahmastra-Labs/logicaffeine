//! Version ratchet — the lockstep version lives in the root `Cargo.toml`, but a
//! release touches surfaces that can't inherit it: the README badge, the roadmap hero, the VSCode extension manifest and
//! lockfile, the installer example, and the generated release history. Each of
//! these has drifted at least once (the license sat at 0.6.0 for four
//! releases). This lock pins every one of them to the workspace version so a
//! bump that misses a surface goes RED instead of shipping stale.
//!
//! `scripts/bump-version.sh` rewrites all of these mechanically. The license names no
//! version; `license_lock` holds it.
//!
//! Run: `cargo nextest run -p logicaffeine-tests -E 'binary(version_lock)'`

#![cfg(not(target_arch = "wasm32"))]

use std::fs;
use std::path::PathBuf;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
}

fn workspace_version() -> String {
    let root: toml::Value = toml::from_str(&read("Cargo.toml")).expect("parse root Cargo.toml");
    root["workspace"]["package"]["version"]
        .as_str()
        .expect("workspace.package.version")
        .to_string()
}

/// The release date of `ver` from the root CHANGELOG (`## [ver] - YYYY-MM-DD`).
/// A bump without a rolled CHANGELOG section fails here — the runbook order
/// (changelog before bump) is part of the lock.
fn json(rel: &str) -> serde_json::Value {
    serde_json::from_str(&read(rel)).unwrap_or_else(|e| panic!("parse {rel}: {e}"))
}

#[test]
fn readme_version_badge_tracks_workspace() {
    let ver = workspace_version();
    assert!(
        read("README.md").contains(&format!("badge/version-{ver}-blue")),
        "README.md version badge must show {ver}"
    );
}

#[test]
fn roadmap_hero_tracks_workspace() {
    let ver = workspace_version();
    assert!(
        read("apps/logicaffeine_web/src/ui/pages/roadmap.rs")
            .contains(&format!("span {{ class: \"version\", \"v{ver}\" }}")),
        "roadmap.rs hero version span must show v{ver}"
    );
}

#[test]
fn roadmap_history_leads_with_current_release() {
    let ver = workspace_version();
    let history = json("apps/logicaffeine_web/src/ui/pages/roadmap_history.json");
    let newest = history[0]["version"].as_str().expect("history[0].version");
    assert_eq!(
        newest, ver,
        "roadmap_history.json is stale — run ./scripts/generate-roadmap.sh"
    );
}

#[test]
fn roadmap_history_marks_the_current_release_published() {
    let ver = workspace_version();
    let changelog = read("CHANGELOG.md");
    if !changelog.contains(&format!("## [{ver}] - ")) {
        return;
    }
    let history = json("apps/logicaffeine_web/src/ui/pages/roadmap_history.json");
    let current = history
        .as_array()
        .expect("history array")
        .iter()
        .find(|r| r["version"] == ver.as_str());
    let current = current.unwrap_or_else(|| {
        panic!("roadmap_history.json has no v{ver} — run ./scripts/generate-roadmap.sh")
    });
    assert_eq!(
        current["tagged"], true,
        "v{ver} is rolled in the CHANGELOG but roadmap_history.json shows it unreleased — \
         run ./scripts/generate-roadmap.sh (the release this tree cuts counts as published before its tag exists)"
    );
}

#[test]
fn vscode_manifest_and_lockfile_track_workspace() {
    let ver = workspace_version();
    let manifest = json("editors/vscode/logicaffeine/package.json");
    assert_eq!(
        manifest["version"].as_str(),
        Some(ver.as_str()),
        "package.json version"
    );
    let lock = json("editors/vscode/logicaffeine/package-lock.json");
    assert_eq!(
        lock["version"].as_str(),
        Some(ver.as_str()),
        "package-lock.json root version"
    );
    assert_eq!(
        lock["packages"][""]["version"].as_str(),
        Some(ver.as_str()),
        "package-lock.json packages[\"\"].version"
    );
}

#[test]
fn installer_example_pins_current_release() {
    let ver = workspace_version();
    assert!(
        read("apps/logicaffeine_web/public/install.ps1").contains(&format!("-Version v{ver}")),
        "install.ps1 example comment must pin v{ver}"
    );
}
