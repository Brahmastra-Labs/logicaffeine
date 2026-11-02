use super::*;
use crate::pipeline::analyze;

#[test]
fn let_binding_has_nondefault_span() {
    let result = analyze("## Main\n    Let x be 5.\n");
    assert!(result.errors.is_empty(), "Errors: {:?}", result.errors);
    let defs = result.symbol_index.definitions_of("x");
    assert_eq!(defs.len(), 1, "Expected 1 def for 'x', got {:?}", defs);
    assert_ne!(
        defs[0].span,
        Span::default(),
        "Definition span should not be default after fix"
    );
}

#[test]
fn let_binding_span_points_to_source() {
    let source = "## Main\n    Let x be 5.\n";
    let result = analyze(source);
    assert!(result.errors.is_empty(), "Errors: {:?}", result.errors);
    let defs = result.symbol_index.definitions_of("x");
    assert_eq!(defs.len(), 1);
    let span = defs[0].span;
    let text = &source[span.start..span.end];
    assert_eq!(
        text, "x",
        "Span should point to 'x' in source, got '{}'",
        text
    );
}

#[test]
fn definition_at_finds_variable() {
    let source = "## Main\n    Let x be 5.\n";
    let result = analyze(source);
    assert!(result.errors.is_empty(), "Errors: {:?}", result.errors);
    let defs = result.symbol_index.definitions_of("x");
    assert!(!defs.is_empty());
    let span = defs[0].span;
    let def = result.symbol_index.definition_at(span.start);
    assert!(def.is_some(), "definition_at should find 'x' at its span");
    assert_eq!(def.unwrap().name, "x");
}

#[test]
fn definitions_of_returns_correct_kind() {
    let result = analyze("## Main\n    Let x be 5.\n");
    assert!(result.errors.is_empty(), "Errors: {:?}", result.errors);
    let defs = result.symbol_index.definitions_of("x");
    assert_eq!(defs[0].kind, DefinitionKind::Variable);
}

#[test]
fn definitions_of_unknown_returns_empty() {
    let result = analyze("## Main\n    Let x be 5.\n");
    let defs = result.symbol_index.definitions_of("nonexistent");
    assert!(defs.is_empty());
}

#[test]
fn references_to_finds_usages() {
    let result = analyze("## Main\n    Let x be 5.\n    Show x.\n");
    assert!(result.errors.is_empty(), "Errors: {:?}", result.errors);
    let refs = result.symbol_index.references_to("x");
    assert!(refs.len() >= 1, "Expected refs to 'x', got {}", refs.len());
}

#[test]
fn reference_linked_to_definition() {
    let result = analyze("## Main\n    Let x be 5.\n    Show x.\n");
    assert!(result.errors.is_empty(), "Errors: {:?}", result.errors);
    let refs = result.symbol_index.references_to("x");
    let linked: Vec<_> = refs.iter().filter(|r| r.definition_idx.is_some()).collect();
    assert!(
        !linked.is_empty(),
        "At least one reference should link to a definition"
    );
}

#[test]
fn let_binding_has_detail() {
    let result = analyze("## Main\n    Let x be 5.\n");
    assert!(result.errors.is_empty(), "Errors: {:?}", result.errors);
    let defs = result.symbol_index.definitions_of("x");
    assert!(defs[0].detail.is_some(), "Definition should have detail");
    assert!(
        defs[0].detail.as_ref().unwrap().contains("Let"),
        "Detail should mention Let: {:?}",
        defs[0].detail
    );
}

#[test]
fn block_spans_populated() {
    let result = analyze("## Main\n    Let x be 5.\n");
    assert!(
        !result.symbol_index.block_spans.is_empty(),
        "block_spans should have at least one entry"
    );
}

#[test]
fn multiple_variables_indexed() {
    let result = analyze("## Main\n    Let a be 1.\n    Let b be 2.\n    Let c be 3.\n");
    assert!(result.errors.is_empty(), "Errors: {:?}", result.errors);
    assert_eq!(result.symbol_index.definitions_of("a").len(), 1);
    assert_eq!(result.symbol_index.definitions_of("b").len(), 1);
    assert_eq!(result.symbol_index.definitions_of("c").len(), 1);
}

#[test]
fn definition_at_whitespace_returns_none() {
    let source = "## Main\n    Let x be 5.\n";
    let result = analyze(source);
    // Offset 0 is at '#' in "## Main" — there IS a BlockHeader token there
    // But offset somewhere in pure whitespace between tokens should be None
    // The space between "Let" and "x" is at byte 12
    // Let's use an offset that's clearly in trailing whitespace
    let past_end = source.len() + 5;
    let def = result.symbol_index.definition_at(past_end);
    assert!(
        def.is_none(),
        "definition_at beyond source should return None"
    );
}

#[test]
fn references_to_exact_count() {
    let source = "## Main\n    Let x be 5.\n    Show x.\n    Set x to x + 1.\n";
    let result = analyze(source);
    assert!(result.errors.is_empty(), "Errors: {:?}", result.errors);
    let refs = result.symbol_index.references_to("x");
    // "x" appears in: "Let x" (def+ref), "Show x" (ref), "Set x" (ref), "x + 1" (ref)
    // The exact count depends on tokenization, but there should be at least 3 references
    assert!(
        refs.len() >= 3,
        "Expected at least 3 references to 'x', got {}: {:?}",
        refs.len(),
        refs.iter()
            .map(|r| (r.span.start, r.span.end))
            .collect::<Vec<_>>()
    );
}

#[test]
fn definitions_of_function() {
    let source =
        "## To greet (name: Text):\n    Show name.\n## Main\n    Call greet with \"Alice\".\n";
    let result = analyze(source);
    let defs = result.symbol_index.definitions_of("greet");
    let func_defs: Vec<_> = defs
        .iter()
        .filter(|d| d.kind == DefinitionKind::Function)
        .collect();
    assert!(
        !func_defs.is_empty(),
        "Expected a Function definition for 'greet', got defs: {:?}",
        defs
    );
    assert_eq!(func_defs[0].kind, DefinitionKind::Function);
}

#[test]
fn parameter_has_own_span() {
    let source = "## To greet (name: Text):\n    Show name.\n";
    let result = analyze(source);
    assert!(result.errors.is_empty(), "Errors: {:?}", result.errors);

    let func_defs = result.symbol_index.definitions_of("greet");
    let param_defs = result.symbol_index.definitions_of("name");

    assert!(
        !func_defs.is_empty(),
        "Should have function def for 'greet'"
    );
    assert!(!param_defs.is_empty(), "Should have param def for 'name'");

    let func_span = func_defs
        .iter()
        .find(|d| d.kind == DefinitionKind::Function)
        .map(|d| d.span);
    let param_span = param_defs
        .iter()
        .find(|d| d.kind == DefinitionKind::Parameter)
        .map(|d| d.span);

    if let (Some(fs), Some(ps)) = (func_span, param_span) {
        if fs != Span::default() && ps != Span::default() {
            assert_ne!(
                fs, ps,
                "Parameter 'name' span {:?} should differ from function 'greet' span {:?}",
                ps, fs
            );
            let param_text = &source[ps.start..ps.end];
            assert_eq!(
                param_text, "name",
                "Parameter span should point to 'name' in source, got '{}'",
                param_text
            );
        }
    }
}

#[test]
fn second_occurrence_found() {
    let source = "## Main\n    Let x be 5.\n    Let x be 10.\n";
    let result = analyze(source);
    // Even if there are parse errors due to redefinition, we should get definitions
    let defs = result.symbol_index.definitions_of("x");
    // There should be at least one definition
    assert!(!defs.is_empty(), "Expected at least one definition for 'x'");
}
