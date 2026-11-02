//! Canonical formatting of a module's `Import` statements — the pit-of-success
//! default so import blocks never need manual bikeshedding.
//!
//! This is a separate pass from [`source_format`](crate::source_format), applied
//! by `largo fmt`, so the formatter's token-stream invariant is untouched:
//! reordering imports is semantics-preserving (import order never matters), but it
//! does change the token stream, so it can't live inside `format_source`.
//!
//! Only explicit `Import …` statement lines in the abstract are touched (literate
//! `[Alias](uri)` links are left as the author wrote them). The block is sorted,
//! de-duplicated, and collapsed to the position of the first import.

use std::collections::HashSet;

/// Rewrite the entry's abstract `Import` statements into a single canonical block:
/// sorted, de-duplicated, placed where the first import was, with the rest
/// removed. Prose and code are untouched. Idempotent.
pub fn normalize_imports(source: &str) -> String {
    let lines: Vec<&str> = source.lines().collect();

    // Collect the `Import …` statement lines that appear BEFORE the first code
    // block (`## `). Imports only ever live in the abstract.
    let mut import_indices: Vec<usize> = Vec::new();
    let mut imports: Vec<String> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let t = line.trim();
        if t.starts_with("## ") {
            break;
        }
        if is_import_statement(t) {
            import_indices.push(i);
            imports.push(t.to_string());
        }
    }

    // Nothing to canonicalize unless there are at least two (to sort/dedupe).
    if imports.len() < 2 {
        return source.to_string();
    }

    imports.sort();
    imports.dedup();

    let first = import_indices[0];
    let drop: HashSet<usize> = import_indices.iter().copied().collect();
    let mut out: Vec<String> = Vec::with_capacity(lines.len());
    for (i, line) in lines.iter().enumerate() {
        if i == first {
            out.extend(imports.iter().cloned());
        } else if drop.contains(&i) {
            continue;
        } else {
            out.push((*line).to_string());
        }
    }

    let mut result = out.join("\n");
    if source.ends_with('\n') {
        result.push('\n');
    }
    result
}

/// Whether a trimmed line is an `Import …` statement (not just prose starting with
/// the word "Import").
fn is_import_statement(trimmed: &str) -> bool {
    let Some(rest) = trimmed.strip_prefix("Import ") else {
        return false;
    };
    // A real import ends with a period and names a Capitalized module or a
    // `from`/`,`/`and` clause — reuse the scanner's own view of what parses.
    trimmed.ends_with('.')
        && !rest.trim().is_empty()
        && !crate::analysis::scan_dependencies(&format!("# _\n\n{trimmed}\n")).is_empty()
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_language/import_format/tests.rs"]
mod tests;
