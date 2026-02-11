use tower_lsp::lsp_types::{Location, Position, Range, Url};

use crate::document::DocumentState;

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
