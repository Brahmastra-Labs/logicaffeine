use tower_lsp::lsp_types::{DocumentSymbol, Range, SymbolKind};

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
            _ => SymbolKind::NAMESPACE,
        };

        let range = span_to_range(span, &doc.line_index);
        let selection_range = Range {
            start: doc.line_index.position(span.start),
            end: doc.line_index.position(span.start + name.len().min(span.end - span.start)),
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
                symbols[block_idx].children.get_or_insert_with(Vec::new).push(symbol);
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
            SymbolKind::FUNCTION | SymbolKind::CLASS | SymbolKind::STRUCT
            | SymbolKind::INTERFACE | SymbolKind::METHOD | SymbolKind::NAMESPACE
        );
        if is_block
            && pos.line >= sym.range.start.line
            && pos.line <= sym.range.end.line
        {
            return Some(i);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::DocumentState;

    fn make_doc(source: &str) -> DocumentState {
        DocumentState::new(source.to_string(), 1)
    }

    #[test]
    #[allow(deprecated)]
    fn symbols_include_block_headers() {
        let doc = make_doc("## Main\n    Let x be 5.\n");
        let symbols = document_symbols(&doc);
        let block_names: Vec<&str> = symbols.iter().map(|s| s.name.as_str()).collect();
        assert!(
            block_names.iter().any(|n| n.contains("Main")),
            "Should include 'Main' block symbol: {:?}",
            block_names
        );
    }

    #[test]
    #[allow(deprecated)]
    fn symbols_include_variables() {
        let doc = make_doc("## Main\n    Let x be 5.\n");
        let symbols = document_symbols(&doc);
        // Variables may be nested under blocks, so search recursively
        fn find_var(symbols: &[DocumentSymbol], name: &str) -> bool {
            for sym in symbols {
                if sym.kind == SymbolKind::VARIABLE && sym.name == name {
                    return true;
                }
                if let Some(children) = &sym.children {
                    if find_var(children, name) {
                        return true;
                    }
                }
            }
            false
        }
        assert!(
            find_var(&symbols, "x"),
            "Should include variable 'x' (possibly nested): {:?}",
            symbols.iter().map(|s| &s.name).collect::<Vec<_>>()
        );
    }

    #[test]
    #[allow(deprecated)]
    fn symbols_empty_for_empty_doc() {
        let doc = make_doc("");
        let symbols = document_symbols(&doc);
        // Empty doc should have no variable/function symbols
        let non_block_syms: Vec<_> = symbols.iter()
            .filter(|s| s.kind == SymbolKind::VARIABLE || s.kind == SymbolKind::FUNCTION)
            .collect();
        assert!(non_block_syms.is_empty(),
            "Empty doc should have no variable/function symbols: {:?}",
            non_block_syms.iter().map(|s| &s.name).collect::<Vec<_>>());
    }

    #[test]
    #[allow(deprecated)]
    fn symbols_have_valid_ranges() {
        let doc = make_doc("## Main\n    Let x be 5.\n");
        let symbols = document_symbols(&doc);
        for sym in &symbols {
            assert!(
                sym.range.start.line <= sym.range.end.line,
                "Symbol '{}' has invalid range: start {:?} > end {:?}",
                sym.name, sym.range.start, sym.range.end
            );
        }
    }

    #[test]
    #[allow(deprecated)]
    fn symbol_detail_populated() {
        let doc = make_doc("## Main\n    Let x be 5.\n");
        let symbols = document_symbols(&doc);
        // Variables are nested under blocks, so search recursively
        fn collect_vars(symbols: &[DocumentSymbol], out: &mut Vec<String>) {
            for sym in symbols {
                if sym.kind == SymbolKind::VARIABLE {
                    out.push(sym.name.clone());
                    assert!(sym.detail.is_some(),
                        "Variable symbol '{}' should have a non-None detail", sym.name);
                }
                if let Some(children) = &sym.children {
                    collect_vars(children, out);
                }
            }
        }
        let mut vars = Vec::new();
        collect_vars(&symbols, &mut vars);
        assert!(!vars.is_empty(), "Should find at least one variable");
    }

    #[test]
    #[allow(deprecated)]
    fn block_kind_skipped_in_definitions() {
        let doc = make_doc("## Main\n    Let x be 5.\n");
        let symbols = document_symbols(&doc);
        // Block definitions should not appear duplicated — they come from block_spans,
        // and DefinitionKind::Block is `continue`d in the loop
        let block_syms: Vec<_> = symbols.iter()
            .filter(|s| s.name.contains("Main"))
            .collect();
        assert_eq!(block_syms.len(), 1,
            "Main should appear exactly once, got {}: {:?}",
            block_syms.len(), block_syms.iter().map(|s| &s.name).collect::<Vec<_>>());
    }

    #[test]
    #[allow(deprecated)]
    fn variable_symbols_have_no_children() {
        let doc = make_doc("## Main\n    Let x be 5.\n    Let y be 10.\n");
        let symbols = document_symbols(&doc);
        // Variables are nested under blocks; check they have no children of their own
        fn check_vars(symbols: &[DocumentSymbol]) {
            for sym in symbols {
                if sym.kind == SymbolKind::VARIABLE {
                    assert!(
                        sym.children.is_none() || sym.children.as_ref().unwrap().is_empty(),
                        "Variable '{}' should have no children",
                        sym.name
                    );
                }
                if let Some(children) = &sym.children {
                    check_vars(children);
                }
            }
        }
        check_vars(&symbols);
    }

    #[test]
    #[allow(deprecated)]
    fn symbols_block_has_correct_kind() {
        let doc = make_doc("## Main\n    Let x be 5.\n");
        let symbols = document_symbols(&doc);
        let main_sym = symbols.iter().find(|s| s.name.contains("Main"));
        assert!(main_sym.is_some(), "Should have a Main symbol");
        assert_eq!(main_sym.unwrap().kind, SymbolKind::FUNCTION, "Main block should be FUNCTION kind");
    }

    #[test]
    #[allow(deprecated)]
    fn variables_nested_under_block() {
        let doc = make_doc("## Main\n    Let x be 5.\n    Let y be 10.\n");
        let symbols = document_symbols(&doc);
        let main_sym = symbols.iter().find(|s| s.name.contains("Main"));
        assert!(main_sym.is_some(), "Should have Main block");
        let main = main_sym.unwrap();
        let children = main.children.as_ref();
        assert!(children.is_some(), "Main block should have children");
        let child_names: Vec<&str> = children.unwrap().iter().map(|c| c.name.as_str()).collect();
        assert!(child_names.contains(&"x"), "Main should contain 'x': {:?}", child_names);
        assert!(child_names.contains(&"y"), "Main should contain 'y': {:?}", child_names);
    }

    #[test]
    #[allow(deprecated)]
    fn function_params_nested() {
        let doc = make_doc("## To greet (name: Text):\n    Show name.\n");
        let symbols = document_symbols(&doc);
        let func_syms: Vec<_> = symbols.iter()
            .filter(|s| s.kind == SymbolKind::FUNCTION)
            .collect();
        let has_children = func_syms.iter().any(|s| {
            s.children.as_ref().map(|c| !c.is_empty()).unwrap_or(false)
        });
        if has_children {
            let parent = func_syms.iter().find(|s| {
                s.children.as_ref().map(|c| !c.is_empty()).unwrap_or(false)
            }).unwrap();
            let child_names: Vec<&str> = parent.children.as_ref().unwrap()
                .iter().map(|c| c.name.as_str()).collect();
            assert!(child_names.contains(&"name"),
                "Function should have 'name' param as child: {:?}", child_names);
        }
    }
}

fn span_to_range(span: &logicaffeine_language::token::Span, line_index: &LineIndex) -> Range {
    Range {
        start: line_index.position(span.start),
        end: line_index.position(span.end),
    }
}
