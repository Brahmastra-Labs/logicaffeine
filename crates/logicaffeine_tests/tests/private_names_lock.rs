//! private_names_lock — no tracked file carries private material.
//!
//! No tracked path lies under a private root, and no tracked file's path or content says a private name or a
//! denylisted text. The lists live outside every commit, in `<git common dir>/info/private-guard/` beside the program
//! that reads them (the private workspace's `work/release-guards/hooks/install.sh` puts them there), so this source
//! names nothing it guards. A checkout without them (CI, a public clone) records a counted
//! `LOGOS_TOOL_SKIP: private-guard`; the full runner requires them through `LOGOS_REQUIRE_TOOLS`, and the release gate
//! installs them into its clone, so a release rests on a run of this lock, never on its skip.
//!
//! A refusal names a file, a line and an entry's number, never the text an entry matches: a red lands in logs. A file
//! shown as `#N` is line N of `git ls-files`.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test private_names_lock`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/mod.rs"]
mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn guard(dir: &Path) -> PathBuf {
    dir.join("private-guard.py")
}

fn git(repo: &Path, args: &[&str]) {
    let out = Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .env("GIT_AUTHOR_NAME", "lock")
        .env("GIT_AUTHOR_EMAIL", "lock@example.com")
        .env("GIT_COMMITTER_NAME", "lock")
        .env("GIT_COMMITTER_EMAIL", "lock@example.com")
        .output()
        .expect("git runs");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

/// The path list's entries: (number, entry, kind), read at run time and never printed.
fn path_entries(dir: &Path) -> Vec<(usize, String, String)> {
    std::fs::read_to_string(dir.join("private-paths.txt"))
        .expect("the installed path list reads")
        .lines()
        .enumerate()
        .filter(|(_, l)| !l.trim().is_empty() && !l.starts_with('#'))
        .map(|(i, l)| {
            let (entry, kind) = l.split_once('\t').expect("an entry is <entry><TAB><kind>");
            (i + 1, entry.to_string(), kind.to_string())
        })
        .collect()
}

#[test]
fn no_tracked_file_carries_private_material() {
    if !common::require_tool("private-guard") {
        return;
    }
    let dir = common::private_guard_at(&repo_root()).expect("the probe found the guard");
    let out = Command::new("python3")
        .arg(guard(&dir))
        .arg("tree")
        .arg(repo_root())
        .output()
        .expect("python3 runs");
    let report = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "private material in the tracked tree (entries by number; `#N` is line N of `git ls-files`):\n{report}"
    );
    assert!(
        report.contains("tracked files clean"),
        "the guard reports a clean scan, not silence:\n{report}"
    );
}

#[test]
fn a_refusal_names_every_entry_by_number_and_echoes_none() {
    if !common::require_tool("private-guard") {
        return;
    }
    let dir = common::private_guard_at(&repo_root()).expect("the probe found the guard");
    let entries = path_entries(&dir);
    assert!(!entries.is_empty(), "the installed path list holds entries");
    let planted = tempfile::tempdir().expect("a temp dir");
    let repo = planted.path();
    git(repo, &["init", "-q", "-b", "main"]);
    std::fs::write(repo.join("README.md"), "a checkout\n").unwrap();
    let mut prose = String::new();
    for (_, entry, kind) in &entries {
        let path = if entry.ends_with('/') {
            format!("{entry}probe.txt")
        } else {
            entry.clone()
        };
        let full = repo.join(&path);
        std::fs::create_dir_all(full.parent().unwrap()).unwrap();
        std::fs::write(&full, "probe\n").unwrap();
        if kind == "name" {
            prose.push_str(&format!("see {}\n", entry.trim_end_matches('/')));
        }
    }
    std::fs::write(repo.join("docs.md"), &prose).unwrap();
    git(repo, &["add", "-A", "-f"]);
    git(repo, &["commit", "-q", "-m", "planted"]);
    let out = Command::new("python3")
        .arg(guard(&dir))
        .arg("tree")
        .arg(repo)
        .output()
        .expect("python3 runs");
    assert_eq!(out.status.code(), Some(1), "a planted checkout is refused");
    let report = String::from_utf8_lossy(&out.stderr).to_lowercase();
    for (n, entry, kind) in &entries {
        let planted_path = entry.to_lowercase();
        assert!(
            !report.contains(&planted_path),
            "the refusal echoes private-paths entry {n}"
        );
        assert!(
            report.contains(&format!("lies under private-paths entry {n}")),
            "entry {n}'s planted path is not refused by number"
        );
        if kind == "name" {
            let name = entry.trim_end_matches('/').to_lowercase();
            assert!(
                !report.contains(&name),
                "the refusal echoes private-paths entry {n}"
            );
            assert!(
                report.contains(&format!(": private-paths entry {n}")),
                "entry {n}'s planted name is not refused by number"
            );
        }
    }
}

#[test]
fn the_full_runner_requires_the_guard() {
    let full = std::fs::read_to_string(repo_root().join("scripts/run-all-tests-fast.sh")).unwrap();
    let default = common::required_by_runner(&full, "run-all-tests-fast.sh")
        .expect("the full runner sets LOGOS_REQUIRE_TOOLS");
    assert!(
        common::tool_required_by(&default, "private-guard"),
        "the full runner must fail when the private-material guard is missing — its LOGOS_REQUIRE_TOOLS default is {default:?}"
    );
}

#[test]
fn the_probe_asks_about_this_checkout() {
    assert_eq!(
        common::tool_available("private-guard"),
        common::private_guard_at(&repo_root()).is_some(),
        "the named probe asks about this checkout's git directory"
    );
    let bare = tempfile::tempdir().expect("a temp dir");
    git(bare.path(), &["init", "-q"]);
    assert!(
        common::private_guard_at(bare.path()).is_none(),
        "a checkout with no installed guard has none"
    );
    let info = bare.path().join(".git/info/private-guard");
    std::fs::create_dir_all(&info).unwrap();
    for f in ["private-guard.py", "private-paths.txt"] {
        std::fs::write(info.join(f), "x\n").unwrap();
    }
    assert!(
        common::private_guard_at(bare.path()).is_none(),
        "a guard without its denylist is not installed"
    );
    std::fs::write(info.join("denylist.src"), "x\n").unwrap();
    assert!(
        common::private_guard_at(bare.path()).is_some(),
        "the program, the path list and the denylist are the guard"
    );
}
