//! Command handlers for every `largo` subcommand.
//!
//! One module per verb (or tightly-coupled verb family). [`crate::cli::run_cli`]
//! parses arguments and dispatches into these modules; each handler owns the
//! full behavior of its command and returns `Result<(), Box<dyn Error>>`.

pub(crate) mod bringup;
pub(crate) mod build;
pub(crate) mod check;
pub(crate) mod clean;
pub(crate) mod completions;
pub(crate) mod deps;
pub(crate) mod doc;
pub(crate) mod doctor;
pub(crate) mod emit;
pub(crate) mod expand;
pub(crate) mod explain;
pub(crate) mod fmt;
pub(crate) mod kernel_document;
pub(crate) mod lean;
pub(crate) mod logic;
pub(crate) mod new;
pub(crate) mod opts;
pub(crate) mod prove;
pub(crate) mod publish;
pub(crate) mod replay;
pub(crate) mod run;
pub(crate) mod sat;
pub(crate) mod serve;
pub(crate) mod serve_dom;
pub(crate) mod serve_web;
pub(crate) mod test;
pub(crate) mod verify;
pub(crate) mod web_bundle;

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

/// Resolve a project's entry program: the manifest's `entry`, else its `.md` sibling — for the
/// commands that are about a program. A site that has none is told so by name, with what to run
/// instead, rather than handed a missing-file error about a file it was never meant to have.
///
/// The entry is confined to the project: absolute paths and `..` escapes are rejected, so a
/// distributed manifest can never point commands at arbitrary files on the machine.
pub(crate) fn resolve_entry_path(
    project_root: &std::path::Path,
    manifest: &crate::project::manifest::Manifest,
) -> Result<std::path::PathBuf, CliError> {
    crate::project::shape::entry_file(project_root, manifest)?
        .ok_or_else(|| crate::project::shape::site_without_entry(project_root))
}

#[cfg(test)]
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_cli/commands/tests.rs"]
mod tests;
