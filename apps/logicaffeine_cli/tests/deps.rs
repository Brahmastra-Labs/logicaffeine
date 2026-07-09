//! `largo add <SPEC>` / `largo remove <NAME>` — manifest dependency editing.
//!
//! Edits are format-preserving (toml_edit): comments, spacing, and ordering
//! in Largo.toml survive every operation.

mod common;

use common::*;
use tempfile::tempdir;

/// A comment-and-odd-spacing-laden manifest that edits must not disturb.
const QUIRKY: &str = "# my project manifest — hands off the comments\n\
[package]\n\
name   = \"quirky\"     # extra spaces are intentional\n\
version = \"0.1.0\"\n\
entry = \"src/main.lg\"\n\
\n\
# dependencies below\n\
[dependencies]\n\
existing = \"1.0\"   # keep me\n";

fn scaffold_quirky(dir: &std::path::Path) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Largo.toml"), QUIRKY).unwrap();
    std::fs::write(dir.join("src/main.lg"), "## Main\n    Show 1.\n").unwrap();
}

fn manifest_text(dir: &std::path::Path) -> String {
    std::fs::read_to_string(dir.join("Largo.toml")).unwrap()
}

/// `largo add name@version` inserts the dep and preserves every comment and
/// spacing quirk of the original manifest.
#[test]
fn add_version_dep_preserves_formatting() {
    let dir = tempdir().unwrap();
    scaffold_quirky(dir.path());

    let out = largo_in(dir.path(), &["add", "foo@1.2"]);
    assert_eq!(out.status.code(), Some(0), "add: {}", stderr(&out));

    let text = manifest_text(dir.path());
    assert!(text.contains("foo = \"1.2\""), "dep must be inserted:\n{text}");
    for preserved in [
        "# my project manifest — hands off the comments",
        "name   = \"quirky\"     # extra spaces are intentional",
        "# dependencies below",
        "existing = \"1.0\"   # keep me",
    ] {
        assert!(text.contains(preserved), "must preserve {preserved:?}:\n{text}");
    }
    let manifest: logicaffeine_cli::project::manifest::Manifest =
        toml::from_str(&text).expect("edited manifest must stay valid");
    assert!(manifest.dependencies.contains_key("foo"));
}

/// `largo add name` without a version records a wildcard requirement.
#[test]
fn add_without_version_is_wildcard() {
    let dir = tempdir().unwrap();
    scaffold_quirky(dir.path());
    let out = largo_in(dir.path(), &["add", "bar"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(manifest_text(dir.path()).contains("bar = \"*\""));
}

/// `--path` records a detailed path dependency that Manifest understands.
#[test]
fn add_path_dependency() {
    let dir = tempdir().unwrap();
    scaffold_quirky(dir.path());
    let out = largo_in(dir.path(), &["add", "math", "--path", "./math"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));

    let manifest: logicaffeine_cli::project::manifest::Manifest =
        toml::from_str(&manifest_text(dir.path())).unwrap();
    match manifest.dependencies.get("math") {
        Some(logicaffeine_cli::project::manifest::DependencySpec::Detailed(detail)) => {
            assert_eq!(detail.path.as_deref(), Some("./math"));
        }
        other => panic!("expected a detailed path dep, got {other:?}"),
    }
}

/// `--git` records a detailed git dependency.
#[test]
fn add_git_dependency() {
    let dir = tempdir().unwrap();
    scaffold_quirky(dir.path());
    let out = largo_in(
        dir.path(),
        &["add", "remote", "--git", "https://example.com/remote.git"],
    );
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(manifest_text(dir.path()).contains("https://example.com/remote.git"));
}

/// `largo add logos:std` uses the URI form: key `std`, value `"logos:std"`.
#[test]
fn add_logos_uri_dependency() {
    let dir = tempdir().unwrap();
    scaffold_quirky(dir.path());
    let out = largo_in(dir.path(), &["add", "logos:std"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    assert!(manifest_text(dir.path()).contains("std = \"logos:std\""));
}

/// Re-adding a dependency updates it in place — no duplicate keys.
#[test]
fn re_add_updates_in_place() {
    let dir = tempdir().unwrap();
    scaffold_quirky(dir.path());
    assert_eq!(largo_in(dir.path(), &["add", "foo@1.2"]).status.code(), Some(0));
    assert_eq!(largo_in(dir.path(), &["add", "foo@2.0"]).status.code(), Some(0));

    let text = manifest_text(dir.path());
    assert!(text.contains("foo = \"2.0\""), "must update:\n{text}");
    assert!(!text.contains("foo = \"1.2\""), "must not duplicate:\n{text}");
    assert_eq!(text.matches("foo = ").count(), 1);
}

/// `largo remove` deletes exactly that dependency; neighbors and comments
/// survive.
#[test]
fn remove_deletes_only_that_dep() {
    let dir = tempdir().unwrap();
    scaffold_quirky(dir.path());
    assert_eq!(largo_in(dir.path(), &["add", "doomed@1"]).status.code(), Some(0));

    let out = largo_in(dir.path(), &["remove", "doomed"]);
    assert_eq!(out.status.code(), Some(0), "remove: {}", stderr(&out));

    let text = manifest_text(dir.path());
    assert!(!text.contains("doomed"), "removed dep must be gone:\n{text}");
    assert!(text.contains("existing = \"1.0\"   # keep me"), "neighbor survives:\n{text}");
}

/// Removing a dependency that isn't there is an error naming it.
#[test]
fn remove_absent_dep_fails() {
    let dir = tempdir().unwrap();
    scaffold_quirky(dir.path());
    let out = largo_in(dir.path(), &["remove", "phantom"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(strip_ansi(&stderr(&out)).contains("phantom"));
}

/// `--path` and `--git` are mutually exclusive (usage error).
#[test]
fn path_and_git_conflict() {
    let dir = tempdir().unwrap();
    scaffold_quirky(dir.path());
    let out = largo_in(dir.path(), &["add", "x", "--path", "./x", "--git", "url"]);
    assert_eq!(out.status.code(), Some(2));
}

/// Outside a project, add fails with the standard project error.
#[test]
fn add_outside_project_fails() {
    let dir = tempdir().unwrap();
    let out = largo_in(dir.path(), &["add", "foo"]);
    assert_eq!(out.status.code(), Some(1));
}

/// An inline-table `[dependencies]` (`dependencies = { … }`) is edited in
/// place, not rejected.
#[test]
fn inline_table_dependencies_are_editable() {
    let dir = tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    // A top-level inline table must precede the [package] header — after it,
    // the key would belong to `package.dependencies`.
    std::fs::write(
        dir.path().join("Largo.toml"),
        "dependencies = { existing = \"1.0\" }\n\n[package]\nname = \"inline\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("src/main.lg"), "## Main\n    Show 1.\n").unwrap();

    let add = largo_in(dir.path(), &["add", "foo@2.0"]);
    assert_eq!(add.status.code(), Some(0), "add to inline table: {}", stderr(&add));
    assert!(manifest_text(dir.path()).contains("foo"), "{}", manifest_text(dir.path()));

    let rm = largo_in(dir.path(), &["remove", "existing"]);
    assert_eq!(rm.status.code(), Some(0), "remove from inline table: {}", stderr(&rm));
    assert!(!manifest_text(dir.path()).contains("existing"));
}

/// Adding to a manifest with no [dependencies] section creates it.
#[test]
fn add_creates_dependencies_section() {
    let dir = tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src")).unwrap();
    std::fs::write(
        dir.path().join("Largo.toml"),
        "[package]\nname = \"bare\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    std::fs::write(dir.path().join("src/main.lg"), "## Main\n    Show 1.\n").unwrap();

    let out = largo_in(dir.path(), &["add", "foo@1.0"]);
    assert_eq!(out.status.code(), Some(0), "{}", stderr(&out));
    let text = manifest_text(dir.path());
    assert!(text.contains("[dependencies]"));
    assert!(text.contains("foo = \"1.0\""));
}
