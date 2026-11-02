use super::*;
use crate::token::Span;

#[test]
fn parse_error_has_span() {
    let error = ParseError {
        kind: ParseErrorKind::ExpectedCopula,
        span: Span::new(5, 10),
    };
    assert_eq!(error.span.start, 5);
    assert_eq!(error.span.end, 10);
}

#[test]
fn display_with_source_shows_line_and_underline() {
    let error = ParseError {
        kind: ParseErrorKind::ExpectedCopula,
        span: Span::new(8, 14),
    };
    let source = "All men mortal are.";
    let display = error.display_with_source(source);
    assert!(
        display.contains("mortal"),
        "Should contain source word: {}",
        display
    );
    assert!(
        display.contains("^^^^^^"),
        "Should contain underline: {}",
        display
    );
}

#[test]
fn display_with_source_suggests_typo_fix() {
    let error = ParseError {
        kind: ParseErrorKind::ExpectedCopula,
        span: Span::new(0, 5),
    };
    let source = "logoc is the study of reason.";
    let display = error.display_with_source(source);
    assert!(
        display.contains("did you mean"),
        "Should suggest fix: {}",
        display
    );
    assert!(
        display.contains("logic"),
        "Should suggest 'logic': {}",
        display
    );
}

#[test]
fn display_with_source_has_color_codes() {
    let error = ParseError {
        kind: ParseErrorKind::ExpectedCopula,
        span: Span::new(0, 3),
    };
    let source = "Alll men are mortal.";
    let display = error.display_with_source(source);
    assert!(
        display.contains("\x1b["),
        "Should contain ANSI escape codes: {}",
        display
    );
}

// =========================================================================
// Phase 4 — Type error reporting variants
// =========================================================================

#[test]
fn type_mismatch_detailed_socratic_mentions_types() {
    let interner = logicaffeine_base::Interner::new();
    let error = ParseError {
        kind: ParseErrorKind::TypeMismatchDetailed {
            expected: "Int".to_string(),
            found: "Bool".to_string(),
            context: "in let binding".to_string(),
        },
        span: Span::new(0, 0),
    };
    let explanation = socratic_explanation(&error, &interner);
    assert!(
        explanation.contains("Int"),
        "Should mention expected type: {}",
        explanation
    );
    assert!(
        explanation.contains("Bool"),
        "Should mention found type: {}",
        explanation
    );
    assert!(
        explanation.contains("let binding"),
        "Should include context: {}",
        explanation
    );
}

#[test]
fn type_mismatch_detailed_without_context_is_clean() {
    let interner = logicaffeine_base::Interner::new();
    let error = ParseError {
        kind: ParseErrorKind::TypeMismatchDetailed {
            expected: "Text".to_string(),
            found: "Int".to_string(),
            context: String::new(),
        },
        span: Span::new(0, 0),
    };
    let explanation = socratic_explanation(&error, &interner);
    assert!(
        explanation.contains("Text"),
        "Should mention expected type: {}",
        explanation
    );
    assert!(
        explanation.contains("Int"),
        "Should mention found type: {}",
        explanation
    );
    // No spurious "()" from empty context
    assert!(
        !explanation.contains("()"),
        "Empty context should not leave '()': {}",
        explanation
    );
}

#[test]
fn infinite_type_socratic_mentions_both_descriptions() {
    let interner = logicaffeine_base::Interner::new();
    let error = ParseError {
        kind: ParseErrorKind::InfiniteType {
            var_description: "type variable α0".to_string(),
            type_description: "Seq of α0".to_string(),
        },
        span: Span::new(0, 0),
    };
    let explanation = socratic_explanation(&error, &interner);
    assert!(
        explanation.contains("α0"),
        "Should mention var: {}",
        explanation
    );
    assert!(
        explanation.contains("Seq of α0"),
        "Should mention type: {}",
        explanation
    );
}

#[test]
fn arity_mismatch_socratic_mentions_function_and_counts() {
    let interner = logicaffeine_base::Interner::new();
    let error = ParseError {
        kind: ParseErrorKind::ArityMismatch {
            function: "double".to_string(),
            expected: 1,
            found: 3,
        },
        span: Span::new(0, 0),
    };
    let explanation = socratic_explanation(&error, &interner);
    assert!(
        explanation.contains("double"),
        "Should name the function: {}",
        explanation
    );
    assert!(
        explanation.contains("1"),
        "Should mention expected count: {}",
        explanation
    );
    assert!(
        explanation.contains("3"),
        "Should mention found count: {}",
        explanation
    );
}

#[test]
fn field_not_found_socratic_mentions_type_and_field() {
    let interner = logicaffeine_base::Interner::new();
    let error = ParseError {
        kind: ParseErrorKind::FieldNotFound {
            type_name: "Point".to_string(),
            field_name: "z".to_string(),
            available: vec!["x".to_string(), "y".to_string()],
        },
        span: Span::new(0, 0),
    };
    let explanation = socratic_explanation(&error, &interner);
    assert!(
        explanation.contains("Point"),
        "Should name the type: {}",
        explanation
    );
    assert!(
        explanation.contains("z"),
        "Should name the missing field: {}",
        explanation
    );
    assert!(
        explanation.contains("x"),
        "Should list available fields: {}",
        explanation
    );
    assert!(
        explanation.contains("y"),
        "Should list available fields: {}",
        explanation
    );
}

#[test]
fn not_a_function_socratic_mentions_found_type() {
    let interner = logicaffeine_base::Interner::new();
    let error = ParseError {
        kind: ParseErrorKind::NotAFunction {
            found_type: "Int".to_string(),
        },
        span: Span::new(0, 0),
    };
    let explanation = socratic_explanation(&error, &interner);
    assert!(
        explanation.contains("Int"),
        "Should mention the type found: {}",
        explanation
    );
    assert!(
        explanation.to_lowercase().contains("function"),
        "Should mention function: {}",
        explanation
    );
}
