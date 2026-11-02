//! A feature a workspace crate shares with a workspace crate it depends on reaches that crate.
//!
//! When crate A declares feature `F` and depends on crate B, which declares `F` too, A's `F` enables B's
//! (`b/F`), so code gated on `F` in A and the `F`-gated items of B it calls build together. A feature that
//! stops short of part of its crate's graph compiles A's gated code against a B without it. The workspace's
//! default build turns none of these features on, so nothing else in the suite builds that combination.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn read_toml(path: &Path) -> toml::Table {
    std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("{}: {e}", path.display()))
        .parse()
        .unwrap_or_else(|e| panic!("{} does not parse: {e}", path.display()))
}

/// Every workspace member's manifest, by package name, from the root `[workspace] members`.
fn members() -> BTreeMap<String, (String, toml::Table)> {
    let root = repo_root();
    let workspace = read_toml(&root.join("Cargo.toml"));
    let patterns = workspace["workspace"]["members"]
        .as_array()
        .expect("[workspace] members");
    let mut dirs: Vec<String> = Vec::new();
    for pattern in patterns {
        let pattern = pattern.as_str().expect("a member is a path");
        match pattern.strip_suffix("/*") {
            Some(parent) => {
                let mut found: Vec<String> = std::fs::read_dir(root.join(parent))
                    .unwrap_or_else(|e| panic!("{parent}: {e}"))
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().join("Cargo.toml").is_file())
                    .map(|e| format!("{parent}/{}", e.file_name().to_string_lossy()))
                    .collect();
                found.sort();
                dirs.extend(found);
            }
            None => dirs.push(pattern.to_string()),
        }
    }
    dirs.into_iter()
        .filter_map(|rel| {
            let manifest = read_toml(&root.join(&rel).join("Cargo.toml"));
            let name = manifest.get("package")?.get("name")?.as_str()?.to_string();
            Some((name, (rel, manifest)))
        })
        .collect()
}

/// A manifest's normal dependencies, target-specific ones included: `(key, package)`. The key is what a
/// feature names (`key/F`); the package is the crate it resolves to.
fn normal_deps(manifest: &toml::Table, workspace_deps: &toml::Table) -> Vec<(String, String)> {
    let mut tables: Vec<&toml::Table> = Vec::new();
    if let Some(t) = manifest.get("dependencies").and_then(|d| d.as_table()) {
        tables.push(t);
    }
    if let Some(targets) = manifest.get("target").and_then(|t| t.as_table()) {
        for spec in targets.values() {
            if let Some(t) = spec.get("dependencies").and_then(|d| d.as_table()) {
                tables.push(t);
            }
        }
    }
    let mut out = Vec::new();
    for table in tables {
        for (key, spec) in table {
            let renamed = spec.get("package").and_then(|p| p.as_str());
            let inherited = spec
                .get("workspace")
                .and_then(|w| w.as_bool())
                .unwrap_or(false);
            let package = match (renamed, inherited) {
                (Some(p), _) => p.to_string(),
                (None, true) => workspace_deps
                    .get(key)
                    .and_then(|d| d.get("package"))
                    .and_then(|p| p.as_str())
                    .unwrap_or(key)
                    .to_string(),
                (None, false) => key.clone(),
            };
            out.push((key.clone(), package));
        }
    }
    out
}

fn features(manifest: &toml::Table) -> BTreeMap<String, Vec<String>> {
    manifest
        .get("features")
        .and_then(|f| f.as_table())
        .map(|t| {
            t.iter()
                .map(|(name, list)| {
                    let enables = list.as_array().map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    });
                    (name.clone(), enables.unwrap_or_default())
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Every `(crate, feature, dependency key, dependency)` where the crate and a workspace dependency both
/// declare the feature, and whether the crate's feature forwards it.
fn shared_features() -> Vec<(String, String, String, String, bool)> {
    let members = members();
    let root = read_toml(&repo_root().join("Cargo.toml"));
    let workspace_deps = root["workspace"]
        .get("dependencies")
        .and_then(|d| d.as_table())
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    for (name, (_, manifest)) in &members {
        let own = features(manifest);
        for (key, dep) in normal_deps(manifest, &workspace_deps) {
            let Some((_, dep_manifest)) = members.get(&dep) else {
                continue;
            };
            let theirs: BTreeSet<String> = features(dep_manifest).into_keys().collect();
            for (feature, enables) in &own {
                if feature == "default" || !theirs.contains(feature) {
                    continue;
                }
                let forwards = enables
                    .iter()
                    .any(|e| *e == format!("{key}/{feature}") || *e == format!("{key}?/{feature}"));
                out.push((
                    name.clone(),
                    feature.clone(),
                    key.clone(),
                    dep.clone(),
                    forwards,
                ));
            }
        }
    }
    out
}

#[test]
fn every_shared_feature_reaches_the_crate_that_shares_it() {
    let missing: Vec<String> = shared_features()
        .into_iter()
        .filter(|(_, _, _, _, forwards)| !forwards)
        .map(|(name, feature, key, dep, _)| {
            format!("{name}: feature `{feature}` does not enable {dep}'s `{feature}` — add \"{key}/{feature}\" to it")
        })
        .collect();
    assert!(
        missing.is_empty(),
        "{} shared feature(s) stop short of the crate that shares them:\n  {}",
        missing.len(),
        missing.join("\n  ")
    );
}

#[test]
fn the_rule_has_subjects() {
    let shared = shared_features();
    assert!(
        shared.iter().any(|(name, feature, _, dep, _)| name == "logicaffeine-compile" && feature == "verification" && dep == "logicaffeine-proof"),
        "the walk no longer sees compile's `verification` reaching proof — the manifest reading broke, not the rule:\n{shared:?}"
    );
    assert!(
        shared.len() >= 3,
        "only {} shared features found — the member walk broke: {shared:?}",
        shared.len()
    );
}
