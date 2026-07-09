//! Command handlers for every `largo` subcommand.
//!
//! One module per verb (or tightly-coupled verb family). [`crate::cli::run_cli`]
//! parses arguments and dispatches into these modules; each handler owns the
//! full behavior of its command and returns `Result<(), Box<dyn Error>>`.

pub(crate) mod build;
pub(crate) mod check;
pub(crate) mod clean;
pub(crate) mod completions;
pub(crate) mod deps;
pub(crate) mod doc;
pub(crate) mod doctor;
pub(crate) mod emit;
pub(crate) mod fmt;
pub(crate) mod logic;
pub(crate) mod new;
pub(crate) mod opts;
pub(crate) mod prove;
pub(crate) mod publish;
pub(crate) mod run;
pub(crate) mod sat;
pub(crate) mod verify;

use crate::ui::CliError;

/// Resolve the enclosing LOGOS project root (the directory holding
/// `Largo.toml`), from the current working directory upward.
///
/// The error carries the standard "not in a project" message and the
/// `largo new` hint every project-scoped command shares.
pub(crate) fn require_project_root() -> Result<std::path::PathBuf, CliError> {
    let current_dir = std::env::current_dir()
        .map_err(|e| CliError::new(format!("cannot determine the current directory: {e}")))?;
    crate::project::build::find_project_root(&current_dir).ok_or_else(|| {
        CliError::with_hint(
            "not in a LOGOS project (no Largo.toml found here or in any parent directory)",
            "run this inside a project, or create one with `largo new <name>`",
        )
    })
}

/// Resolve a project's entry file with the same `.md` fallback the cargo
/// build path applies: the manifest's `entry`, else the sibling `.md`.
/// Errors name both candidates instead of a bare file-not-found.
///
/// The entry is confined to the project: absolute paths and `..` escapes
/// are rejected, so a distributed manifest can never point commands at
/// arbitrary files on the machine.
pub(crate) fn resolve_entry_path(
    project_root: &std::path::Path,
    manifest: &crate::project::manifest::Manifest,
) -> Result<std::path::PathBuf, CliError> {
    let declared = std::path::Path::new(&manifest.package.entry);
    if declared.is_absolute()
        || declared
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err(CliError::with_hint(
            format!(
                "the manifest entry {:?} escapes the project directory",
                manifest.package.entry
            ),
            "`entry` must be a relative path inside the project (like `src/main.lg`)",
        ));
    }
    let entry = project_root.join(declared);
    if entry.exists() {
        return Ok(entry);
    }
    let md = entry.with_extension("md");
    if md.exists() {
        return Ok(md);
    }
    Err(CliError::with_hint(
        format!("entry point not found: {} (also tried {})", entry.display(), md.display()),
        "check the `entry` field in Largo.toml",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::manifest::Manifest;

    fn manifest_with_entry(entry: &str) -> Manifest {
        let mut m = Manifest::new("probe");
        m.package.entry = entry.to_string();
        m
    }

    #[test]
    fn entry_escaping_the_project_is_rejected() {
        let dir = tempfile::tempdir().unwrap();
        for bad in ["../outside.lg", "src/../../x.lg", "/etc/hostname"] {
            let err = resolve_entry_path(dir.path(), &manifest_with_entry(bad))
                .expect_err(&format!("{bad} must be rejected"));
            assert!(err.message.contains("escapes"), "{bad}: {}", err.message);
        }
    }

    #[test]
    fn relative_entry_resolves_with_md_fallback() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::write(dir.path().join("src/main.md"), "## Main\nShow 1.\n").unwrap();
        let path = resolve_entry_path(dir.path(), &manifest_with_entry("src/main.lg"))
            .expect("md fallback");
        assert!(path.ends_with("src/main.md"));
    }
}
