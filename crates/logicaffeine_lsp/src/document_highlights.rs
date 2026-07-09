//! Document highlights: every occurrence of the symbol under the cursor,
//! with WRITE kind on binding and mutation sites and READ everywhere else —
//! the cursor-hold highlight editors render natively.

use tower_lsp::lsp_types::{DocumentHighlight, DocumentHighlightKind, Position, Range};

use crate::document::DocumentState;
use crate::semantic_tokens::find_mutation_targets;

pub fn document_highlights(
    doc: &DocumentState,
    position: Position,
) -> Option<Vec<DocumentHighlight>> {
    let offset = doc.line_index.offset(position);

    // Resolve the cursor to a definition: through the reference at the
    // cursor, or directly when the cursor sits on the definition itself.
    let def_idx = doc
        .symbol_index
        .references
        .iter()
        .find(|r| r.span.start <= offset && offset < r.span.end)
        .and_then(|r| r.definition_idx)
        .or_else(|| {
            doc.symbol_index
                .definitions
                .iter()
                .position(|d| d.span.start <= offset && offset < d.span.end)
        })?;
    let def = &doc.symbol_index.definitions[def_idx];

    let writes = find_mutation_targets(&doc.tokens);
    let to_range = |span: logicaffeine_language::token::Span| Range {
        start: doc.line_index.position(span.start),
        end: doc.line_index.position(span.end),
    };

    let mut highlights = Vec::new();
    if def.span != logicaffeine_language::token::Span::default() {
        // The binding itself writes the name into existence.
        highlights.push(DocumentHighlight {
            range: to_range(def.span),
            kind: Some(DocumentHighlightKind::WRITE),
        });
    }
    for reference in &doc.symbol_index.references {
        if reference.definition_idx != Some(def_idx) || reference.span.start == def.span.start {
            continue;
        }
        let kind = if writes.contains(&reference.span.start) {
            DocumentHighlightKind::WRITE
        } else {
            DocumentHighlightKind::READ
        };
        highlights.push(DocumentHighlight {
            range: to_range(reference.span),
            kind: Some(kind),
        });
    }

    if highlights.is_empty() {
        None
    } else {
        Some(highlights)
    }
}
