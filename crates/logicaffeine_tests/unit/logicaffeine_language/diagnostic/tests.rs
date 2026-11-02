use super::*;
use crate::token::Span;

fn err(kind: ParseErrorKind, start: usize, end: usize) -> ParseError {
    ParseError {
        kind,
        span: Span::new(start, end),
    }
}

#[test]
fn every_kind_has_nonempty_kebab_code() {
    let kinds = sample_kinds();
    assert_eq!(kinds.len(), 39, "one sample per ParseErrorKind variant");
    for (variant, kind) in &kinds {
        let code = kind.code();
        assert!(!code.is_empty(), "{variant} has an empty code");
        assert!(
            code.chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-'),
            "{variant} code '{code}' is not kebab-case"
        );
        assert!(
            !code.starts_with('-') && !code.ends_with('-') && !code.contains("--"),
            "{variant} code '{code}' has stray hyphens"
        );
        assert_eq!(
            *variant,
            kind.variant_name(),
            "sample list and variant_name must agree"
        );
    }
}

#[test]
fn codes_are_unique_per_semantic_class() {
    // The two type-mismatch variants deliberately share one code — the
    // language server shipped that collapse. Every other code is unique.
    let mut by_code: std::collections::BTreeMap<&'static str, Vec<&'static str>> =
        Default::default();
    for (variant, kind) in sample_kinds() {
        by_code.entry(kind.code()).or_default().push(variant);
    }
    for (code, variants) in by_code {
        if code == "type-mismatch" {
            assert_eq!(
                variants,
                vec!["TypeMismatch", "TypeMismatchDetailed"],
                "'type-mismatch' is shared by exactly the two type-mismatch variants"
            );
        } else {
            assert_eq!(variants.len(), 1, "code '{code}' is shared by {variants:?}");
        }
    }
}

#[test]
fn diagnostic_serializes_with_all_schema_fields() {
    let interner = Interner::new();
    let source = "Show x.\n";
    let e = err(ParseErrorKind::UndefinedVariable { name: "x".into() }, 5, 6);
    let d = diagnostic_for(&e, source, Some("src/main.lg"), &interner);

    assert_eq!(d.code, "undefined-variable");
    assert_eq!(d.severity, "error");
    assert_eq!(d.file.as_deref(), Some("src/main.lg"));
    assert!(!d.message.is_empty(), "headline must not be empty");
    assert!(
        !d.explanation.is_empty(),
        "socratic prose must not be empty"
    );
    assert!(
        !d.explanation.contains('\u{1b}'),
        "JSON prose must be ANSI-free"
    );

    let v = serde_json::to_value(&d).unwrap();
    let obj = v.as_object().unwrap();
    let mut keys: Vec<&str> = obj.keys().map(|s| s.as_str()).collect();
    keys.sort_unstable();
    assert_eq!(
        keys,
        vec![
            "code",
            "docs",
            "explanation",
            "file",
            "message",
            "quickfix",
            "related",
            "severity",
            "span",
            "suggestion"
        ],
        "logos-diagnostic/1 key set is pinned"
    );
    let span = v["span"].as_object().unwrap();
    let mut skeys: Vec<&str> = span.keys().map(|s| s.as_str()).collect();
    skeys.sort_unstable();
    assert_eq!(
        skeys,
        vec![
            "end",
            "end_col",
            "end_line",
            "start",
            "start_col",
            "start_line"
        ]
    );

    // The shape round-trips.
    let back: Diagnostic = serde_json::from_value(v).unwrap();
    assert_eq!(back, d);
}

#[test]
fn line_col_is_one_based_and_char_counted() {
    let source = "Let sum be 1.\ncdé xyz.\n";
    let byte = source.find("xyz").unwrap();
    assert_eq!(
        line_col(source, byte),
        (2, 5),
        "é is one column, not two bytes"
    );
    assert_eq!(line_col(source, 0), (1, 1));

    let interner = Interner::new();
    let e = err(ParseErrorKind::ExpectedExpression, byte, byte + 3);
    let d = diagnostic_for(&e, source, None, &interner);
    assert_eq!((d.span.start_line, d.span.start_col), (2, 5));
    assert_eq!((d.span.end_line, d.span.end_col), (2, 8));
    assert_eq!((d.span.start, d.span.end), (byte, byte + 3));
    assert_eq!(d.file, None);
}

#[test]
fn suggestion_matches_display_with_source_hint() {
    let source = "logoc is the study of reason.";
    let e = err(ParseErrorKind::ExpectedCopula, 0, 5);

    assert_eq!(e.suggestion(source), Some("logic"));
    let display = e.display_with_source(source);
    assert!(
        display.contains("logic"),
        "display hint and structured suggestion must agree: {display}"
    );

    let interner = Interner::new();
    let d = diagnostic_for(&e, source, None, &interner);
    assert_eq!(d.suggestion.as_deref(), Some("did you mean 'logic'?"));
}

#[test]
fn catalog_entries_are_fully_rendered() {
    let entries = catalog();
    assert_eq!(entries.len(), 39);
    for entry in &entries {
        assert!(!entry.headline.is_empty(), "{} headline", entry.variant);
        assert!(
            !entry.explanation.is_empty(),
            "{} explanation",
            entry.variant
        );
        if let Some(docs) = &entry.docs {
            assert!(
                docs.starts_with("https://"),
                "{} docs must be a full URL: {docs}",
                entry.variant
            );
        }
    }
}
