//! Selection ranges: expand-selection follows English structure —
//! word → sentence → block → document.

use tower_lsp::lsp_types::{Position, Range, SelectionRange};

use logicaffeine_language::token::Span;

use crate::document::DocumentState;

pub fn selection_ranges(doc: &DocumentState, positions: &[Position]) -> Vec<SelectionRange> {
    positions
        .iter()
        .map(|position| selection_chain(doc, *position))
        .collect()
}

fn selection_chain(doc: &DocumentState, position: Position) -> SelectionRange {
    let offset = doc.line_index.offset(position);

    let mut spans: Vec<Span> = Vec::new();

    // Word: the token under the cursor.
    if let Some(token) = doc
        .tokens
        .iter()
        .find(|t| t.span.start <= offset && offset < t.span.end)
    {
        spans.push(token.span);
    }

    // Sentence: the smallest statement containing the cursor.
    if let Some((_, span)) = doc
        .symbol_index
        .statement_spans
        .iter()
        .filter(|(_, s)| s.start <= offset && offset < s.end)
        .min_by_key(|(_, s)| s.end - s.start)
    {
        spans.push(*span);
    }

    // Block: the smallest `##` section containing the cursor.
    if let Some((_, _, span)) = doc
        .symbol_index
        .block_spans
        .iter()
        .filter(|(_, _, s)| s.start <= offset && offset < s.end)
        .min_by_key(|(_, _, s)| s.end - s.start)
    {
        spans.push(*span);
    }

    // Document.
    spans.push(Span::new(0, doc.source.len()));

    // Keep the chain strictly widening; drop non-nesting or equal steps.
    let mut widening: Vec<Span> = Vec::new();
    for span in spans {
        match widening.last() {
            Some(prev) if span.start <= prev.start && prev.end <= span.end && span != *prev => {
                widening.push(span)
            }
            None => widening.push(span),
            _ => {}
        }
    }

    // Build innermost-out: the LSP shape is child-with-parent.
    let mut chain: Option<SelectionRange> = None;
    for span in widening.into_iter().rev() {
        chain = Some(SelectionRange {
            range: Range {
                start: doc.line_index.position(span.start),
                end: doc.line_index.position(span.end),
            },
            parent: chain.map(Box::new),
        });
    }
    chain.unwrap_or(SelectionRange {
        range: Range {
            start: position,
            end: position,
        },
        parent: None,
    })
}
