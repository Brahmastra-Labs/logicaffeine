//! Phase 37: Largo.toml Manifest Parser
//!
//! Parses project manifests for LOGOS build configuration.
//!
//! The manifest file (`Largo.toml`) defines project metadata, dependencies,
//! and build settings. This module provides types for parsing and serializing
//! these manifests.
//!
//! # Example Manifest
//!
//! ```toml
//! [package]
//! name = "my_project"
//! version = "1.0.0"
//! description = "A LOGOS project"
//! entry = "src/main.lg"
//!
//! [dependencies]
//! std = "logos:std"
//! math = { path = "./math" }
//! ```

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::Path;

/// Project manifest (`Largo.toml`).
///
/// The root structure of a LOGOS project manifest, containing package
/// metadata and dependency specifications.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    /// Package metadata section.
    pub package: Package,
    /// Map of dependency names to their specifications.
    #[serde(default)]
    pub dependencies: HashMap<String, DependencySpec>,
    /// Test-framework configuration (`[test]`) — the discipline ledger's
    /// budgets. Budgets should
    /// only ever shrink.
    #[serde(default)]
    pub test: Option<TestSection>,
    /// Web-site configuration (`[web]`) — whether a page ships the built-in theme.
    #[serde(default)]
    pub web: Option<WebSection>,
}

/// The `[web]` section: a site's opt-out of the framework's built-in theme.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WebSection {
    /// Whether a built page carries the built-in design-token `<style>` — `true` unless a
    /// project brings its whole design and writes `theme = false`.
    #[serde(default = "yes")]
    pub theme: bool,
}

fn yes() -> bool {
    true
}

/// The `[test]` section: conscious debt budgets for the test runner.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TestSection {
    /// How many loud skips the suite may carry (default 0 — an unbudgeted
    /// skip fails the run).
    #[serde(default)]
    pub skip_budget: usize,
}

/// Package metadata from the `[package]` section.
///
/// Contains identifying information about the package used for
/// building and publishing.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Package {
    /// Package name (used for registry publishing).
    pub name: String,
    /// Semantic version string. Defaults to "0.1.0".
    #[serde(default = "default_version")]
    pub version: String,
    /// Short description of the package.
    #[serde(default)]
    pub description: Option<String>,
    /// List of package authors.
    #[serde(default)]
    pub authors: Vec<String>,
    /// Entry point file path. Defaults to "src/main.lg".
    #[serde(default = "default_entry")]
    pub entry: String,
    /// The address this site is served at, e.g. `https://acme.example` — the one
    /// thing a project knows about itself that its own files cannot.
    ///
    /// A `sitemap.xml`'s `<loc>` is ABSOLUTE by specification, so without this there
    /// is no honest sitemap to write and none is written. Optional on purpose: a
    /// project that is not a public website has no address to declare, and demanding
    /// one would make every non-web project carry a field it must invent.
    #[serde(default)]
    pub site: Option<String>,
}

/// Dependency specification.
///
/// Dependencies can be specified in two forms:
/// - Simple: Just a version string or URI (`"1.0.0"` or `"logos:std"`)
/// - Detailed: A table with version, path, or git fields
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DependencySpec {
    /// Simple version string or URI (e.g., `"1.0.0"`, `"logos:std"`).
    Simple(String),
    /// Detailed specification with explicit fields.
    Detailed(DependencyDetail),
}

impl std::fmt::Display for DependencySpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DependencySpec::Simple(s) => write!(f, "{}", s),
            DependencySpec::Detailed(d) => {
                if let Some(v) = &d.version {
                    write!(f, "{}", v)
                } else if let Some(p) = &d.path {
                    write!(f, "path:{}", p)
                } else if let Some(g) = &d.git {
                    write!(f, "git:{}", g)
                } else {
                    write!(f, "*")
                }
            }
        }
    }
}

/// Detailed dependency specification.
///
/// Allows specifying dependencies from multiple sources:
/// version requirements, local paths, or git repositories.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyDetail {
    /// Version requirement (e.g., `"^1.0"`, `">=2.0.0"`).
    #[serde(default)]
    pub version: Option<String>,
    /// Local filesystem path to the dependency.
    #[serde(default)]
    pub path: Option<String>,
    /// Git repository URL.
    #[serde(default)]
    pub git: Option<String>,
    /// Exact commit for a `git` dependency — a full 40-character hex SHA-1. A
    /// git dependency MUST pin an exact commit: a floating `branch`/`tag` alone
    /// is an irreproducible (false) build and is rejected during resolution.
    #[serde(default)]
    pub rev: Option<String>,
    /// A git branch. Recorded for diagnostics/vendoring, but NEVER sufficient on
    /// its own — an exact `rev` is still required.
    #[serde(default)]
    pub branch: Option<String>,
    /// A git tag. Like `branch`, informational only — an exact `rev` is required.
    #[serde(default)]
    pub tag: Option<String>,
}

/// Where a declared dependency's source is rooted on disk, resolved from the
/// manifest against a canonical project root.
///
/// PATH dependencies resolve their `path` relative to the project root and
/// canonicalize it. GIT dependencies require an exact commit `rev` and resolve
/// against a preserved local checkout (the vendored source) — see the module's
/// [fetch/vendor documentation](ResolvedDep#vendoring). Both forms yield an
/// existing, canonicalized directory that the module loader may traverse.
///
/// # Vendoring
///
/// A git dependency's source is expected to already be present on disk: `largo`
/// vendors an exactly-pinned checkout under `<project>/.logos-deps/<name>-<rev>/`
/// (a `git clone` + `git checkout <rev>`, done once and cached — the checkout is
/// content-addressed by its commit, so it is reproducible and never re-fetched).
/// The manifest's `path` field on a git dependency names that resolved checkout
/// explicitly, which is what makes the exact-rev + closure-linking contract
/// provable offline: resolution reads the pinned source directory, never the
/// network. When only `git`/`rev` are given (no explicit `path`), resolution
/// looks in the conventional vendor directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDep {
    /// The dependency's declared name (its `[dependencies]` key), also the alias
    /// an `Import <Name>.` binds to.
    pub name: String,
    /// The canonicalized root directory the dependency's modules live under.
    pub root: std::path::PathBuf,
    /// The origin the source was resolved from, for the build manifest.
    pub source: DepSource,
}

/// The origin a [`ResolvedDep`] was resolved from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DepSource {
    /// A local `path = "…"` dependency.
    Path,
    /// A `git = "…"` dependency pinned to an exact commit `rev`.
    Git {
        /// The repository URL.
        url: String,
        /// The exact 40-hex commit the source is pinned to.
        rev: String,
    },
}

/// An error resolving a `[dependencies]` declaration into a [`ResolvedDep`].
#[derive(Debug)]
pub enum DependencyError {
    /// A `path`/`git` dependency's resolved root does not exist on disk.
    Missing {
        name: String,
        root: std::path::PathBuf,
    },
    /// A `git` dependency did not pin an exact commit `rev`.
    FloatingGit { name: String },
    /// A `git` dependency's `rev` is not a full 40-character hex commit.
    MalformedRev { name: String, rev: String },
    /// A dependency form we cannot LINK locally (a registry version, or an empty
    /// detail table). Registry linking is intentionally unsupported for the local
    /// closure — path/git-local only.
    NotLinkable { name: String, detail: String },
}

impl std::fmt::Display for DependencyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DependencyError::Missing { name, root } => write!(
                f,
                "dependency `{name}` points at `{}`, which does not exist",
                root.display()
            ),
            DependencyError::FloatingGit { name } => write!(
                f,
                "git dependency `{name}` must pin an exact commit — add `rev = \"<40-hex commit>\"`; \
                 a branch or tag alone is an irreproducible build"
            ),
            DependencyError::MalformedRev { name, rev } => write!(
                f,
                "git dependency `{name}` has rev `{rev}`, which is not a full 40-character hex commit"
            ),
            DependencyError::NotLinkable { name, detail } => write!(
                f,
                "dependency `{name}` ({detail}) cannot be linked from the local closure — \
                 use a `path` or an exactly-pinned `git` dependency (registry linking is not supported here)"
            ),
        }
    }
}

impl std::error::Error for DependencyError {}

/// True for a full 40-character lowercase-or-uppercase hex commit SHA-1.
fn is_exact_commit(rev: &str) -> bool {
    rev.len() == 40 && rev.chars().all(|c| c.is_ascii_hexdigit())
}

fn default_version() -> String {
    "0.1.0".to_string()
}

fn default_entry() -> String {
    "src/main.lg".to_string()
}

/// Errors that can occur when loading a manifest
#[derive(Debug)]
pub enum ManifestError {
    Io(std::path::PathBuf, String),
    Parse(std::path::PathBuf, String),
    Serialize(String),
}

impl std::fmt::Display for ManifestError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ManifestError::Io(path, e) => write!(f, "Failed to read {}: {}", path.display(), e),
            ManifestError::Parse(path, e) => write!(f, "Failed to parse {}: {}", path.display(), e),
            ManifestError::Serialize(e) => write!(f, "Failed to serialize manifest: {}", e),
        }
    }
}

impl std::error::Error for ManifestError {}

impl Manifest {
    /// Load manifest from a directory (looks for Largo.toml)
    pub fn load(dir: &Path) -> Result<Self, ManifestError> {
        let path = dir.join("Largo.toml");
        let content = fs::read_to_string(&path)
            .map_err(|e| ManifestError::Io(path.clone(), e.to_string()))?;
        toml::from_str(&content).map_err(|e| ManifestError::Parse(path, e.to_string()))
    }

    /// Whether a built page carries the framework's theme — the ONE rule every page
    /// composition site reads, so a project's `[web]` `theme = false` reaches all of them alike.
    pub fn shell_theme(&self) -> logicaffeine_compile::web_host::ShellTheme {
        match &self.web {
            Some(web) if !web.theme => logicaffeine_compile::web_host::ShellTheme::None,
            _ => logicaffeine_compile::web_host::ShellTheme::Default,
        }
    }

    /// Create a new manifest with default values
    pub fn new(name: &str) -> Self {
        Manifest {
            package: Package {
                name: name.to_string(),
                version: default_version(),
                description: None,
                authors: Vec::new(),
                entry: default_entry(),
                site: None,
            },
            dependencies: HashMap::new(),
            test: None,
            web: None,
        }
    }

    /// Serialize to TOML string
    pub fn to_toml(&self) -> Result<String, ManifestError> {
        toml::to_string_pretty(self).map_err(|e| ManifestError::Serialize(e.to_string()))
    }

    /// Resolve every LINKABLE `[dependencies]` declaration against `project_root`
    /// into a canonicalized [`ResolvedDep`].
    ///
    /// Only PATH and exactly-pinned GIT dependencies are linkable from the local
    /// closure — a plain version string (`foo = "1.2"`) or a `logos:` intrinsic is
    /// resolved elsewhere (the AOT `## Requires` path / the built-in prelude) and
    /// is skipped here, never treated as an error. This is what keeps the local
    /// closure registry-free: `resolve_dependencies` reaches only the filesystem.
    ///
    /// Enforced invariants:
    /// - A `git` dependency MUST carry an exact 40-hex commit `rev`
    ///   ([`DependencyError::FloatingGit`] / [`DependencyError::MalformedRev`]).
    /// - A `path`/`git` dependency's resolved root MUST exist
    ///   ([`DependencyError::Missing`]).
    ///
    /// Determinism: names are resolved in sorted order, so the returned vec is
    /// stable regardless of the manifest's `HashMap` iteration order.
    pub fn resolve_dependencies(
        &self,
        project_root: &Path,
    ) -> Result<Vec<ResolvedDep>, DependencyError> {
        let mut names: Vec<&String> = self.dependencies.keys().collect();
        names.sort();
        let mut resolved = Vec::new();
        for name in names {
            if let Some(dep) = self.resolve_one(name, &self.dependencies[name], project_root)? {
                resolved.push(dep);
            }
        }
        Ok(resolved)
    }

    /// Resolve one declaration; `Ok(None)` for a non-linkable-but-legal form
    /// (a version string / `logos:` intrinsic — resolved outside the local closure).
    fn resolve_one(
        &self,
        name: &str,
        spec: &DependencySpec,
        project_root: &Path,
    ) -> Result<Option<ResolvedDep>, DependencyError> {
        let detail = match spec {
            // A bare version / `logos:` URI is not a local-closure dependency.
            DependencySpec::Simple(_) => return Ok(None),
            DependencySpec::Detailed(d) => d,
        };
        // GIT takes precedence: an entry carrying `git` is a git dependency even
        // if a `path` is also present (the path names the resolved checkout).
        if let Some(url) = &detail.git {
            let rev = detail
                .rev
                .as_deref()
                .ok_or_else(|| DependencyError::FloatingGit {
                    name: name.to_string(),
                })?;
            if !is_exact_commit(rev) {
                return Err(DependencyError::MalformedRev {
                    name: name.to_string(),
                    rev: rev.to_string(),
                });
            }
            // The preserved local checkout: an explicit `path`, else the
            // conventional vendor directory keyed by name + commit.
            let checkout = match &detail.path {
                Some(p) => project_root.join(p),
                None => project_root
                    .join(".logos-deps")
                    .join(format!("{name}-{rev}")),
            };
            let root = checkout
                .canonicalize()
                .map_err(|_| DependencyError::Missing {
                    name: name.to_string(),
                    root: checkout,
                })?;
            return Ok(Some(ResolvedDep {
                name: name.to_string(),
                root,
                source: DepSource::Git {
                    url: url.clone(),
                    rev: rev.to_string(),
                },
            }));
        }
        if let Some(p) = &detail.path {
            let joined = project_root.join(p);
            let root = joined
                .canonicalize()
                .map_err(|_| DependencyError::Missing {
                    name: name.to_string(),
                    root: joined,
                })?;
            return Ok(Some(ResolvedDep {
                name: name.to_string(),
                root,
                source: DepSource::Path,
            }));
        }
        if detail.version.is_some() {
            // A detailed version-only dependency is a registry dependency —
            // not linkable from the local closure.
            return Ok(None);
        }
        Err(DependencyError::NotLinkable {
            name: name.to_string(),
            detail: "no path or git source".to_string(),
        })
    }
}

#[cfg(test)]
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_cli/project/manifest/tests.rs"]
mod tests;
