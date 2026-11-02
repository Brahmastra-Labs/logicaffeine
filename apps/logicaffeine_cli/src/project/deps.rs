//! G10 — local dependency linking: resolve `Largo.toml` `[dependencies]` into the
//! loader's admitted dependency roots, and describe the assembled linked closure.
//!
//! This is the CLI-side seam that turns a manifest's `path`/`git` declarations
//! into the [`DepRoot`]s the module loader threads through EVERY tier's assembly
//! (`assemble_closure`), so a bare `Import <Dep>.` links a package living OUTSIDE
//! the project root. Registry linking is intentionally unsupported here — the
//! local closure reaches only the filesystem, so a fully-local project builds
//! with no crates.io access (proved by [`BuildManifest::registry`]).

use std::path::Path;

use logicaffeine_compile::loader::DepRoot;
use serde::Serialize;

use crate::project::manifest::{DepSource, Manifest, ResolvedDep};
use crate::ui::CliError;

/// Resolve the manifest's linkable dependencies against `project_root` into the
/// loader's [`DepRoot`]s. A resolution failure (a floating git rev, a missing
/// path, an unlinkable form) surfaces as a clean [`CliError`], so every command
/// that links deps reports the same message.
pub fn resolve_dep_roots(
    project_root: &Path,
    manifest: &Manifest,
) -> Result<Vec<DepRoot>, CliError> {
    let resolved = resolve(project_root, manifest)?;
    Ok(resolved
        .into_iter()
        .map(|d| DepRoot {
            name: d.name,
            root: d.root,
        })
        .collect())
}

/// The resolved dependencies, surfacing the manifest error as a `CliError`.
fn resolve(project_root: &Path, manifest: &Manifest) -> Result<Vec<ResolvedDep>, CliError> {
    manifest
        .resolve_dependencies(project_root)
        .map_err(|e| CliError::new(e.to_string()))
}

/// One file in the assembled closure, tagged with the dependency it came from.
#[derive(Debug, Clone, Serialize)]
pub struct ClosureFile {
    /// The file's path (absolute, canonical).
    pub path: String,
    /// The dependency this file belongs to: the local package name for the
    /// project's own files, else a declared dependency's name.
    pub dependency: String,
}

/// Whether — and how — a registry was consulted assembling the closure.
#[derive(Debug, Clone, Serialize)]
pub struct RegistryState {
    /// Always `false` for a local closure: `path`/`git`-local linking reaches
    /// only the filesystem, never crates.io.
    pub accessed: bool,
    /// A human note on why the registry was not needed.
    pub note: String,
}

/// The local package's identity in the build manifest.
#[derive(Debug, Clone, Serialize)]
pub struct PackageIdentity {
    pub name: String,
    pub version: String,
}

/// A resolved dependency as it appears in the build manifest.
#[derive(Debug, Clone, Serialize)]
pub struct ManifestDep {
    /// The dependency name (its `Import` alias).
    pub name: String,
    /// `"path"` or `"git"`.
    pub kind: String,
    /// The canonicalized root the dependency's source lives under.
    pub root: String,
    /// For a git dependency, the exact commit it is pinned to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rev: Option<String>,
    /// For a git dependency, the repository URL.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
}

/// A machine-readable account of a linked build: the local package identity,
/// feature selection, every resolved dependency, the transitive closure (each
/// linked `.lg` file tagged with its source dependency), and the registry state.
#[derive(Debug, Clone, Serialize)]
pub struct BuildManifest {
    pub package: PackageIdentity,
    /// The selected feature set. Empty today (LOGOS projects declare no cargo
    /// features), but the key is stable so tooling can rely on it.
    pub features: Vec<String>,
    /// Every resolved `path`/`git` dependency.
    pub dependencies: Vec<ManifestDep>,
    /// The transitive closure: every linked `.lg` file with its source dep.
    pub closure: Vec<ClosureFile>,
    /// Whether a registry was accessed (always `false` for a local closure).
    pub registry: RegistryState,
}

impl BuildManifest {
    /// Serialize to a stable, pretty JSON string.
    pub fn to_json(&self) -> String {
        serde_json::to_string_pretty(self).unwrap_or_else(|_| "{}".to_string())
    }
}

/// Assemble the linked closure for `entry` and describe it as a [`BuildManifest`].
///
/// Every file the closure links is attributed to a dependency: a file rooted
/// under a declared dependency's root carries that dependency's name; every other
/// file (the project's own `src/`) carries the local package name. This is the
/// same `assemble_closure` seam
/// every tier consumes, so the manifest describes exactly what the tiers link.
pub fn build_manifest(
    project_root: &Path,
    manifest: &Manifest,
    entry: &Path,
) -> Result<BuildManifest, CliError> {
    let resolved = resolve(project_root, manifest)?;
    let dep_roots: Vec<DepRoot> = resolved
        .iter()
        .map(|d| DepRoot {
            name: d.name.clone(),
            root: d.root.clone(),
        })
        .collect();

    let closure = crate::closure::assemble_closure_with_deps(entry, dep_roots)
        .map_err(|e| CliError::new(format!("could not assemble project: {e}")))?;

    let local_name = manifest.package.name.clone();
    let closure_files = closure
        .files
        .iter()
        .map(|path| {
            let dependency = resolved
                .iter()
                .find(|d| path.starts_with(&d.root))
                .map(|d| d.name.clone())
                .unwrap_or_else(|| local_name.clone());
            ClosureFile {
                path: path.display().to_string(),
                dependency,
            }
        })
        .collect();

    let dependencies = resolved
        .iter()
        .map(|d| match &d.source {
            DepSource::Path => ManifestDep {
                name: d.name.clone(),
                kind: "path".to_string(),
                root: d.root.display().to_string(),
                rev: None,
                url: None,
            },
            DepSource::Git { url, rev } => ManifestDep {
                name: d.name.clone(),
                kind: "git".to_string(),
                root: d.root.display().to_string(),
                rev: Some(rev.clone()),
                url: Some(url.clone()),
            },
        })
        .collect();

    Ok(BuildManifest {
        package: PackageIdentity {
            name: manifest.package.name.clone(),
            version: manifest.package.version.clone(),
        },
        features: Vec::new(),
        dependencies,
        closure: closure_files,
        registry: RegistryState {
            accessed: false,
            note: "a fully-local (path/git) closure links from the filesystem only — no registry"
                .to_string(),
        },
    })
}
