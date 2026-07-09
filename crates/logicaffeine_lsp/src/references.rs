use tower_lsp::lsp_types::{Location, Position, Range, Url};

use crate::document::DocumentState;

/// The name under the cursor, exactly as the reference machinery reads it.
pub fn name_at(doc: &DocumentState, position: Position) -> Option<String> {
    let offset = doc.line_index.offset(position);
    let token = doc
        .tokens
        .iter()
        .find(|t| offset >= t.span.start && offset < t.span.end)?;
    doc.source.get(token.span.start..token.span.end).map(str::to_string)
}

/// The occurrences of `name` in ANOTHER open document that belong to a
/// cross-file symbol: unresolved locally (they reach across files), or any
/// occurrence when this document itself defines the symbol. Definition spans
/// ride along when `include_defs` (rename edits the definition too).
pub fn cross_file_candidates(
    doc: &DocumentState,
    name: &str,
    include_defs: bool,
) -> Vec<Range> {
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
    let token = match doc.tokens.iter().find(|t| {
        offset >= t.span.start && offset < t.span.end
    }) {
        Some(t) => t,
        None => return vec![],
    };

    let name = match doc.source.get(token.span.start..token.span.end) {
        Some(n) => n.to_string(),
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
mod tests {
    use super::*;
    use crate::document::DocumentState;

    fn make_doc(source: &str) -> DocumentState {
        DocumentState::new(source.to_string(), 1)
    }

    fn test_uri() -> Url {
        Url::parse("file:///test.logos").unwrap()
    }

    #[test]
    fn find_references_includes_usages() {
        let doc = make_doc("## Main\n    Let x be 5.\n    Show x.\n");
        // Position on "x" in the Let binding (line 1)
        let pos = Position { line: 1, character: 8 };
        let refs = find_references(&doc, pos, &test_uri(), false);
        // Should find at least the reference in "Show x."
        assert!(!refs.is_empty(), "Expected at least one reference to 'x'");
    }

    #[test]
    fn find_references_with_declaration() {
        let doc = make_doc("## Main\n    Let x be 5.\n    Show x.\n");
        let pos = Position { line: 1, character: 8 };
        let refs_with = find_references(&doc, pos, &test_uri(), true);
        let refs_without = find_references(&doc, pos, &test_uri(), false);
        assert!(
            refs_with.len() >= refs_without.len(),
            "With declarations should return >= without: {} vs {}",
            refs_with.len(),
            refs_without.len()
        );
    }

    #[test]
    fn find_references_unknown_returns_empty() {
        let doc = make_doc("## Main\n    Let x be 5.\n");
        // Position in whitespace
        let pos = Position { line: 0, character: 50 };
        let refs = find_references(&doc, pos, &test_uri(), true);
        assert!(refs.is_empty(), "Expected empty for out-of-range position");
    }

    #[test]
    fn find_references_exact_count() {
        let doc = make_doc("## Main\n    Let x be 5.\n    Show x.\n    Set x to x + 1.\n");
        let pos = Position { line: 1, character: 8 };
        let refs = find_references(&doc, pos, &test_uri(), false);
        // x appears as: "Let x"(def-ref), "Show x"(ref), "Set x"(ref), "x + 1"(ref) → at least 3
        assert!(refs.len() >= 3,
            "Expected at least 3 references to 'x', got {}", refs.len());
    }

    #[test]
    fn find_references_positions_are_correct() {
        let source = "## Main\n    Let x be 5.\n    Show x.\n";
        let doc = make_doc(source);
        let pos = Position { line: 2, character: 9 };
        let refs = find_references(&doc, pos, &test_uri(), true);
        for r in &refs {
            // Each reference range should point to "x" in the source
            let start = doc.line_index.offset(r.range.start);
            let end = doc.line_index.offset(r.range.end);
            let text = &source[start..end];
            assert_eq!(text, "x", "Reference range should point to 'x', got '{}'", text);
        }
    }

    #[test]
    fn find_references_include_declaration_adds_one() {
        let doc = make_doc("## Main\n    Let x be 5.\n    Show x.\n");
        let pos = Position { line: 1, character: 8 };
        let refs_with = find_references(&doc, pos, &test_uri(), true);
        let refs_without = find_references(&doc, pos, &test_uri(), false);
        assert!(refs_with.len() > refs_without.len(),
            "With declaration should have more refs: {} vs {}",
            refs_with.len(), refs_without.len());
    }

    #[test]
    fn find_references_correct_uri() {
        let doc = make_doc("## Main\n    Let x be 5.\n    Show x.\n");
        let uri = test_uri();
        let pos = Position { line: 2, character: 9 };
        let refs = find_references(&doc, pos, &uri, true);
        for r in &refs {
            assert_eq!(r.uri, uri, "All references should use the provided URI");
        }
    }
}
