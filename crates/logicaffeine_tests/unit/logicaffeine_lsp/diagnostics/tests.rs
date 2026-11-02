use super::*;
use logicaffeine_language::token::Span;

#[test]
fn parse_error_produces_diagnostic() {
    let interner = Interner::new();
    let line_index = LineIndex::new("Let x be 5.\nSet x to 10.");

    let error = ParseError {
        kind: ParseErrorKind::ExpectedExpression,
        span: Span::new(12, 15),
    };

    let diagnostics = convert_errors(&[error], &[], &interner, &line_index, None);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].range.start.line, 1);
    assert_eq!(diagnostics[0].severity, Some(DiagnosticSeverity::ERROR));
    assert!(diagnostics[0].message.contains("expression"));
}

#[test]
fn is_value_equality_is_warning() {
    let interner = Interner::new();
    let line_index = LineIndex::new("x is 5");

    let error = ParseError {
        kind: ParseErrorKind::IsValueEquality {
            variable: "x".to_string(),
            value: "5".to_string(),
        },
        span: Span::new(0, 6),
    };

    let diagnostics = convert_errors(&[error], &[], &interner, &line_index, None);
    assert_eq!(diagnostics[0].severity, Some(DiagnosticSeverity::WARNING));
}

#[test]
fn grammar_error_is_warning() {
    let interner = Interner::new();
    let line_index = LineIndex::new("bad grammar");

    let error = ParseError {
        kind: ParseErrorKind::GrammarError("test".to_string()),
        span: Span::new(0, 3),
    };

    let diagnostics = convert_errors(&[error], &[], &interner, &line_index, None);
    assert_eq!(diagnostics[0].severity, Some(DiagnosticSeverity::WARNING));
}

#[test]
fn zero_index_is_information() {
    let interner = Interner::new();
    let line_index = LineIndex::new("list[0]");

    let error = ParseError {
        kind: ParseErrorKind::ZeroIndex,
        span: Span::new(4, 7),
    };

    let diagnostics = convert_errors(&[error], &[], &interner, &line_index, None);
    assert_eq!(
        diagnostics[0].severity,
        Some(DiagnosticSeverity::INFORMATION)
    );
}

#[test]
fn multiple_errors_produce_multiple_diagnostics() {
    let interner = Interner::new();
    let line_index = LineIndex::new("abc\ndef\nghi");

    let errors = vec![
        ParseError {
            kind: ParseErrorKind::ExpectedExpression,
            span: Span::new(0, 3),
        },
        ParseError {
            kind: ParseErrorKind::ExpectedExpression,
            span: Span::new(4, 7),
        },
    ];

    let diagnostics = convert_errors(&errors, &[], &interner, &line_index, None);
    assert_eq!(diagnostics.len(), 2);
    assert_eq!(diagnostics[0].range.start.line, 0);
    assert_eq!(diagnostics[1].range.start.line, 1);
}

#[test]
fn empty_errors_produce_empty_diagnostics() {
    let interner = Interner::new();
    let line_index = LineIndex::new("fine");
    let diagnostics = convert_errors(&[], &[], &interner, &line_index, None);
    assert!(diagnostics.is_empty());
}

#[test]
fn use_after_move_is_error_severity() {
    let decision = decision_for(&ParseErrorKind::UseAfterMove {
        name: "x".to_string(),
    });
    assert_eq!(decision.severity, DiagnosticSeverity::ERROR);
}

#[test]
fn diagnostic_range_spans_correct_positions() {
    let interner = Interner::new();
    let line_index = LineIndex::new("abc\ndef");
    let error = ParseError {
        kind: ParseErrorKind::ExpectedExpression,
        span: Span::new(4, 7),
    };
    let diagnostics = convert_errors(&[error], &[], &interner, &line_index, None);
    assert_eq!(diagnostics[0].range.start.line, 1);
    assert_eq!(diagnostics[0].range.start.character, 0);
    assert_eq!(diagnostics[0].range.end.line, 1);
    assert_eq!(diagnostics[0].range.end.character, 3);
}

#[test]
fn is_value_equality_has_diagnostic_code() {
    let interner = Interner::new();
    let line_index = LineIndex::new("x is 5");

    let error = ParseError {
        kind: ParseErrorKind::IsValueEquality {
            variable: "x".to_string(),
            value: "5".to_string(),
        },
        span: Span::new(0, 6),
    };

    let diagnostics = convert_errors(&[error], &[], &interner, &line_index, None);
    assert_eq!(
        diagnostics[0].code,
        Some(lsp_types::NumberOrString::String(
            "is-value-equality".to_string()
        )),
        "IsValueEquality should have diagnostic code"
    );
}

#[test]
fn use_after_move_has_diagnostic_code() {
    let interner = Interner::new();
    let line_index = LineIndex::new("x moved");

    let error = ParseError {
        kind: ParseErrorKind::UseAfterMove {
            name: "x".to_string(),
        },
        span: Span::new(0, 1),
    };

    let diagnostics = convert_errors(&[error], &[], &interner, &line_index, None);
    assert_eq!(
        diagnostics[0].code,
        Some(lsp_types::NumberOrString::String(
            "use-after-move".to_string()
        )),
        "UseAfterMove should have diagnostic code"
    );
}

#[test]
fn regular_error_has_no_diagnostic_code() {
    let interner = Interner::new();
    let line_index = LineIndex::new("bad");

    let error = ParseError {
        kind: ParseErrorKind::ExpectedExpression,
        span: Span::new(0, 3),
    };

    let diagnostics = convert_errors(&[error], &[], &interner, &line_index, None);
    assert_eq!(
        diagnostics[0].code, None,
        "ExpectedExpression should have no code"
    );
}

#[test]
fn diagnostic_has_source() {
    let interner = Interner::new();
    let line_index = LineIndex::new("bad");

    let error = ParseError {
        kind: ParseErrorKind::ExpectedExpression,
        span: Span::new(0, 3),
    };

    let diagnostics = convert_errors(&[error], &[], &interner, &line_index, None);
    assert_eq!(diagnostics[0].source, Some("logicaffeine".to_string()));
}

#[test]
fn escape_error_produces_diagnostic_with_code() {
    let error = AnalysisError {
        message: "Reference 'x' cannot escape zone 'temp'.".to_string(),
        variable: "x".to_string(),
        code: "escape-return",
        cause_context: Some("zone 'temp'".to_string()),
        use_span: None,
        cause_span: None,
    };
    let interner = Interner::new();
    let line_index = LineIndex::new("Let x be 5.");
    let diagnostics = convert_analysis_errors(&[error], &[], &interner, &line_index, None);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(
        diagnostics[0].code,
        Some(lsp_types::NumberOrString::String(
            "escape-return".to_string()
        ))
    );
    assert_eq!(diagnostics[0].severity, Some(DiagnosticSeverity::ERROR));
}

#[test]
fn ownership_error_has_error_severity() {
    let error = AnalysisError {
        message: "Cannot use 'x' after giving it away.".to_string(),
        variable: "x".to_string(),
        code: "use-after-move",
        cause_context: None,
        use_span: None,
        cause_span: None,
    };
    let interner = Interner::new();
    let line_index = LineIndex::new("Let x be 5.");
    let diagnostics = convert_analysis_errors(&[error], &[], &interner, &line_index, None);
    assert_eq!(diagnostics[0].severity, Some(DiagnosticSeverity::ERROR));
}

#[test]
fn ownership_error_message_is_socratic() {
    let error = AnalysisError {
        message: "Cannot use 'x' after giving it away.\n\nYou transferred ownership.".to_string(),
        variable: "x".to_string(),
        code: "use-after-move",
        cause_context: None,
        use_span: None,
        cause_span: None,
    };
    let interner = Interner::new();
    let line_index = LineIndex::new("Let x be 5.");
    let diagnostics = convert_analysis_errors(&[error], &[], &interner, &line_index, None);
    assert!(
        diagnostics[0].message.contains("giving it away"),
        "Message should be Socratic: {}",
        diagnostics[0].message
    );
    assert!(
        diagnostics[0].message.contains("ownership"),
        "Message should explain ownership: {}",
        diagnostics[0].message
    );
}

#[test]
fn undefined_variable_has_diagnostic_code() {
    let interner = Interner::new();
    let line_index = LineIndex::new("Show z.");

    let error = ParseError {
        kind: ParseErrorKind::UndefinedVariable {
            name: "z".to_string(),
        },
        span: Span::new(5, 6),
    };

    let diagnostics = convert_errors(&[error], &[], &interner, &line_index, None);
    assert_eq!(
        diagnostics[0].code,
        Some(lsp_types::NumberOrString::String(
            "undefined-variable".to_string()
        ))
    );
}

#[test]
fn type_mismatch_has_diagnostic_code() {
    let interner = Interner::new();
    let line_index = LineIndex::new("Let x: Int be \"hello\".");

    let error = ParseError {
        kind: ParseErrorKind::TypeMismatch {
            expected: "Int".to_string(),
            found: "Text".to_string(),
        },
        span: Span::new(0, 22),
    };

    let diagnostics = convert_errors(&[error], &[], &interner, &line_index, None);
    assert_eq!(
        diagnostics[0].code,
        Some(lsp_types::NumberOrString::String(
            "type-mismatch".to_string()
        ))
    );
}

#[test]
fn use_after_move_diagnostic_has_related_information() {
    // Use the full pipeline to get real tokens with Give
    let source = "## Main\n    Let x be 5.\n    Let y be 0.\n    Give x to y.\n    Show x.\n";
    let result = crate::pipeline::analyze(source);
    let line_index = LineIndex::new(source);
    let uri = Url::parse("file:///test.la").unwrap();

    // Create a move error with cause_context (as pipeline does)
    let error = AnalysisError {
        message: "Cannot use 'x' after giving it away.".to_string(),
        variable: "x".to_string(),
        code: "use-after-move",
        cause_context: Some("'x' was given away here".to_string()),
        use_span: None,
        cause_span: None,
    };

    let diagnostics = convert_analysis_errors(
        &[error],
        &result.tokens,
        &result.interner,
        &line_index,
        Some(&uri),
    );

    assert_eq!(diagnostics.len(), 1);
    let related = diagnostics[0].related_information.as_ref();
    assert!(
        related.is_some(),
        "Use-after-move diagnostic should have related information"
    );
    let related = related.unwrap();
    assert_eq!(related.len(), 1);
    assert!(
        related[0].message.contains("given away"),
        "Related info should mention giving away: {}",
        related[0].message
    );
    assert_eq!(related[0].location.uri, uri);
}

#[test]
fn related_info_points_to_give_statement() {
    let source = "## Main\n    Let x be 5.\n    Let y be 0.\n    Give x to y.\n    Show x.\n";
    let result = crate::pipeline::analyze(source);
    let line_index = LineIndex::new(source);
    let uri = Url::parse("file:///test.la").unwrap();

    let error = AnalysisError {
        message: "Cannot use 'x' after giving it away.".to_string(),
        variable: "x".to_string(),
        code: "use-after-move",
        cause_context: Some("'x' was given away here".to_string()),
        use_span: None,
        cause_span: None,
    };

    let diagnostics = convert_analysis_errors(
        &[error],
        &result.tokens,
        &result.interner,
        &line_index,
        Some(&uri),
    );

    let related = diagnostics[0].related_information.as_ref().unwrap();
    // The Give keyword should be on line 3 (0-indexed): "    Give x to y."
    assert_eq!(
        related[0].location.range.start.line, 3,
        "Related info should point to the Give statement on line 3"
    );
}

#[test]
fn no_related_info_without_uri() {
    let source = "## Main\n    Let x be 5.\n    Let y be 0.\n    Give x to y.\n    Show x.\n";
    let result = crate::pipeline::analyze(source);
    let line_index = LineIndex::new(source);

    let error = AnalysisError {
        message: "Cannot use 'x' after giving it away.".to_string(),
        variable: "x".to_string(),
        code: "use-after-move",
        cause_context: Some("'x' was given away here".to_string()),
        use_span: None,
        cause_span: None,
    };

    let diagnostics = convert_analysis_errors(
        &[error],
        &result.tokens,
        &result.interner,
        &line_index,
        None,
    );

    assert!(
        diagnostics[0].related_information.is_none(),
        "Without URI, related information should be None"
    );
}

#[test]
fn escape_diagnostic_has_related_info_pointing_to_zone() {
    // Escape errors should have related info pointing to the Zone keyword
    let error = AnalysisError {
        message: "Reference 'x' cannot escape zone 'temp'.".to_string(),
        variable: "x".to_string(),
        code: "escape-return",
        cause_context: Some("zone 'temp'".to_string()),
        use_span: None,
        cause_span: None,
    };

    // Without real tokens containing a Zone keyword, related info won't resolve.
    // This tests the fallback when no matching keyword is found.
    let interner = Interner::new();
    let line_index = LineIndex::new("Zone temp: Let x be 5.");
    let uri = Url::parse("file:///test.la").unwrap();
    let diagnostics = convert_analysis_errors(&[error], &[], &interner, &line_index, Some(&uri));
    // With empty tokens, the keyword search fails gracefully
    assert_eq!(diagnostics.len(), 1);
    // related_information is None because no tokens matched
    assert!(
        diagnostics[0].related_information.is_none(),
        "With no tokens, related info should be None"
    );
}
