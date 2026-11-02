use super::*;

#[test]
fn parse_rustc_json_extracts_errors() {
    let json_output = r#"{"reason":"compiler-message","message":{"message":"use of moved value: `x`","code":{"code":"E0382"},"level":"error","spans":[{"file_name":"src/main.rs","line_start":5,"line_end":5,"column_start":10,"column_end":11,"is_primary":true,"label":null,"text":[]}],"children":[]}}
{"reason":"build-finished","success":false}"#;

    let diagnostics = parse_rustc_json(json_output);
    assert_eq!(diagnostics.len(), 1);
    assert_eq!(diagnostics[0].message, "use of moved value: `x`");
    assert_eq!(get_error_code(&diagnostics[0]), Some("E0382"));
}

#[test]
fn remap_points_a_rustc_error_at_the_logos_line() {
    use crate::sourcemap::SourceMapBuilder;
    use crate::token::Span;

    // `Show mystery(1).` is on LOGOS line 4.
    let logos = "# App\n\n## Main\nShow mystery(1).\n";
    let show_offset = logos.find("Show mystery").unwrap();
    let mut builder = SourceMapBuilder::new(logos);
    // The generated Rust line 10 maps back to that LOGOS span.
    builder.record_line_at(10, Span::new(show_offset, show_offset + 4));
    let source_map = builder.build();

    let diag = RustcDiagnostic {
        message: "cannot find function `mystery` in this scope".to_string(),
        code: None,
        level: "error".to_string(),
        spans: vec![RustcSpan {
            file_name: "src/main.rs".to_string(),
            line_start: 10,
            line_end: 10,
            column_start: 5,
            column_end: 12,
            is_primary: true,
            label: None,
            text: vec![],
        }],
        children: vec![],
    };

    let remapped = remap_rustc_diagnostics(&[diag], &source_map, &|_| None);
    assert_eq!(
        remapped.len(),
        1,
        "the error must remap to a LOGOS location"
    );
    assert_eq!(
        remapped[0].line, 4,
        "maps to the LOGOS line of `Show mystery`"
    );
    assert!(remapped[0].source_line.contains("Show mystery"));
    assert!(remapped[0].message.contains("mystery"));
}

#[test]
fn byte_offset_to_line_col_is_utf8_safe_at_every_offset() {
    // `café` (é = 2 bytes), `π` (2), `🎉` (4). Landing on ANY byte offset —
    // including the middle of a multi-byte char, and past the end — must
    // never panic and must yield a sane 1-based (line, column).
    let source = "# café\n\nShow π here.\n🎉 done\n";
    for offset in 0..=source.len() + 4 {
        let (line, column, _line_text) = byte_offset_to_line_col(source, offset);
        assert!(line >= 1, "line is 1-based at offset {offset}");
        assert!(column >= 1, "column is 1-based at offset {offset}");
    }
}

#[test]
fn remap_tolerates_a_diagnostic_with_no_spans() {
    use crate::sourcemap::SourceMapBuilder;
    // A diagnostic with no spans (or an unmappable line) must be dropped, not
    // panic.
    let source_map = SourceMapBuilder::new("## Main\nShow 1.\n").build();
    let diag = RustcDiagnostic {
        message: "some rustc note".to_string(),
        code: None,
        level: "error".to_string(),
        spans: vec![],
        children: vec![],
    };
    let remapped = remap_rustc_diagnostics(&[diag], &source_map, &|_| None);
    assert!(
        remapped.is_empty(),
        "an unmappable diagnostic is dropped, not remapped"
    );
}

#[test]
fn extract_var_from_message_works() {
    assert_eq!(
        extract_var_from_message("use of moved value: `data`", "value: `", "`"),
        Some("data".to_string())
    );
    assert_eq!(
        extract_var_from_message("cannot move out of `x` because", "out of `", "`"),
        Some("x".to_string())
    );
}

#[test]
fn translate_e0382_creates_friendly_error() {
    let interner = Interner::new();
    let source_map = SourceMap::new("Let data be 5.\nGive data to processor.".to_string());

    let diag = RustcDiagnostic {
        message: "use of moved value: `data`".to_string(),
        code: Some(RustcCode {
            code: "E0382".to_string(),
        }),
        level: "error".to_string(),
        spans: vec![RustcSpan {
            file_name: "src/main.rs".to_string(),
            line_start: 3,
            line_end: 3,
            column_start: 10,
            column_end: 14,
            is_primary: true,
            label: None,
            text: vec![],
        }],
        children: vec![],
    };

    let bridge = DiagnosticBridge::new(&source_map, &interner);
    let error = bridge.translate(&diag).expect("Should translate");

    assert!(error.title.contains("data"));
    assert!(error.title.contains("giving it away"));
    assert!(error.explanation.contains("moved"));
    assert!(error.suggestion.is_some());
}

#[test]
fn translate_e0597_creates_hotel_california_error() {
    let interner = Interner::new();
    let source_map = SourceMap::new("Inside a zone:\n    Let x be 5.".to_string());

    let diag = RustcDiagnostic {
        message: "borrowed value does not live long enough".to_string(),
        code: Some(RustcCode {
            code: "E0597".to_string(),
        }),
        level: "error".to_string(),
        spans: vec![RustcSpan {
            file_name: "src/main.rs".to_string(),
            line_start: 5,
            line_end: 5,
            column_start: 1,
            column_end: 10,
            is_primary: true,
            label: None,
            text: vec![],
        }],
        children: vec![],
    };

    let bridge = DiagnosticBridge::new(&source_map, &interner);
    let error = bridge.translate(&diag).expect("Should translate");

    assert!(error.title.contains("outlive"));
    assert!(error.explanation.contains("Zone") || error.explanation.contains("borrowed"));
}
