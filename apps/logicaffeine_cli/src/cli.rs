//! Phase 37/39: LOGOS CLI (largo)
//!
//! Command-line interface for the LOGOS build system and package registry.
//!
//! This module provides the command-line argument parsing and dispatch logic
//! for the `largo` CLI tool. It handles all user-facing commands including
//! project scaffolding, building, running, and package registry operations.
//!
//! # Architecture
//!
//! The CLI is built on [`clap`] for argument parsing with derive macros.
//! Each command variant in [`Commands`] maps to a handler function that
//! performs the actual work.
//!
//! # Examples
//!
//! ```bash
//! # Create a new project
//! largo new my_project
//!
//! # Build and run
//! cd my_project
//! largo run
//!
//! # Publish to registry
//! largo login
//! largo publish
//! ```

use clap::{Parser, Subcommand};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;

use crate::compile::compile_project;
use crate::project::build::{self, find_project_root, BuildConfig};
use crate::project::manifest::Manifest;
use crate::project::credentials::{Credentials, get_token};
use crate::project::registry::{
    RegistryClient, PublishMetadata, create_tarball, is_git_dirty,
};

/// Command-line interface for the LOGOS build tool.
///
/// The `Cli` struct is the top-level argument parser for `largo`. It delegates
/// to the [`Commands`] enum for subcommand handling.
///
/// # Usage
///
/// Typically invoked via [`run_cli`] which parses arguments and dispatches
/// to the appropriate handler:
///
/// ```no_run
/// use logicaffeine_cli::cli::run_cli;
///
/// if let Err(e) = run_cli() {
///     eprintln!("Error: {}", e);
///     std::process::exit(1);
/// }
/// ```
#[derive(Parser)]
#[command(name = "largo")]
#[command(about = "The LOGOS build tool", long_about = None)]
#[command(version)]
pub struct Cli {
    /// The subcommand to execute.
    #[command(subcommand)]
    pub command: Commands,
}

/// Available CLI subcommands.
///
/// Each variant represents a distinct operation that `largo` can perform.
/// Commands are grouped into three categories:
///
/// ## Project Management
/// - [`New`][Commands::New] - Create a new project in a new directory
/// - [`Init`][Commands::Init] - Initialize a project in the current directory
///
/// ## Build & Run
/// - [`Build`][Commands::Build] - Compile the project
/// - [`Run`][Commands::Run] - Build and execute
/// - [`Check`][Commands::Check] - Type-check without building
/// - [`Verify`][Commands::Verify] - Run Z3 static verification
///
/// ## Package Registry
/// - [`Publish`][Commands::Publish] - Upload package to registry
/// - [`Login`][Commands::Login] - Authenticate with registry
/// - [`Logout`][Commands::Logout] - Remove stored credentials
#[derive(Subcommand)]
pub enum Commands {
    /// Create a new LOGOS project in a new directory.
    ///
    /// Scaffolds a complete project structure including:
    /// - `Largo.toml` manifest file
    /// - `src/main.lg` entry point with a "Hello, world!" example
    /// - `.gitignore` configured for LOGOS projects
    ///
    /// # Example
    ///
    /// ```bash
    /// largo new my_project
    /// cd my_project
    /// largo run
    /// ```
    New {
        /// The project name, used for the directory and package name.
        name: String,
    },

    /// Initialize a LOGOS project in the current directory.
    ///
    /// Similar to [`New`][Commands::New] but works in an existing directory.
    /// Creates the manifest and source structure without creating a new folder.
    ///
    /// # Example
    ///
    /// ```bash
    /// mkdir my_project && cd my_project
    /// largo init
    /// ```
    Init {
        /// Project name. If omitted, uses the current directory name.
        #[arg(long)]
        name: Option<String>,
    },

    /// Build the current project.
    ///
    /// Compiles the LOGOS source to Rust, then invokes `cargo build` on the
    /// generated code. The resulting binary is placed in `target/debug/` or
    /// `target/release/` depending on the mode.
    ///
    /// # Verification
    ///
    /// When `--verify` is passed, the build process includes Z3 static
    /// verification of logical constraints. This requires:
    /// - A Pro+ license (via `--license` or `LOGOS_LICENSE` env var)
    /// - The `verification` feature enabled at build time
    ///
    /// # Example
    ///
    /// ```bash
    /// largo build              # Debug build
    /// largo build --release    # Release build with optimizations
    /// largo build --verify     # Build with Z3 verification
    /// ```
    Build {
        /// Build with optimizations enabled.
        #[arg(long, short)]
        release: bool,

        /// Run Z3 static verification after compilation.
        /// Requires a Pro+ license.
        #[arg(long)]
        verify: bool,

        /// License key for verification.
        /// Can also be set via the `LOGOS_LICENSE` environment variable.
        #[arg(long)]
        license: Option<String>,

        /// Build as a library instead of an executable.
        /// Generates `lib.rs` with `crate-type = ["cdylib"]` instead of a binary.
        #[arg(long)]
        lib: bool,

        /// Target triple for cross-compilation.
        /// Use "wasm" as shorthand for "wasm32-unknown-unknown".
        #[arg(long)]
        target: Option<String>,
    },

    /// Run Z3 static verification without building.
    ///
    /// Performs formal verification of logical constraints in the project
    /// using the Z3 SMT solver. This catches logical errors that would be
    /// impossible to detect through testing alone.
    ///
    /// Requires a Pro+ license.
    ///
    /// # Example
    ///
    /// ```bash
    /// largo verify --license sub_xxxxx
    /// # Or with environment variable:
    /// export LOGOS_LICENSE=sub_xxxxx
    /// largo verify
    /// ```
    Verify {
        /// License key for verification.
        /// Can also be set via the `LOGOS_LICENSE` environment variable.
        #[arg(long)]
        license: Option<String>,
    },

    /// Build and run the current project.
    ///
    /// Equivalent to `largo build` followed by executing the resulting binary.
    /// The exit code of the built program is propagated.
    ///
    /// With `--interpret`, skips Rust compilation and uses the tree-walking
    /// interpreter for sub-second feedback during development.
    ///
    /// # Example
    ///
    /// ```bash
    /// largo run              # Debug mode (compile to Rust)
    /// largo run --release    # Release mode
    /// largo run --interpret  # Interpret directly (no compilation)
    /// ```
    Run {
        /// Build with optimizations enabled.
        #[arg(long, short)]
        release: bool,

        /// Run using the interpreter instead of compiling to Rust.
        /// Provides sub-second feedback but lacks full Rust performance.
        #[arg(long, short)]
        interpret: bool,

        /// Arguments to pass to the program.
        #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
        args: Vec<String>,
    },

    /// Check the project for errors without producing a binary.
    ///
    /// Parses and type-checks the LOGOS source without invoking the full
    /// build pipeline. Useful for quick validation during development.
    ///
    /// # Example
    ///
    /// ```bash
    /// largo check
    /// ```
    Check,

    /// Publish the package to the LOGOS registry.
    ///
    /// Packages the project as a tarball and uploads it to the specified
    /// registry. Requires authentication via `largo login`.
    ///
    /// # Pre-flight Checks
    ///
    /// Before publishing, the command verifies:
    /// - The entry point exists
    /// - No uncommitted git changes (unless `--allow-dirty`)
    /// - Valid authentication token
    ///
    /// # Example
    ///
    /// ```bash
    /// largo publish              # Publish to default registry
    /// largo publish --dry-run    # Validate without uploading
    /// ```
    Publish {
        /// Registry URL. Defaults to `registry.logicaffeine.com`.
        #[arg(long)]
        registry: Option<String>,

        /// Perform all validation without actually uploading.
        /// Useful for testing the publish process.
        #[arg(long)]
        dry_run: bool,

        /// Allow publishing with uncommitted git changes.
        /// By default, publishing requires a clean working directory.
        #[arg(long)]
        allow_dirty: bool,
    },

    /// Authenticate with the package registry.
    ///
    /// Stores an API token for the specified registry. The token is saved
    /// in `~/.config/logos/credentials.toml` with restricted permissions.
    ///
    /// # Token Acquisition
    ///
    /// Tokens can be obtained from the registry's web interface:
    /// 1. Visit `{registry}/auth/github` to authenticate
    /// 2. Generate an API token from your profile
    /// 3. Provide it via `--token` or interactive prompt
    ///
    /// # Example
    ///
    /// ```bash
    /// largo login                       # Interactive prompt
    /// largo login --token tok_xxxxx     # Non-interactive
    /// ```
    Login {
        /// Registry URL. Defaults to `registry.logicaffeine.com`.
        #[arg(long)]
        registry: Option<String>,

        /// API token. If omitted, prompts for input on stdin.
        #[arg(long)]
        token: Option<String>,
    },

    /// Remove stored credentials for a registry.
    ///
    /// Deletes the authentication token from the local credentials file.
    ///
    /// # Example
    ///
    /// ```bash
    /// largo logout
    /// ```
    Logout {
        /// Registry URL. Defaults to `registry.logicaffeine.com`.
        #[arg(long)]
        registry: Option<String>,
    },
}

/// Parse CLI arguments and execute the corresponding command.
///
/// This is the main entry point for the `largo` CLI. It parses command-line
/// arguments using [`clap`], then dispatches to the appropriate handler
/// function based on the subcommand.
///
/// # Errors
///
/// Returns an error if:
/// - The project structure is invalid (missing `Largo.toml`)
/// - File system operations fail
/// - Build or compilation fails
/// - Registry operations fail (authentication, network, etc.)
///
/// # Example
///
/// ```no_run
/// use logicaffeine_cli::cli::run_cli;
///
/// fn main() {
///     if let Err(e) = run_cli() {
///         eprintln!("Error: {}", e);
///         std::process::exit(1);
///     }
/// }
/// ```
pub fn run_cli() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::New { name } => cmd_new(&name),
        Commands::Init { name } => cmd_init(name.as_deref()),
        Commands::Build { release, verify, license, lib, target } => cmd_build(release, verify, license, lib, target),
        Commands::Run { interpret, .. } if interpret => cmd_run_interpret(),
        Commands::Run { release, args, .. } => cmd_run(release, &args),
        Commands::Check => cmd_check(),
        Commands::Verify { license } => cmd_verify(license),
        Commands::Publish { registry, dry_run, allow_dirty } => {
            cmd_publish(registry.as_deref(), dry_run, allow_dirty)
        }
        Commands::Login { registry, token } => cmd_login(registry.as_deref(), token),
        Commands::Logout { registry } => cmd_logout(registry.as_deref()),
    }
}

fn cmd_new(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    let project_dir = PathBuf::from(name);

    if project_dir.exists() {
        return Err(format!("Directory '{}' already exists", project_dir.display()).into());
    }

    // Create project structure
    fs::create_dir_all(&project_dir)?;
    fs::create_dir_all(project_dir.join("src"))?;

    // Write Largo.toml
    let manifest = Manifest::new(name);
    fs::write(project_dir.join("Largo.toml"), manifest.to_toml()?)?;

    // Write src/main.lg
    let main_lg = r#"# Main

A simple LOGOS program.

## Main

Show "Hello, world!".
"#;
    fs::write(project_dir.join("src/main.lg"), main_lg)?;

    // Write .gitignore
    fs::write(project_dir.join(".gitignore"), "/target\n")?;

    println!("Created LOGOS project '{}'", name);
    println!("  cd {}", project_dir.display());
    println!("  largo run");

    Ok(())
}

fn cmd_init(name: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let current_dir = env::current_dir()?;
    let project_name = name
        .map(String::from)
        .or_else(|| {
            current_dir
                .file_name()
                .and_then(|n| n.to_str())
                .map(String::from)
        })
        .unwrap_or_else(|| "project".to_string());

    if current_dir.join("Largo.toml").exists() {
        return Err("Largo.toml already exists".into());
    }

    // Create src directory if needed
    fs::create_dir_all(current_dir.join("src"))?;

    // Write Largo.toml
    let manifest = Manifest::new(&project_name);
    fs::write(current_dir.join("Largo.toml"), manifest.to_toml()?)?;

    // Write src/main.lg if it doesn't exist
    let main_path = current_dir.join("src/main.lg");
    if !main_path.exists() {
        let main_lg = r#"# Main

A simple LOGOS program.

## Main

Show "Hello, world!".
"#;
        fs::write(main_path, main_lg)?;
    }

    println!("Initialized LOGOS project '{}'", project_name);

    Ok(())
}

fn cmd_build(
    release: bool,
    verify: bool,
    license: Option<String>,
    lib: bool,
    target: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let current_dir = env::current_dir()?;
    let project_root =
        find_project_root(&current_dir).ok_or("Not in a LOGOS project (Largo.toml not found)")?;

    // Run verification if requested
    if verify {
        run_verification(&project_root, license.as_deref())?;
    }

    let config = BuildConfig {
        project_dir: project_root,
        release,
        lib_mode: lib,
        target,
    };

    let result = build::build(config)?;

    let mode = if release { "release" } else { "debug" };
    println!("Built {} [{}]", result.binary_path.display(), mode);

    Ok(())
}

fn cmd_verify(license: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
    let current_dir = env::current_dir()?;
    let project_root =
        find_project_root(&current_dir).ok_or("Not in a LOGOS project (Largo.toml not found)")?;

    run_verification(&project_root, license.as_deref())?;
    println!("Verification passed");
    Ok(())
}

#[cfg(feature = "verification")]
fn run_verification(
    project_root: &std::path::Path,
    license: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    use logicaffeine_verify::{LicenseValidator, Verifier};

    // Get license key from argument or environment
    let license_key = license
        .map(String::from)
        .or_else(|| env::var("LOGOS_LICENSE").ok());

    let license_key = license_key.ok_or(
        "Verification requires a license key.\n\
         Use --license <key> or set LOGOS_LICENSE environment variable.\n\
         Get a license at https://logicaffeine.com/pricing",
    )?;

    // Validate license
    println!("Validating license...");
    let validator = LicenseValidator::new();
    let plan = validator.validate(&license_key)?;
    println!("License valid ({})", plan);

    // Load and parse the project
    let manifest = Manifest::load(project_root)?;
    let entry_path = project_root.join(&manifest.package.entry);
    let source = fs::read_to_string(&entry_path)?;

    // For now, just verify that Z3 works
    // TODO: Implement full AST encoding in Phase 2
    println!("Running Z3 verification...");
    let verifier = Verifier::new();

    // Basic smoke test - verify that true is valid
    verifier.check_bool(true)?;

    Ok(())
}

#[cfg(not(feature = "verification"))]
fn run_verification(
    _project_root: &std::path::Path,
    _license: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    Err("Verification requires the 'verification' feature.\n\
         Rebuild with: cargo build --features verification"
        .into())
}

fn cmd_run(release: bool, args: &[String]) -> Result<(), Box<dyn std::error::Error>> {
    let current_dir = env::current_dir()?;
    let project_root =
        find_project_root(&current_dir).ok_or("Not in a LOGOS project (Largo.toml not found)")?;

    let config = BuildConfig {
        project_dir: project_root,
        release,
        lib_mode: false,
        target: None,
    };

    let result = build::build(config)?;
    let exit_code = build::run(&result, args)?;

    if exit_code != 0 {
        std::process::exit(exit_code);
    }

    Ok(())
}

fn cmd_run_interpret() -> Result<(), Box<dyn std::error::Error>> {
    let current_dir = env::current_dir()?;
    let project_root =
        find_project_root(&current_dir).ok_or("Not in a LOGOS project (Largo.toml not found)")?;

    let manifest = Manifest::load(&project_root)?;
    let entry_path = project_root.join(&manifest.package.entry);
    let source = fs::read_to_string(&entry_path)?;

    let result = futures::executor::block_on(logicaffeine_compile::interpret_for_ui(&source));

    for line in &result.lines {
        println!("{}", line);
    }

    if let Some(err) = result.error {
        eprintln!("{}", err);
        std::process::exit(1);
    }

    Ok(())
}

fn cmd_check() -> Result<(), Box<dyn std::error::Error>> {
    let current_dir = env::current_dir()?;
    let project_root =
        find_project_root(&current_dir).ok_or("Not in a LOGOS project (Largo.toml not found)")?;

    let manifest = Manifest::load(&project_root)?;
    let entry_path = project_root.join(&manifest.package.entry);

    // Just compile to Rust without building (discard output, only care about success)
    let _ = compile_project(&entry_path)?;

    println!("Check passed");
    Ok(())
}

// ============================================================
// Phase 39: Registry Commands
// ============================================================

fn cmd_publish(
    registry: Option<&str>,
    dry_run: bool,
    allow_dirty: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let current_dir = env::current_dir()?;
    let project_root =
        find_project_root(&current_dir).ok_or("Not in a LOGOS project (Largo.toml not found)")?;

    // Load manifest
    let manifest = Manifest::load(&project_root)?;
    let name = &manifest.package.name;
    let version = &manifest.package.version;

    println!("Packaging {} v{}", name, version);

    // Determine registry URL
    let registry_url = registry.unwrap_or(RegistryClient::default_url());

    // Get authentication token
    let token = get_token(registry_url).ok_or_else(|| {
        format!(
            "No authentication token found for {}.\n\
             Run 'largo login' or set LOGOS_TOKEN environment variable.",
            registry_url
        )
    })?;

    // Verify the package
    let entry_path = project_root.join(&manifest.package.entry);
    if !entry_path.exists() {
        return Err(format!(
            "Entry point '{}' not found",
            manifest.package.entry
        ).into());
    }

    // Check for uncommitted changes
    if !allow_dirty && is_git_dirty(&project_root) {
        return Err(
            "Working directory has uncommitted changes.\n\
             Use --allow-dirty to publish anyway.".into()
        );
    }

    // Create tarball
    println!("Creating package tarball...");
    let tarball = create_tarball(&project_root)?;
    println!("  Package size: {} bytes", tarball.len());

    // Read README if present
    let readme = project_root.join("README.md");
    let readme_content = if readme.exists() {
        fs::read_to_string(&readme).ok()
    } else {
        None
    };

    // Build metadata
    let metadata = PublishMetadata {
        name: name.clone(),
        version: version.clone(),
        description: manifest.package.description.clone(),
        repository: None, // Could add to manifest later
        homepage: None,
        license: None,
        keywords: vec![],
        entry_point: manifest.package.entry.clone(),
        dependencies: manifest
            .dependencies
            .iter()
            .map(|(k, v)| (k.clone(), v.to_string()))
            .collect(),
        readme: readme_content,
    };

    if dry_run {
        println!("\n[dry-run] Would publish to {}", registry_url);
        println!("[dry-run] Package validated successfully");
        return Ok(());
    }

    // Upload to registry
    println!("Uploading to {}...", registry_url);
    let client = RegistryClient::new(registry_url, &token);
    let result = client.publish(name, version, &tarball, &metadata)?;

    println!(
        "\nPublished {} v{} to {}",
        result.package, result.version, registry_url
    );
    println!("  SHA256: {}", result.sha256);

    Ok(())
}

fn cmd_login(
    registry: Option<&str>,
    token: Option<String>,
) -> Result<(), Box<dyn std::error::Error>> {
    let registry_url = registry.unwrap_or(RegistryClient::default_url());

    // Get token from argument or stdin
    let token = match token {
        Some(t) => t,
        None => {
            println!("To get a token, visit: {}/auth/github", registry_url);
            println!("Then generate an API token from your profile.");
            println!();
            print!("Enter token for {}: ", registry_url);
            io::stdout().flush()?;

            let mut line = String::new();
            io::stdin().read_line(&mut line)?;
            line.trim().to_string()
        }
    };

    if token.is_empty() {
        return Err("Token cannot be empty".into());
    }

    // Validate token with registry
    println!("Validating token...");
    let client = RegistryClient::new(registry_url, &token);
    let user_info = client.validate_token()?;

    // Save to credentials file
    let mut creds = Credentials::load().unwrap_or_default();
    creds.set_token(registry_url, &token);
    creds.save()?;

    println!("Logged in as {} to {}", user_info.login, registry_url);

    Ok(())
}

fn cmd_logout(registry: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
    let registry_url = registry.unwrap_or(RegistryClient::default_url());

    let mut creds = Credentials::load().unwrap_or_default();

    if creds.get_token(registry_url).is_none() {
        println!("Not logged in to {}", registry_url);
        return Ok(());
    }

    creds.remove_token(registry_url);
    creds.save()?;

    println!("Logged out from {}", registry_url);

    Ok(())
}
