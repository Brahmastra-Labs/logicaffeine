use lsp_types::{Location, Position, Range, Url};

use crate::document::DocumentState;

/// The name under the cursor, exactly as the reference machinery reads it.
pub fn name_at(doc: &DocumentState, position: Position) -> Option<String> {
    let offset = doc.line_index.offset(position);
    let token = doc
        .tokens
        .iter()
        .find(|t| offset >= t.span.start && offset < t.span.end)?;
    doc.source
        .get(token.span.start..token.span.end)
        .map(str::to_string)
}

/// The occurrences of `name` in ANOTHER open document that belong to a
/// cross-file symbol: unresolved locally (they reach across files), or any
/// occurrence when this document itself defines the symbol. Definition spans
/// ride along when `include_defs` (rename edits the definition too).
pub fn cross_file_candidates(doc: &DocumentState, name: &str, include_defs: bool) -> Vec<Range> {
    let local_defs = doc.symbol_index.definitions_of(name);
    let doc_defines = local_defs.iter().any(|d| {
        matches!(
            d.kind,
            crate::index::DefinitionKind::Function
                | crate::index::DefinitionKind::Struct
                | crate::index::DefinitionKind::Enum
                | crate::index::DefinitionKind::Variant
                | crate::index::DefinitionKind::Theorem
        )
    });

    let mut ranges = Vec::new();
    if include_defs && doc_defines {
        for def in &local_defs {
            if def.span != logicaffeine_language::token::Span::default() {
                ranges.push(Range {
                    start: doc.line_index.position(def.span.start),
                    end: doc.line_index.position(def.span.end),
                });
            }
        }
    }
    for reference in &doc.symbol_index.references {
        if reference.name != name {
            continue;
        }
        if reference.definition_idx.is_none() || doc_defines {
            ranges.push(Range {
                start: doc.line_index.position(reference.span.start),
                end: doc.line_index.position(reference.span.end),
            });
        }
    }
    ranges
}

/// Is `name` a symbol other files can see from THIS document's point of
/// view? True when the document doesn't define it (the usage reaches across
/// files) or defines it as API shape. A local `Let x` is nobody's business.
pub fn is_cross_file_symbol(doc: &DocumentState, name: &str) -> bool {
    let defs = doc.symbol_index.definitions_of(name);
    if defs.is_empty() {
        return true;
    }
    defs.iter().any(|d| {
        matches!(
            d.kind,
            crate::index::DefinitionKind::Function
                | crate::index::DefinitionKind::Struct
                | crate::index::DefinitionKind::Enum
                | crate::index::DefinitionKind::Variant
                | crate::index::DefinitionKind::Theorem
        )
    })
}

/// Handle find-all-references request.
pub fn find_references(
    doc: &DocumentState,
    position: Position,
    uri: &Url,
    include_declaration: bool,
) -> Vec<Location> {
    let offset = doc.line_index.offset(position);

    // Find the token at the cursor position
    // ★ THROUGH THE SHARED RULE. "Find all references" asked from inside a `View:` read the whole
    // `RawBlock` as the name and answered with nothing — the same failure hover, rename and
    // go-to-definition each had, for the same reason, in four separate functions.
    let name = match crate::document::name_at(doc, offset) {
        Some((n, _)) => n,
        None => return vec![],
    };

    let mut locations = Vec::new();

    // Include definition locations if requested
    if include_declaration {
        for def in doc.symbol_index.definitions_of(&name) {
            if def.span != logicaffeine_language::token::Span::default() {
                locations.push(Location {
                    uri: uri.clone(),
                    range: Range {
                        start: doc.line_index.position(def.span.start),
                        end: doc.line_index.position(def.span.end),
                    },
                });
            }
        }
    }

    // Include all references
    for reference in doc.symbol_index.references_to(&name) {
        locations.push(Location {
            uri: uri.clone(),
            range: Range {
                start: doc.line_index.position(reference.span.start),
                end: doc.line_index.position(reference.span.end),
            },
        });
    }

    locations
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_lsp/references/tests.rs"]
mod tests;
