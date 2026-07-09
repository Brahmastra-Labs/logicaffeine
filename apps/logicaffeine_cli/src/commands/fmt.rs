//! `largo fmt` — format LOGOS sources with the canonical rules.
//!
//! The rules live in [`logicaffeine_language::source_format`] — the exact
//! formatter behind the LSP's format-on-save — so every surface agrees.

use std::fs;
use std::path::{Path, PathBuf};

use logicaffeine_language::source_format::format_source;

use crate::commands::require_project_root;
use crate::project::manifest::Manifest;
use crate::ui::{self, CliError};

/// Handle `largo fmt [PATHS…] [--check]`.
///
/// Without paths, formats every `.lg`/`.md` source under the project's
/// `src/` plus the manifest entry. With paths, formats exactly those files.
/// `--check` writes nothing: it lists the files that would change and exits
/// 1 if there are any (the CI mode).
pub(crate) fn cmd_fmt(paths: Vec<PathBuf>, check: bool) -> Result<(), Box<dyn std::error::Error>> {
    let files = if paths.is_empty() {
        project_source_files()?
    } else {
        for p in &paths {
            if !p.exists() {
                return Err(CliError::new(format!("no such file: {}", p.display())).into());
            }
        }
        paths
    };

    let mut dirty = Vec::new();
    for file in &files {
        let source = fs::read_to_string(file)
            .map_err(|e| CliError::new(format!("cannot read {}: {e}", file.display())))?;
        let formatted = format_source(&source);
        if formatted != source {
            if !check {
                fs::write(file, &formatted)
                    .map_err(|e| CliError::new(format!("cannot write {}: {e}", file.display())))?;
            }
            dirty.push(file.clone());
        }
    }

    if check {
        if dirty.is_empty() {
            ui::info("All files formatted");
            return Ok(());
        }
        for file in &dirty {
            println!("{}", file.display());
        }
        return Err(CliError::with_hint(
            format!("{} file(s) need formatting", dirty.len()),
            "run `largo fmt` to fix them",
        )
        .into());
    }

    ui::info(format!("Formatted {} file(s)", dirty.len()));
    Ok(())
}

/// All formattable sources of the enclosing project: `.lg`/`.md` files under
/// `src/` (recursively) plus the manifest entry, deduplicated and sorted.
fn project_source_files() -> Result<Vec<PathBuf>, Box<dyn std::error::Error>> {
    let root = require_project_root()?;
    let mut files = Vec::new();
    collect_sources(&root.join("src"), &mut files)?;

    let manifest = Manifest::load(&root)?;
    let entry = root.join(&manifest.package.entry);
    if entry.exists() && !files.contains(&entry) {
        files.push(entry);
    }
    files.sort();
    files.dedup();
    Ok(files)
}

/// Recursively collect `.lg` and `.md` files under `dir`.
fn collect_sources(dir: &Path, files: &mut Vec<PathBuf>) -> Result<(), std::io::Error> {
    if !dir.exists() {
        return Ok(());
    }
    for entry in fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_sources(&path, files)?;
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("lg") | Some("md")
        ) {
            files.push(path);
        }
    }
    Ok(())
}
