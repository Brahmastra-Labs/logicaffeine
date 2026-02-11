use tower_lsp::lsp_types::{
    CodeAction, CodeActionKind, CodeActionOrCommand, Range, TextEdit, WorkspaceEdit,
};
use std::collections::HashMap;

use logicaffeine_language::suggest::{find_similar, KNOWN_WORDS};

use crate::document::DocumentState;

/// Handle code action request.
///
/// Provides quick-fix suggestions based on diagnostics in the given range.
pub fn code_actions(
    doc: &DocumentState,
    range: Range,
    uri: &tower_lsp::lsp_types::Url,
) -> Vec<CodeActionOrCommand> {
    let mut actions = Vec::new();

    // Generate code actions from diagnostics that overlap the requested range
    let is_default_range = range == Range::default();
    for diagnostic in &doc.diagnostics {
        if !is_default_range && !ranges_overlap(&diagnostic.range, &range) {
            continue;
        }
        let start_offset = doc.line_index.offset(diagnostic.range.start);
        let end_offset = doc.line_index.offset(diagnostic.range.end);

        let word = doc.source.get(start_offset..end_offset).unwrap_or("");

        // Spelling fix suggestions
        if let Some(suggestion) = find_similar(word, KNOWN_WORDS, 2) {
            let mut changes = HashMap::new();
            changes.insert(
                uri.clone(),
                vec![TextEdit {
                    range: diagnostic.range,
                    new_text: suggestion.to_string(),
                }],
            );

            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: format!("Did you mean '{}'?", suggestion),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![diagnostic.clone()]),
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                    ..Default::default()
                }),
                ..Default::default()
            }));
        }

        // "x is y" → "x equals y" fix
        let is_value_eq = diagnostic.code.as_ref().map_or(false, |c| {
            matches!(c, tower_lsp::lsp_types::NumberOrString::String(s) if s == "is-value-equality")
        });
        if is_value_eq {
            if let Some(is_pos) = doc.source.get(start_offset..end_offset).and_then(|s| s.find(" is ")) {
                let abs_pos = start_offset + is_pos;
                let is_range = Range {
                    start: doc.line_index.position(abs_pos),
                    end: doc.line_index.position(abs_pos + 4), // " is "
                };

                let mut changes = HashMap::new();
                changes.insert(
                    uri.clone(),
                    vec![TextEdit {
                        range: is_range,
                        new_text: " equals ".to_string(),
                    }],
                );

                actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                    title: "Use 'equals' for value comparison".to_string(),
                    kind: Some(CodeActionKind::QUICKFIX),
                    diagnostics: Some(vec![diagnostic.clone()]),
                    edit: Some(WorkspaceEdit {
                        changes: Some(changes),
                        ..Default::default()
                    }),
                    ..Default::default()
                }));
            }
        }

        // UseAfterMove → suggest "copy of variable"
        let is_use_after_move = diagnostic.code.as_ref().map_or(false, |c| {
            matches!(c, tower_lsp::lsp_types::NumberOrString::String(s) if s == "use-after-move")
        });
        if is_use_after_move {
            let mut changes = HashMap::new();
            changes.insert(
                uri.clone(),
                vec![TextEdit {
                    range: diagnostic.range,
                    new_text: format!("a copy of {}", word),
                }],
            );

            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: format!("Use 'a copy of {}' instead", word),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![diagnostic.clone()]),
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                    ..Default::default()
                }),
                ..Default::default()
            }));
        }

        // Escape errors → suggest copying before return/assignment
        let diag_code_str = diagnostic.code.as_ref().and_then(|c| {
            if let tower_lsp::lsp_types::NumberOrString::String(s) = c { Some(s.as_str()) } else { None }
        });

        if matches!(diag_code_str, Some("escape-return")) {
            let mut changes = HashMap::new();
            changes.insert(
                uri.clone(),
                vec![TextEdit {
                    range: diagnostic.range,
                    new_text: format!("a copy of {}", word),
                }],
            );
            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: "Copy before returning".to_string(),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![diagnostic.clone()]),
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                    ..Default::default()
                }),
                ..Default::default()
            }));
        }

        if matches!(diag_code_str, Some("escape-assignment")) {
            let mut changes = HashMap::new();
            changes.insert(
                uri.clone(),
                vec![TextEdit {
                    range: diagnostic.range,
                    new_text: format!("a copy of {}", word),
                }],
            );
            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: "Copy before assignment".to_string(),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![diagnostic.clone()]),
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                    ..Default::default()
                }),
                ..Default::default()
            }));
        }

        // DoubleMoved → suggest copying instead of second Give
        if matches!(diag_code_str, Some("double-move")) {
            let mut changes = HashMap::new();
            changes.insert(
                uri.clone(),
                vec![TextEdit {
                    range: diagnostic.range,
                    new_text: format!("a copy of {}", word),
                }],
            );
            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: format!("Give 'a copy of {}' instead", word),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![diagnostic.clone()]),
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                    ..Default::default()
                }),
                ..Default::default()
            }));
        }

        // ZeroIndex → suggest 1-based indexing
        if matches!(diag_code_str, Some("zero-index")) {
            let mut changes = HashMap::new();
            changes.insert(
                uri.clone(),
                vec![TextEdit {
                    range: diagnostic.range,
                    new_text: "1".to_string(),
                }],
            );
            actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                title: "Use 1-based indexing".to_string(),
                kind: Some(CodeActionKind::QUICKFIX),
                diagnostics: Some(vec![diagnostic.clone()]),
                edit: Some(WorkspaceEdit {
                    changes: Some(changes),
                    ..Default::default()
                }),
                ..Default::default()
            }));
        }

        // UndefinedVariable → suggest closest match from definitions
        if matches!(diag_code_str, Some("undefined-variable")) && !word.is_empty() {
            let def_names: Vec<&str> = doc.symbol_index.definitions.iter()
                .map(|d| d.name.as_str())
                .collect();
            if let Some(suggestion) = find_similar(word, &def_names, 2) {
                let mut changes = HashMap::new();
                changes.insert(
                    uri.clone(),
                    vec![TextEdit {
                        range: diagnostic.range,
                        new_text: suggestion.to_string(),
                    }],
                );
                actions.push(CodeActionOrCommand::CodeAction(CodeAction {
                    title: format!("Did you mean '{}'?", suggestion),
                    kind: Some(CodeActionKind::QUICKFIX),
                    diagnostics: Some(vec![diagnostic.clone()]),
                    edit: Some(WorkspaceEdit {
                        changes: Some(changes),
                        ..Default::default()
                    }),
                    ..Default::default()
                }));
            }
        }
    }

    actions
}

fn ranges_overlap(a: &Range, b: &Range) -> bool {
    !(a.end.line < b.start.line
        || (a.end.line == b.start.line && a.end.character < b.start.character)
        || b.end.line < a.start.line
        || (b.end.line == a.start.line && b.end.character < a.start.character))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::DocumentState;

    fn make_doc(source: &str) -> DocumentState {
        DocumentState::new(source.to_string(), 1)
    }

    fn test_uri() -> tower_lsp::lsp_types::Url {
        tower_lsp::lsp_types::Url::parse("file:///test.logos").unwrap()
    }

    #[test]
    fn code_actions_returns_empty_for_valid_code() {
        let doc = make_doc("## Main\n    Let x be 5.\n");
        let range = Range::default();
        let actions = code_actions(&doc, range, &test_uri());
        assert!(doc.diagnostics.is_empty(), "Valid code should have no diagnostics");
        assert!(actions.is_empty(), "No diagnostics → no actions");
    }

    #[test]
    fn code_actions_no_crash_on_syntax_error() {
        let doc = make_doc("## Main\n    Let be.\n");
        let range = Range::default();
        let actions = code_actions(&doc, range, &test_uri());
        // Should not panic on syntax errors — actions may or may not be produced
        assert!(actions.len() < 100, "Unreasonable number of actions for a tiny doc");
    }

    #[test]
    fn code_actions_empty_for_empty_doc() {
        let doc = make_doc("");
        let range = Range::default();
        let actions = code_actions(&doc, range, &test_uri());
        assert!(actions.is_empty(), "Empty doc should have no actions");
    }

    fn make_doc_with_diagnostic(source: &str, diag_code: &str, range: Range) -> DocumentState {
        use tower_lsp::lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString};
        let mut doc = DocumentState::new(source.to_string(), 1);
        doc.diagnostics.push(Diagnostic {
            range,
            severity: Some(DiagnosticSeverity::ERROR),
            code: Some(NumberOrString::String(diag_code.to_string())),
            source: Some("logicaffeine".to_string()),
            message: format!("Test diagnostic: {}", diag_code),
            ..Default::default()
        });
        doc
    }

    fn make_range(line: u32, start: u32, end: u32) -> Range {
        Range {
            start: tower_lsp::lsp_types::Position { line, character: start },
            end: tower_lsp::lsp_types::Position { line, character: end },
        }
    }

    #[test]
    fn code_action_for_escape_return_suggests_copy() {
        let source = "## Main\n    Return x.\n";
        let r = make_range(1, 11, 12); // "x" at position
        let doc = make_doc_with_diagnostic(source, "escape-return", r);
        let actions = code_actions(&doc, Range::default(), &test_uri());
        let escape_actions: Vec<_> = actions.iter()
            .filter(|a| match a {
                CodeActionOrCommand::CodeAction(ca) => ca.title.contains("Copy before returning"),
                _ => false,
            })
            .collect();
        assert!(
            !escape_actions.is_empty(),
            "Should have 'Copy before returning' action. Got: {:?}",
            actions.iter().map(|a| match a {
                CodeActionOrCommand::CodeAction(ca) => ca.title.clone(),
                _ => "command".to_string(),
            }).collect::<Vec<_>>()
        );
    }

    #[test]
    fn code_action_for_zero_index_suggests_one() {
        let source = "## Main\n    Let x be items at 0.\n";
        let r = make_range(1, 29, 30); // "0"
        let doc = make_doc_with_diagnostic(source, "zero-index", r);
        let actions = code_actions(&doc, Range::default(), &test_uri());
        let zero_actions: Vec<_> = actions.iter()
            .filter(|a| match a {
                CodeActionOrCommand::CodeAction(ca) => ca.title == "Use 1-based indexing",
                _ => false,
            })
            .collect();
        assert!(!zero_actions.is_empty(), "Should have 'Use 1-based indexing' action");
    }

    #[test]
    fn code_action_for_double_move_suggests_copy() {
        let source = "## Main\n    Let x be 5.\n    Let y be 0.\n    Give x to y.\n    Give x to y.\n";
        let r = make_range(4, 9, 10); // second "x"
        let doc = make_doc_with_diagnostic(source, "double-move", r);
        let actions = code_actions(&doc, Range::default(), &test_uri());
        let dm_actions: Vec<_> = actions.iter()
            .filter(|a| match a {
                CodeActionOrCommand::CodeAction(ca) => ca.title.contains("copy of"),
                _ => false,
            })
            .collect();
        assert!(!dm_actions.is_empty(), "Should have 'Give a copy of' action");
    }

    #[test]
    fn code_action_escape_assignment_suggests_copy() {
        let source = "## Main\n    Set outer to x.\n";
        let r = make_range(1, 18, 19); // "x"
        let doc = make_doc_with_diagnostic(source, "escape-assignment", r);
        let actions = code_actions(&doc, Range::default(), &test_uri());
        let ea_actions: Vec<_> = actions.iter()
            .filter(|a| match a {
                CodeActionOrCommand::CodeAction(ca) => ca.title == "Copy before assignment",
                _ => false,
            })
            .collect();
        assert!(!ea_actions.is_empty(), "Should have 'Copy before assignment' action");
    }

    #[test]
    fn no_code_action_for_valid_code() {
        let doc = make_doc("## Main\n    Let x be 5.\n    Show x.\n");
        let actions = code_actions(&doc, Range::default(), &test_uri());
        assert!(actions.is_empty(), "Valid code should produce no code actions");
    }
}
