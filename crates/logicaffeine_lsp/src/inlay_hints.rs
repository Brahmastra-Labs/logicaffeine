use tower_lsp::lsp_types::{InlayHint, InlayHintKind, InlayHintLabel, InlayHintTooltip, MarkupContent, MarkupKind, Range};

use logicaffeine_compile::analysis::VarState;
use logicaffeine_language::token::TokenType;

use crate::document::DocumentState;
use crate::index::{DefinitionKind, resolve_token_name};

/// Handle inlay hints request.
///
/// Shows type annotations for variables declared without explicit types.
pub fn inlay_hints(doc: &DocumentState, range: Range) -> Vec<InlayHint> {
    let mut hints = Vec::new();
    let is_default_range = range == Range::default();

    for def in &doc.symbol_index.definitions {
        if def.kind != DefinitionKind::Variable {
            continue;
        }
        if def.span == logicaffeine_language::token::Span::default() {
            continue;
        }

        if let Some(detail) = &def.detail {
            if detail.contains("(inferred)") {
                let pos = doc.line_index.position(def.span.end);
                if !is_default_range && (pos.line < range.start.line || pos.line > range.end.line) {
                    continue;
                }
                let type_label = extract_inferred_type(detail);
                hints.push(InlayHint {
                    position: pos,
                    label: InlayHintLabel::String(format!(": {}", type_label)),
                    kind: Some(InlayHintKind::TYPE),
                    text_edits: None,
                    tooltip: None,
                    padding_left: Some(false),
                    padding_right: Some(true),
                    data: None,
                });
            }
        }
    }

    // Ownership state hints for non-Owned variables
    for token in &doc.tokens {
        if !matches!(
            &token.kind,
            TokenType::Identifier | TokenType::ProperName(_)
            | TokenType::Adjective(_) | TokenType::Noun(_)
        ) {
            continue;
        }
        if token.span == logicaffeine_language::token::Span::default() {
            continue;
        }

        let pos = doc.line_index.position(token.span.end);
        if !is_default_range && (pos.line < range.start.line || pos.line > range.end.line) {
            continue;
        }

        if let Some(name) = resolve_token_name(token, &doc.interner) {
            if let Some(state) = doc.ownership_states.get(name) {
                let (label, tooltip) = match state {
                    VarState::Moved => ("moved", "This variable has been given away and can no longer be used."),
                    VarState::MaybeMoved => ("maybe moved", "This variable might have been given away in a conditional branch."),
                    VarState::Borrowed => ("borrowed", "This variable is currently borrowed (lent via Show)."),
                    VarState::Owned => continue, // Don't show for Owned — that's noise
                };
                hints.push(InlayHint {
                    position: pos,
                    label: InlayHintLabel::String(format!(" {}", label)),
                    kind: Some(InlayHintKind::PARAMETER),
                    text_edits: None,
                    tooltip: Some(InlayHintTooltip::MarkupContent(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: tooltip.to_string(),
                    })),
                    padding_left: Some(true),
                    padding_right: Some(false),
                    data: None,
                });
            }
        }
    }

    hints
}

/// Extract the inferred type from a detail string like "Let x: Int (inferred)".
/// Returns the type name (e.g. "Int") or "auto" if not found.
fn extract_inferred_type(detail: &str) -> &str {
    // Format: "Let [mut ]name: TypeName (inferred)"
    if let Some(colon_pos) = detail.rfind(": ") {
        let after_colon = &detail[colon_pos + 2..];
        if let Some(paren_pos) = after_colon.find(" (inferred)") {
            let ty = after_colon[..paren_pos].trim();
            if !ty.is_empty() {
                return ty;
            }
        }
    }
    "auto"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::DocumentState;

    fn make_doc(source: &str) -> DocumentState {
        DocumentState::new(source.to_string(), 1)
    }

    #[test]
    fn inlay_hints_for_inferred_integer() {
        let doc = make_doc("## Main\n    Let x be 5.\n");
        let range = Range::default();
        let hints = inlay_hints(&doc, range);
        assert!(!hints.is_empty(), "Expected inlay hint for inferred variable");
        match &hints[0].label {
            InlayHintLabel::String(s) => {
                assert_eq!(s, ": Int", "Integer literal should infer Int, got '{}'", s);
            }
            _ => panic!("Expected string label"),
        }
    }

    #[test]
    fn inlay_hints_for_inferred_text() {
        let doc = make_doc("## Main\n    Let msg be \"hello\".\n");
        let range = Range::default();
        let hints = inlay_hints(&doc, range);
        assert!(!hints.is_empty(), "Expected inlay hint for text variable");
        match &hints[0].label {
            InlayHintLabel::String(s) => {
                assert_eq!(s, ": Text", "Text literal should infer Text, got '{}'", s);
            }
            _ => panic!("Expected string label"),
        }
    }

    #[test]
    fn inlay_hints_for_inferred_bool() {
        let doc = make_doc("## Main\n    Let flag be true.\n");
        let range = Range::default();
        let hints = inlay_hints(&doc, range);
        assert!(!hints.is_empty(), "Expected inlay hint for bool variable");
        match &hints[0].label {
            InlayHintLabel::String(s) => {
                assert_eq!(s, ": Bool", "Bool literal should infer Bool, got '{}'", s);
            }
            _ => panic!("Expected string label"),
        }
    }

    #[test]
    fn inlay_hints_empty_for_empty_doc() {
        let doc = make_doc("");
        let range = Range::default();
        let hints = inlay_hints(&doc, range);
        assert!(hints.is_empty());
    }

    #[test]
    fn inlay_hints_skip_default_spans() {
        let doc = make_doc("## Main\n    Let x be 5.\n");
        let range = Range::default();
        let hints = inlay_hints(&doc, range);
        for hint in &hints {
            assert_eq!(hint.kind, Some(InlayHintKind::TYPE), "Hints should be TYPE kind");
        }
    }

    #[test]
    fn inlay_hints_hint_position_correct() {
        let doc = make_doc("## Main\n    Let x be 5.\n");
        let range = Range::default();
        let hints = inlay_hints(&doc, range);
        assert!(!hints.is_empty(), "Should have at least one inlay hint for inferred variable");
        let hint = &hints[0];
        assert_eq!(hint.position.line, 1, "Hint should be on line 1 (the Let statement)");
    }

    #[test]
    fn extract_inferred_type_from_detail() {
        assert_eq!(extract_inferred_type("Let x: Int (inferred)"), "Int");
        assert_eq!(extract_inferred_type("Let x: Text (inferred)"), "Text");
        assert_eq!(extract_inferred_type("Let x: auto (inferred)"), "auto");
        assert_eq!(extract_inferred_type("Let x: Int"), "auto"); // no "(inferred)" marker
    }

    #[test]
    fn inlay_hints_respects_range_parameter() {
        let doc = make_doc("## Main\n    Let x be 5.\n    Let y be 10.\n");
        let range = tower_lsp::lsp_types::Range {
            start: tower_lsp::lsp_types::Position { line: 1, character: 0 },
            end: tower_lsp::lsp_types::Position { line: 1, character: 99 },
        };
        let restricted_hints = inlay_hints(&doc, range);
        let all_hints = inlay_hints(&doc, Range::default());
        assert!(restricted_hints.len() <= all_hints.len(),
            "Restricted range should have <= hints: {} vs {}", restricted_hints.len(), all_hints.len());
    }

    #[test]
    fn inlay_hint_borrowed_after_show() {
        let doc = make_doc("## Main\n    Let x be 5.\n    Show x.\n");
        let hints = inlay_hints(&doc, Range::default());
        let borrowed_hints: Vec<_> = hints.iter()
            .filter(|h| matches!(&h.label, InlayHintLabel::String(s) if s.contains("borrowed")))
            .collect();
        // x should be Borrowed after Show
        if doc.ownership_states.get("x").map_or(false, |s| matches!(s, VarState::Borrowed)) {
            assert!(
                !borrowed_hints.is_empty(),
                "Should have 'borrowed' inlay hint for x after Show"
            );
        }
    }

    #[test]
    fn inlay_hint_no_marker_for_owned() {
        let doc = make_doc("## Main\n    Let x be 5.\n");
        let hints = inlay_hints(&doc, Range::default());
        let ownership_hints: Vec<_> = hints.iter()
            .filter(|h| matches!(&h.label, InlayHintLabel::String(s) if s.contains("moved") || s.contains("borrowed")))
            .collect();
        // x is Owned after just Let — no ownership hint should appear
        assert!(
            ownership_hints.is_empty(),
            "Owned variables should not have ownership inlay hints. Got: {:?}",
            ownership_hints.iter().map(|h| &h.label).collect::<Vec<_>>()
        );
    }
}
