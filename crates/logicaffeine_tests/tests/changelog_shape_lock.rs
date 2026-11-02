//! Every CHANGELOG's `[Unreleased]` section is shaped the way Keep a Changelog shapes a release.
//!
//! `[Unreleased]` becomes the next release's notes word for word: the release process rolls it
//! into `## [x.y.z]`, `release.yml` publishes that section as the GitHub release body, and the
//! roadmap's Next release block reads its additions straight off it. So it must read as one
//! release: it comes first, each kind of change has one heading (Added, Breaking changes,
//! Changed, Deprecated, Removed, Fixed, Security, in that order), and no heading repeats. A second `### Fixed` further
//! down is how five additions once came to be filed as fixes. Released sections are history and
//! are left as they were written.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test changelog_shape_lock`

#![cfg(not(target_arch = "wasm32"))]

use std::path::PathBuf;

const KINDS: &[&str] = &["Added", "Breaking changes", "Changed", "Deprecated", "Removed", "Fixed", "Security"];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join("..")
}

/// Every `CHANGELOG.md` the repository tracks, by its path from the root: the ones it ships. Third-party checkouts
/// and build output on this machine are not the project's, whatever their names.
fn changelogs() -> Vec<(String, String)> {
    let root = repo_root().canonicalize().expect("the repository root");
    let listed = std::process::Command::new("git")
        .args(["ls-files", "-z", "--", "CHANGELOG.md", "**/CHANGELOG.md"])
        .current_dir(&root)
        .output()
        .expect("run git ls-files");
    assert!(listed.status.success(), "git ls-files failed: {}", String::from_utf8_lossy(&listed.stderr));
    let mut out: Vec<(String, String)> = String::from_utf8(listed.stdout)
        .expect("tracked paths are UTF-8")
        .split('\0')
        .filter(|rel| !rel.is_empty())
        .map(|rel| {
            let text = std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("read {rel}: {e}"));
            (rel.to_string(), text)
        })
        .collect();
    out.sort();
    out
}

/// What is wrong with a CHANGELOG's `[Unreleased]` section, if anything.
fn shape_problems(text: &str) -> Vec<String> {
    let mut problems = Vec::new();
    let mut releases = text.lines().filter(|l| l.starts_with("## "));
    match releases.next() {
        Some("## [Unreleased]") => {}
        Some(first) => problems.push(format!("its first release section is {first:?}, not \"## [Unreleased]\"")),
        None => {
            problems.push("it has no release sections".to_string());
            return problems;
        }
    }
    let headings: Vec<&str> = text
        .lines()
        .skip_while(|l| *l != "## [Unreleased]")
        .skip(1)
        .take_while(|l| !l.starts_with("## "))
        .filter_map(|l| l.strip_prefix("### "))
        .collect();
    let mut last = None;
    for (i, heading) in headings.iter().enumerate() {
        let Some(rank) = KINDS.iter().position(|k| k == heading) else {
            problems.push(format!("\"### {heading}\" is not one of {}", KINDS.join(", ")));
            continue;
        };
        if headings[..i].contains(heading) {
            problems.push(format!("\"### {heading}\" appears twice"));
        } else if last.is_some_and(|l| rank < l) {
            problems.push(format!("\"### {heading}\" comes after \"### {}\"", KINDS[last.unwrap()]));
        }
        last = Some(last.map_or(rank, |l: usize| l.max(rank)));
    }
    problems
}

#[test]
fn every_unreleased_section_is_one_release() {
    let logs = changelogs();
    assert!(logs.len() >= 20, "only {} tracked CHANGELOGs were found — the listing has lost its way", logs.len());
    let wrong: Vec<String> = logs
        .iter()
        .flat_map(|(path, text)| shape_problems(text).into_iter().map(move |p| format!("{path}: {p}")))
        .collect();
    assert!(wrong.is_empty(), "{} problem(s) in [Unreleased]:\n{}", wrong.len(), wrong.join("\n"));
}

#[test]
fn a_well_shaped_section_passes() {
    let log = "# Changelog\n\n## [Unreleased]\n\n### Added\n- a\n\n### Changed\n- b\n\n### Fixed\n- c\n\n## [0.1.0] - 2026-01-01\n\n### Fixed\n- x\n### Fixed\n- y\n";
    assert_eq!(shape_problems(log), Vec::<String>::new(), "a repeat in a released section is history, not a problem");
}

#[test]
fn a_repeated_heading_is_caught() {
    let log = "## [Unreleased]\n### Added\n- a\n### Fixed\n- b\n### Fixed\n- c\n";
    assert_eq!(shape_problems(log), vec!["\"### Fixed\" appears twice".to_string()]);
}

#[test]
fn a_heading_out_of_order_is_caught() {
    let log = "## [Unreleased]\n### Fixed\n- a\n### Added\n- b\n";
    assert_eq!(shape_problems(log), vec!["\"### Added\" comes after \"### Fixed\"".to_string()]);
}

#[test]
fn an_unknown_heading_is_caught() {
    let log = "## [Unreleased]\n### Added (the big one)\n- a\n";
    assert_eq!(
        shape_problems(log),
        vec!["\"### Added (the big one)\" is not one of Added, Breaking changes, Changed, Deprecated, Removed, Fixed, Security".to_string()]
    );
}

#[test]
fn unreleased_must_come_first() {
    let log = "## [0.2.0] - 2026-02-01\n### Added\n- a\n## [Unreleased]\n";
    assert_eq!(shape_problems(log), vec!["its first release section is \"## [0.2.0] - 2026-02-01\", not \"## [Unreleased]\"".to_string()]);
}
