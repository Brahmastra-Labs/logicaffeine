use lsp_types::{
    InlayHint, InlayHintKind, InlayHintLabel, InlayHintTooltip, MarkupContent, MarkupKind, Range,
};

use logicaffeine_compile::analysis::VarState;
use logicaffeine_language::token::TokenType;

use crate::document::DocumentState;
use crate::index::{resolve_token_name, DefinitionKind};

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
            TokenType::Identifier
                | TokenType::ProperName(_)
                | TokenType::Adjective(_)
                | TokenType::Noun(_)
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
                    VarState::Moved => (
                        "moved",
                        "This variable has been given away and can no longer be used.".to_string(),
                    ),
                    VarState::MaybeMoved(given_in) => (
                        "maybe moved",
                        format!("This variable might have been given away in {given_in}."),
                    ),
                    VarState::Borrowed => (
                        "borrowed",
                        "This variable is currently borrowed (lent via Show).".to_string(),
                    ),
                    VarState::Owned => continue, // Don't show for Owned — that's noise
                };
                hints.push(InlayHint {
                    position: pos,
                    label: InlayHintLabel::String(format!(" {}", label)),
                    kind: Some(InlayHintKind::PARAMETER),
                    text_edits: None,
                    tooltip: Some(InlayHintTooltip::MarkupContent(MarkupContent {
                        kind: MarkupKind::Markdown,
                        value: tooltip,
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
#[path = "../../logicaffeine_tests/unit/logicaffeine_lsp/inlay_hints/tests.rs"]
mod tests;
