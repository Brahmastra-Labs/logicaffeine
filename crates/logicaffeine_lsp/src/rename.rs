use lsp_types::{Position, Range, TextEdit, Url, WorkspaceEdit};
use std::collections::HashMap;

use crate::document::DocumentState;

/// Validate a proposed new name for a rename operation.
/// Returns an error message if invalid, None if valid.
pub fn validate_new_name(new_name: &str) -> Option<String> {
    if new_name.is_empty() {
        return Some("Name cannot be empty".to_string());
    }
    if new_name.chars().any(|c| c.is_whitespace()) {
        return Some("Name cannot contain whitespace".to_string());
    }
    let first = new_name.chars().next().unwrap();
    if !first.is_alphabetic() && first != '_' {
        return Some("Name must start with a letter or underscore".to_string());
    }
    let reserved = [
        "Let",
        "Set",
        "If",
        "While",
        "Repeat",
        "Return",
        "Show",
        "Give",
        "Push",
        "Pop",
        "Call",
        "Inspect",
        "Check",
        "Assert",
        "Trust",
        "Escape",
        "Be",
        "New",
        "Otherwise",
        "Else",
    ];
    if reserved.contains(&new_name) {
        return Some(format!("'{}' is a reserved keyword", new_name));
    }
    None
}

/// Handle rename request.
///
/// Finds all definitions and references with the given name and returns
/// a workspace edit that renames them all.
pub fn rename(
    doc: &DocumentState,
    position: Position,
    new_name: String,
    uri: &Url,
) -> Option<WorkspaceEdit> {
    if validate_new_name(&new_name).is_some() {
        return None;
    }

    let offset = doc.line_index.offset(position);

    // ★ THROUGH THE SHARED RULE, so a name written inside a `View:` renames like any other. The
    // token under the cursor there is the whole `RawBlock`, which is not a name anything defines.
    let (old_name, _) = crate::document::name_at(doc, offset)?;
    let old_name = old_name.as_str();

    let mut edits = Vec::new();

    // Rename definitions
    for def in doc.symbol_index.definitions_of(old_name) {
        if def.span != logicaffeine_language::token::Span::default() {
            edits.push(TextEdit {
                range: Range {
                    start: doc.line_index.position(def.span.start),
                    end: doc.line_index.position(def.span.end),
                },
                new_text: new_name.clone(),
            });
        }
    }

    // Rename references
    for reference in doc.symbol_index.references_to(old_name) {
        edits.push(TextEdit {
            range: Range {
                start: doc.line_index.position(reference.span.start),
                end: doc.line_index.position(reference.span.end),
            },
            new_text: new_name.clone(),
        });
    }

    if edits.is_empty() {
        return None;
    }

    let mut changes = HashMap::new();
    changes.insert(uri.clone(), edits);

    Some(WorkspaceEdit {
        changes: Some(changes),
        ..Default::default()
    })
}

/// Handle prepare rename request.
///
/// Returns the range and placeholder text for the rename.
pub fn prepare_rename(doc: &DocumentState, position: Position) -> Option<(Range, String)> {
    let offset = doc.line_index.offset(position);

    // ★ A NAME RECORDED FROM INSIDE A TEMPLATE IS ALREADY KNOWN TO BE A NAME — the parser put it
    // there because the template MENTIONS it. So the token-kind gate below, which exists to stop
    // an author renaming a period, is asked only of a name that came from a token in the first
    // place; asking it of a `RawBlock` would refuse every rename inside a `View:`.
    if let Some(reference) = doc
        .symbol_index
        .references
        .iter()
        .find(|r| offset >= r.span.start && offset < r.span.end)
    {
        return Some((
            Range {
                start: doc.line_index.position(reference.span.start),
                end: doc.line_index.position(reference.span.end),
            },
            reference.name.clone(),
        ));
    }

    let token = doc
        .tokens
        .iter()
        .find(|t| offset >= t.span.start && offset < t.span.end)?;

    let text = doc.source.get(token.span.start..token.span.end)?;

    // Only allow renaming name-bearing tokens
    match &token.kind {
        logicaffeine_language::token::TokenType::Identifier
        | logicaffeine_language::token::TokenType::ProperName(_)
        | logicaffeine_language::token::TokenType::Noun(_)
        | logicaffeine_language::token::TokenType::Adjective(_)
        | logicaffeine_language::token::TokenType::Verb { .. } => {}
        _ => return None,
    }

    Some((
        Range {
            start: doc.line_index.position(token.span.start),
            end: doc.line_index.position(token.span.end),
        },
        text.to_string(),
    ))
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_lsp/rename/tests.rs"]
mod tests;
