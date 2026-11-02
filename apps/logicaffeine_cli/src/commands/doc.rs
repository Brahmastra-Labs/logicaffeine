//! `largo doc` — generate project documentation from `##` blocks.
//!
//! Literate LOGOS sources already carry their structure in markdown block
//! headers (`## To …`, `## A … has`, `## Note`, `## Example`, …), so the
//! extractor is a pure line scan over the source — robust even when a body
//! doesn't parse. `## Main` (the program) and `## Requires` (build
//! plumbing) are omitted from documentation.

use std::fs;
use std::path::PathBuf;

use crate::commands::require_project_root;
use crate::project::manifest::Manifest;
use crate::ui::{self, CliError};

/// What kind of documentation a block contributes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BlockKind {
    /// `## To …` — a function; the header line is the signature.
    Function,
    /// `## A … has` / `## An … has` — a type definition.
    TypeDef,
    /// `## Note` — prose, rendered verbatim.
    Note,
    /// `## Example` — code, rendered fenced.
    Example,
    /// `## Definition` / `## Define` — a definition.
    Definition,
    /// `## Axiom` / `## Theory` / `## Theorem` / `## Proof` — formal content.
    Formal,
    /// Anything else documentable (`## Policy`, `## Hardware`, …).
    Other,
    /// Omitted from docs (`## Main`, `## Requires`).
    Skip,
}

/// One extracted block: its header line (without `## `), kind, and body.
#[derive(Debug)]
struct Block {
    kind: BlockKind,
    header: String,
    body: String,
}

/// Classify a block header line (the text after `## `).
fn classify(header: &str) -> BlockKind {
    let first = header.split_whitespace().next().unwrap_or("");
    match first {
        "To" => BlockKind::Function,
        "A" | "An" => BlockKind::TypeDef,
        "Note" => BlockKind::Note,
        "Example" => BlockKind::Example,
        "Definition" | "Define" => BlockKind::Definition,
        "Axiom" | "Theory" | "Theorem" | "Proof" => BlockKind::Formal,
        "Main" | "Requires" => BlockKind::Skip,
        _ => BlockKind::Other,
    }
}

/// Extract all `## ` blocks from a literate source, in order. Returns the
/// leading prose (before the first block) and the blocks. Lines inside
/// fenced code blocks (``` … ```) are always content, never headers.
fn extract_blocks(source: &str) -> (String, Vec<Block>) {
    let mut preamble = String::new();
    let mut blocks: Vec<Block> = Vec::new();
    let mut in_fence = false;

    for line in source.lines() {
        if line.trim_start().starts_with("```") {
            in_fence = !in_fence;
        }
        if let Some(header) = line.strip_prefix("## ").filter(|_| !in_fence) {
            blocks.push(Block {
                kind: classify(header),
                header: header.trim_end_matches(':').trim().to_string(),
                body: String::new(),
            });
        } else if let Some(block) = blocks.last_mut() {
            block.body.push_str(line);
            block.body.push('\n');
        } else if !line.starts_with("# ") && !(is_import_declaration(line) && !in_fence) {
            preamble.push_str(line);
            preamble.push('\n');
        }
    }

    (preamble.trim().to_string(), blocks)
}

/// Render the extracted blocks as a markdown document.
fn render_markdown(name: &str, description: Option<&str>, source: &str) -> String {
    let (preamble, blocks) = extract_blocks(source);

    let mut md = format!("# {name}\n\n");
    if let Some(desc) = description {
        md.push_str(desc);
        md.push_str("\n\n");
    }
    if !preamble.is_empty() {
        md.push_str(&preamble);
        md.push_str("\n\n");
    }

    for block in &blocks {
        let body = block.body.trim_matches('\n');
        match block.kind {
            BlockKind::Skip => {}
            BlockKind::Note => {
                md.push_str(&format!("{}\n\n", dedent(body)));
            }
            BlockKind::Function | BlockKind::TypeDef => {
                md.push_str(&format!("### {}\n\n", block.header));
                if !body.is_empty() {
                    md.push_str(&format!("```logos\n{}\n```\n\n", dedent(body)));
                }
            }
            BlockKind::Example => {
                md.push_str("### Example\n\n");
                md.push_str(&format!("```logos\n{}\n```\n\n", dedent(body)));
            }
            BlockKind::Definition | BlockKind::Formal | BlockKind::Other => {
                md.push_str(&format!("### {}\n\n", block.header));
                if !body.is_empty() {
                    md.push_str(&format!("{}\n\n", dedent(body)));
                }
            }
        }
    }

    md
}

/// An `Import …` declaration line — build plumbing (the module is already spliced
/// into the assembled unit, so the directive is vestigial), never documentation
/// prose. Omitted from the doc preamble so a project's imports don't render as body
/// text. Mirrors how `## Main` / `## Requires` blocks are skipped.
fn is_import_declaration(line: &str) -> bool {
    let t = line.trim_start();
    t.starts_with("Import ") || t.starts_with("import ")
}

/// Strip the common 4-space indent literate bodies carry.
fn dedent(body: &str) -> String {
    body.lines()
        .map(|l| l.strip_prefix("    ").unwrap_or(l))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Handle `largo doc [--out DIR]`.
pub(crate) fn cmd_doc(out: Option<PathBuf>) -> Result<(), Box<dyn std::error::Error>> {
    let root = require_project_root()?;
    let manifest = Manifest::load(&root)?;
    let entry = crate::commands::resolve_entry_path(&root, &manifest)?;
    // Document the ASSEMBLED module closure — the same source every other command
    // consumes — so an IMPORTED module's definitions appear in the docs, not just
    // the entry file's. Byte-identical to the raw entry for a single-file project.
    let dep_roots = crate::project::deps::resolve_dep_roots(&root, &manifest)?;
    let source = crate::closure::assemble_closure_with_deps(&entry, dep_roots)
        .map_err(|e| CliError::new(format!("could not assemble project: {e}")))?
        .source;

    let md = render_markdown(
        &manifest.package.name,
        manifest.package.description.as_deref(),
        &source,
    );

    let out_dir = match out {
        Some(dir) if dir.is_absolute() => dir,
        Some(dir) => root.join(dir),
        None => root.join("target/doc"),
    };
    if out_dir.is_file() {
        return Err(CliError::with_hint(
            format!("{} is a file", out_dir.display()),
            "`--out` takes a directory to write the documentation into",
        )
        .into());
    }
    fs::create_dir_all(&out_dir)
        .map_err(|e| CliError::new(format!("cannot create {}: {e}", out_dir.display())))?;
    let out_path = out_dir.join(format!("{}.md", manifest.package.name));
    fs::write(&out_path, md)?;
    ui::info(format!(
        "Documented {} -> {}",
        manifest.package.name,
        out_path.display()
    ));
    Ok(())
}

#[cfg(test)]
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_cli/commands/doc/tests.rs"]
mod tests;
