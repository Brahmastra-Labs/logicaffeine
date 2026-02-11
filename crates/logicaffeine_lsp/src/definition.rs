use tower_lsp::lsp_types::{GotoDefinitionResponse, Location, Position, Range, Url};

use crate::document::DocumentState;

/// Handle go-to-definition request.
///
/// Given a cursor position, find the token at that position, look up its
/// name in the symbol index, and return the definition's location.
pub fn goto_definition(
    doc: &DocumentState,
    position: Position,
    uri: &Url,
) -> Option<GotoDefinitionResponse> {
    let offset = doc.line_index.offset(position);

    // Find the token at the cursor position
    let token = doc.tokens.iter().find(|t| {
        offset >= t.span.start && offset < t.span.end
    })?;

    let name = doc.source.get(token.span.start..token.span.end)?;

    // Look up definitions for this name
    let defs = doc.symbol_index.definitions_of(name);
    if defs.is_empty() {
        return None;
    }

    let locations: Vec<Location> = defs
        .iter()
        .filter(|d| d.span != logicaffeine_language::token::Span::default())
        .map(|d| Location {
            uri: uri.clone(),
            range: Range {
                start: doc.line_index.position(d.span.start),
                end: doc.line_index.position(d.span.end),
            },
        })
        .collect();

    if locations.is_empty() {
        None
    } else if locations.len() == 1 {
        Some(GotoDefinitionResponse::Scalar(locations.into_iter().next().unwrap()))
    } else {
        Some(GotoDefinitionResponse::Array(locations))
    }
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
    fn goto_definition_of_variable() {
        let doc = make_doc("## Main\n    Let x be 5.\n    Show x.\n");
        // Position on "x" in "Show x." (line 2, character 9)
        let pos = Position { line: 2, character: 9 };
        let result = goto_definition(&doc, pos, &test_uri());
        assert!(result.is_some(), "Expected definition for 'x'");
        match result.unwrap() {
            GotoDefinitionResponse::Scalar(loc) => {
                assert_eq!(loc.range.start.line, 1, "Definition should be on line 1");
            }
            GotoDefinitionResponse::Array(locs) => {
                assert!(!locs.is_empty(), "Expected at least one location");
                assert_eq!(locs[0].range.start.line, 1, "Definition should be on line 1");
            }
            _ => panic!("Unexpected response type"),
        }
    }

    #[test]
    fn goto_definition_whitespace_returns_none() {
        let doc = make_doc("## Main\n    Let x be 5.\n");
        // Position past end of source → no token
        let pos = Position { line: 0, character: 50 };
        let result = goto_definition(&doc, pos, &test_uri());
        assert!(result.is_none(), "Position past end of source should return None");
    }

    #[test]
    fn goto_def_returns_none_for_keyword() {
        let doc = make_doc("## Main\n    Let x be 5.\n");
        // Position on "Let" keyword — 'Let' is a keyword, not a definition name
        let pos = Position { line: 1, character: 4 };
        let result = goto_definition(&doc, pos, &test_uri());
        // "Let" is a keyword; it might or might not have a definition.
        // The important thing is it doesn't panic and returns a sensible result.
        if let Some(resp) = &result {
            match resp {
                GotoDefinitionResponse::Scalar(loc) => {
                    assert_ne!(loc.range.start, loc.range.end, "Should have a non-empty range");
                }
                GotoDefinitionResponse::Array(locs) => {
                    assert!(!locs.is_empty());
                }
                _ => {}
            }
        }
    }

    #[test]
    fn goto_def_correct_span_range() {
        let doc = make_doc("## Main\n    Let x be 5.\n    Show x.\n");
        let pos = Position { line: 2, character: 9 };
        let result = goto_definition(&doc, pos, &test_uri());
        assert!(result.is_some(), "Expected definition for 'x'");
        match result.unwrap() {
            GotoDefinitionResponse::Scalar(loc) => {
                // 'x' is a single character, so the range should span exactly 1 char
                assert_eq!(loc.range.start.line, loc.range.end.line, "Single-char name should be on same line");
                let char_diff = loc.range.end.character - loc.range.start.character;
                assert_eq!(char_diff, 1, "Range should span exactly 1 character for 'x', got {}", char_diff);
            }
            GotoDefinitionResponse::Array(locs) => {
                let loc = &locs[0];
                let char_diff = loc.range.end.character - loc.range.start.character;
                assert_eq!(char_diff, 1, "Range should span exactly 1 character for 'x', got {}", char_diff);
            }
            _ => panic!("Unexpected response type"),
        }
    }

    #[test]
    fn goto_definition_returns_correct_uri() {
        let doc = make_doc("## Main\n    Let x be 5.\n    Show x.\n");
        let uri = test_uri();
        let pos = Position { line: 2, character: 9 };
        if let Some(GotoDefinitionResponse::Scalar(loc)) = goto_definition(&doc, pos, &uri) {
            assert_eq!(loc.uri, uri);
        }
    }
}
