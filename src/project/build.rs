//! Phase 37: Build Orchestration
//!
//! Coordinates the build process for LOGOS projects.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::compile::{compile_project, copy_logos_core, CompileError};

use super::manifest::{Manifest, ManifestError};

/// Build configuration
pub struct BuildConfig {
    pub project_dir: PathBuf,
    pub release: bool,
}

/// Result of a build operation
#[derive(Debug)]
pub struct BuildResult {
    pub target_dir: PathBuf,
    pub binary_path: PathBuf,
}

/// Errors that can occur during the build process
#[derive(Debug)]
pub enum BuildError {
    Manifest(ManifestError),
    Compile(CompileError),
    Io(String),
    Cargo(String),
    NotFound(String),
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildError::Manifest(e) => write!(f, "{}", e),
            BuildError::Compile(e) => write!(f, "{}", e),
            BuildError::Io(e) => write!(f, "IO error: {}", e),
            BuildError::Cargo(e) => write!(f, "Cargo error: {}", e),
            BuildError::NotFound(e) => write!(f, "Not found: {}", e),
        }
    }
}

impl std::error::Error for BuildError {}

impl From<ManifestError> for BuildError {
    fn from(e: ManifestError) -> Self {
        BuildError::Manifest(e)
    }
}

impl From<CompileError> for BuildError {
    fn from(e: CompileError) -> Self {
        BuildError::Compile(e)
    }
}

/// Find project root by walking up directory tree looking for Largo.toml
pub fn find_project_root(start: &Path) -> Option<PathBuf> {
    let mut current = if start.is_file() {
        start.parent()?.to_path_buf()
    } else {
        start.to_path_buf()
    };

    loop {
        if current.join("Largo.toml").exists() {
            return Some(current);
        }
        if !current.pop() {
            return None;
        }
    }
}

/// Build a LOGOS project
pub fn build(config: BuildConfig) -> Result<BuildResult, BuildError> {
    // Load manifest
    let manifest = Manifest::load(&config.project_dir)?;

    // Resolve entry point (supports .lg and .md)
    let entry_path = config.project_dir.join(&manifest.package.entry);
    if entry_path.exists() {
        return build_with_entry(&config, &manifest, &entry_path);
    }

    // Try .md fallback if .lg not found
    let md_path = entry_path.with_extension("md");
    if md_path.exists() {
        return build_with_entry(&config, &manifest, &md_path);
    }

    Err(BuildError::NotFound(format!(
        "Entry point not found: {} (also tried .md)",
        entry_path.display()
    )))
}

fn build_with_entry(
    config: &BuildConfig,
    manifest: &Manifest,
    entry_path: &Path,
) -> Result<BuildResult, BuildError> {
    // Create target directory structure
    let target_dir = config.project_dir.join("target");
    let build_dir = if config.release {
        target_dir.join("release")
    } else {
        target_dir.join("debug")
    };
    let rust_project_dir = build_dir.join("build");

    // Clean and recreate build directory
    if rust_project_dir.exists() {
        fs::remove_dir_all(&rust_project_dir).map_err(|e| BuildError::Io(e.to_string()))?;
    }
    fs::create_dir_all(&rust_project_dir).map_err(|e| BuildError::Io(e.to_string()))?;

    // Compile LOGOS to Rust using Phase 36 compile_project
    let rust_code = compile_project(entry_path)?;

    // Write generated Rust code
    let src_dir = rust_project_dir.join("src");
    fs::create_dir_all(&src_dir).map_err(|e| BuildError::Io(e.to_string()))?;

    let main_rs = format!("use logos_core::prelude::*;\n\n{}", rust_code);
    fs::write(src_dir.join("main.rs"), main_rs).map_err(|e| BuildError::Io(e.to_string()))?;

    // Write Cargo.toml for the generated project
    let cargo_toml = format!(
        r#"[package]
name = "{}"
version = "{}"
edition = "2021"

[dependencies]
logos_core = {{ path = "./logos_core" }}
"#,
        manifest.package.name, manifest.package.version
    );
    fs::write(rust_project_dir.join("Cargo.toml"), cargo_toml)
        .map_err(|e| BuildError::Io(e.to_string()))?;

    // Copy logos_core runtime
    copy_logos_core(&rust_project_dir)?;

    // Run cargo build
    let mut cmd = Command::new("cargo");
    cmd.arg("build").current_dir(&rust_project_dir);
    if config.release {
        cmd.arg("--release");
    }

    let output = cmd.output().map_err(|e| BuildError::Io(e.to_string()))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(BuildError::Cargo(stderr.to_string()));
    }

    // Determine binary path
    let binary_name = if cfg!(windows) {
        format!("{}.exe", manifest.package.name)
    } else {
        manifest.package.name.clone()
    };
    let cargo_target = if config.release { "release" } else { "debug" };
    let binary_path = rust_project_dir
        .join("target")
        .join(cargo_target)
        .join(&binary_name);

    Ok(BuildResult {
        target_dir: build_dir,
        binary_path,
    })
}

/// Run a built project
pub fn run(build_result: &BuildResult) -> Result<i32, BuildError> {
    let mut child = Command::new(&build_result.binary_path)
        .spawn()
        .map_err(|e| BuildError::Io(e.to_string()))?;

    let status = child.wait().map_err(|e| BuildError::Io(e.to_string()))?;

    Ok(status.code().unwrap_or(1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn find_project_root_finds_largo_toml() {
        let temp = tempdir().unwrap();
        let sub = temp.path().join("a/b/c");
        fs::create_dir_all(&sub).unwrap();
        fs::write(temp.path().join("Largo.toml"), "[package]\nname=\"test\"\n").unwrap();

        let found = find_project_root(&sub);
        assert!(found.is_some());
        assert_eq!(found.unwrap(), temp.path());
    }

    #[test]
    fn find_project_root_returns_none_if_not_found() {
        let temp = tempdir().unwrap();
        let found = find_project_root(temp.path());
        assert!(found.is_none());
    }
}
