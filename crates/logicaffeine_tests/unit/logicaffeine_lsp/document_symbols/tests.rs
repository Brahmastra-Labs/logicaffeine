use super::*;
use crate::document::DocumentState;

fn make_doc(source: &str) -> DocumentState {
    DocumentState::new(source.to_string(), 1)
}

/// ★ An LGX file's outline. Every LGX block fell into the `_ => NAMESPACE` arm, so a page,
/// a route, a store and a directive all looked like the same anonymous thing in the outline —
/// the one view of a file an editor gives you for free, showing nothing about its shape.
///
/// The kinds are chosen for what the reader is looking AT: a `## Component` and a `## Page`
/// are things with state and behaviour (CLASS), a `## Route` is something you call (METHOD),
/// a `## Store` is shared state (MODULE), a `## Directive` is behaviour attached to an
/// element (FUNCTION).
#[test]
#[allow(deprecated)]
fn an_lgx_file_has_a_readable_outline() {
    let doc = make_doc(
        "## Component Card\nView:\n    <p>hi</p>\n\n\
             ## Page /todos\nView:\n    <p>list</p>\n\n\
             ## Store Session\nState:\n    Let name be \"guest\".\n\n\
             ## Directive tip (el: Element) and (t: Text):\n    Show t.\n\n\
             ## Route GET /api/todos -> Text:\n    Return \"[]\".\n",
    );
    let symbols = document_symbols(&doc);
    let by_name = |n: &str| symbols.iter().find(|s| s.name.contains(n)).cloned();

    for (name, want) in [
        ("Card", SymbolKind::CLASS),
        ("todos", SymbolKind::CLASS),
        ("Session", SymbolKind::MODULE),
        ("tip", SymbolKind::FUNCTION),
    ] {
        let sym = by_name(name).unwrap_or_else(|| {
            panic!(
                "`{name}` must appear in the outline; got {:?}",
                symbols.iter().map(|s| &s.name).collect::<Vec<_>>()
            )
        });
        assert_eq!(sym.kind, want, "`{name}` has the wrong kind in the outline");
    }
}

/// A route is what a reader of a server file scans for, and its METHOD and PATH are the
/// whole identity — `GET /api/todos` and `POST /api/todos` are different routes, so an
/// outline showing only one of them, or neither, is worse than none.
#[test]
#[allow(deprecated)]
fn a_route_shows_its_method_and_path() {
    let doc = make_doc(
        "## Route GET /api/todos -> Text:\n    Return \"[]\".\n\n\
             ## Route POST /api/todos -> Text:\n    Return \"ok\".\n",
    );
    let symbols = document_symbols(&doc);
    let routes: Vec<&str> = symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::METHOD)
        .map(|s| s.name.as_str())
        .collect();
    assert_eq!(routes.len(), 2, "both routes appear: {routes:?}");
    assert!(routes
        .iter()
        .any(|r| r.contains("GET") && r.contains("/api/todos")));
    assert!(routes
        .iter()
        .any(|r| r.contains("POST") && r.contains("/api/todos")));
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
    let non_block_syms: Vec<_> = symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::VARIABLE || s.kind == SymbolKind::FUNCTION)
        .collect();
    assert!(
        non_block_syms.is_empty(),
        "Empty doc should have no variable/function symbols: {:?}",
        non_block_syms.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
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
            sym.name,
            sym.range.start,
            sym.range.end
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
                assert!(
                    sym.detail.is_some(),
                    "Variable symbol '{}' should have a non-None detail",
                    sym.name
                );
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
    let block_syms: Vec<_> = symbols.iter().filter(|s| s.name.contains("Main")).collect();
    assert_eq!(
        block_syms.len(),
        1,
        "Main should appear exactly once, got {}: {:?}",
        block_syms.len(),
        block_syms.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
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
    assert_eq!(
        main_sym.unwrap().kind,
        SymbolKind::FUNCTION,
        "Main block should be FUNCTION kind"
    );
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
    assert!(
        child_names.contains(&"x"),
        "Main should contain 'x': {:?}",
        child_names
    );
    assert!(
        child_names.contains(&"y"),
        "Main should contain 'y': {:?}",
        child_names
    );
}

#[test]
#[allow(deprecated)]
fn function_params_nested() {
    let doc = make_doc("## To greet (name: Text):\n    Show name.\n");
    let symbols = document_symbols(&doc);
    let func_syms: Vec<_> = symbols
        .iter()
        .filter(|s| s.kind == SymbolKind::FUNCTION)
        .collect();
    let has_children = func_syms
        .iter()
        .any(|s| s.children.as_ref().map(|c| !c.is_empty()).unwrap_or(false));
    if has_children {
        let parent = func_syms
            .iter()
            .find(|s| s.children.as_ref().map(|c| !c.is_empty()).unwrap_or(false))
            .unwrap();
        let child_names: Vec<&str> = parent
            .children
            .as_ref()
            .unwrap()
            .iter()
            .map(|c| c.name.as_str())
            .collect();
        assert!(
            child_names.contains(&"name"),
            "Function should have 'name' param as child: {:?}",
            child_names
        );
    }
}
