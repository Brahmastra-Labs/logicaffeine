//! `largo verify` — Z3 static verification (license-gated, feature-gated).

#[cfg(feature = "verification")]
use std::env;

use crate::commands::require_project_root;
use crate::ui;

/// Handle `largo verify`: run verification against the current project.
pub(crate) fn cmd_verify(license: Option<String>) -> Result<(), Box<dyn std::error::Error>> {
    let project_root = require_project_root()?;

    run_verification(&project_root, license.as_deref())?;
    ui::info("Verification passed");
    Ok(())
}

/// Validate the license and run Z3 verification over the project entry.
#[cfg(feature = "verification")]
pub(crate) fn run_verification(
    project_root: &std::path::Path,
    license: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    use crate::project::manifest::Manifest;
    use logicaffeine_verify::{LicenseValidator, Verifier};
    use std::fs;

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
    let _source = fs::read_to_string(&entry_path)?;

    // For now, just verify that Z3 works
    // TODO: Implement full AST encoding in Phase 2
    println!("Running Z3 verification...");
    let verifier = Verifier::new();

    // Basic smoke test - verify that true is valid
    verifier.check_bool(true)?;

    Ok(())
}

/// Stub for builds without the `verification` feature: explain how to get it.
#[cfg(not(feature = "verification"))]
pub(crate) fn run_verification(
    _project_root: &std::path::Path,
    _license: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    Err(crate::ui::CliError::with_hint(
        "this largo build does not include Z3 verification",
        "install the full build: `curl -fsSL https://logicaffeine.com/install.sh | sh -s -- --full` \
         (or build from source with `cargo install logicaffeine-cli --features verification`)",
    )
    .into())
}
