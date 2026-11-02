//! The release notes cite only what the public tree carries.
//!
//! Every `CHANGELOG.md` ships in the public tree. A backticked lock is a claim a reader can run and a
//! backticked path one they can open; a lock the public tree does not carry, or a path into what the
//! manifest withholds, is a claim nobody outside can check, and it names the private layout.
//!
//! Invariants, for `[Unreleased]` and every release from 0.11.0 on (earlier sections stay as they shipped):
//! every `<name>_lock` or `<name>_lock::<test>` in backticks is a public suite in
//! `crates/logicaffeine_tests/tests/` that holds that test; any other backticked suite name is public; every
//! backticked repo path is public or untracked (a local install location such as `tools/OpenLane`); and no
//! text names a top-level directory the manifest withholds, except the run-output directories public
//! scripts create (`RUN_OUTPUT_DIRS`).
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test changelog_citation_lock`

#![cfg(not(target_arch = "wasm32"))]

#[path = "common/public_manifest.rs"]
mod public_manifest;

use public_manifest::{repo_root, Classifier};
use std::collections::BTreeSet;
use std::path::Path;

/// Directories public scripts create in any checkout: the manifest withholds what is in them, not their names.
const RUN_OUTPUT_DIRS: [&str; 1] = ["logs/"];

const TESTS: &str = "crates/logicaffeine_tests/tests/";

/// `[Unreleased]` and every release from 0.11.0 on, as (version, body).
fn checked_sections(text: &str) -> Vec<(String, String)> {
    text.split("\n## [")
        .skip(1)
        .filter_map(|part| {
            let (head, body) = part.split_once('\n').unwrap_or((part, ""));
            let version = head.split(']').next().unwrap_or_default();
            let numbers: Option<Vec<u64>> = version.split('.').map(|n| n.parse().ok()).collect();
            let checked = version == "Unreleased" || numbers.is_some_and(|n| n >= vec![0, 11, 0]);
            checked.then(|| (version.to_string(), body.to_string()))
        })
        .collect()
}

fn is_ident(s: &str) -> bool {
    !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Every citation in `text` the public tree cannot back.
fn violations(text: &str, classifier: &Classifier, root: &Path) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let withheld: Vec<String> = classifier
        .withheld_dirs()
        .into_iter()
        .filter(|d| !RUN_OUTPUT_DIRS.contains(&d.as_str()))
        .collect();
    for span in text.split('`').skip(1).step_by(2) {
        let (suite, test) = match span.split_once("::") {
            Some((suite, test)) => (suite, Some(test)),
            None => (span, None),
        };
        let file = format!("{TESTS}{suite}.rs");
        if is_ident(suite) && suite.ends_with("_lock") {
            if !root.join(&file).is_file() {
                out.insert(format!("`{span}`: the public tree has no {file}"));
            } else if !classifier.public(&file) {
                out.insert(format!(
                    "`{span}`: {file} is {}, not public",
                    classifier.class(&file).unwrap_or("unclassified")
                ));
            } else if let Some(test) = test.filter(|t| is_ident(t)) {
                let body = std::fs::read_to_string(root.join(&file)).unwrap_or_default();
                if !body.contains(&format!("fn {test}(")) {
                    out.insert(format!("`{span}`: {file} has no test `{test}`"));
                }
            }
        } else if is_ident(suite) && root.join(&file).is_file() && !classifier.public(&file) {
            out.insert(format!(
                "`{span}`: {file} is {}, not public",
                classifier.class(&file).unwrap_or("unclassified")
            ));
        }
        if span.contains('/') && !span.contains(char::is_whitespace) {
            let path = span.split(['*', '…', '<', '{']).next().unwrap_or_default();
            let first = path.split('/').next().unwrap_or_default();
            if root.join(first).exists()
                || withheld.iter().any(|d| d.trim_end_matches('/') == first)
            {
                let key = if !path.ends_with('/') && root.join(path).is_dir() {
                    format!("{path}/")
                } else {
                    path.to_string()
                };
                let output = RUN_OUTPUT_DIRS.iter().any(|d| key.starts_with(d));
                if let Some(class) = classifier.class(&key).filter(|c| *c != "public" && !output) {
                    out.insert(format!("`{span}`: {key} is {class}, not public"));
                }
            }
        }
    }
    for dir in &withheld {
        let mut from = 0;
        while let Some(i) = text[from..].find(dir.as_str()).map(|i| i + from) {
            let before = text[..i].chars().next_back();
            if !before.is_some_and(|c| c.is_alphanumeric() || "_./-".contains(c)) {
                let end = (i + dir.len() + 30).min(text.len());
                let end = (end..=text.len())
                    .find(|&e| text.is_char_boundary(e))
                    .unwrap_or(text.len());
                out.insert(format!(
                    "names the withheld directory {dir}: `{}`",
                    &text[i..end]
                ));
            }
            from = i + dir.len();
        }
    }
    out
}

fn changelogs(root: &Path) -> Vec<String> {
    let mut out = vec!["CHANGELOG.md".to_string()];
    for parent in ["crates", "apps"] {
        let mut crates: Vec<String> = std::fs::read_dir(root.join(parent))
            .unwrap_or_else(|e| panic!("{parent}/: {e}"))
            .flatten()
            .map(|e| format!("{parent}/{}/CHANGELOG.md", e.file_name().to_string_lossy()))
            .filter(|p| root.join(p).is_file())
            .collect();
        crates.sort();
        out.extend(crates);
    }
    out
}

#[test]
fn the_release_notes_cite_only_what_the_public_tree_carries() {
    let root = repo_root();
    let classifier = Classifier::load();
    let files = changelogs(&root);
    assert!(
        files.len() >= 10,
        "found only {} CHANGELOGs — the discovery walk broke: {files:?}",
        files.len()
    );
    let mut found = Vec::new();
    for file in &files {
        let text =
            std::fs::read_to_string(root.join(file)).unwrap_or_else(|e| panic!("{file}: {e}"));
        for (version, body) in checked_sections(&text) {
            for v in violations(&body, &classifier, &root) {
                found.push(format!("{file} [{version}]: {v}"));
            }
        }
    }
    assert!(
        found.is_empty(),
        "the release notes cite what the public tree does not carry — name a public lock or path, or \
         describe the change without the citation:\n  {}",
        found.join("\n  ")
    );
}

#[test]
fn the_rules_fire_on_the_shapes_they_guard() {
    let root = repo_root();
    let fixture = Classifier::from_rules(&[
        ("", "public"),
        ("work/", "internal"),
        ("marketing/", "internal"),
        ("logs/", "internal"),
        (
            "crates/logicaffeine_tests/tests/benchmark_metadata_lock.rs",
            "internal",
        ),
    ]);
    let fires = |text: &str| !violations(text, &fixture, &root).is_empty();
    for bad in [
        "pinned by a new `private_ledger_lock` ratchet.",
        "held by `benchmark_metadata_lock`.",
        "`size_budget_lock::no_such_test` holds it.",
        "The instruments live in `work/ledger/`.",
        "the license text (`marketing/licenses/`)",
        "uploaded only where the private marketing/ directory existed",
        "(work/drafts) carries the table",
    ] {
        assert!(fires(bad), "the lock must refuse: {bad}");
    }
    for good in [
        "`size_budget_lock::the_deploy_gate_weighs_the_web_wasm_against_the_file` runs it.",
        "Every test lives in `crates/logicaffeine_tests`.",
        "the runner reads `tools/OpenLane` when it is installed",
        "`check-binary-sizes.sh` logs under `logs/`.",
        "a network/ path and a framework/ one",
        "`codegen_c::stepped`, `spatial/passes/tile.rs` and `github.com/Brahmastra-Labs/...`",
    ] {
        assert!(
            !fires(good),
            "the lock must accept: {good} — got {:?}",
            violations(good, &fixture, &root)
        );
    }
    let sections = checked_sections("# Changelog\n\n## [Unreleased]\n\n- a\n\n## [0.11.0] - x\n\n- b\n\n## [0.10.1] - y\n\n- c\n");
    assert_eq!(
        sections.iter().map(|(v, _)| v.as_str()).collect::<Vec<_>>(),
        ["Unreleased", "0.11.0"],
        "0.10.1 and older stay as they shipped"
    );
    assert!(
        Classifier::from_rules(&[("", "public")])
            .withheld_dirs()
            .is_empty(),
        "the exported tree withholds nothing"
    );
}
