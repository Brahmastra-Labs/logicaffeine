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
///     lib_mode: false,
///     target: None,
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
    /// If `true`, build as a library (cdylib) instead of a binary.
    pub lib_mode: bool,
    /// Target triple for cross-compilation (e.g., "wasm32-unknown-unknown").
    /// "wasm" is expanded to "wasm32-unknown-unknown".
    pub target: Option<String>,
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

    let rust_code = output.rust_code.clone();

    if config.lib_mode {
        // Library mode: strip fn main() wrapper, write to lib.rs
        let lib_code = strip_main_wrapper(&rust_code);
        fs::write(src_dir.join("lib.rs"), lib_code).map_err(|e| BuildError::Io(e.to_string()))?;
    } else {
        fs::write(src_dir.join("main.rs"), &rust_code).map_err(|e| BuildError::Io(e.to_string()))?;
    }

    // Universal ABI: Write C header alongside generated code if present
    if let Some(ref c_header) = output.c_header {
        let header_name = format!("{}.h", manifest.package.name);
        fs::write(rust_project_dir.join(&header_name), c_header)
            .map_err(|e| BuildError::Io(e.to_string()))?;
    }

    // Resolve target triple (expand "wasm" shorthand)
    let resolved_target = config.target.as_deref().map(|t| {
        if t.eq_ignore_ascii_case("wasm") {
            "wasm32-unknown-unknown"
        } else {
            t
        }
    });

    // Write Cargo.toml for the generated project
    let mut cargo_toml = format!(
        r#"[package]
name = "{}"
version = "{}"
edition = "2021"
"#,
        manifest.package.name, manifest.package.version
    );

    // Library mode: add [lib] section with cdylib crate type
    if config.lib_mode {
        let _ = writeln!(cargo_toml, "\n[lib]\ncrate-type = [\"cdylib\"]");
    }

    let _ = writeln!(cargo_toml, "\n[dependencies]");
    let _ = writeln!(cargo_toml, "logicaffeine-data = {{ path = \"./crates/logicaffeine_data\" }}");
    let _ = writeln!(cargo_toml, "logicaffeine-system = {{ path = \"./crates/logicaffeine_system\", features = [\"full\"] }}");
    let _ = writeln!(cargo_toml, "tokio = {{ version = \"1\", features = [\"rt-multi-thread\", \"macros\"] }}");

    // Auto-inject wasm-bindgen when targeting wasm32
    let mut has_wasm_bindgen = false;
    if let Some(target) = resolved_target {
        if target.starts_with("wasm32") {
            let _ = writeln!(cargo_toml, "wasm-bindgen = \"0.2\"");
            has_wasm_bindgen = true;
        }
    }

    // Release profile: maximize optimization for compiled programs
    let _ = writeln!(cargo_toml, "\n[profile.release]\nlto = true\nopt-level = 3\ncodegen-units = 1\npanic = \"abort\"\nstrip = true");

    // Append user-declared dependencies from ## Requires blocks
    for dep in &output.dependencies {
        if dep.name == "wasm-bindgen" && has_wasm_bindgen {
            continue; // Already injected
        }
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

    fs::write(rust_project_dir.join("Cargo.toml"), &cargo_toml)
        .map_err(|e| BuildError::Io(e.to_string()))?;

    // Copy runtime crates
    copy_runtime_crates(&rust_project_dir)?;

    // Run cargo build
    let mut cmd = Command::new("cargo");
    cmd.arg("build").current_dir(&rust_project_dir);
    if config.release {
        cmd.arg("--release");
    }
    if let Some(target) = resolved_target {
        cmd.arg("--target").arg(target);
    }

    let cmd_output = cmd.output().map_err(|e| BuildError::Io(e.to_string()))?;

    if !cmd_output.status.success() {
        let stderr = String::from_utf8_lossy(&cmd_output.stderr);
        return Err(BuildError::Cargo(stderr.to_string()));
    }

    // Determine binary/library path
    let cargo_target_str = if config.release { "release" } else { "debug" };
    let binary_path = if config.lib_mode {
        // Library output
        let lib_name = format!("lib{}", manifest.package.name.replace('-', "_"));
        let ext = if cfg!(target_os = "macos") { "dylib" } else { "so" };
        if let Some(target) = resolved_target {
            rust_project_dir
                .join("target")
                .join(target)
                .join(cargo_target_str)
                .join(format!("{}.{}", lib_name, ext))
        } else {
            rust_project_dir
                .join("target")
                .join(cargo_target_str)
                .join(format!("{}.{}", lib_name, ext))
        }
    } else {
        let binary_name = if cfg!(windows) {
            format!("{}.exe", manifest.package.name)
        } else {
            manifest.package.name.clone()
        };
        if let Some(target) = resolved_target {
            rust_project_dir
                .join("target")
                .join(target)
                .join(cargo_target_str)
                .join(&binary_name)
        } else {
            rust_project_dir
                .join("target")
                .join(cargo_target_str)
                .join(&binary_name)
        }
    };

    // Universal ABI: Copy .h file to the same directory as the binary/library
    if let Some(ref _c_header) = output.c_header {
        let header_name = format!("{}.h", manifest.package.name);
        let src_header = rust_project_dir.join(&header_name);
        if src_header.exists() {
            if let Some(parent) = binary_path.parent() {
                let _ = fs::copy(&src_header, parent.join(&header_name));
            }
        }
    }

    Ok(BuildResult {
        target_dir: build_dir,
        binary_path,
    })
}

/// Strip the `fn main() { ... }` wrapper from generated code for library mode.
/// Keeps everything before `fn main()` (imports, types, functions) intact.
fn strip_main_wrapper(code: &str) -> String {
    // Find "fn main() {" and extract content before it
    if let Some(main_pos) = code.find("fn main() {") {
        let before_main = &code[..main_pos];
        // Extract the body of main (between the opening { and closing })
        let after_opening = &code[main_pos + "fn main() {".len()..];
        if let Some(close_pos) = after_opening.rfind('}') {
            let main_body = &after_opening[..close_pos];
            // Dedent main body
            let dedented: Vec<&str> = main_body.lines()
                .map(|line| line.strip_prefix("    ").unwrap_or(line))
                .collect();
            format!("{}\n{}", before_main.trim_end(), dedented.join("\n"))
        } else {
            before_main.to_string()
        }
    } else {
        code.to_string()
    }
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
pub fn run(build_result: &BuildResult, args: &[String]) -> Result<i32, BuildError> {
    let mut child = Command::new(&build_result.binary_path)
        .args(args)
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

    #[test]
    fn strip_main_wrapper_extracts_body() {
        let code = r#"use logicaffeine_data::*;

fn add(a: i64, b: i64) -> i64 {
    a + b
}

fn main() {
    let x = add(1, 2);
    println!("{}", x);
}"#;
        let result = strip_main_wrapper(code);
        assert!(result.contains("fn add(a: i64, b: i64) -> i64"));
        assert!(result.contains("let x = add(1, 2);"));
        assert!(result.contains("println!(\"{}\", x);"));
        assert!(!result.contains("fn main()"));
    }

    #[test]
    fn strip_main_wrapper_preserves_imports() {
        let code = "use logicaffeine_data::*;\nuse logicaffeine_system::*;\n\nfn main() {\n    println!(\"hello\");\n}\n";
        let result = strip_main_wrapper(code);
        assert!(result.contains("use logicaffeine_data::*;"));
        assert!(result.contains("use logicaffeine_system::*;"));
        assert!(result.contains("println!(\"hello\");"));
        assert!(!result.contains("fn main()"));
    }

    #[test]
    fn strip_main_wrapper_no_main_returns_unchanged() {
        let code = "fn add(a: i64, b: i64) -> i64 { a + b }";
        let result = strip_main_wrapper(code);
        assert_eq!(result, code);
    }

    #[test]
    fn strip_main_wrapper_dedents_body() {
        let code = "fn main() {\n    let x = 1;\n    let y = 2;\n}\n";
        let result = strip_main_wrapper(code);
        // Body lines should be dedented by 4 spaces
        assert!(result.contains("let x = 1;"));
        assert!(result.contains("let y = 2;"));
        // Should not have leading 4-space indent
        for line in result.lines() {
            if line.contains("let x") || line.contains("let y") {
                assert!(!line.starts_with("    "), "Line should be dedented: {}", line);
            }
        }
    }
}
