//! `largo prove --kernel <file>` / `largo check --kernel <file>` — a LOGOS kernel-vernacular
//! literate document (prose, literate English forms, `Definition`/`Inductive`/`Theorem`/… commands),
//! checked by LOGOS's own document door, `Repl::execute_document`: every block, in order, fail-early,
//! balanced. This is how an emitted `<Target>.lg` is checked like any other LOGOS proof.

use std::path::Path;

use logicaffeine_kernel::interface::Repl;
use serde_json::json;

use crate::ui::CliError;

/// What checking one document established.
pub(crate) struct DocumentReport {
    /// Each block's output, in order, when the whole document checked.
    pub(crate) blocks: Vec<String>,
    /// The first refused block (1-based) and why, when it did not.
    pub(crate) refusal: Option<(Option<usize>, String)>,
}

/// Check the document at `path` in a fresh standard-library context.
pub(crate) fn check_document(path: &Path) -> Result<DocumentReport, CliError> {
    let source = std::fs::read_to_string(path)
        .map_err(|e| CliError::new(format!("cannot read {}: {e}", path.display())))?;
    match Repl::new().execute_document(&source) {
        Ok(blocks) => Ok(DocumentReport {
            blocks,
            refusal: None,
        }),
        Err(error) => Ok(DocumentReport {
            blocks: Vec::new(),
            refusal: Some((failing_block(&source), error.to_string())),
        }),
    }
}

/// The first block that does not check. The document door stops at it without numbering it; the
/// batch door runs the same blocks the same way, block by block, so its first refusal is the same
/// block. A document refused only as a whole (an unbalanced scope) has no failing block.
fn failing_block(source: &str) -> Option<usize> {
    let blocks = document_blocks(source)?;
    Repl::new()
        .execute_batch(&blocks)
        .iter()
        .position(Result::is_err)
        .map(|i| i + 1)
}

/// The document's blocks, split exactly as the document door splits them: prefix by prefix, in
/// the context the door starts from.
fn document_blocks(source: &str) -> Option<Vec<String>> {
    use logicaffeine_kernel::interface::{parse_command_prefix_in_ctx, Surface};
    let repl = Repl::new();
    let surface = Surface::new();
    let mut blocks = Vec::new();
    let mut offset = 0;
    while !source[offset..].trim().is_empty() {
        let (_, consumed) =
            parse_command_prefix_in_ctx(repl.context(), &surface, &source[offset..]).ok()?;
        if consumed == 0 {
            return None;
        }
        let text = source[offset..offset + consumed].trim();
        if !text.is_empty() {
            blocks.push(text.to_string());
        }
        offset += consumed;
    }
    Some(blocks)
}

/// The shared command body: print the report (human or JSON) and fail when a block was refused.
pub(crate) fn run(
    path: &Path,
    json_output: bool,
    verb: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let report = check_document(path)?;
    if json_output {
        println!(
            "{}",
            json!({
                "file": path.display().to_string(),
                "surface": "kernel_document",
                "checked": report.refusal.is_none(),
                "blocks": report.blocks.iter().enumerate()
                    .map(|(i, output)| json!({"block": i + 1, "output": output}))
                    .collect::<Vec<_>>(),
                "failed_block": report.refusal.as_ref().and_then(|(block, _)| *block),
                "error": report.refusal.as_ref().map(|(_, message)| message),
            })
        );
    }
    match report.refusal {
        None => {
            if !json_output {
                anstream::println!(
                    "{verb} {} (LOGOS kernel document): {} block(s) ✓",
                    path.display(),
                    report.blocks.len()
                );
            }
            Ok(())
        }
        Some((block, message)) => {
            let at = block.map(|b| format!("block {b} ")).unwrap_or_default();
            Err(CliError::new(format!("{}: {at}does not check: {message}", path.display())).into())
        }
    }
}
