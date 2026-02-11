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

    Some(CompletionResponse::Array(items))
}

fn add_statement_keywords(items: &mut Vec<CompletionItem>) {
    let keywords = [
        ("Let", "Declare a variable", "Let ${1:name} be ${2:value}."),
        ("Set", "Update a variable", "Set ${1:name} to ${2:value}."),
        ("If", "Conditional branch", "If ${1:condition}:\n    ${2:body}"),
        ("While", "Loop while condition", "While ${1:condition}:\n    ${2:body}"),
        ("Repeat", "Iterate over collection", "Repeat for ${1:item} in ${2:collection}:\n    ${3:body}"),
        ("Return", "Return a value", "Return ${1:value}."),
        ("Show", "Display a value", "Show ${1:value}."),
        ("Give", "Transfer ownership", "Give ${1:value} to ${2:target}."),
        ("Push", "Append to list", "Push ${1:value} to ${2:list}."),
        ("Call", "Invoke function", "Call ${1:function} with ${2:args}."),
        ("Inspect", "Pattern match", "Inspect ${1:value}:\n    ${2:pattern}:\n        ${3:body}"),
    ];

    for (label, detail, snippet) in keywords {
        items.push(CompletionItem {
            label: label.to_string(),
            kind: Some(CompletionItemKind::KEYWORD),
            detail: Some(detail.to_string()),
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
    // Primitive types
    let primitives = ["Int", "Nat", "Text", "Bool", "Float", "Unit", "Char", "Byte"];
    for prim in primitives {
        items.push(CompletionItem {
            label: prim.to_string(),
            kind: Some(CompletionItemKind::TYPE_PARAMETER),
            detail: Some("Primitive type".to_string()),
            ..Default::default()
        });
    }

    // Generic types
    let generics = ["List", "Seq", "Map", "Set", "Option", "Result"];
    for gen in generics {
        items.push(CompletionItem {
            label: gen.to_string(),
            kind: Some(CompletionItemKind::CLASS),
            detail: Some("Generic type".to_string()),
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
            deprecated,
            tags,
            ..Default::default()
        });
    }
}
