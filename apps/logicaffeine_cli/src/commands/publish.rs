//! `largo publish` / `largo login` / `largo logout` — package registry commands.

use std::fs;
use std::io::{self, Write};

use crate::commands::require_project_root;
use crate::project::credentials::{get_token, Credentials};
use crate::project::manifest::Manifest;
use crate::project::registry::{
    create_tarball, is_git_dirty, PublishMetadata, RegistryClient,
};

/// Handle `largo publish`: tarball the project and upload it to the registry.
pub(crate) fn cmd_publish(
    registry: Option<&str>,
    dry_run: bool,
    allow_dirty: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let project_root = require_project_root()?;

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

/// Handle `largo login`: validate a token and store it in the credentials file.
pub(crate) fn cmd_login(
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

/// Handle `largo logout`: remove the stored token for a registry.
pub(crate) fn cmd_logout(registry: Option<&str>) -> Result<(), Box<dyn std::error::Error>> {
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
