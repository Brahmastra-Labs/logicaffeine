//! What the public tree holds, for locks that reason about it. Private material lives in root entries git
//! ignores, so a path is withheld when its first component is one — unless that entry is a local location a
//! public script creates or installs into (`LOCAL_LOCATIONS`: build output, logs, fetched tools and clones),
//! which may be named. Below the root, an ignored path is generated output (`benchmarks/bin/`, `__pycache__`),
//! nameable like the tree it sits in. Build artifacts and secrets are never public. Also the workspace's package
//! directories, for locks that reason about which crates the public tree holds.

#![allow(dead_code)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// Ignored root entries a public script creates or installs into; naming them reveals nothing private.
pub const LOCAL_LOCATIONS: &[&str] = &[
    "target",
    "logs",
    "tools",
    "external",
    "node_modules",
    "models",
    "dist",
    ".cargo",
    "reference-kernels",
];

pub fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repo root")
        .to_path_buf()
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()))
}

/// Every path in the checkout: tracked, and untracked-not-ignored, present on disk.
pub fn checkout() -> Vec<String> {
    let root = repo_root();
    let out = Command::new("git")
        .args([
            "ls-files",
            "-z",
            "--cached",
            "--others",
            "--exclude-standard",
        ])
        .current_dir(&root)
        .output()
        .expect("git ls-files");
    assert!(
        out.status.success(),
        "git ls-files failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    let mut paths: Vec<String> = String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|p| !p.is_empty())
        .filter(|p| root.join(p).symlink_metadata().is_ok())
        .map(str::to_string)
        .collect();
    paths.sort();
    paths.dedup();
    paths
}

/// Which of `paths` git ignores; a tracked path is never ignored, and the empty path is the root.
///
/// A path beneath a symlink in the checkout is ignored without asking git: git tracks nothing beyond a
/// symlink, and a checkout links only machine-local inputs there (a `tools/`, `models/` or
/// `reference-kernels/` that is a gitignored directory wherever it is real).
pub fn git_ignored(paths: &[String]) -> Vec<bool> {
    let asked: Vec<&String> = paths.iter().filter(|p| !p.is_empty()).collect();
    if asked.len() < paths.len() {
        let mut answers =
            git_ignored(&asked.iter().map(|p| p.to_string()).collect::<Vec<_>>()).into_iter();
        return paths
            .iter()
            .map(|p| !p.is_empty() && answers.next().unwrap_or(false))
            .collect();
    }
    let linked: Vec<bool> = paths.iter().map(|p| beneath_a_symlink(p)).collect();
    if linked.iter().any(|l| *l) {
        let rest: Vec<String> = paths
            .iter()
            .zip(&linked)
            .filter(|(_, l)| !**l)
            .map(|(p, _)| p.clone())
            .collect();
        let mut answers = git_ignored(&rest).into_iter();
        return linked
            .iter()
            .map(|l| *l || answers.next().unwrap_or(false))
            .collect();
    }
    if paths.is_empty() {
        return Vec::new();
    }
    let mut child = Command::new("git")
        .args([
            "check-ignore",
            "--stdin",
            "--verbose",
            "--non-matching",
            "-z",
        ])
        .current_dir(repo_root())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("git check-ignore");
    let mut input = Vec::new();
    for p in paths {
        input.extend_from_slice(p.as_bytes());
        input.push(0);
    }
    child.stdin.take().unwrap().write_all(&input).unwrap();
    let out = child.wait_with_output().expect("git check-ignore");
    assert!(
        out.status.code().is_some_and(|c| c <= 1),
        "git check-ignore failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    ignored_by(&out.stdout, paths.len())
}

/// Which of the `asked` paths `git check-ignore --stdin --verbose --non-matching -z` printed `out` for are ignored.
/// It prints four fields a path — the deciding rule's source, line and pattern, then the path — and the rule decides:
/// a path no rule matches has an empty source, and one a negated rule (`!…`) matches last is kept.
pub fn ignored_by(out: &[u8], asked: usize) -> Vec<bool> {
    let fields: Vec<&[u8]> = out.split(|b| *b == 0).collect();
    (0..asked)
        .map(|i| {
            fields.get(i * 4).is_some_and(|source| !source.is_empty())
                && !fields.get(i * 4 + 2).is_some_and(|pattern| pattern.starts_with(b"!"))
        })
        .collect()
}

/// Whether a strict ancestor of `path` (relative to the repository root) is a symlink in the checkout.
fn beneath_a_symlink(path: &str) -> bool {
    let components: Vec<&str> = path.split('/').filter(|c| !c.is_empty()).collect();
    let mut at = repo_root();
    for c in &components[..components.len().saturating_sub(1)] {
        at.push(c);
        if std::fs::symlink_metadata(&at).is_ok_and(|m| m.file_type().is_symlink()) {
            return true;
        }
    }
    false
}

/// Longest prefix wins (a prefix ending in `/` covers a directory, any other names one file) for a fixture
/// classifier; the checkout's classifier asks git. A few patterns are never public either way.
pub struct Classifier {
    rules: Option<Vec<(String, &'static str)>>,
    cache: RefCell<BTreeMap<String, bool>>,
}

impl Classifier {
    /// The classifier of this checkout: git's own ignore rules.
    pub fn load() -> Classifier {
        Classifier {
            rules: None,
            cache: RefCell::new(BTreeMap::new()),
        }
    }

    fn ignored(&self, path: &str) -> bool {
        if path.starts_with('/') || path.split('/').any(|c| c == "..") {
            return false;
        }
        if let Some(hit) = self.cache.borrow().get(path) {
            return *hit;
        }
        let hit = git_ignored(&[path.to_string()])[0];
        self.cache.borrow_mut().insert(path.to_string(), hit);
        hit
    }

    pub fn class(&self, path: &str) -> Option<&'static str> {
        let never = path
            .split('/')
            .any(|c| c == "__pycache__" || c == "target" || c == ".DS_Store")
            || path.ends_with(".pyc")
            || path.ends_with(".pyo")
            || path.ends_with(".pem")
            || path
                .rsplit('/')
                .next()
                .is_some_and(|f| f == ".env" || f.starts_with(".env."));
        if never {
            return Some("confidential");
        }
        match &self.rules {
            Some(rules) => rules
                .iter()
                .find(|(prefix, _)| {
                    if prefix.is_empty() || prefix.ends_with('/') {
                        path.starts_with(prefix.as_str())
                    } else {
                        path == prefix
                    }
                })
                .map(|(_, class)| *class),
            None => {
                let first = path.split('/').next().unwrap_or_default();
                if LOCAL_LOCATIONS.contains(&first) || !self.ignored(first) {
                    Some("public")
                } else {
                    Some("withheld")
                }
            }
        }
    }

    pub fn public(&self, path: &str) -> bool {
        self.class(path) == Some("public")
    }

    /// A classifier from explicit rules, for a lock that proves its own logic on fixtures.
    pub fn from_rules(rules: &[(&str, &'static str)]) -> Classifier {
        let mut rules: Vec<(String, &'static str)> =
            rules.iter().map(|(p, c)| (p.to_string(), *c)).collect();
        rules.sort_by_key(|(prefix, _)| std::cmp::Reverse(prefix.len()));
        Classifier {
            rules: Some(rules),
            cache: RefCell::new(BTreeMap::new()),
        }
    }

    /// The top-level directories withheld whole (`work/`, …): for a fixture, every one-component directory prefix
    /// it classifies as anything but public; for the checkout, every root directory git ignores that is not a
    /// local location.
    pub fn withheld_dirs(&self) -> Vec<String> {
        match &self.rules {
            Some(rules) => rules
                .iter()
                .filter(|(prefix, class)| {
                    *class != "public" && prefix.ends_with('/') && prefix.matches('/').count() == 1
                })
                .map(|(prefix, _)| prefix.clone())
                .collect(),
            None => {
                let mut dirs: Vec<String> = std::fs::read_dir(repo_root())
                    .unwrap()
                    .flatten()
                    .filter(|e| e.path().is_dir())
                    .map(|e| e.file_name().to_string_lossy().to_string())
                    .filter(|n| n != ".git" && !LOCAL_LOCATIONS.contains(&n.as_str()))
                    .collect();
                dirs.sort();
                let ignored = git_ignored(&dirs);
                dirs.into_iter()
                    .zip(ignored)
                    .filter(|(_, i)| *i)
                    .map(|(d, _)| format!("{d}/"))
                    .collect()
            }
        }
    }
}

/// Every workspace package directory (`crates/*`, `apps/*`, `scripts/*` holding a `Cargo.toml`
/// with a `[package]`), relative to the repo root, with its package name.
pub fn packages() -> BTreeMap<String, String> {
    let root = repo_root();
    let mut out = BTreeMap::new();
    for top in ["crates", "apps", "scripts"] {
        for entry in std::fs::read_dir(root.join(top)).unwrap().flatten() {
            let manifest = entry.path().join("Cargo.toml");
            if !manifest.is_file() {
                continue;
            }
            let table: toml::Table = read(&manifest)
                .parse()
                .unwrap_or_else(|e| panic!("{}: {e}", manifest.display()));
            if let Some(name) = table
                .get("package")
                .and_then(|p| p.get("name"))
                .and_then(|n| n.as_str())
            {
                out.insert(
                    format!("{top}/{}", entry.file_name().to_string_lossy()),
                    name.to_string(),
                );
            }
        }
    }
    out
}

/// The packages the public tree does not hold: directory → package name.
pub fn held_back(c: &Classifier) -> BTreeMap<String, String> {
    packages()
        .into_iter()
        .filter(|(dir, _)| !c.public(&format!("{dir}/Cargo.toml")))
        .collect()
}
