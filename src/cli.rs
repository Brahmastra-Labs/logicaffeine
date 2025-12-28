//! Phase 37: LOGOS CLI (largo)
//!
//! Command-line interface for the LOGOS build system.

use clap::{Parser, Subcommand};
use std::env;
use std::fs;
use std::path::PathBuf;

use crate::compile::compile_project;
use crate::project::build::{self, find_project_root, BuildConfig};
use crate::project::manifest::Manifest;

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
