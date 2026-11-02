//! `scripts/bump-version.sh` rewrites the lockstep version and nothing else.
//!
//! The script is run for real, against a scratch tree that carries every surface it edits plus the traps a
//! release could spring: external dependencies pinned at the same version as the workspace, an npm dependency
//! at that version beside the extension's own entries, and a `[patch.crates-io]` entry. A stub `cargo` first on
//! `PATH` turns the script's closing `cargo check --workspace` into a no-op; nothing else is faked.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test bump_version_script_lock`

#![cfg(unix)]

use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;

const OLD: &str = "0.10.1";
const NEW: &str = "0.11.0";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn put(root: &Path, rel: &str, text: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, text).unwrap();
}

fn read(root: &Path, rel: &str) -> String {
    std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("{rel}: {e}"))
}

/// A scratch tree holding every surface the script edits, at `OLD`, with `changelog` as its CHANGELOG.
fn scratch(label: &str, changelog: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!("logos_bump_{}_{label}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(root.join("scripts")).unwrap();
    std::fs::copy(
        repo_root().join("scripts/bump-version.sh"),
        root.join("scripts/bump-version.sh"),
    )
    .unwrap();
    put(&root, "CHANGELOG.md", changelog);
    put(
        &root,
        "Cargo.toml",
        &format!(
            "[workspace]\nmembers = []\n\n[workspace.package]\nversion = \"{OLD}\"\nedition = \"2021\"\n\n\
             [workspace.dependencies]\n\
             logicaffeine-base = {{ version = \"{OLD}\", path = \"crates/logicaffeine_base\" }}\n\
             logicaffeine-web = {{ version = \"{OLD}\", path = \"apps/logicaffeine_web\" }}\n\
             unrelated-inline = {{ version = \"{OLD}\", default-features = false }}\n\
             unrelated-bare = \"{OLD}\"\n\n\
             [patch.crates-io]\nunrelated-patched = {{ version = \"{OLD}\", git = \"https://example.invalid/x\" }}\n"
        ),
    );
    put(
        &root,
        "editors/vscode/logicaffeine/package.json",
        &format!("{{\n  \"name\": \"logicaffeine\",\n  \"version\": \"{OLD}\",\n  \"dependencies\": {{\n    \"unrelated\": \"{OLD}\"\n  }}\n}}\n"),
    );
    put(
        &root,
        "editors/vscode/logicaffeine/package-lock.json",
        &format!(
            "{{\n  \"name\": \"logicaffeine\",\n  \"version\": \"{OLD}\",\n  \"packages\": {{\n    \"\": {{\n      \"name\": \"logicaffeine\",\n      \"version\": \"{OLD}\"\n    }},\n    \"node_modules/unrelated\": {{\n      \"version\": \"{OLD}\"\n    }}\n  }}\n}}\n"
        ),
    );
    // The license names no version: the script must leave both copies alone.
    let license = "MIT License\n\nCopyright (c) 2025-2026 Example\n";
    put(&root, "LICENSE.md", license);
    put(&root, "editors/vscode/logicaffeine/LICENSE.md", license);
    put(&root, "README.md", &format!("![version](https://img.shields.io/badge/version-{OLD}-blue)\nThe unrelated crate is pinned at {OLD}.\n"));
    put(
        &root,
        "apps/logicaffeine_web/src/ui/pages/roadmap.rs",
        &format!("rsx! {{ span {{ class: \"version\", \"v{OLD}\" }} }}\n"),
    );
    put(
        &root,
        "apps/logicaffeine_web/public/install.ps1",
        &format!("# .\\install.ps1 -Version v{OLD}\n"),
    );
    std::fs::create_dir_all(root.join("bin")).unwrap();
    put(&root, "bin/cargo", "#!/bin/sh\nexit 0\n");
    std::fs::set_permissions(
        root.join("bin/cargo"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    root
}

fn bump(root: &Path) -> std::process::Output {
    let path = format!(
        "{}:{}",
        root.join("bin").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    Command::new("bash")
        .arg(root.join("scripts/bump-version.sh"))
        .args([OLD, NEW])
        .env("PATH", path)
        .output()
        .expect("bash")
}

const RELEASED: &str = "# Changelog\n\n## [Unreleased]\n\n## [0.11.0] - 2026-10-01\n\n- The release.\n\n## [0.10.1] - 2026-09-01\n";

#[test]
fn the_lockstep_version_moves_and_nothing_else_does() {
    let root = scratch("moves", RELEASED);
    let out = bump(&root);
    assert!(
        out.status.success(),
        "the bump succeeds:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );

    let cargo = read(&root, "Cargo.toml");
    assert!(
        cargo.contains(&format!("[workspace.package]\nversion = \"{NEW}\"\n")),
        "the workspace version moves:\n{cargo}"
    );
    for internal in ["logicaffeine-base", "logicaffeine-web"] {
        assert!(
            cargo.contains(&format!("{internal} = {{ version = \"{NEW}\", path = ")),
            "{internal} moves with the workspace:\n{cargo}"
        );
    }
    for external in [
        format!("unrelated-inline = {{ version = \"{OLD}\", default-features = false }}"),
        format!("unrelated-bare = \"{OLD}\""),
        format!("unrelated-patched = {{ version = \"{OLD}\", git = "),
    ] {
        assert!(
            cargo.contains(&external),
            "an external dependency pinned at {OLD} keeps its pin (`{external}`):\n{cargo}"
        );
    }

    let package = read(&root, "editors/vscode/logicaffeine/package.json");
    assert!(
        package.contains(&format!("\"version\": \"{NEW}\""))
            && package.contains(&format!("\"unrelated\": \"{OLD}\"")),
        "{package}"
    );
    let lock = read(&root, "editors/vscode/logicaffeine/package-lock.json");
    assert_eq!(
        lock.matches(&format!("\"version\": \"{NEW}\"")).count(),
        2,
        "the extension's two entries move:\n{lock}"
    );
    assert!(
        lock.contains(&format!(
            "\"node_modules/unrelated\": {{\n      \"version\": \"{OLD}\""
        )),
        "a dependency at {OLD} does not:\n{lock}"
    );

    for license in ["LICENSE.md", "editors/vscode/logicaffeine/LICENSE.md"] {
        assert_eq!(read(&root, license), "MIT License\n\nCopyright (c) 2025-2026 Example\n", "{license}: the script touched the license");
    }
    let readme = read(&root, "README.md");
    assert!(
        readme.contains(&format!("badge/version-{NEW}-blue"))
            && readme.contains(&format!("pinned at {OLD}.")),
        "{readme}"
    );
    assert!(read(&root, "apps/logicaffeine_web/src/ui/pages/roadmap.rs")
        .contains(&format!("\"v{NEW}\"")));
    assert!(read(&root, "apps/logicaffeine_web/public/install.ps1")
        .contains(&format!("-Version v{NEW}")));
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn without_a_released_changelog_section_nothing_is_written() {
    let root = scratch("refuses", "# Changelog\n\n## [Unreleased]\n\n- Pending.\n");
    let before = read(&root, "Cargo.toml");
    let out = bump(&root);
    assert!(
        !out.status.success(),
        "the bump refuses before the CHANGELOG is rolled"
    );
    assert!(
        String::from_utf8_lossy(&out.stderr).contains("roll [Unreleased] first"),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        read(&root, "Cargo.toml"),
        before,
        "a refused bump writes nothing"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// `scripts/bump-version.sh OLD NEW` against `root`, with the stub `cargo` first on `PATH`.
fn bump_with(root: &Path, old: &str, new: &str) -> std::process::Output {
    let path = format!(
        "{}:{}",
        root.join("bin").display(),
        std::env::var("PATH").unwrap_or_default()
    );
    Command::new("bash")
        .arg(root.join("scripts/bump-version.sh"))
        .args([old, new])
        .env("PATH", path)
        .output()
        .expect("bash")
}

const HOOK: &str = "work/release-guards/bump/post-bump.sh";

fn hook(root: &Path, body: &str, mode: u32) {
    put(root, HOOK, body);
    std::fs::set_permissions(root.join(HOOK), std::fs::Permissions::from_mode(mode)).unwrap();
}

/// A workspace's own version stamps move through its hook: `work/release-guards/bump/post-bump.sh OLD NEW`, run once
/// the public surfaces have moved.
#[test]
fn a_workspace_post_bump_hook_runs_once_the_public_surfaces_have_moved() {
    let root = scratch("hook", RELEASED);
    hook(
        &root,
        "#!/bin/sh\nhere=\"$(dirname \"$0\")\"\necho \"$1 $2\" >> \"$here/called\"\n\
         grep -q \"version = \\\"$2\\\"\" \"$here/../../../Cargo.toml\" && echo versioned >> \"$here/called\"\nexit 0\n",
        0o755,
    );
    let out = bump(&root);
    assert!(
        out.status.success(),
        "the bump succeeds:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(
        read(&root, "work/release-guards/bump/called"),
        format!("{OLD} {NEW}\nversioned\n"),
        "the hook runs once, given the two versions, after the manifest names the new one"
    );
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_failing_post_bump_hook_fails_the_bump() {
    let root = scratch("hook_fails", RELEASED);
    hook(&root, "#!/bin/sh\nexit 3\n", 0o755);
    let out = bump(&root);
    assert!(
        !out.status.success(),
        "a hook that fails fails the bump:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// A hook that lost its exec bit (a copy, a restore) would skip the workspace's stamps without a word: the bump
/// refuses it by name, before it writes anything.
#[test]
fn a_post_bump_hook_that_is_not_executable_is_refused_before_anything_is_written() {
    let root = scratch("hook_mode", RELEASED);
    hook(&root, "#!/bin/sh\nexit 0\n", 0o644);
    let before = read(&root, "Cargo.toml");
    let out = bump(&root);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        !out.status.success(),
        "a hook that is present but not executable fails the bump"
    );
    assert!(
        err.contains(HOOK) && err.contains("not executable"),
        "the refusal names the hook:\n{err}"
    );
    assert_eq!(
        read(&root, "Cargo.toml"),
        before,
        "a refused bump writes nothing"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// Where the workspace is absent (a public clone) the bump still succeeds, and says the private stamps did not move.
#[test]
fn a_bump_without_a_post_bump_hook_says_the_private_stamps_did_not_move() {
    let root = scratch("hook_absent", RELEASED);
    let out = bump(&root);
    assert!(
        out.status.success(),
        "a checkout without the workspace bumps"
    );
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("No post-bump hook"),
        "the absent hook is announced, not silent:\n{err}"
    );
    let _ = std::fs::remove_dir_all(&root);
}

/// The tag-day re-date: a bump from a version to itself after the CHANGELOG's release date moved changes nothing
/// (the license names no date) and refuses nothing.
#[test]
fn a_bump_to_the_same_version_after_a_re_date_changes_nothing() {
    let root = scratch("redate", RELEASED);
    assert!(bump(&root).status.success(), "the first bump succeeds");
    let surfaces = [
        "Cargo.toml",
        "editors/vscode/logicaffeine/package.json",
        "editors/vscode/logicaffeine/package-lock.json",
        "README.md",
        "apps/logicaffeine_web/src/ui/pages/roadmap.rs",
        "apps/logicaffeine_web/public/install.ps1",
        "LICENSE.md",
        "editors/vscode/logicaffeine/LICENSE.md",
    ];
    let before: Vec<String> = surfaces.iter().map(|s| read(&root, s)).collect();
    put(
        &root,
        "CHANGELOG.md",
        &RELEASED.replace("## [0.11.0] - 2026-10-01", "## [0.11.0] - 2026-10-02"),
    );
    let out = bump_with(&root, NEW, NEW);
    assert!(
        out.status.success(),
        "a re-date bump succeeds:\n{}\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    for (surface, text) in surfaces.iter().zip(&before) {
        assert_eq!(&read(&root, surface), text, "{surface} does not move on a re-date");
    }
    let _ = std::fs::remove_dir_all(&root);
}
