//! `largo check` — parse and type-check without producing a binary.

use crate::commands::require_project_root;
use crate::compile::compile_project;
use crate::project::manifest::Manifest;
use crate::ui;

/// Handle `largo check`: compile to Rust in memory, discard the output.
/// With `--deep`, additionally run `cargo check` over the generated code and
/// translate rustc's findings back to LOGOS — the IDE flycheck's exact pass,
/// reproducible from the command line.
pub(crate) fn cmd_check(deep: bool) -> Result<(), Box<dyn std::error::Error>> {
    let project_root = require_project_root()?;

    let manifest = Manifest::load(&project_root)?;
    let entry_path = crate::commands::resolve_entry_path(&project_root, &manifest)?;

    // Just compile to Rust without building (discard output, only care about success)
    if let Err(e) = compile_project(&entry_path) {
        // Parse errors render with the caret excerpt + the Socratic
        // explanation — never the raw Debug struct.
        if let logicaffeine_compile::compile::CompileError::Parse(pe) = &e {
            let source = std::fs::read_to_string(&entry_path).unwrap_or_default();
            let interner = logicaffeine_language::Interner::new();
            // The socratic explanation leads; the caret excerpt follows
            // (dropping display_with_source's own `error:` first line — the
            // CLI renderer adds the prefix).
            let excerpt: String = pe
                .display_with_source(&source)
                .lines()
                .skip(1)
                .collect::<Vec<_>>()
                .join("\n");
            return Err(crate::ui::CliError::new(format!(
                "{}\n{excerpt}",
                logicaffeine_language::socratic_explanation(pe, &interner)
            ))
            .into());
        }
        return Err(e.into());
    }

    if deep {
        let source = std::fs::read_to_string(&entry_path)?;
        // Key the flycheck cache on the PROJECT PATH, not just the package
        // name: two projects both named `hello` (the common default) must
        // never share — or concurrently race on — one generated workspace.
        let root_hash = {
            use std::hash::{Hash, Hasher};
            let mut h = std::collections::hash_map::DefaultHasher::new();
            project_root.hash(&mut h);
            h.finish()
        };
        let cache_dir = std::env::temp_dir().join(format!(
            "logicaffeine-flycheck-{}-{root_hash:016x}",
            manifest.package.name
        ));
        let findings =
            logicaffeine_compile::compile::rustc_check(&source, &cache_dir).map_err(|e| {
                format!("deep check could not run: {e:?}")
            })?;
        if !findings.is_empty() {
            for finding in &findings {
                let mut message = format!("{}\n{}", finding.title, finding.explanation);
                if let Some(suggestion) = &finding.suggestion {
                    message.push('\n');
                    message.push_str(suggestion);
                }
                if let Some(span) = finding.logos_span {
                    // Clamp to a char boundary before slicing — a span edge
                    // inside a multibyte char must not panic the reporter.
                    let mut upto = span.start.min(source.len());
                    while upto > 0 && !source.is_char_boundary(upto) {
                        upto -= 1;
                    }
                    let line = source[..upto].bytes().filter(|&b| b == b'\n').count() + 1;
                    message.push_str(&format!("\n  --> {}:{line}", entry_path.display()));
                }
                eprintln!("{message}\n");
            }
            return Err(format!("{} deep-check finding(s)", findings.len()).into());
        }
        ui::info("Deep check passed (rustc agrees)");
        return Ok(());
    }

    ui::info("Check passed");
    Ok(())
}
