use lsp_types::{DocumentSymbol, Range, SymbolKind};

use crate::document::DocumentState;
use crate::index::DefinitionKind;
use crate::line_index::LineIndex;

#[allow(deprecated)]
pub fn document_symbols(doc: &DocumentState) -> Vec<DocumentSymbol> {
    let mut symbols = Vec::new();

    // Add block-level symbols (## Main, ## To, etc.)
    for (name, block_type, span) in &doc.symbol_index.block_spans {
        let kind = match block_type {
            logicaffeine_language::token::BlockType::Main => SymbolKind::FUNCTION,
            logicaffeine_language::token::BlockType::Function => SymbolKind::FUNCTION,
            logicaffeine_language::token::BlockType::Theorem => SymbolKind::CLASS,
            logicaffeine_language::token::BlockType::Definition
            | logicaffeine_language::token::BlockType::TypeDef => SymbolKind::STRUCT,
            logicaffeine_language::token::BlockType::Policy => SymbolKind::INTERFACE,
            logicaffeine_language::token::BlockType::Proof => SymbolKind::METHOD,
            // The LGX blocks, kinded for what the reader is looking AT rather than left in the
            // catch-all: a page, a route, a store and a directive all landing on NAMESPACE made
            // the outline — the one view of a file an editor gives you for free — say nothing
            // about the file's shape.
            logicaffeine_language::token::BlockType::Component
            | logicaffeine_language::token::BlockType::Page { .. }
            | logicaffeine_language::token::BlockType::ErrorPage { .. } => SymbolKind::CLASS,
            logicaffeine_language::token::BlockType::Route { .. }
            | logicaffeine_language::token::BlockType::Middleware
            | logicaffeine_language::token::BlockType::Redirect { .. } => SymbolKind::METHOD,
            // A store is shared state a whole page reads through, which is what a module is.
            logicaffeine_language::token::BlockType::Store => SymbolKind::MODULE,
            // A directive is behaviour attached to an element — a function, called by the view.
            logicaffeine_language::token::BlockType::Directive => SymbolKind::FUNCTION,
            _ => SymbolKind::NAMESPACE,
        };

        let range = span_to_range(span, &doc.line_index);
        let selection_range = Range {
            start: doc.line_index.position(span.start),
            end: doc
                .line_index
                .position(span.start + name.len().min(span.end - span.start)),
        };

        symbols.push(DocumentSymbol {
            name: name.clone(),
            detail: None,
            kind,
            tags: None,
            deprecated: None,
            range,
            selection_range,
            children: None,
        });
    }

    // Add definitions from the symbol index, nesting children under parents.
    // Track index of most recent parent symbol (Struct, Enum, Function) for nesting.
    let mut current_parent_idx: Option<usize> = None;

    for def in &doc.symbol_index.definitions {
        if def.span == logicaffeine_language::token::Span::default() {
            continue; // Skip definitions without known positions
        }

        let kind = match def.kind {
            DefinitionKind::Function => SymbolKind::FUNCTION,
            DefinitionKind::Variable => SymbolKind::VARIABLE,
            DefinitionKind::Struct => SymbolKind::STRUCT,
            DefinitionKind::Enum => SymbolKind::ENUM,
            DefinitionKind::Field => SymbolKind::FIELD,
            DefinitionKind::Parameter => SymbolKind::VARIABLE,
            DefinitionKind::Variant => SymbolKind::ENUM_MEMBER,
            DefinitionKind::Block => continue, // Already added from block_spans
            DefinitionKind::Theorem => SymbolKind::CLASS,
            DefinitionKind::Component => SymbolKind::CLASS,
            DefinitionKind::Action => SymbolKind::METHOD,
            DefinitionKind::StateField => SymbolKind::PROPERTY,
        };

        let range = span_to_range(&def.span, &doc.line_index);

        let symbol = DocumentSymbol {
            name: def.name.clone(),
            detail: def.detail.clone(),
            kind,
            tags: None,
            deprecated: None,
            range,
            selection_range: range,
            children: None,
        };

        // Decide whether this is a child of the current parent
        let is_child = matches!(
            def.kind,
            DefinitionKind::Field | DefinitionKind::Parameter | DefinitionKind::Variant
        );

        if is_child {
            if let Some(parent_idx) = current_parent_idx {
                let parent = &mut symbols[parent_idx];
                parent.children.get_or_insert_with(Vec::new).push(symbol);
                continue;
            }
        }

        // This is a top-level symbol; track it as a potential parent
        if matches!(
            def.kind,
            DefinitionKind::Struct | DefinitionKind::Enum | DefinitionKind::Function
        ) {
            current_parent_idx = Some(symbols.len());
        } else {
            // Non-parent top-level (Variable, Theorem, etc.) — reset parent tracking
            current_parent_idx = None;
        }

        // Try nesting Variable/Theorem under their containing block
        if matches!(def.kind, DefinitionKind::Variable | DefinitionKind::Theorem) {
            if let Some(block_idx) = find_containing_block(&symbols, &def.span, &doc.line_index) {
                symbols[block_idx]
                    .children
                    .get_or_insert_with(Vec::new)
                    .push(symbol);
                continue;
            }
        }

        symbols.push(symbol);
    }

    symbols
}

/// Find the block symbol whose range contains the given span.
#[allow(deprecated)]
fn find_containing_block(
    symbols: &[DocumentSymbol],
    span: &logicaffeine_language::token::Span,
    line_index: &LineIndex,
) -> Option<usize> {
    let pos = line_index.position(span.start);
    for (i, sym) in symbols.iter().enumerate() {
        let is_block = matches!(
            sym.kind,
            SymbolKind::FUNCTION
                | SymbolKind::CLASS
                | SymbolKind::STRUCT
                | SymbolKind::INTERFACE
                | SymbolKind::METHOD
                | SymbolKind::NAMESPACE
        );
        if is_block && pos.line >= sym.range.start.line && pos.line <= sym.range.end.line {
            return Some(i);
        }
    }
    None
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_lsp/document_symbols/tests.rs"]
mod tests;

fn span_to_range(span: &logicaffeine_language::token::Span, line_index: &LineIndex) -> Range {
    Range {
        start: line_index.position(span.start),
        end: line_index.position(span.end),
    }
}
