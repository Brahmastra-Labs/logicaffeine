//! `largo new` / `largo init` — project scaffolding.

use std::env;
use std::fs;
use std::path::PathBuf;

use crate::project::manifest::Manifest;
use crate::ui::CliError;

/// Validate a project/package name: ASCII alphanumerics, `-` and `_`,
/// starting with a letter. Everything else breaks a downstream layer —
/// path separators split the directory, quotes corrupt the generated wasm
/// host shim, leading dashes are shell footguns.
fn validate_project_name(name: &str) -> Result<(), CliError> {
    let ok = !name.is_empty()
        && name.chars().next().is_some_and(|c| c.is_ascii_alphabetic())
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(CliError::with_hint(
            format!("invalid project name {name:?}"),
            "a name starts with a letter and uses only letters, digits, `-` and `_` (like a crate name)",
        ))
    }
}

/// Handle `largo new <name>`: scaffold a fresh project directory.
pub(crate) fn cmd_new(name: &str) -> Result<(), Box<dyn std::error::Error>> {
    validate_project_name(name)?;
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

/// Handle `largo init [--name]`: scaffold in the current directory.
pub(crate) fn cmd_init(name: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
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
    validate_project_name(&project_name)?;

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
