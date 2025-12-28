//! Phase 37/39: LOGOS CLI (largo)
//!
//! Command-line interface for the LOGOS build system and package registry.

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

#[derive(Parser)]
#[command(name = "largo")]
#[command(about = "The LOGOS build tool", long_about = None)]
#[command(version)]
pub struct Cli {
    #[command(subcommand)]
    pub command: Commands,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Create a new LOGOS project
    New {
        /// Project name
        name: String,
    },
    /// Initialize a LOGOS project in the current directory
    Init {
        /// Project name (defaults to directory name)
        #[arg(long)]
        name: Option<String>,
    },
    /// Build the current project
    Build {
        /// Build in release mode
        #[arg(long, short)]
        release: bool,
    },
    /// Build and run the current project
    Run {
        /// Build in release mode
        #[arg(long, short)]
        release: bool,
    },
    /// Check the project for errors without building
    Check,

    // Phase 39: Package Registry Commands
    /// Publish the package to the registry
    Publish {
        /// Registry URL (defaults to registry.logicaffeine.com)
        #[arg(long)]
        registry: Option<String>,

        /// Perform all checks without actually publishing
        #[arg(long)]
        dry_run: bool,

        /// Allow publishing with uncommitted changes
        #[arg(long)]
        allow_dirty: bool,
    },
    /// Log in to the package registry
    Login {
        /// Registry URL
        #[arg(long)]
        registry: Option<String>,

        /// Token to store (reads from stdin if not provided)
        #[arg(long)]
        token: Option<String>,
    },
    /// Log out from the package registry
    Logout {
        /// Registry URL
        #[arg(long)]
        registry: Option<String>,
    },
}

/// Entry point for the CLI
pub fn run_cli() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    match cli.command {
        Commands::New { name } => cmd_new(&name),
        Commands::Init { name } => cmd_init(name.as_deref()),
        Commands::Build { release } => cmd_build(release),
        Commands::Run { release } => cmd_run(release),
        Commands::Check => cmd_check(),
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

fn cmd_build(release: bool) -> Result<(), Box<dyn std::error::Error>> {
    let current_dir = env::current_dir()?;
    let project_root =
        find_project_root(&current_dir).ok_or("Not in a LOGOS project (Largo.toml not found)")?;

    let config = BuildConfig {
        project_dir: project_root,
        release,
    };

    let result = build::build(config)?;

    let mode = if release { "release" } else { "debug" };
    println!("Built {} [{}]", result.binary_path.display(), mode);

    Ok(())
}

fn cmd_run(release: bool) -> Result<(), Box<dyn std::error::Error>> {
    let current_dir = env::current_dir()?;
    let project_root =
        find_project_root(&current_dir).ok_or("Not in a LOGOS project (Largo.toml not found)")?;

    let config = BuildConfig {
        project_dir: project_root,
        release,
    };

    let result = build::build(config)?;
    let exit_code = build::run(&result)?;

    if exit_code != 0 {
        std::process::exit(exit_code);
    }

    Ok(())
}

fn cmd_check() -> Result<(), Box<dyn std::error::Error>> {
    let current_dir = env::current_dir()?;
    let project_root =
        find_project_root(&current_dir).ok_or("Not in a LOGOS project (Largo.toml not found)")?;

    let manifest = Manifest::load(&project_root)?;
    let entry_path = project_root.join(&manifest.package.entry);

    // Just compile to Rust without building
    compile_project(&entry_path)?;

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
