use super::*;
use crate::document::DocumentState;

fn make_doc(source: &str) -> DocumentState {
    DocumentState::new(source.to_string(), 1)
}

#[test]
fn completion_returns_items() {
    let doc = make_doc("## Main\n    Let x be 5.\n");
    // Position after the period (end of line 1)
    let pos = Position {
        line: 1,
        character: 18,
    };
    let result = completions(&doc, pos);
    assert!(result.is_some(), "Expected completion response");
    if let Some(CompletionResponse::Array(items)) = result {
        assert!(!items.is_empty(), "Expected non-empty completions");
    }
}

fn labels(resp: &CompletionResponse) -> Vec<String> {
    match resp {
        CompletionResponse::Array(items) => items.iter().map(|i| i.label.clone()).collect(),
        CompletionResponse::List(list) => list.items.iter().map(|i| i.label.clone()).collect(),
    }
}

#[test]
fn module_names_complete_in_an_import_line() {
    use crate::workspace::WorkspaceIndex;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("util.lg"),
        "# Util\n\n## To double (n: Int) -> Int:\n    Return n times 2.\n",
    )
    .unwrap();
    let index = WorkspaceIndex::new();
    index.index_file(&dir.path().join("util.lg"));

    let doc = make_doc("# App\n\nImport \n\n## Main\nShow 1.\n");
    let uri = Url::parse("file:///main.lg").unwrap();
    let pos = Position {
        line: 2,
        character: 7,
    }; // right after `Import `
    let resp = completions_with_workspace(&doc, pos, &index, &uri).unwrap();
    assert!(
        labels(&resp).contains(&"Util".to_string()),
        "module name must complete after Import: {:?}",
        labels(&resp)
    );
}

#[test]
fn imported_symbols_complete_in_code() {
    use crate::workspace::WorkspaceIndex;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("util.lg"),
        "# Util\n\n## To double (n: Int) -> Int:\n    Return n times 2.\n",
    )
    .unwrap();
    let main_path = dir.path().join("main.lg");
    let main_src = "# App\n\nImport Util.\n\n## Main\nShow \n";
    std::fs::write(&main_path, main_src).unwrap();
    let index = WorkspaceIndex::new();
    index.index_file(&dir.path().join("util.lg"));
    index.index_file(&main_path);

    let doc = make_doc(main_src);
    let uri = Url::from_file_path(&main_path).unwrap();
    let pos = Position {
        line: 5,
        character: 5,
    }; // after `Show ` in the Main body
    let resp = completions_with_workspace(&doc, pos, &index, &uri).unwrap();
    assert!(
        labels(&resp).contains(&"double".to_string()),
        "imported symbol must complete: {:?}",
        labels(&resp)
    );
}

#[test]
fn selective_import_completion_offers_only_selected_names() {
    use crate::workspace::WorkspaceIndex;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("util.lg"),
        "# Util\n\n## To double (n: Int) -> Int:\n    Return n times 2.\n\n\
             ## To triple (n: Int) -> Int:\n    Return n times 3.\n",
    )
    .unwrap();
    let main_path = dir.path().join("main.lg");
    // Selective import: only `double` is brought into scope, NOT `triple`.
    let main_src = "# App\n\nImport double from Util.\n\n## Main\nShow \n";
    std::fs::write(&main_path, main_src).unwrap();
    let index = WorkspaceIndex::new();
    index.index_file(&dir.path().join("util.lg"));
    index.index_file(&main_path);

    let doc = make_doc(main_src);
    let uri = Url::from_file_path(&main_path).unwrap();
    let pos = Position {
        line: 5,
        character: 5,
    }; // after `Show `
    let resp = completions_with_workspace(&doc, pos, &index, &uri).unwrap();
    let labels = labels(&resp);
    assert!(
        labels.contains(&"double".to_string()),
        "selected `double` completes: {labels:?}"
    );
    assert!(
        !labels.contains(&"triple".to_string()),
        "unselected `triple` must NOT complete for a selective import: {labels:?}"
    );
}

#[test]
fn completion_includes_keywords_after_period() {
    // Position right after the period on line 1
    // The previous token should be Period, which triggers keyword completions
    let doc = make_doc("## Main\n    Let x be 5.\n    ");
    // Position at start of empty line 2 (after Newline/Indent)
    let pos = Position {
        line: 2,
        character: 4,
    };
    let result = completions(&doc, pos);
    if let Some(CompletionResponse::Array(items)) = result {
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        // After Newline/Indent, should get keywords OR at least fallback with keywords
        let has_keywords = labels.contains(&"Let") || labels.contains(&"Show");
        let has_identifiers = labels.contains(&"x");
        assert!(
            has_keywords || has_identifiers,
            "Should include keywords or identifiers: {:?}",
            labels
        );
    }
}

#[test]
fn completion_includes_variables_in_default_context() {
    let doc = make_doc("## Main\n    Let x be 5.\n    Show x.\n");
    // Position right after "Show" (the "x" token) - the previous token is Show
    // which triggers the default _ path → add_identifier_completions
    let pos = Position {
        line: 2,
        character: 9,
    };
    let result = completions(&doc, pos);
    if let Some(CompletionResponse::Array(items)) = result {
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(
            labels.contains(&"x"),
            "Should include variable 'x': {:?}",
            labels
        );
    }
}

#[test]
fn completion_no_crash_empty_doc() {
    let doc = make_doc("");
    let pos = Position {
        line: 0,
        character: 0,
    };
    let result = completions(&doc, pos);
    // Empty doc has no prev_token, so completions returns None
    // OR if lexer emits an EOF token, it will still return keywords via fallback
    // Either way, should not panic
    match result {
        None => {} // OK
        Some(CompletionResponse::Array(items)) => {
            // Keywords are always valid as a fallback
            assert!(!items.is_empty(), "If result is Some, it should have items");
        }
        _ => panic!("Unexpected response type"),
    }
}

#[test]
fn completion_before_the_first_token_still_offers_keywords() {
    // Cursor at the very start, before any token ends: the old code
    // short-circuited to None (no completions at all). It must fall through
    // to the keyword/stdlib fallback so a fresh file is never a dead end.
    let doc = make_doc("Show x.\n");
    let pos = Position {
        line: 0,
        character: 0,
    };
    let resp = completions(&doc, pos).expect("completion before the first token still works");
    let labels = labels(&resp);
    assert!(
        labels.iter().any(|l| l == "Let" || l == "Show"),
        "offers statement keywords before the first token: {labels:?}"
    );
}

#[test]
fn completion_before_the_first_token_offers_keywords_even_when_other_items_exist() {
    // A cursor before the first token is a statement start regardless of what else the document
    // already has to offer there. This document names `HttpError` (so `add_identifier_completions`
    // does NOT come back empty at this cursor) — keywords must not depend on it doing so to be
    // reached, or a future item crowding this same list would silently drop them again.
    let src =
        "## To describe (e: HttpError) -> Text:\n    Return e's message.\n\n## Main\nShow x.\n";
    let doc = make_doc(src);
    let pos = Position {
        line: 0,
        character: 0,
    };
    let resp = completions(&doc, pos).expect("completion before the first token still works");
    let labels = labels(&resp);
    assert!(
        labels.iter().any(|l| l == "HttpError"),
        "fixture check: this document must already offer a non-keyword item at this cursor: {labels:?}"
    );
    assert!(
        labels.iter().any(|l| l == "Let" || l == "Show"),
        "offers statement keywords before the first token even though other items already exist: {labels:?}"
    );
}

/// `HttpError (struct)` / `File (struct)` — the `detail` `index_type_registry` gives a builtin
/// struct it indexed from `doc.symbol_index.definitions`, distinct from the SAME name's own
/// always-on stdlib teaching entry (`add_stdlib_completions`, whose `detail` is its `## Note`
/// signature line) — a real concept authors may type a type annotation for, offered alongside
/// `Request`/`Response`/`Json` regardless of use, exactly as those are. Only the INDEXED form
/// crowded the fallback that used to drop keywords, so only it is what this checks.
fn indexed_struct_details(resp: &CompletionResponse) -> Vec<String> {
    let items: &[CompletionItem] = match resp {
        CompletionResponse::Array(items) => items,
        CompletionResponse::List(list) => &list.items,
    };
    items
        .iter()
        .filter_map(|i| i.detail.clone())
        .filter(|d| d == "HttpError (struct)" || d == "File (struct)")
        .collect()
}

#[test]
fn completion_before_the_first_token_does_not_index_an_unused_builtin_struct() {
    // `HttpError` and `File` are registered in EVERY document's type registry
    // (`TypeRegistry::with_primitives`) whether this program ever names them or not — indexing
    // them unconditionally used to crowd a fresh file's very first completion with two structs
    // (and their fields) most pages never touch.
    let doc = make_doc("Show x.\n");
    let pos = Position {
        line: 0,
        character: 0,
    };
    let resp = completions(&doc, pos).expect("completion before the first token still works");
    assert!(
        indexed_struct_details(&resp).is_empty(),
        "must not index a compiler-registered builtin struct this program never names: {:?}",
        indexed_struct_details(&resp)
    );
}

#[test]
fn completion_indexes_a_builtin_struct_the_program_actually_names() {
    let src =
        "## To describe (e: HttpError) -> Text:\n    Return e's message.\n\n## Main\nShow x.\n";
    let doc = make_doc(src);
    let pos = Position {
        line: 4,
        character: 4,
    };
    let resp = completions(&doc, pos).expect("completion still works");
    assert_eq!(
        indexed_struct_details(&resp),
        vec!["HttpError (struct)".to_string()],
        "a builtin struct the program actually names must still be indexed"
    );
}

#[test]
fn completion_after_colon_type_completions() {
    let doc = make_doc("## Main\n    Let x: Int be 5.\n");
    // Position after the colon — find the colon's position
    // "Let x:" → colon at character 9, so after it at character 10
    let pos = Position {
        line: 1,
        character: 10,
    };
    let result = completions(&doc, pos);
    if let Some(CompletionResponse::Array(items)) = result {
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        let has_types =
            labels.contains(&"Int") || labels.contains(&"Text") || labels.contains(&"Bool");
        assert!(
            has_types,
            "After colon should include type completions: {:?}",
            labels
        );
    }
}

#[test]
fn completion_items_have_correct_kind() {
    let doc = make_doc("## Main\n    Let x be 5.\n    ");
    let pos = Position {
        line: 2,
        character: 4,
    };
    let result = completions(&doc, pos);
    if let Some(CompletionResponse::Array(items)) = result {
        let keyword_items: Vec<_> = items
            .iter()
            .filter(|i| i.label == "Let" || i.label == "Show")
            .collect();
        for item in &keyword_items {
            assert_eq!(
                item.kind,
                Some(CompletionItemKind::KEYWORD),
                "'{}' should have KEYWORD kind",
                item.label
            );
        }
    }
}

#[test]
fn completion_snippets_have_snippet_format() {
    let doc = make_doc("## Main\n    ");
    let pos = Position {
        line: 1,
        character: 4,
    };
    let result = completions(&doc, pos);
    if let Some(CompletionResponse::Array(items)) = result {
        let keyword_items: Vec<_> = items
            .iter()
            .filter(|i| i.label == "Let" || i.label == "If")
            .collect();
        for item in &keyword_items {
            assert_eq!(
                item.insert_text_format,
                Some(lsp_types::InsertTextFormat::SNIPPET),
                "'{}' should have SNIPPET format",
                item.label
            );
        }
    }
}

#[test]
fn keyword_completions_teach_with_detail_and_documentation() {
    let doc = make_doc("## Main\n    ");
    let pos = Position {
        line: 1,
        character: 4,
    };
    let Some(CompletionResponse::Array(items)) = completions(&doc, pos) else {
        panic!("expected completions after indent");
    };
    for label in [
        "Let", "Set", "If", "While", "Repeat", "Return", "Show", "Give", "Push", "Call", "Inspect",
    ] {
        let item = items
            .iter()
            .find(|i| i.label == label && i.kind == Some(CompletionItemKind::KEYWORD))
            .unwrap_or_else(|| panic!("keyword completion for {label}"));
        let lesson = logicaffeine_language::teach::doc_for_word(label)
            .unwrap_or_else(|| panic!("{label} must be taught"));
        assert_eq!(
            item.detail.as_deref(),
            Some(lesson.what),
            "{label}: completion detail must be the lesson's one-liner"
        );
        assert!(
            item.documentation.is_some(),
            "{label}: completion must carry teaching documentation"
        );
    }
}

#[test]
fn primitive_type_completions_teach_from_the_lesson_table() {
    let doc = make_doc("## Main\n    Let x: Int be 5.\n");
    // Position right after the colon.
    let pos = Position {
        line: 1,
        character: 10,
    };
    let Some(CompletionResponse::Array(items)) = completions(&doc, pos) else {
        panic!("expected type completions after colon");
    };
    let int_item = items
        .iter()
        .find(|i| i.label == "Int")
        .expect("Int must be offered after a colon");
    let lesson = logicaffeine_language::teach::doc_for_primitive("Int").unwrap();
    assert_eq!(
        int_item.detail.as_deref(),
        Some(lesson.what),
        "Int detail must come from the lesson table, not a generic placeholder"
    );
    assert!(
        int_item.documentation.is_some(),
        "Int completion must carry teaching documentation"
    );
}

#[test]
fn documented_function_completion_carries_its_prose() {
    let doc = make_doc(
        "## Note\nDoubles a number.\n\n## To double (n: Int) -> Int:\n    Return n * 2.\n\n## Main\nShow x.\n",
    );
    // Default context after "Show".
    let pos = Position {
        line: 7,
        character: 5,
    };
    let Some(CompletionResponse::Array(items)) = completions(&doc, pos) else {
        panic!("expected completions");
    };
    let item = items
        .iter()
        .find(|i| i.label == "double")
        .expect("double completes");
    let Some(lsp_types::Documentation::MarkupContent(content)) = &item.documentation else {
        panic!("documented function must carry markdown docs");
    };
    assert!(
        content.value.contains("Doubles a number."),
        "{}",
        content.value
    );
}

#[test]
fn stdlib_names_complete_with_their_documentation() {
    let doc = make_doc("## Main\n    Show x.\n");
    let pos = Position {
        line: 1,
        character: 9,
    };
    let Some(CompletionResponse::Array(items)) = completions(&doc, pos) else {
        panic!("expected completions");
    };
    let md5 = items
        .iter()
        .find(|i| i.label == "md5")
        .expect("md5 offered from stdlib");
    assert_eq!(md5.kind, Some(CompletionItemKind::FUNCTION));
    assert!(
        md5.detail.as_deref().is_some_and(|d| d.contains("md5")),
        "{:?}",
        md5.detail
    );
    assert!(md5.documentation.is_some(), "stdlib completion teaches");
}

#[test]
fn variant_completion_no_crash_with_inspect() {
    // After "Inspect" keyword, should not crash even without a defined enum
    let doc = make_doc("## Main\n    Let x be 5.\n    Inspect x:\n");
    let pos = Position {
        line: 2,
        character: 14,
    };
    let result = completions(&doc, pos);
    // Should return something (even if empty or keywords) but not panic
    assert!(
        result.is_some(),
        "Should return some completions after Inspect"
    );
}

#[test]
fn completion_moved_variable_has_deprecated_tag() {
    use logicaffeine_compile::analysis::VarState;
    let mut doc =
        make_doc("## Main\n    Let x be 5.\n    Let y be 0.\n    Give x to y.\n    Show x.\n");
    // Ensure x is marked as Moved
    doc.ownership_states
        .insert("x".to_string(), VarState::Moved);
    // Find completions in some context
    let pos = Position {
        line: 4,
        character: 9,
    };
    let result = completions(&doc, pos);
    if let Some(CompletionResponse::Array(items)) = result {
        let x_items: Vec<_> = items.iter().filter(|i| i.label == "x").collect();
        if !x_items.is_empty() {
            assert_eq!(
                x_items[0].deprecated,
                Some(true),
                "Moved variable 'x' should be marked deprecated"
            );
            assert!(
                x_items[0]
                    .tags
                    .as_ref()
                    .map_or(false, |t| t.contains(&CompletionItemTag::DEPRECATED)),
                "Moved variable should have DEPRECATED tag"
            );
            assert!(
                x_items[0]
                    .detail
                    .as_ref()
                    .map_or(false, |d| d.contains("(moved)")),
                "Moved variable detail should indicate moved: {:?}",
                x_items[0].detail
            );
        }
    }
}

#[test]
fn completion_owned_variable_no_tag() {
    let doc = make_doc("## Main\n    Let x be 5.\n    Show x.\n");
    let pos = Position {
        line: 2,
        character: 9,
    };
    let result = completions(&doc, pos);
    if let Some(CompletionResponse::Array(items)) = result {
        let x_items: Vec<_> = items.iter().filter(|i| i.label == "x").collect();
        if !x_items.is_empty() {
            assert!(
                x_items[0].deprecated.is_none() || x_items[0].deprecated == Some(false),
                "Owned variable should not be deprecated"
            );
        }
    }
}
