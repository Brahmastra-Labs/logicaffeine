//! Phase 37: Build Orchestration
//!
//! Coordinates the build process for LOGOS projects.
//!
//! This module handles the complete build pipeline:
//! 1. Load the project manifest (`Largo.toml`)
//! 2. Compile LOGOS source to Rust code
//! 3. Set up a Cargo project with runtime dependencies
//! 4. Invoke `cargo build` to produce the final binary
//!
//! # Build Directory Structure
//!
//! ```text
//! target/
//! ├── debug/
//! │   └── build/           # Generated Cargo project (debug)
//! │       ├── Cargo.toml
//! │       ├── src/main.rs  # Generated Rust code
//! │       └── target/      # Cargo's output
//! └── release/
//!     └── build/           # Generated Cargo project (release)
//! ```

use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::compile::compile_project;
use logicaffeine_compile::compile::{copy_runtime_crates, CompileError};

use super::manifest::{Manifest, ManifestError};

/// Configuration for a build operation.
///
/// Specifies the project location and build mode (debug/release).
///
/// # Example
///
/// ```no_run
/// use std::path::PathBuf;
/// use logicaffeine_cli::project::build::{BuildConfig, build};
///
/// let config = BuildConfig {
///     project_dir: PathBuf::from("my_project"),
///     release: false,
/// };
///
/// let result = build(config)?;
/// println!("Built: {}", result.binary_path.display());
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
pub struct BuildConfig {
    /// Root directory of the LOGOS project (contains `Largo.toml`).
    pub project_dir: PathBuf,
    /// If `true`, build with optimizations (`cargo build --release`).
    pub release: bool,
}

/// Result of a successful build operation.
///
/// Contains paths to the build outputs, used by subsequent commands
/// like [`run`] to execute the compiled binary.
#[derive(Debug)]
pub struct BuildResult {
    /// Directory containing build artifacts (`target/debug` or `target/release`).
    pub target_dir: PathBuf,
    /// Path to the compiled executable.
    pub binary_path: PathBuf,
}

/// Errors that can occur during the build process.
#[derive(Debug)]
pub enum BuildError {
    /// Failed to load or parse the project manifest.
    Manifest(ManifestError),
    /// LOGOS-to-Rust compilation failed.
    Compile(CompileError),
    /// File system operation failed.
    Io(String),
    /// Cargo build command failed.
    Cargo(String),
    /// A required file or directory was not found.
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

/// Find the project root by walking up the directory tree.
///
/// Searches for a `Largo.toml` file starting from `start` and moving
/// up through parent directories. Returns the directory containing
/// the manifest, or `None` if no manifest is found.
///
/// # Arguments
///
/// * `start` - Starting path (can be a file or directory)
///
/// # Example
///
/// ```no_run
/// use std::path::Path;
/// use logicaffeine_cli::project::build::find_project_root;
///
/// // Find project root from a subdirectory
/// let root = find_project_root(Path::new("/projects/myapp/src/lib.lg"));
/// assert_eq!(root, Some("/projects/myapp".into()));
/// ```
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

/// Build a LOGOS project.
///
/// Compiles the project specified in `config` through the full build pipeline:
/// 1. Load and validate the manifest
/// 2. Compile LOGOS source to Rust
/// 3. Generate a Cargo project with runtime dependencies
/// 4. Run `cargo build`
///
/// The entry point is determined from the manifest's `package.entry` field,
/// with a `.md` extension fallback if the `.lg` file doesn't exist.
///
/// # Errors
///
/// Returns an error if:
/// - The manifest cannot be loaded
/// - The entry point file doesn't exist
/// - LOGOS compilation fails
/// - Cargo build fails
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
    let output = compile_project(entry_path)?;

    // Write generated Rust code
    let src_dir = rust_project_dir.join("src");
    fs::create_dir_all(&src_dir).map_err(|e| BuildError::Io(e.to_string()))?;

    let main_rs = format!("use logicaffeine_data::*;\nuse logicaffeine_system::*;\n\n{}", output.rust_code);
    fs::write(src_dir.join("main.rs"), main_rs).map_err(|e| BuildError::Io(e.to_string()))?;

    // Write Cargo.toml for the generated project
    let mut cargo_toml = format!(
        r#"[package]
name = "{}"
version = "{}"
edition = "2021"

[dependencies]
logicaffeine-data = {{ path = "./crates/logicaffeine_data" }}
logicaffeine-system = {{ path = "./crates/logicaffeine_system", features = ["full"] }}
tokio = {{ version = "1", features = ["rt-multi-thread", "macros"] }}
"#,
        manifest.package.name, manifest.package.version
    );

    // Append user-declared dependencies from ## Requires blocks
    for dep in &output.dependencies {
        if dep.features.is_empty() {
            let _ = writeln!(cargo_toml, "{} = \"{}\"", dep.name, dep.version);
        } else {
            let feats = dep.features.iter()
                .map(|f| format!("\"{}\"", f))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(
                cargo_toml,
                "{} = {{ version = \"{}\", features = [{}] }}",
                dep.name, dep.version, feats
            );
        }
    }

    fs::write(rust_project_dir.join("Cargo.toml"), cargo_toml)
        .map_err(|e| BuildError::Io(e.to_string()))?;

    // Copy runtime crates
    copy_runtime_crates(&rust_project_dir)?;

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

/// Execute a built LOGOS project.
///
/// Spawns the compiled binary and waits for it to complete.
/// Returns the process exit code.
///
/// # Arguments
///
/// * `build_result` - Result from a previous [`build`] call
///
/// # Returns
///
/// The exit code of the process (0 for success, non-zero for failure).
///
/// # Errors
///
/// Returns [`BuildError::Io`] if the process cannot be spawned.
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
