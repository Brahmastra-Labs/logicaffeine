use super::*;
use crate::document::DocumentState;
use crate::workspace::ModuleExport;

fn make_doc(source: &str) -> DocumentState {
    DocumentState::new(source.to_string(), 1)
}

/// A hand-built workspace view: `exports` are (name, module) pairs; `imports`
/// are the module aliases the file already imports.
struct FakeWs {
    exports: Vec<(&'static str, &'static str)>,
    imports: Vec<&'static str>,
}
impl WorkspaceView for FakeWs {
    fn module_exporting(&self, name: &str) -> Vec<ModuleExport> {
        self.exports
            .iter()
            .filter(|(n, _)| *n == name)
            .map(|(_, m)| ModuleExport {
                module_url: Url::parse("file:///x.lg").unwrap(),
                module_name: (*m).to_string(),
            })
            .collect()
    }
    fn imports_of(&self, _uri: &Url) -> Vec<String> {
        self.imports.iter().map(|s| s.to_string()).collect()
    }
    fn module_names(&self) -> Vec<(String, Url)> {
        Vec::new()
    }
}

fn action_titles(actions: &[CodeActionOrCommand]) -> Vec<String> {
    actions
        .iter()
        .filter_map(|a| match a {
            CodeActionOrCommand::CodeAction(ca) => Some(ca.title.clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn missing_import_offers_import_action() {
    let doc = make_doc("# App\n\n## Main\nShow double(21).\n");
    let ws = FakeWs {
        exports: vec![("double", "Util")],
        imports: vec![],
    };
    let uri = Url::parse("file:///main.lg").unwrap();
    let actions = import_code_actions(&doc, Range::default(), &uri, &ws);
    assert!(
        action_titles(&actions).contains(&"Import Util".to_string()),
        "{:?}",
        action_titles(&actions)
    );
}

#[test]
fn import_inserted_after_an_existing_import() {
    let doc = make_doc("# App\n\nImport Geo.\n\n## Main\nShow double(21).\n");
    let ws = FakeWs {
        exports: vec![("double", "Util")],
        imports: vec![],
    };
    let uri = Url::parse("file:///main.lg").unwrap();
    let edit = import_code_actions(&doc, Range::default(), &uri, &ws)
        .iter()
        .find_map(|a| match a {
            CodeActionOrCommand::CodeAction(ca) if ca.title == "Import Util" => ca
                .edit
                .as_ref()?
                .changes
                .as_ref()?
                .get(&uri)
                .and_then(|e| e.first())
                .cloned(),
            _ => None,
        })
        .expect("Import Util action");
    // `Import Geo.` is line 2 → the new import lands on line 3, not inside a block.
    assert_eq!(edit.range.start.line, 3, "insert AFTER the existing import");
    assert!(
        edit.new_text.starts_with("Import Util."),
        "{}",
        edit.new_text
    );
}

#[test]
fn no_import_action_when_already_imported() {
    let doc = make_doc("# App\n\nImport Util.\n\n## Main\nShow double(21).\n");
    let ws = FakeWs {
        exports: vec![("double", "Util")],
        imports: vec!["Util"],
    };
    let uri = Url::parse("file:///main.lg").unwrap();
    let actions = import_code_actions(&doc, Range::default(), &uri, &ws);
    assert!(
        action_titles(&actions).is_empty(),
        "already-imported name needs no action: {:?}",
        action_titles(&actions)
    );
}

#[test]
fn ambiguous_name_offers_each_module() {
    let doc = make_doc("# App\n\n## Main\nShow helper(21).\n");
    let ws = FakeWs {
        exports: vec![("helper", "A"), ("helper", "B")],
        imports: vec![],
    };
    let uri = Url::parse("file:///main.lg").unwrap();
    let titles = action_titles(&import_code_actions(&doc, Range::default(), &uri, &ws));
    assert!(
        titles.contains(&"Import A".to_string()) && titles.contains(&"Import B".to_string()),
        "{titles:?}"
    );
}

#[test]
fn no_import_action_for_a_name_no_module_exports() {
    let doc = make_doc("# App\n\n## Main\nShow mystery(21).\n");
    let ws = FakeWs {
        exports: vec![("double", "Util")],
        imports: vec![],
    };
    let uri = Url::parse("file:///main.lg").unwrap();
    assert!(action_titles(&import_code_actions(&doc, Range::default(), &uri, &ws)).is_empty());
}

/// Apply an insert edit at its `line:0` position (the shape auto-import emits).
fn apply_insert(source: &str, pos: Position, text: &str) -> String {
    let mut offset = 0usize;
    for (i, line) in source.split_inclusive('\n').enumerate() {
        if i as u32 == pos.line {
            break;
        }
        offset += line.len();
    }
    let mut out = source.to_string();
    out.insert_str(offset.min(out.len()), text);
    out
}

#[test]
fn unused_import_offers_remove_action() {
    use crate::workspace::WorkspaceIndex;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("util.lg"),
        "# Util\n\n## To double (n: Int) -> Int:\n    Return n times 2.\n",
    )
    .unwrap();
    let main_path = dir.path().join("main.lg");
    let main_src = "# App\n\nImport Util.\n\n## Main\nShow 42.\n"; // imports Util, never uses it
    std::fs::write(&main_path, main_src).unwrap();
    let index = WorkspaceIndex::new();
    index.index_file(&dir.path().join("util.lg"));
    index.index_file(&main_path);

    let doc = make_doc(main_src);
    let uri = Url::from_file_path(&main_path).unwrap();
    let titles = action_titles(&unused_import_actions(&doc, Range::default(), &uri, &index));
    assert!(
        titles.iter().any(|t| t.contains("Remove unused import")),
        "{titles:?}"
    );
}

#[test]
fn used_import_has_no_remove_action() {
    use crate::workspace::WorkspaceIndex;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("util.lg"),
        "# Util\n\n## To double (n: Int) -> Int:\n    Return n times 2.\n",
    )
    .unwrap();
    let main_path = dir.path().join("main.lg");
    let main_src = "# App\n\nImport Util.\n\n## Main\nShow double(21).\n"; // uses double
    std::fs::write(&main_path, main_src).unwrap();
    let index = WorkspaceIndex::new();
    index.index_file(&dir.path().join("util.lg"));
    index.index_file(&main_path);

    let doc = make_doc(main_src);
    let uri = Url::from_file_path(&main_path).unwrap();
    assert!(action_titles(&unused_import_actions(&doc, Range::default(), &uri, &index)).is_empty());
}

/// CAPSTONE (cross-brain correctness): the auto-import quickfix is not just
/// plausible — APPLYING its edit makes the real assembler succeed. Uses a real
/// `WorkspaceIndex` over on-disk files and the real `assemble_project`.
#[test]
fn applying_the_auto_import_makes_assembly_succeed() {
    use crate::workspace::WorkspaceIndex;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("util.lg"),
        "# Util\n\n## To double (n: Int) -> Int:\n    Return n times 2.\n",
    )
    .unwrap();
    // main uses `double` but does NOT import Util → currently unresolved.
    let main_path = dir.path().join("main.lg");
    let main_src = "# App\n\n## Main\nShow double(21).\n";
    std::fs::write(&main_path, main_src).unwrap();

    // Before the fix, the assembler builds but `double` is undefined at run.
    // The auto-import must offer exactly the fix that resolves it.
    let index = WorkspaceIndex::new();
    index.index_file(&dir.path().join("util.lg"));
    index.index_file(&main_path);

    let doc = make_doc(main_src);
    let uri = Url::from_file_path(&main_path).unwrap();
    let actions = import_code_actions(&doc, Range::default(), &uri, &index);

    // Find the "Import Util" action and pull out its single insert edit.
    let edit = actions.iter().find_map(|a| match a {
        CodeActionOrCommand::CodeAction(ca) if ca.title == "Import Util" => ca
            .edit
            .as_ref()?
            .changes
            .as_ref()?
            .get(&uri)
            .and_then(|e| e.first())
            .cloned(),
        _ => None,
    });
    let edit = edit.expect("the auto-import must offer `Import Util`");

    // Apply the edit and write it back — the assembler must now resolve `double`.
    let fixed = apply_insert(main_src, edit.range.start, &edit.new_text);
    assert!(
        fixed.contains("Import Util."),
        "the fix inserts the import: {fixed}"
    );
    std::fs::write(&main_path, &fixed).unwrap();
    let assembled = logicaffeine_compile::analysis::assemble_project(&main_path)
        .expect("after applying the auto-import, the project assembles");
    assert!(
        assembled.source.contains("## To double"),
        "the imported module's definition is now spliced in: {}",
        assembled.source
    );
}

fn test_uri() -> lsp_types::Url {
    lsp_types::Url::parse("file:///test.logos").unwrap()
}

#[test]
fn code_actions_returns_empty_for_valid_code() {
    let doc = make_doc("## Main\n    Let x be 5.\n    Show x.\n");
    let range = Range::default();
    let actions = code_actions(&doc, range, &test_uri());
    assert!(
        doc.diagnostics.is_empty(),
        "Valid code should have no diagnostics"
    );
    assert!(actions.is_empty(), "No diagnostics → no actions");
}

#[test]
fn code_actions_no_crash_on_syntax_error() {
    let doc = make_doc("## Main\n    Let be.\n");
    let range = Range::default();
    let actions = code_actions(&doc, range, &test_uri());
    // Should not panic on syntax errors — actions may or may not be produced
    assert!(
        actions.len() < 100,
        "Unreasonable number of actions for a tiny doc"
    );
}

#[test]
fn code_actions_empty_for_empty_doc() {
    let doc = make_doc("");
    let range = Range::default();
    let actions = code_actions(&doc, range, &test_uri());
    assert!(actions.is_empty(), "Empty doc should have no actions");
}

fn make_doc_with_diagnostic(source: &str, diag_code: &str, range: Range) -> DocumentState {
    use lsp_types::{Diagnostic, DiagnosticSeverity, NumberOrString};
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
        start: lsp_types::Position {
            line,
            character: start,
        },
        end: lsp_types::Position {
            line,
            character: end,
        },
    }
}

#[test]
fn code_action_for_escape_return_suggests_copy() {
    let source = "## Main\n    Return x.\n";
    let r = make_range(1, 11, 12); // "x" at position
    let doc = make_doc_with_diagnostic(source, "escape-return", r);
    let actions = code_actions(&doc, Range::default(), &test_uri());
    let escape_actions: Vec<_> = actions
        .iter()
        .filter(|a| match a {
            CodeActionOrCommand::CodeAction(ca) => ca.title.contains("Copy before returning"),
            _ => false,
        })
        .collect();
    assert!(
        !escape_actions.is_empty(),
        "Should have 'Copy before returning' action. Got: {:?}",
        actions
            .iter()
            .map(|a| match a {
                CodeActionOrCommand::CodeAction(ca) => ca.title.clone(),
                _ => "command".to_string(),
            })
            .collect::<Vec<_>>()
    );
}

#[test]
fn code_action_for_zero_index_suggests_one() {
    let source = "## Main\n    Let x be items at 0.\n";
    let r = make_range(1, 29, 30); // "0"
    let doc = make_doc_with_diagnostic(source, "zero-index", r);
    let actions = code_actions(&doc, Range::default(), &test_uri());
    let zero_actions: Vec<_> = actions
        .iter()
        .filter(|a| match a {
            CodeActionOrCommand::CodeAction(ca) => ca.title == "Use 1-based indexing",
            _ => false,
        })
        .collect();
    assert!(
        !zero_actions.is_empty(),
        "Should have 'Use 1-based indexing' action"
    );
}

#[test]
fn use_after_move_fix_names_the_moved_variable_not_the_span_text() {
    // Real pipeline: the parse-path diagnostic's range can sit on the
    // statement's period — the fix must still name and target 'x'.
    let doc = make_doc("## Main\nLet x be 5.\nLet a be 0.\nGive x to a.\nShow x.\n");
    let actions = code_actions(&doc, Range::default(), &test_uri());
    let copy_fix = actions
        .iter()
        .find_map(|a| match a {
            CodeActionOrCommand::CodeAction(ca) if ca.title.contains("copy of") => Some(ca),
            _ => None,
        })
        .expect("use-after-move offers the copy fix");
    assert_eq!(copy_fix.title, "Use 'a copy of x' instead");

    let edits = copy_fix
        .edit
        .as_ref()
        .and_then(|e| e.changes.as_ref())
        .and_then(|c| c.values().next())
        .expect("the fix edits the document");
    assert_eq!(edits[0].new_text, "a copy of x");
    let start = doc.line_index.offset(edits[0].range.start);
    let end = doc.line_index.offset(edits[0].range.end);
    assert_eq!(
        &doc.source[start..end],
        "x",
        "the edit must replace the variable, never stray punctuation"
    );
}

#[test]
fn spelling_suggestions_never_fire_on_coded_diagnostics() {
    // A moved `x` must not draw "Did you mean 'a'?" — coded diagnostics
    // are about semantics, not spelling.
    let doc = make_doc("## Main\nLet x be 5.\nLet a be 0.\nGive x to a.\nShow x.\n");
    let actions = code_actions(&doc, Range::default(), &test_uri());
    for action in &actions {
        if let CodeActionOrCommand::CodeAction(ca) = action {
            assert!(
                !ca.title.starts_with("Did you mean"),
                "spelling noise on a coded diagnostic: {}",
                ca.title
            );
        }
    }
}

#[test]
fn code_action_for_double_move_suggests_copy() {
    let source = "## Main\n    Let x be 5.\n    Let y be 0.\n    Give x to y.\n    Give x to y.\n";
    let r = make_range(4, 9, 10); // second "x"
    let doc = make_doc_with_diagnostic(source, "double-move", r);
    let actions = code_actions(&doc, Range::default(), &test_uri());
    let dm_actions: Vec<_> = actions
        .iter()
        .filter(|a| match a {
            CodeActionOrCommand::CodeAction(ca) => ca.title.contains("copy of"),
            _ => false,
        })
        .collect();
    assert!(
        !dm_actions.is_empty(),
        "Should have 'Give a copy of' action"
    );
}

#[test]
fn code_action_escape_assignment_suggests_copy() {
    let source = "## Main\n    Set outer to x.\n";
    let r = make_range(1, 18, 19); // "x"
    let doc = make_doc_with_diagnostic(source, "escape-assignment", r);
    let actions = code_actions(&doc, Range::default(), &test_uri());
    let ea_actions: Vec<_> = actions
        .iter()
        .filter(|a| match a {
            CodeActionOrCommand::CodeAction(ca) => ca.title == "Copy before assignment",
            _ => false,
        })
        .collect();
    assert!(
        !ea_actions.is_empty(),
        "Should have 'Copy before assignment' action"
    );
}

#[test]
fn no_code_action_for_valid_code() {
    let doc = make_doc("## Main\n    Let x be 5.\n    Show x.\n");
    let actions = code_actions(&doc, Range::default(), &test_uri());
    assert!(
        actions.is_empty(),
        "Valid code should produce no code actions"
    );
}
