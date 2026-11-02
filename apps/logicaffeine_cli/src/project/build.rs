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

use crate::compile::compile_project_with_deps;
use logicaffeine_compile::compile::{copy_runtime_crates, CompileError, RuntimeCrateSource};

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

/// Native executable backend selected by `largo build` / `largo run`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    /// The existing LOGOS → Rust → rustc pipeline.
    Rust,
    /// A LOGOS → C lowering compiled by the host C compiler.
    C(super::build_c::CLowering),
}

impl Backend {
    /// Parse a native C backend name accepted by `--emit`.
    pub fn from_emit(emit: &str) -> Option<Self> {
        match emit {
            "c" => Some(Self::C(super::build_c::CLowering::Ast)),
            "c-bytecode" => Some(Self::C(super::build_c::CLowering::Bytecode)),
            _ => None,
        }
    }
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
    /// The program does not compile: the diagnostic, rendered at the author's `file:line:col`.
    Diagnostic(String),
    /// File system operation failed.
    Io(String),
    /// Cargo build command failed (classified; the raw output already
    /// streamed to the user's terminal live).
    Cargo(CargoFailure),
    /// Cargo itself was not found on PATH (no Rust toolchain installed).
    Toolchain(String),
    /// The selected C compiler was not found.
    CToolchain(String),
    /// The selected C compiler rejected generated code.
    CCompiler(String),
    /// The selected backend does not support this build shape.
    Unsupported(String),
    /// A required file or directory was not found.
    NotFound(String),
}

impl std::fmt::Display for BuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BuildError::Manifest(e) => write!(f, "{}", e),
            BuildError::Compile(e) => write!(f, "{}", e),
            BuildError::Diagnostic(rendered) => write!(f, "{rendered}"),
            BuildError::Io(e) => write!(f, "IO error: {}", e),
            BuildError::Cargo(e) => write!(f, "{}", e),
            BuildError::Toolchain(e) => write!(f, "{}", e),
            BuildError::CToolchain(e) => write!(f, "{}", e),
            BuildError::CCompiler(e) => write!(f, "{}", e),
            BuildError::Unsupported(e) => write!(f, "{}", e),
            BuildError::NotFound(e) => write!(f, "Not found: {}", e),
        }
    }
}

impl std::error::Error for BuildError {}

/// What actually went wrong when `cargo build` failed, judged from its
/// stderr tail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CargoFailureKind {
    /// A dependency could not be resolved or fetched — a `## Requires`
    /// problem (bad crate name/version, or no network), not a compiler bug.
    DependencyResolution,
    /// rustc rejected the generated code — a LOGOS compiler bug, never the
    /// user's fault.
    GeneratedCode,
}

/// A classified `cargo build` failure. The full output already streamed to
/// the terminal; this carries the verdict plus the retained tail for
/// programmatic callers.
#[derive(Debug)]
pub struct CargoFailure {
    /// The classification verdict.
    pub kind: CargoFailureKind,
    /// The last portion of cargo's stderr (bounded).
    pub tail: String,
    /// The generated Cargo project the failure occurred in.
    pub project_dir: PathBuf,
}

impl std::fmt::Display for CargoFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.kind {
            CargoFailureKind::DependencyResolution => write!(
                f,
                "a `## Requires` dependency could not be resolved (cargo's output above has the details)"
            ),
            CargoFailureKind::GeneratedCode => write!(
                f,
                "the generated Rust failed to compile — this is a LOGOS compiler bug, not an error in your program (generated project: {})",
                self.project_dir.display()
            ),
        }
    }
}

/// Whether a `## Requires` crate name / version / feature is safe to write
/// into the generated Cargo.toml verbatim: strictly alphanumerics plus the
/// characters cargo specs actually use. Anything else (quotes, newlines,
/// brackets) could smuggle TOML structure into the manifest.
pub fn requires_component_is_safe(s: &str) -> bool {
    !s.is_empty()
        && s.chars().all(|c| {
            c.is_ascii_alphanumeric()
                || matches!(
                    c,
                    '-' | '_' | '.' | '^' | '~' | '*' | '<' | '>' | '=' | '+' | ',' | ' '
                )
        })
}

/// Judge a failed `cargo build` from its stderr tail.
///
/// Dependency-resolution failures (unknown crate, impossible version
/// requirement, index/network fetch trouble) are the user-actionable class;
/// everything else — above all rustc errors in the generated code — is a
/// compiler bug on our side and is framed that way.
pub fn classify_cargo_failure(stderr_tail: &str) -> CargoFailureKind {
    const DEPENDENCY_MARKERS: [&str; 6] = [
        "no matching package",
        "failed to select a version",
        "failed to get ",
        "failed to fetch",
        "failed to load source for dependency",
        "unable to get packages from source",
    ];
    if DEPENDENCY_MARKERS.iter().any(|m| stderr_tail.contains(m)) {
        CargoFailureKind::DependencyResolution
    } else {
        CargoFailureKind::GeneratedCode
    }
}

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

/// Plan the embedded site for a full-stack project, or `None` when this is not one — the
/// project's own [`BuildPlan`](super::shape::BuildPlan), the same decision `largo serve` routes on.
/// A broken page (or an unassemblable import) is a LOUD build error here, before cargo — never a
/// silently-dropped asset.
///
/// The returned table is the whole site in the dev router's URL namespace, so a binary built from
/// it serves BYTE-for-byte what `largo serve` serves from memory (dev ≡ prod by construction).
fn plan_web_bundle(
    project_dir: &Path,
) -> Result<Option<Vec<crate::commands::web_bundle::Asset>>, BuildError> {
    // An entry whose shape cannot be decided does not parse; the compile step below reports that
    // at its line, so it is not reported twice here.
    let embeds_site = super::shape::Project::load(project_dir)
        .and_then(|project| project.shape())
        .map(|shape| shape.build_plan() == super::shape::BuildPlan::Binary { embeds_site: true })
        .unwrap_or(false);
    if !embeds_site {
        return Ok(None);
    }
    let assets = crate::commands::web_bundle::plan_site(
        &project_dir.join("pages"),
        &project_dir.join("static"),
    )
    .map_err(|e| BuildError::Io(e.to_string()))?;
    Ok(Some(assets))
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
    build_backend(config, Backend::Rust)
}

/// Build through an explicitly selected native executable backend.
pub fn build_backend(config: BuildConfig, backend: Backend) -> Result<BuildResult, BuildError> {
    // Load manifest
    let manifest = Manifest::load(&config.project_dir)?;

    // The entry program (`.lg`, else its `.md` sibling), confined to the project. A site has
    // none to build: `largo build` exports it instead, and a binary of it is not a thing.
    let entry_path = super::shape::entry_file(&config.project_dir, &manifest)
        .map_err(|e| BuildError::NotFound(e.message))?
        .ok_or_else(|| {
            BuildError::NotFound(super::shape::site_without_entry(&config.project_dir).message)
        })?;
    match backend {
        Backend::Rust => build_with_entry(&config, &manifest, &entry_path),
        Backend::C(lowering) => {
            super::build_c::build_c_with_entry(&config, &manifest, &entry_path, lowering)
        }
    }
}

fn build_with_entry(
    config: &BuildConfig,
    manifest: &Manifest,
    entry_path: &Path,
) -> Result<BuildResult, BuildError> {
    let started = std::time::Instant::now();
    crate::ui::phase(
        "Compiling",
        format!(
            "{} v{} (LOGOS → Rust)",
            manifest.package.name, manifest.package.version
        ),
    );

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

    // B4 — full-stack embed. A web project (a `Serve` entry alongside `pages/`
    // or `static/`) bundles its whole site INTO the binary: every page compiled
    // to wasm/js/html and every static asset collected, planned into ONE table
    // in the dev router's URL namespace. `None` for a plain program (or a
    // library build — a cdylib does not serve), where the build stays exactly as
    // before. A broken page fails the build LOUDLY here, before cargo.
    let site_assets = if config.lib_mode {
        None
    } else {
        plan_web_bundle(&config.project_dir)?
    };

    // Link the declared local (`path`/`git`) dependencies: their roots are
    // admitted by the loader so a bare `Import <Dep>.` resolves outside the
    // project, threaded identically to every other tier. A floating git rev or
    // a missing path fails the build loudly here.
    let dep_roots = manifest
        .resolve_dependencies(&config.project_dir)
        .map_err(|e| BuildError::Io(e.to_string()))?
        .into_iter()
        .map(|d| logicaffeine_compile::loader::DepRoot {
            name: d.name,
            root: d.root,
        })
        .collect::<Vec<_>>();

    // Compile LOGOS to Rust using Phase 36 compile_project. For a web build the
    // `Serve` closure must consult the embedded `logos_assets` table before the
    // handler (assets-first, the dev router's precedence) — a thread-local
    // codegen scope threads that in without touching the plain-program path.
    let compiled = if site_assets.is_some() {
        logicaffeine_compile::codegen::with_embed_assets(|| {
            compile_project_with_deps(entry_path, dep_roots.clone())
        })
    } else {
        compile_project_with_deps(entry_path, dep_roots.clone())
    };
    let output = compiled.map_err(|e| match e {
        CompileError::Parse(_) => BuildError::Diagnostic(
            crate::diagnostics::compile_error(&e, entry_path, &dep_roots).message,
        ),
        other => BuildError::Compile(other),
    })?;

    // Write generated Rust code
    let src_dir = rust_project_dir.join("src");
    fs::create_dir_all(&src_dir).map_err(|e| BuildError::Io(e.to_string()))?;

    let rust_code = output.rust_code.clone();

    if config.lib_mode {
        // Library mode: strip fn main() wrapper, write to lib.rs
        let lib_code = strip_main_wrapper(&rust_code);
        fs::write(src_dir.join("lib.rs"), lib_code).map_err(|e| BuildError::Io(e.to_string()))?;
    } else if let Some(ref assets) = site_assets {
        // Emit the `logos_assets` module + stage each asset's bytes beside it in
        // `src/`, and declare the module at the top of the generated program so
        // its `Serve` closure's `logos_assets::lookup` resolves. The assets ride
        // via `include_bytes!` — the binary is genuinely one self-contained file.
        use crate::commands::web_bundle::{page_patterns, site_assets_module, staged_asset_files};
        // The page patterns the binary must answer — read from the SAME `pages/` the assets were
        // planned from, so the shipped table and the shipped documents cannot disagree.
        let patterns = page_patterns(&config.project_dir.join("pages"));
        fs::write(
            src_dir.join("logos_assets.rs"),
            site_assets_module(&config.project_dir.join("pages"), assets, &patterns),
        )
        .map_err(|e| BuildError::Io(e.to_string()))?;
        for (name, bytes) in staged_asset_files(assets) {
            fs::write(src_dir.join(&name), bytes).map_err(|e| BuildError::Io(e.to_string()))?;
        }
        let main_code = format!("mod logos_assets;\n{rust_code}");
        fs::write(src_dir.join("main.rs"), main_code).map_err(|e| BuildError::Io(e.to_string()))?;
    } else {
        fs::write(src_dir.join("main.rs"), &rust_code)
            .map_err(|e| BuildError::Io(e.to_string()))?;
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

    // Write Cargo.toml for the generated project. Its one target carries the project's
    // [`artifact_identity`], never the bare package name: every project that shares a
    // `CARGO_TARGET_DIR` writes into one directory, and two projects called `app` would otherwise
    // be two builds of one file.
    let identity = artifact_identity(&manifest.package.name, &config.project_dir);
    let mut cargo_toml = format!(
        r#"[package]
name = "{}"
version = "{}"
edition = "2021"
autobins = false
"#,
        manifest.package.name, manifest.package.version
    );
    if config.lib_mode {
        let _ = writeln!(
            cargo_toml,
            "\n[lib]\nname = \"{}\"\npath = \"src/lib.rs\"\ncrate-type = [\"cdylib\"]",
            identity.replace('-', "_")
        );
    } else {
        let _ = writeln!(
            cargo_toml,
            "\n[[bin]]\nname = \"{identity}\"\npath = \"src/main.rs\""
        );
    }

    // In a checkout we stage runtime sources and path-depend on them; an
    // installed `largo` (no checkout) depends on the matching crates.io release.
    let runtime_source = RuntimeCrateSource::detect();
    let _ = writeln!(cargo_toml, "\n[dependencies]");
    let _ = writeln!(cargo_toml, "{}", runtime_source.data_dep());
    let _ = writeln!(
        cargo_toml,
        "{}",
        runtime_source.system_dep(&["full", "http"])
    );
    let _ = writeln!(
        cargo_toml,
        "tokio = {{ version = \"1\", features = [\"rt-multi-thread\", \"macros\"] }}"
    );

    // Third-party crates the generated code names directly (rayon, memchr, serde) — the one list
    // the staged checking manifest shares, so a program that checks is a program that builds.
    for line in logicaffeine_compile::compile::implied_crate_deps(&rust_code, &output.dependencies)
    {
        let _ = writeln!(cargo_toml, "{line}");
    }

    // Auto-inject wasm-bindgen when targeting wasm32
    let mut has_wasm_bindgen = false;
    if let Some(target) = resolved_target {
        if target.starts_with("wasm32") {
            let _ = writeln!(cargo_toml, "wasm-bindgen = \"0.2\"");
            has_wasm_bindgen = true;
        }
    }

    // Append user-declared dependencies from ## Requires blocks — these must
    // stay inside [dependencies], before any later section header, and every
    // component must be manifest-safe (no TOML structure smuggled in).
    for dep in &output.dependencies {
        if dep.name == "wasm-bindgen" && has_wasm_bindgen {
            continue; // Already injected
        }
        if !requires_component_is_safe(&dep.name)
            || !requires_component_is_safe(&dep.version)
            || dep.features.iter().any(|f| !requires_component_is_safe(f))
        {
            return Err(BuildError::Compile(CompileError::Io(format!(
                "`## Requires` declaration for '{}' contains characters that are not valid in a Cargo manifest",
                dep.name
            ))));
        }
        if dep.features.is_empty() {
            let _ = writeln!(cargo_toml, "{} = \"{}\"", dep.name, dep.version);
        } else {
            let feats = dep
                .features
                .iter()
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

    let _ = write!(cargo_toml, "\n{}", runtime_source.linux_engine_section());

    // Release profile: maximize optimization for compiled programs
    let _ = writeln!(
        cargo_toml,
        "\n[profile.release]\nlto = true\nopt-level = 3\ncodegen-units = 1\npanic = \"{}\"\nstrip = true",
        logicaffeine_compile::codegen::release_panic_strategy(&rust_code)
    );

    fs::write(rust_project_dir.join("Cargo.toml"), &cargo_toml)
        .map_err(|e| BuildError::Io(e.to_string()))?;

    // Emit .cargo/config.toml with target-cpu=native for release builds.
    // Only when not cross-compiling (target-cpu=native refers to the host CPU).
    if config.release && resolved_target.is_none() {
        let cargo_config_dir = rust_project_dir.join(".cargo");
        fs::create_dir_all(&cargo_config_dir).map_err(|e| BuildError::Io(e.to_string()))?;
        fs::write(
            cargo_config_dir.join("config.toml"),
            "[build]\nrustflags = [\"-C\", \"target-cpu=native\"]\n",
        )
        .map_err(|e| BuildError::Io(e.to_string()))?;
    }

    // Stage runtime sources only inside a checkout; a standalone install pulls
    // them from crates.io, so there is nothing to copy.
    if runtime_source.is_workspace() {
        copy_runtime_crates(&rust_project_dir)?;
    }

    // Run cargo build: stdout inherits, stderr streams through a tee so the
    // user watches cargo's progress live while we retain a bounded tail for
    // failure classification.
    crate::ui::phase("Building", "generated Rust (cargo)");
    let mut cmd = Command::new("cargo");
    // `json-render-diagnostics`: cargo still renders every diagnostic and its progress to stderr
    // for the author, and reports each file it produced as a JSON message on stdout — which is
    // how this build learns the path of ITS artifact rather than assuming one.
    cmd.arg("build")
        .arg("--message-format=json-render-diagnostics")
        .current_dir(&rust_project_dir);
    if config.release {
        cmd.arg("--release");
    }
    if let Some(target) = resolved_target {
        cmd.arg("--target").arg(target);
    }
    // Cargo sees a pipe on stderr and would disable its own color; pass the
    // resolved choice explicitly so cargo and largo agree.
    let color_arg = match anstream::AutoStream::choice(&std::io::stderr()) {
        anstream::ColorChoice::Never => "never",
        _ => "always",
    };
    cmd.arg("--color").arg(color_arg);
    if crate::ui::is_quiet() {
        cmd.arg("--quiet");
    } else if crate::ui::verbosity() > 0 {
        cmd.arg("--verbose");
    }
    cmd.stdout(std::process::Stdio::piped());
    cmd.stderr(std::process::Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            BuildError::Toolchain(
                "cargo was not found on PATH — a Rust toolchain is required for `largo build`"
                    .to_string(),
            )
        } else {
            BuildError::Io(e.to_string())
        }
    })?;

    // Tee thread: copy raw bytes (cargo redraws progress with `\r`, so no
    // line buffering) to our stderr while retaining the last TAIL_CAP bytes.
    const TAIL_CAP: usize = 64 * 1024;
    let mut child_err = child.stderr.take().expect("stderr was piped");
    let tee = std::thread::spawn(move || {
        use std::io::{Read, Write};
        let mut tail: Vec<u8> = Vec::new();
        let mut buf = [0u8; 8192];
        let mut err_out = std::io::stderr();
        loop {
            match child_err.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    let _ = err_out.write_all(&buf[..n]);
                    let _ = err_out.flush();
                    tail.extend_from_slice(&buf[..n]);
                    if tail.len() > TAIL_CAP {
                        let cut = tail.len() - TAIL_CAP;
                        tail.drain(..cut);
                    }
                }
            }
        }
        tail
    });

    let mut child_out = child.stdout.take().expect("stdout was piped");
    let messages = std::thread::spawn(move || {
        use std::io::Read;
        let mut all = String::new();
        let _ = child_out.read_to_string(&mut all);
        all
    });

    let status = child.wait().map_err(|e| BuildError::Io(e.to_string()))?;
    let tail_bytes = tee.join().unwrap_or_default();
    let messages = messages.join().unwrap_or_default();

    if !status.success() {
        let tail = String::from_utf8_lossy(&tail_bytes).into_owned();
        let kind = classify_cargo_failure(&tail);

        // A generated-code error points at the emitted Rust; re-check with the
        // source-mapped codegen and show the user WHERE in their LOGOS source it
        // came from. Best-effort: on any failure to remap, the raw cargo output
        // (already streamed above) stands alone.
        if kind == CargoFailureKind::GeneratedCode {
            if let Ok(assembled) = logicaffeine_compile::analysis::assemble_project(&entry_path) {
                if let Ok(remapped) = logicaffeine_compile::compile::remap_project_errors(
                    &assembled.source,
                    &rust_project_dir,
                    Some(&assembled.source_map),
                ) {
                    if !remapped.is_empty() {
                        eprintln!("\n— in your LOGOS source —");
                        for err in &remapped {
                            eprintln!("{err}");
                        }
                    }
                }
            }
        }

        return Err(BuildError::Cargo(CargoFailure {
            kind,
            tail,
            project_dir: rust_project_dir,
        }));
    }

    crate::ui::phase(
        "Finished",
        format!(
            "{} profile in {:.1}s",
            if config.release { "release" } else { "dev" },
            started.elapsed().as_secs_f64()
        ),
    );

    // The file this build wrote, as cargo reports it — whatever `CARGO_TARGET_DIR`, `--target` or
    // a cargo config moved it to — published at the project's stable `target/<mode>/`, which only
    // this project writes. The published file is the one reported and the one `largo run` runs.
    let built = built_artifact(&messages, &identity, config.lib_mode).ok_or_else(|| {
        BuildError::Io(format!(
            "cargo finished but reported no {} named `{identity}`; its output is in {}",
            if config.lib_mode { "library" } else { "binary" },
            rust_project_dir.display()
        ))
    })?;
    let built_name = built
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let stable_name = if config.lib_mode {
        built_name.replacen(
            &identity.replace('-', "_"),
            &manifest.package.name.replace('-', "_"),
            1,
        )
    } else {
        built_name.replacen(&identity, &manifest.package.name, 1)
    };
    let binary_path = build_dir.join(&stable_name);
    // A failed publish fails the build: whatever sits at the stable path would be an EARLIER
    // build's, and there is no correct thing to do with a half-published one.
    fs::copy(&built, &binary_path).map_err(|e| {
        BuildError::Io(format!(
            "the build succeeded but its artifact could not be published to {}: {e}\n\
             The freshly built file is at {}. Anything at the stable path is from an EARLIER \
             build — do not ship it.",
            binary_path.display(),
            built.display()
        ))
    })?;

    // Universal ABI: the C header rides beside the published library or binary.
    if output.c_header.is_some() {
        let header_name = format!("{}.h", manifest.package.name);
        let src_header = rust_project_dir.join(&header_name);
        if src_header.exists() {
            let _ = fs::copy(&src_header, build_dir.join(&header_name));
        }
    }

    // A web binary is self-contained — its whole site rides inside via `include_bytes!`, runtime
    // deps resolve from version pins, and the LOGOS stdlib is embedded in the compiler — so it
    // deploys by copying this ONE file.
    if let Some(assets) = &site_assets {
        crate::ui::phase(
            "Bundled",
            format!(
                "{} embedded asset(s) — self-contained web server",
                assets.len()
            ),
        );
    }
    if !config.lib_mode {
        crate::ui::phase("Artifact", binary_path.display().to_string());
    }

    Ok(BuildResult {
        target_dir: build_dir,
        binary_path,
    })
}

/// The name a project's cargo target builds under: its package name and a digest of its
/// canonical directory, so two projects that share a name never share an output file in a shared
/// `CARGO_TARGET_DIR`, and one project always rebuilds the same file.
fn artifact_identity(name: &str, project_dir: &Path) -> String {
    let dir = project_dir
        .canonicalize()
        .unwrap_or_else(|_| project_dir.to_path_buf());
    let digest = crate::commands::web_bundle::content_hash(dir.to_string_lossy().as_bytes());
    format!("{name}-{digest}")
}

/// The file cargo reports producing for the target named `identity`: a binary's `executable`, or
/// a library's shared object — read from cargo's own `compiler-artifact` messages.
fn built_artifact(messages: &str, identity: &str, lib: bool) -> Option<PathBuf> {
    let lib_name = identity.replace('-', "_");
    messages
        .lines()
        .filter_map(|line| serde_json::from_str::<serde_json::Value>(line).ok())
        .filter(|m| m["reason"] == "compiler-artifact")
        .filter(|m| {
            let name = m["target"]["name"].as_str().unwrap_or("");
            if lib {
                name == lib_name
            } else {
                name == identity
            }
        })
        .find_map(|m| {
            if lib {
                m["filenames"]
                    .as_array()?
                    .iter()
                    .filter_map(|f| f.as_str())
                    .find(|f| {
                        let f = f.to_ascii_lowercase();
                        f.ends_with(".so")
                            || f.ends_with(".dylib")
                            || f.ends_with(".dll")
                            || f.ends_with(".wasm")
                    })
                    .map(PathBuf::from)
            } else {
                m["executable"].as_str().map(PathBuf::from)
            }
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
            let dedented: Vec<&str> = main_body
                .lines()
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
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_cli/project/build/tests.rs"]
mod tests;
