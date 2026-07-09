use tower_lsp::lsp_types::{
    CompletionItem, CompletionItemKind, CompletionItemTag, CompletionResponse, Position,
};

use logicaffeine_compile::analysis::VarState;
use logicaffeine_language::token::TokenType;

use crate::document::DocumentState;
use crate::index::DefinitionKind;

/// Handle completion request.
///
/// Provides context-aware completions based on the preceding tokens.
pub fn completions(doc: &DocumentState, position: Position) -> Option<CompletionResponse> {
    let offset = doc.line_index.offset(position);

    // Find the preceding token to determine context
    let prev_token = doc
        .tokens
        .iter()
        .rev()
        .find(|t| t.span.end <= offset)?;

    let mut items = Vec::new();

    match &prev_token.kind {
        // After period or at start of line → statement keywords
        TokenType::Period | TokenType::Indent | TokenType::Newline => {
            add_statement_keywords(&mut items);
        }

        // After "Let x be" or similar → expression completions
        TokenType::Be => {
            add_expression_completions(doc, &mut items);
        }

        // After ":" → type completions
        TokenType::Colon => {
            add_type_completions(doc, &mut items);
        }

        // After possessive ('s) → field completions
        TokenType::Possessive => {
            // Look back to find the object before possessive
            add_field_completions(doc, offset, &mut items);
        }

        // After "Inspect x:" → variant completions
        TokenType::Inspect => {
            add_variant_completions(doc, &mut items);
        }

        // Default → variables and functions in scope
        _ => {
            add_identifier_completions(doc, &mut items);
        }
    }

    if items.is_empty() {
        // Fallback: always offer identifiers + keywords
        add_identifier_completions(doc, &mut items);
        add_statement_keywords(&mut items);
    }

    // Stdlib prelude names ride along in every context, after the locals —
    // additive, so they can never displace the context-specific items above.
    add_stdlib_completions(doc, &mut items);

    Some(CompletionResponse::Array(items))
}

fn add_statement_keywords(items: &mut Vec<CompletionItem>) {
    // Snippet skeletons are editing mechanics and live here; the teaching
    // text (detail + documentation) comes from the shared lesson table, so
    // completion, hover, and the REPL always say the same thing.
    let keywords = [
        (TokenType::Let, "Let ${1:name} be ${2:value}."),
        (TokenType::Set, "Set ${1:name} to ${2:value}."),
        (TokenType::If, "If ${1:condition}:\n    ${2:body}"),
        (TokenType::While, "While ${1:condition}:\n    ${2:body}"),
        (TokenType::Repeat, "Repeat for ${1:item} in ${2:collection}:\n    ${3:body}"),
        (TokenType::Return, "Return ${1:value}."),
        (TokenType::Show, "Show ${1:value}."),
        (TokenType::Give, "Give ${1:value} to ${2:target}."),
        (TokenType::Push, "Push ${1:value} to ${2:list}."),
        (TokenType::Call, "Call ${1:function} with ${2:args}."),
        (TokenType::Inspect, "Inspect ${1:value}:\n    ${2:pattern}:\n        ${3:body}"),
    ];

    for (kind, snippet) in keywords {
        let lesson = logicaffeine_language::teach::doc_for(&kind)
            .expect("every statement-keyword completion is taught");
        items.push(CompletionItem {
            label: lesson.name.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some(lesson.what.to_string()),
            documentation: Some(crate::teach_md::completion_docs(lesson)),
            insert_text: Some(snippet.to_string()),
            insert_text_format: Some(tower_lsp::lsp_types::InsertTextFormat::SNIPPET),
            ..Default::default()
        });
    }
}

fn add_expression_completions(doc: &DocumentState, items: &mut Vec<CompletionItem>) {
    // Add variables in scope
    add_identifier_completions(doc, items);

    // Add "a new Type" constructor
    for (name, typedef) in doc.type_registry.iter_types() {
        let type_name = doc.interner.resolve(*name);
        if matches!(typedef, logicaffeine_language::analysis::TypeDef::Struct { .. }) {
            items.push(CompletionItem {
                label: format!("a new {}", type_name),
                kind: Some(CompletionItemKind::CONSTRUCTOR),
                detail: Some(format!("Create a new {} instance", type_name)),
                ..Default::default()
            });
        }
    }
}

fn add_type_completions(doc: &DocumentState, items: &mut Vec<CompletionItem>) {
    // Primitive types — each teaches from the shared lesson table.
    let primitives = ["Int", "Nat", "Text", "Bool", "Float", "Unit", "Char", "Byte"];
    for prim in primitives {
        let lesson = logicaffeine_language::teach::doc_for_primitive(prim)
            .expect("every offered primitive is taught");
        items.push(CompletionItem {
            label: prim.to_string(),
            kind: Some(CompletionItemKind::TYPE_PARAMETER),
            detail: Some(lesson.what.to_string()),
            documentation: Some(crate::teach_md::completion_docs(lesson)),
            ..Default::default()
        });
    }

    // Generic types — same table.
    let generics = ["List", "Seq", "Map", "Set", "Option", "Result"];
    for gen in generics {
        let lesson = logicaffeine_language::teach::doc_for_primitive(gen)
            .expect("every offered generic is taught");
        items.push(CompletionItem {
            label: gen.to_string(),
            kind: Some(CompletionItemKind::CLASS),
            detail: Some(lesson.what.to_string()),
            documentation: Some(crate::teach_md::completion_docs(lesson)),
            ..Default::default()
        });
    }

    // User-defined types from registry
    for (name, _typedef) in doc.type_registry.iter_types() {
        let type_name = doc.interner.resolve(*name);
        items.push(CompletionItem {
            label: type_name.to_string(),
            kind: Some(CompletionItemKind::CLASS),
            detail: Some("User-defined type".to_string()),
            ..Default::default()
        });
    }
}

fn add_field_completions(doc: &DocumentState, offset: usize, items: &mut Vec<CompletionItem>) {
    // Try to resolve the type of the object before the possessive
    if let Some(type_name) = resolve_possessive_type(doc, offset) {
        // Only offer fields from the resolved type
        for (name, typedef) in doc.type_registry.iter_types() {
            let tname = doc.interner.resolve(*name);
            if tname != type_name {
                continue;
            }
            if let logicaffeine_language::analysis::TypeDef::Struct { fields, .. } = typedef {
                for field in fields {
                    let field_name = doc.interner.resolve(field.name);
                    items.push(CompletionItem {
                        label: field_name.to_string(),
                        kind: Some(CompletionItemKind::FIELD),
                        detail: Some(format!("{}: {}", field_name, crate::hover::format_field_type(&field.ty, &doc.interner))),
                        ..Default::default()
                    });
                }
            }
        }
    }

    // Fall back: if no fields were added, show all fields from all structs
    if items.iter().all(|i| i.kind != Some(CompletionItemKind::FIELD)) {
        for (_name, typedef) in doc.type_registry.iter_types() {
            if let logicaffeine_language::analysis::TypeDef::Struct { fields, .. } = typedef {
                for field in fields {
                    let field_name = doc.interner.resolve(field.name);
                    items.push(CompletionItem {
                        label: field_name.to_string(),
                        kind: Some(CompletionItemKind::FIELD),
                        detail: Some(format!("{}", field_name)),
                        ..Default::default()
                    });
                }
            }
        }
    }
}

/// Look back from the possessive token to find the object's type name.
fn resolve_possessive_type(doc: &DocumentState, offset: usize) -> Option<String> {
    // Find the token right before the possessive
    let prev_token = doc
        .tokens
        .iter()
        .rev()
        .find(|t| t.span.end <= offset && !matches!(t.kind, TokenType::Possessive))?;
    let var_name = doc.source.get(prev_token.span.start..prev_token.span.end)?;

    // Look up the variable's definition to find its type
    let defs = doc.symbol_index.definitions_of(var_name);
    let def = defs.first()?;

    // Extract type from detail string like "Let x: Point" or "Let x: Point: inferred"
    let detail = def.detail.as_ref()?;
    // Match pattern "Let name: TypeName" or "name: TypeName"
    let after_colon = detail.rsplit_once(": ")?;
    let type_name = after_colon.1.trim();
    // Strip trailing ": inferred" or other suffixes
    let type_name = type_name.split(':').next().unwrap_or(type_name).trim();
    if type_name.is_empty() || type_name == "inferred" {
        return None;
    }
    Some(type_name.to_string())
}

fn add_variant_completions(doc: &DocumentState, items: &mut Vec<CompletionItem>) {
    // Fall back to showing all variants from all enums (type-aware filtering
    // would require resolving the Inspect target, which we can do in the future)
    for (_name, typedef) in doc.type_registry.iter_types() {
        if let logicaffeine_language::analysis::TypeDef::Enum { variants, .. } = typedef {
            for variant in variants {
                let variant_name = doc.interner.resolve(variant.name);
                items.push(CompletionItem {
                    label: variant_name.to_string(),
                    kind: Some(CompletionItemKind::ENUM_MEMBER),
                    ..Default::default()
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::DocumentState;

    fn make_doc(source: &str) -> DocumentState {
        DocumentState::new(source.to_string(), 1)
    }

    #[test]
    fn completion_returns_items() {
        let doc = make_doc("## Main\n    Let x be 5.\n");
        // Position after the period (end of line 1)
        let pos = Position { line: 1, character: 18 };
        let result = completions(&doc, pos);
        assert!(result.is_some(), "Expected completion response");
        if let Some(CompletionResponse::Array(items)) = result {
            assert!(!items.is_empty(), "Expected non-empty completions");
        }
    }

    #[test]
    fn completion_includes_keywords_after_period() {
        // Position right after the period on line 1
        // The previous token should be Period, which triggers keyword completions
        let doc = make_doc("## Main\n    Let x be 5.\n    ");
        // Position at start of empty line 2 (after Newline/Indent)
        let pos = Position { line: 2, character: 4 };
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
        let pos = Position { line: 2, character: 9 };
        let result = completions(&doc, pos);
        if let Some(CompletionResponse::Array(items)) = result {
            let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
            assert!(labels.contains(&"x"), "Should include variable 'x': {:?}", labels);
        }
    }

    #[test]
    fn completion_no_crash_empty_doc() {
        let doc = make_doc("");
        let pos = Position { line: 0, character: 0 };
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
    fn completion_after_colon_type_completions() {
        let doc = make_doc("## Main\n    Let x: Int be 5.\n");
        // Position after the colon — find the colon's position
        // "Let x:" → colon at character 9, so after it at character 10
        let pos = Position { line: 1, character: 10 };
        let result = completions(&doc, pos);
        if let Some(CompletionResponse::Array(items)) = result {
            let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
            let has_types = labels.contains(&"Int") || labels.contains(&"Text") || labels.contains(&"Bool");
            assert!(has_types, "After colon should include type completions: {:?}", labels);
        }
    }

    #[test]
    fn completion_items_have_correct_kind() {
        let doc = make_doc("## Main\n    Let x be 5.\n    ");
        let pos = Position { line: 2, character: 4 };
        let result = completions(&doc, pos);
        if let Some(CompletionResponse::Array(items)) = result {
            let keyword_items: Vec<_> = items.iter()
                .filter(|i| i.label == "Let" || i.label == "Show")
                .collect();
            for item in &keyword_items {
                assert_eq!(item.kind, Some(CompletionItemKind::KEYWORD),
                    "'{}' should have KEYWORD kind", item.label);
            }
        }
    }

    #[test]
    fn completion_snippets_have_snippet_format() {
        let doc = make_doc("## Main\n    ");
        let pos = Position { line: 1, character: 4 };
        let result = completions(&doc, pos);
        if let Some(CompletionResponse::Array(items)) = result {
            let keyword_items: Vec<_> = items.iter()
                .filter(|i| i.label == "Let" || i.label == "If")
                .collect();
            for item in &keyword_items {
                assert_eq!(item.insert_text_format,
                    Some(tower_lsp::lsp_types::InsertTextFormat::SNIPPET),
                    "'{}' should have SNIPPET format", item.label);
            }
        }
    }

    #[test]
    fn keyword_completions_teach_with_detail_and_documentation() {
        let doc = make_doc("## Main\n    ");
        let pos = Position { line: 1, character: 4 };
        let Some(CompletionResponse::Array(items)) = completions(&doc, pos) else {
            panic!("expected completions after indent");
        };
        for label in [
            "Let", "Set", "If", "While", "Repeat", "Return", "Show", "Give", "Push", "Call",
            "Inspect",
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
        let pos = Position { line: 1, character: 10 };
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
        let pos = Position { line: 7, character: 5 };
        let Some(CompletionResponse::Array(items)) = completions(&doc, pos) else {
            panic!("expected completions");
        };
        let item = items
            .iter()
            .find(|i| i.label == "double")
            .expect("double completes");
        let Some(tower_lsp::lsp_types::Documentation::MarkupContent(content)) =
            &item.documentation
        else {
            panic!("documented function must carry markdown docs");
        };
        assert!(content.value.contains("Doubles a number."), "{}", content.value);
    }

    #[test]
    fn stdlib_names_complete_with_their_documentation() {
        let doc = make_doc("## Main\n    Show x.\n");
        let pos = Position { line: 1, character: 9 };
        let Some(CompletionResponse::Array(items)) = completions(&doc, pos) else {
            panic!("expected completions");
        };
        let md5 = items.iter().find(|i| i.label == "md5").expect("md5 offered from stdlib");
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
        let pos = Position { line: 2, character: 14 };
        let result = completions(&doc, pos);
        // Should return something (even if empty or keywords) but not panic
        assert!(result.is_some(), "Should return some completions after Inspect");
    }

    #[test]
    fn completion_moved_variable_has_deprecated_tag() {
        use logicaffeine_compile::analysis::VarState;
        let mut doc = make_doc("## Main\n    Let x be 5.\n    Let y be 0.\n    Give x to y.\n    Show x.\n");
        // Ensure x is marked as Moved
        doc.ownership_states.insert("x".to_string(), VarState::Moved);
        // Find completions in some context
        let pos = Position { line: 4, character: 9 };
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
                    x_items[0].tags.as_ref().map_or(false, |t| t.contains(&CompletionItemTag::DEPRECATED)),
                    "Moved variable should have DEPRECATED tag"
                );
                assert!(
                    x_items[0].detail.as_ref().map_or(false, |d| d.contains("(moved)")),
                    "Moved variable detail should indicate moved: {:?}",
                    x_items[0].detail
                );
            }
        }
    }

    #[test]
    fn completion_owned_variable_no_tag() {
        let doc = make_doc("## Main\n    Let x be 5.\n    Show x.\n");
        let pos = Position { line: 2, character: 9 };
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
}

fn add_identifier_completions(doc: &DocumentState, items: &mut Vec<CompletionItem>) {
    for def in &doc.symbol_index.definitions {
        let kind = match def.kind {
            DefinitionKind::Variable => CompletionItemKind::VARIABLE,
            DefinitionKind::Function => CompletionItemKind::FUNCTION,
            DefinitionKind::Struct => CompletionItemKind::CLASS,
            DefinitionKind::Enum => CompletionItemKind::ENUM,
            DefinitionKind::Field => CompletionItemKind::FIELD,
            DefinitionKind::Parameter => CompletionItemKind::VARIABLE,
            DefinitionKind::Block => CompletionItemKind::MODULE,
            DefinitionKind::Variant => CompletionItemKind::ENUM_MEMBER,
            DefinitionKind::Theorem => CompletionItemKind::CLASS,
        };

        // Check ownership state for moved variables
        let is_moved = doc.ownership_states.get(&def.name).map_or(false, |state| {
            matches!(state, VarState::Moved | VarState::MaybeMoved)
        });

        let mut detail = def.detail.clone();
        let mut tags = None;
        let mut deprecated = None;
        if is_moved {
            deprecated = Some(true);
            tags = Some(vec![CompletionItemTag::DEPRECATED]);
            detail = Some(format!(
                "{} (moved)",
                detail.as_deref().unwrap_or(&def.name)
            ));
        }

        items.push(CompletionItem {
            label: def.name.clone(),
            kind: Some(kind),
            detail,
            documentation: def.doc.clone().map(markdown_docs),
            deprecated,
            tags,
            ..Default::default()
        });
    }
}

/// Stdlib prelude names, documented from their literate `## Note`s.
/// Declarer wins: a local definition of the same name suppresses the
/// stdlib entry, mirroring the loader's auto-import rule.
fn add_stdlib_completions(doc: &DocumentState, items: &mut Vec<CompletionItem>) {
    for (name, entry) in crate::stdlib_docs::all() {
        if doc.symbol_index.name_to_defs.contains_key(name) {
            continue;
        }
        items.push(CompletionItem {
            label: name.to_string(),
            kind: Some(if entry.is_type {
                CompletionItemKind::CLASS
            } else {
                CompletionItemKind::FUNCTION
            }),
            detail: Some(entry.signature.trim_start_matches("## ").to_string()),
            documentation: Some(tower_lsp::lsp_types::Documentation::MarkupContent(
                tower_lsp::lsp_types::MarkupContent {
                    kind: tower_lsp::lsp_types::MarkupKind::Markdown,
                    value: crate::stdlib_docs::hover_md(name, entry),
                },
            )),
            ..Default::default()
        });
    }
}

/// Plain prose as markdown completion documentation.
fn markdown_docs(value: String) -> tower_lsp::lsp_types::Documentation {
    tower_lsp::lsp_types::Documentation::MarkupContent(tower_lsp::lsp_types::MarkupContent {
        kind: tower_lsp::lsp_types::MarkupKind::Markdown,
        value,
    })
}
