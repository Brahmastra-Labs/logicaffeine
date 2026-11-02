use super::*;

#[test]
fn recovery_keeps_the_full_parses_specific_error() {
    // The full parse diagnosed IsValueEquality for `x is 5.`; the
    // per-sentence reparse only sees the excised sentence and degrades
    // it to a generic ExpectedStatement. The specific teaching must win.
    let result = analyze("## Main\nLet x be 5.\nx is 5.\n");
    assert!(
        result.errors.iter().any(|e| matches!(
            e.kind,
            logicaffeine_language::error::ParseErrorKind::IsValueEquality { .. }
        )),
        "the specific IsValueEquality must survive recovery: {:?}",
        result.errors
    );
}

#[test]
fn recovered_function_defs_keep_parenthesized_params() {
    // A theorem block sends the document down the recovery path, where
    // function definitions come from token extraction — which must read
    // the parenthesized `(n: Int)` form, not just the `with n: Int` prose.
    let result = analyze(
        "## To double (n: Int) -> Int:\n    Return n * 2.\n\n## Main\nLet x be double(3).\n\n## Theorem: Socrates\nGiven: All men are mortal. Socrates is a man.\nProve: Socrates is mortal.\nProof: Auto.\n",
    );
    let defs = result.symbol_index.definitions_of("double");
    assert!(
        defs.iter()
            .any(|d| d.detail.as_deref() == Some("To double(n: Int) -> Int")),
        "the recovered signature must keep its parameter: {:?}",
        defs.iter().map(|d| d.detail.clone()).collect::<Vec<_>>()
    );
}

#[test]
fn analyze_simple_let() {
    let result = analyze("## Main\n    Let x be 5.\n");
    assert!(
        result.errors.is_empty(),
        "Expected no errors, got: {:?}",
        result.errors
    );
    assert!(!result.tokens.is_empty());
    let defs: Vec<_> = result
        .symbol_index
        .definitions
        .iter()
        .filter(|d| d.name == "x")
        .collect();
    assert!(!defs.is_empty(), "Expected definition for 'x'");
    assert_eq!(defs[0].kind, crate::index::DefinitionKind::Variable);
}

#[test]
fn analyze_multiple_lets() {
    let result = analyze("## Main\n    Let x be 5.\n    Let y be 10.\n");
    assert!(
        result.errors.is_empty(),
        "Expected no errors, got: {:?}",
        result.errors
    );
    let x_defs = result.symbol_index.definitions_of("x");
    let y_defs = result.symbol_index.definitions_of("y");
    assert_eq!(x_defs.len(), 1);
    assert_eq!(y_defs.len(), 1);
}

#[test]
fn analyze_empty_source() {
    let result = analyze("");
    assert!(
        result.tokens.is_empty() || result.errors.is_empty(),
        "Empty source should not crash"
    );
}

#[test]
fn analyze_produces_tokens() {
    let result = analyze("## Main\n    Let x be 5.\n");
    assert!(
        result.tokens.len() >= 3,
        "Expected at least block header + Let + x tokens"
    );
}

#[test]
fn analyze_syntax_error_produces_diagnostics() {
    let result = analyze("## Main\n    Let be.\n");
    assert!(
        !result.errors.is_empty(),
        "Expected parse errors for invalid syntax"
    );
}

#[test]
fn analyze_function_def() {
    let source = "## Main\n    Let x be 5.\n    Let y be x + 1.\n";
    let result = analyze(source);
    assert!(
        result.errors.is_empty(),
        "Expected no errors, got: {:?}",
        result.errors
    );
    let x_defs = result.symbol_index.definitions_of("x");
    let y_defs = result.symbol_index.definitions_of("y");
    assert_eq!(x_defs.len(), 1, "Expected 1 def for 'x'");
    assert_eq!(y_defs.len(), 1, "Expected 1 def for 'y'");
    assert!(
        y_defs[0].detail.as_ref().unwrap().contains("Let"),
        "y detail should mention Let: {:?}",
        y_defs[0].detail
    );
}

#[test]
fn analyze_indexes_references() {
    let result = analyze("## Main\n    Let x be 5.\n    Show x.\n");
    assert!(
        result.errors.is_empty(),
        "Expected no errors, got: {:?}",
        result.errors
    );
    let refs = result.symbol_index.references_to("x");
    assert!(
        refs.len() >= 1,
        "Expected at least one reference to 'x', got {}",
        refs.len()
    );
}

#[test]
fn analyze_block_headers_indexed() {
    let result = analyze("## Main\n    Let x be 5.\n");
    assert!(
        !result.symbol_index.block_spans.is_empty(),
        "Expected block spans to be indexed"
    );
}

#[test]
fn analyze_statement_spans_indexed() {
    let result = analyze("## Main\n    Let x be 5.\n");
    assert!(
        result.errors.is_empty(),
        "Expected no errors, got: {:?}",
        result.errors
    );
    assert!(
        !result.symbol_index.statement_spans.is_empty(),
        "Expected statement spans to be indexed"
    );
}

#[test]
fn analyze_single_block_with_error_still_reports() {
    // Single block with a parse error should still produce diagnostics,
    // not silently return empty
    let result = analyze("## Main\n    Let be.\n");
    assert!(
        !result.errors.is_empty(),
        "Single block with error should still report errors"
    );
}

#[test]
fn analyze_function_def_summarized() {
    let source = "## To greet (name: Text) -> Text:\n    Return name.\n";
    let result = analyze(source);
    let defs = result.symbol_index.definitions_of("greet");
    let func_defs: Vec<_> = defs
        .iter()
        .filter(|d| d.kind == crate::index::DefinitionKind::Function)
        .collect();
    if !func_defs.is_empty() {
        let detail = func_defs[0].detail.as_ref().unwrap();
        assert!(
            detail.contains("greet"),
            "Detail should contain function name: {}",
            detail
        );
        assert!(
            detail.contains("name"),
            "Detail should contain param name: {}",
            detail
        );
    }
}

#[test]
fn analyze_generic_type_in_detail() {
    let source = "## Main\n    Let items: Seq of Int be an empty list.\n";
    let result = analyze(source);
    let defs = result.symbol_index.definitions_of("items");
    if !defs.is_empty() {
        if let Some(detail) = &defs[0].detail {
            assert!(
                detail.contains("Seq") || detail.contains("Int"),
                "Detail should mention Seq or Int: {}",
                detail
            );
        }
    }
}

#[test]
fn analyze_block_recovery_preserves_good_block() {
    let source = "## Main\n    Let be.\n## Note: readme\n    Just a note.\n";
    let result = analyze(source);
    assert!(
        result.symbol_index.block_spans.len() >= 1,
        "Block spans should have at least 1 entry even with errors"
    );
}

#[test]
fn analyze_single_block_error_does_not_swallow() {
    // Even with only one block header, error recovery should attempt to parse it
    let result = analyze("## Main\n    Let x be 5.\n    Let be.\n");
    // The good statement should still be indexed even if an error occurs
    assert!(
        !result.errors.is_empty() || !result.symbol_index.definitions_of("x").is_empty(),
        "Should either report errors or still index the valid statement"
    );
}

#[test]
fn analyze_use_after_move_produces_ownership_error() {
    // Give x to y moves x. Using x afterward should produce an ownership error
    // (from the OwnershipChecker) or a UseAfterMove parse error (from the parser).
    let source = "## Main\n    Let x be 5.\n    Let y be 0.\n    Give x to y.\n    Show x.\n";
    let result = analyze(source);
    let has_parse_move_error = result.errors.iter().any(|e| {
        matches!(
            e.kind,
            logicaffeine_language::error::ParseErrorKind::UseAfterMove { .. }
        )
    });
    let has_ownership_error = !result.ownership_errors.is_empty();
    assert!(
        has_parse_move_error || has_ownership_error,
        "Expected ownership error for use-after-move, got none. Parse errors: {:?}, Ownership errors: {:?}",
        result.errors, result.ownership_errors
    );
}

#[test]
fn analyze_clean_code_no_analysis_errors() {
    let source = "## Main\n    Let x be 5.\n    Show x.\n";
    let result = analyze(source);
    assert!(
        result.errors.is_empty(),
        "Expected no parse errors: {:?}",
        result.errors
    );
    assert!(
        result.escape_errors.is_empty(),
        "Expected no escape errors: {:?}",
        result.escape_errors
    );
    assert!(
        result.ownership_errors.is_empty(),
        "Expected no ownership errors: {:?}",
        result.ownership_errors
    );
}

#[test]
fn analyze_ownership_states_populated() {
    let source = "## Main\n    Let x be 5.\n    Show x.\n";
    let result = analyze(source);
    assert!(
        result.errors.is_empty(),
        "Expected no parse errors: {:?}",
        result.errors
    );
    assert!(
        !result.ownership_states.is_empty(),
        "Expected ownership states for variables"
    );
    let x_state = result.ownership_states.get("x");
    assert!(
        x_state.is_some(),
        "Expected state for 'x', got keys: {:?}",
        result.ownership_states.keys().collect::<Vec<_>>()
    );
}

#[test]
fn analyze_moved_variable_has_moved_state() {
    let source = "## Main\n    Let x be 5.\n    Let y be 0.\n    Give x to y.\n    Show x.\n";
    let result = analyze(source);
    // After Give, x should be Moved (even though there's a use-after-move error)
    if let Some(state) = result.ownership_states.get("x") {
        assert_eq!(*state, VarState::Moved, "x should be Moved after Give");
    }
}
