use super::*;

#[test]
fn new_document_parses_source() {
    let doc = DocumentState::new("## Main\n    Let x be 5.\n    Show x.\n".to_string(), 1);
    assert_eq!(doc.version, 1);
    assert!(
        doc.diagnostics.is_empty(),
        "Valid source should have no diagnostics: {:?}",
        doc.diagnostics
    );
    assert!(!doc.tokens.is_empty());
    assert!(!doc.symbol_index.definitions.is_empty());
}

#[test]
fn fresh_snapshot_replaces_analysis() {
    let doc = DocumentState::new("## Main\n    Let x be 5.\n".to_string(), 1);
    assert_eq!(doc.symbol_index.definitions_of("x").len(), 1);
    assert_eq!(doc.symbol_index.definitions_of("y").len(), 0);

    // Documents are immutable snapshots: an edit produces a new one.
    let doc = DocumentState::new("## Main\n    Let y be 10.\n".to_string(), 2);
    assert_eq!(doc.version, 2);
    assert_eq!(doc.symbol_index.definitions_of("y").len(), 1);
}

#[test]
fn empty_document() {
    let doc = DocumentState::new("".to_string(), 0);
    assert_eq!(doc.version, 0);
    assert_eq!(doc.source, "");
}

#[test]
fn document_source_stored() {
    let source = "## Main\n    Let x be 5.\n";
    let doc = DocumentState::new(source.to_string(), 1);
    assert_eq!(doc.source, source);
}

#[test]
fn snapshot_of_broken_source_carries_diagnostics() {
    let doc = DocumentState::new("## Main\n    Let x be 5.\n    Show x.\n".to_string(), 1);
    assert!(
        doc.diagnostics.is_empty(),
        "Valid source should have no diagnostics"
    );
    let doc = DocumentState::new("## Main\n    Let be.\n".to_string(), 2);
    assert!(
        !doc.diagnostics.is_empty(),
        "Invalid source should produce diagnostics"
    );
}

#[test]
fn line_index_matches_snapshot_source() {
    let doc = DocumentState::new("line0\nline1\nline2\nline3\nline4\n".to_string(), 2);
    let pos = doc.line_index.position(doc.source.len().saturating_sub(2));
    assert_eq!(pos.line, 4, "In a 5-line doc, near-end should be line 4");
}

#[test]
fn document_with_move_error_has_diagnostics() {
    // Give x to y moves x; Show x afterward is use-after-move
    let source = "## Main\n    Let x be 5.\n    Let y be 0.\n    Give x to y.\n    Show x.\n";
    let doc = DocumentState::new(source.to_string(), 1);
    let move_diags: Vec<_> = doc
        .diagnostics
        .iter()
        .filter(|d| {
            let is_move_code = d.code.as_ref().map_or(
                false,
                |c| matches!(c, lsp_types::NumberOrString::String(s) if s == "use-after-move"),
            );
            let is_move_msg = d.message.contains("after") && d.message.contains("move");
            is_move_code || is_move_msg
        })
        .collect();
    assert!(
        !move_diags.is_empty(),
        "Document with use-after-move should have ownership diagnostics. All diags: {:?}",
        doc.diagnostics
    );
}

#[test]
fn document_ownership_states_available() {
    let source = "## Main\n    Let x be 5.\n    Show x.\n";
    let doc = DocumentState::new(source.to_string(), 1);
    assert!(
        !doc.ownership_states.is_empty(),
        "Document should have ownership states for variables"
    );
}

#[test]
fn value_copy_warning_is_published() {
    // `opens` is bound from `b's opens`, mutated by `Pop`, and never read or written back —
    // the value-semantics copy lint's own canonical example (`crate::lint`'s module doc, and
    // `logicaffeine_language`'s `parser::tests::pop_from_a_copied_field_and_never_writing_back_warns`).
    let source = "## To useOpens (b: Box) -> Int:\n\
                  \x20   Let opens be b's opens.\n\
                  \x20   Pop from opens into at.\n\
                  \x20   Return at.\n";
    let doc = DocumentState::new(source.to_string(), 1);
    let copy_diags: Vec<_> = doc
        .diagnostics
        .iter()
        .filter(|d| {
            matches!(
                &d.code,
                Some(tower_lsp::lsp_types::NumberOrString::String(s)) if s == "copy-orphaned-copy"
            )
        })
        .collect();
    assert_eq!(
        copy_diags.len(),
        1,
        "the copy lint must publish exactly one warning: {:?}",
        doc.diagnostics
    );
    assert_eq!(
        copy_diags[0].severity,
        Some(tower_lsp::lsp_types::DiagnosticSeverity::WARNING)
    );
    assert!(
        copy_diags[0].message.contains("opens"),
        "{}",
        copy_diags[0].message
    );
}
