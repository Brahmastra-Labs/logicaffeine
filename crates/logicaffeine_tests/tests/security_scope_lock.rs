//! The security policy's scope names what this repository actually ships.
//!
//! A reporter reads `.github/SECURITY.md`'s `## Scope` to decide whether a finding is covered. Every
//! backticked name there is a workspace package (as crates.io spells it), a binary a package builds, or a path
//! in the tree; and the section covers the published crates, either by naming each one or by the sentence
//! "every crate this workspace publishes".
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test security_scope_lock`

#![cfg(not(target_arch = "wasm32"))]

use std::collections::BTreeSet;
use std::path::PathBuf;

const POLICY: &str = ".github/SECURITY.md";
const COVERS_ALL: &str = "every crate this workspace publishes";

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn read(rel: &str) -> String {
    let path = repo_root().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {path:?}: {e}"))
}

/// (package names, published package names, binary names) of every crate and app manifest.
fn packages() -> (BTreeSet<String>, BTreeSet<String>, BTreeSet<String>) {
    let (mut all, mut published, mut bins) = (BTreeSet::new(), BTreeSet::new(), BTreeSet::new());
    for parent in ["crates", "apps"] {
        for entry in std::fs::read_dir(repo_root().join(parent))
            .unwrap()
            .flatten()
        {
            let Ok(text) = std::fs::read_to_string(entry.path().join("Cargo.toml")) else {
                continue;
            };
            let manifest: toml::Value = text.parse().unwrap();
            let Some(package) = manifest.get("package") else {
                continue;
            };
            let name = package["name"].as_str().unwrap().to_string();
            if package
                .get("publish")
                .and_then(|p| p.as_bool())
                .unwrap_or(true)
            {
                published.insert(name.clone());
            }
            for bin in manifest
                .get("bin")
                .and_then(|b| b.as_array())
                .into_iter()
                .flatten()
            {
                if let Some(n) = bin.get("name").and_then(|n| n.as_str()) {
                    bins.insert(n.to_string());
                }
            }
            all.insert(name);
        }
    }
    (all, published, bins)
}

fn scope() -> String {
    let text = read(POLICY);
    let start = text
        .find("\n## Scope\n")
        .unwrap_or_else(|| panic!("`{POLICY}` has no `## Scope` section"));
    let body = &text[start + "\n## Scope\n".len()..];
    body[..body.find("\n## ").unwrap_or(body.len())].to_string()
}

#[test]
fn every_name_in_scope_is_something_this_repository_ships() {
    let (all, _, bins) = packages();
    let unknown: Vec<String> = scope()
        .split('`')
        .skip(1)
        .step_by(2)
        .filter(|name| {
            !all.contains(*name)
                && !bins.contains(*name)
                && !repo_root().join(name.trim_end_matches('/')).exists()
        })
        .map(str::to_string)
        .collect();
    assert!(
        unknown.is_empty(),
        "`{POLICY}` scope names {unknown:?}: not a workspace package, binary, or path"
    );
}

#[test]
fn the_scope_covers_the_published_crates() {
    let scope = scope();
    if scope.contains(COVERS_ALL) {
        return;
    }
    let (_, published, _) = packages();
    let named: BTreeSet<&str> = scope.split('`').skip(1).step_by(2).collect();
    let gap: Vec<&String> = published
        .iter()
        .filter(|p| !named.contains(p.as_str()))
        .collect();
    assert!(
        gap.is_empty(),
        "`{POLICY}` scope omits published {gap:?} (name each, or say \"{COVERS_ALL}\")"
    );
}
