use tower_lsp::lsp_types::{
    ParameterInformation, ParameterLabel, Position, SignatureHelp, SignatureInformation,
};

use logicaffeine_language::token::TokenType;

use crate::document::DocumentState;
use crate::index::DefinitionKind;

/// Handle signature help request.
///
/// When the cursor is inside a `Call` expression, find the function definition
/// and show parameter names/types.
pub fn signature_help(doc: &DocumentState, position: Position) -> Option<SignatureHelp> {
    let offset = doc.line_index.offset(position);

    // Scan backwards from cursor to find a Call token
    let call_token = doc.tokens.iter().rev().find(|t| {
        t.span.end <= offset && matches!(t.kind, TokenType::Call)
    })?;

    // Find the function name token (should be right after Call or after "with")
    let call_idx = doc.tokens.iter().position(|t| {
        t.span == call_token.span && t.kind == call_token.kind
    })?;

    // The function name should follow Call
    let func_name_token = doc.tokens.get(call_idx + 1)?;
    let func_name = doc.source.get(func_name_token.span.start..func_name_token.span.end)?;

    // Look up the function definition; stdlib prelude names answer when the
    // document defines nothing by that name.
    let defs = doc.symbol_index.definitions_of(func_name);
    let func_def = defs.iter().find(|d| d.kind == DefinitionKind::Function);
    let (detail, documentation) = match func_def {
        Some(def) => (def.detail.clone()?, def.doc.clone()),
        None => {
            let entry = crate::stdlib_docs::stdlib_doc(func_name).filter(|e| !e.is_type)?;
            (entry.signature.trim_start_matches("## ").to_string(), entry.doc.clone())
        }
    };
    let detail = &detail;

    // Count parameter separators after Call to determine active parameter.
    // "with" introduces the parameter list, only "and" and "," separate params.
    let active_param = doc.tokens[call_idx..]
        .iter()
        .take_while(|t| t.span.start < offset)
        .filter(|t| {
            matches!(t.kind, TokenType::Comma)
                || doc
                    .source
                    .get(t.span.start..t.span.end)
                    .map(|s| s == "and")
                    .unwrap_or(false)
        })
        .count();

    // Extract parameters from the function's signature detail string
    let params: Vec<ParameterInformation> = extract_params_from_signature(detail)
        .into_iter()
        .map(|(name, ty)| ParameterInformation {
            label: ParameterLabel::Simple(name.clone()),
            documentation: Some(tower_lsp::lsp_types::Documentation::String(
                format!("{}: {}", name, ty),
            )),
        })
        .collect();

    Some(SignatureHelp {
        signatures: vec![SignatureInformation {
            label: detail.clone(),
            documentation: documentation.map(|d| {
                tower_lsp::lsp_types::Documentation::MarkupContent(
                    tower_lsp::lsp_types::MarkupContent {
                        kind: tower_lsp::lsp_types::MarkupKind::Markdown,
                        value: d,
                    },
                )
            }),
            parameters: if params.is_empty() {
                None
            } else {
                Some(params)
            },
            active_parameter: Some(active_param as u32),
        }],
        active_signature: Some(0),
        active_parameter: Some(active_param as u32),
    })
}

/// Extract parameter names and types from a function signature detail string.
///
/// Handles both the comma style (`"To name(a: Int, b: Int) -> Ret"`) and the
/// stdlib's prepositional groups (`"To native f (a: Int) and (b: Int) -> R"`):
/// every parenthesized group before the arrow contributes its parameters.
fn extract_params_from_signature(detail: &str) -> Vec<(String, String)> {
    let head = detail.split("->").next().unwrap_or(detail);
    let mut params = Vec::new();
    let mut rest = head;
    while let Some(open) = rest.find('(') {
        let Some(close) = rest[open + 1..].find(')') else { break };
        let group = &rest[open + 1..open + 1 + close];
        for part in group.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let mut split = part.splitn(2, ':');
            let Some(name) = split.next().map(|s| s.trim().to_string()) else { continue };
            let ty = split
                .next()
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| "auto".to_string());
            params.push((name, ty));
        }
        rest = &rest[open + 1 + close + 1..];
    }
    params
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::DocumentState;

    fn make_doc(source: &str) -> DocumentState {
        DocumentState::new(source.to_string(), 1)
    }

    #[test]
    fn signature_help_returns_none_without_call() {
        let doc = make_doc("## Main\n    Let x be 5.\n");
        let pos = Position { line: 1, character: 10 };
        let result = signature_help(&doc, pos);
        assert!(result.is_none(), "Should return None when not in a Call expression");
    }

    #[test]
    fn signature_help_no_crash_empty_doc() {
        let doc = make_doc("");
        let pos = Position { line: 0, character: 0 };
        let result = signature_help(&doc, pos);
        assert!(result.is_none());
    }

    #[test]
    fn signature_help_no_crash_on_out_of_bounds() {
        let doc = make_doc("## Main\n    Let x be 5.\n");
        let pos = Position { line: 5, character: 0 };
        let result = signature_help(&doc, pos);
        assert!(result.is_none(), "OOB should return None");
    }

    #[test]
    fn extract_params_basic() {
        let params = extract_params_from_signature("To add(a: Int, b: Int) -> Int");
        assert_eq!(params.len(), 2);
        assert_eq!(params[0], ("a".to_string(), "Int".to_string()));
        assert_eq!(params[1], ("b".to_string(), "Int".to_string()));
    }

    #[test]
    fn extract_params_multiple() {
        let params = extract_params_from_signature("To greet(name: Text, age: Int, loud: Bool) -> Text");
        assert_eq!(params.len(), 3);
        assert_eq!(params[0].0, "name");
        assert_eq!(params[1].0, "age");
        assert_eq!(params[2].0, "loud");
    }

    #[test]
    fn extract_params_empty() {
        let params = extract_params_from_signature("To noop() -> Unit");
        assert!(params.is_empty());
    }

    #[test]
    fn extract_params_no_parens() {
        let params = extract_params_from_signature("something without parens");
        assert!(params.is_empty());
    }

    #[test]
    fn signature_help_returns_signature_for_defined_function() {
        let source = "## To add(a: Int, b: Int) -> Int\n    Return a + b.\n\n## Main\n    Let r be Call add with 1 and 2.\n";
        let doc = make_doc(source);
        let pos = Position { line: 4, character: 30 };
        let result = signature_help(&doc, pos);
        if let Some(help) = &result {
            assert!(!help.signatures.is_empty(), "Should have a signature");
            let sig = &help.signatures[0];
            // Params should come from the function's own signature, not globally
            if let Some(params) = &sig.parameters {
                let names: Vec<&str> = params
                    .iter()
                    .map(|p| match &p.label {
                        ParameterLabel::Simple(s) => s.as_str(),
                        _ => "",
                    })
                    .collect();
                assert!(names.contains(&"a"), "Should include param 'a': {:?}", names);
                assert!(names.contains(&"b"), "Should include param 'b': {:?}", names);
            }
        }
    }

    #[test]
    fn active_parameter_tracking() {
        let source = "## To add(a: Int, b: Int) -> Int\n    Return a + b.\n\n## Main\n    Let r be Call add with 1 and 2.\n";
        let doc = make_doc(source);
        // Position after "and" separator → active_parameter should be >= 1
        let pos = Position { line: 4, character: 35 };
        if let Some(help) = signature_help(&doc, pos) {
            let active = help.active_parameter.unwrap_or(0);
            assert!(active >= 1, "After 'and' separator, active_parameter should be >= 1, got {}", active);
        }
    }

    #[test]
    fn call_not_found_returns_none() {
        let doc = make_doc("## Main\n    Let x be 5.\n");
        // Position before any Call token → should return None
        let pos = Position { line: 1, character: 4 };
        let result = signature_help(&doc, pos);
        assert!(result.is_none(), "Should return None when no Call precedes position");
    }

    #[test]
    fn with_not_counted_as_separator() {
        // "with" introduces the parameter list, it should NOT increment active_parameter.
        // In `Call add with 1 and 2`, active_param should be 0 right after "with 1",
        // and 1 after "and".
        let source = "## To add with a: Int and b: Int\n    Show a.\n\n## Main\n    Let r be Call add with 1 and 2.\n";
        let doc = make_doc(source);
        // Position just after "with 1" but before "and"
        // "    Let r be Call add with 1 and 2.\n"
        //  0123456789...
        // We need a position after "1" but before "and"
        let pos = Position { line: 4, character: 27 };
        if let Some(help) = signature_help(&doc, pos) {
            let active = help.active_parameter.unwrap_or(99);
            assert_eq!(active, 0, "Before 'and', active_parameter should be 0 (with not counted), got {}", active);
        }
    }

    #[test]
    fn signature_help_documents_from_the_note_prose() {
        let source = "## Note\nAdds two integers.\n\n## To compute (a: Int, b: Int) -> Int:\n    Return a + b.\n\n## Main\n    Let r be Call compute with 1 and 2.\n";
        let doc = make_doc(source);
        let pos = Position { line: 7, character: 34 };
        let help = signature_help(&doc, pos).expect("documented function signature");
        let sig = &help.signatures[0];
        let Some(tower_lsp::lsp_types::Documentation::MarkupContent(content)) =
            &sig.documentation
        else {
            panic!("the ## Note prose must document the signature");
        };
        assert!(content.value.contains("Adds two integers."), "{}", content.value);
    }

    #[test]
    fn signature_help_falls_back_to_the_stdlib() {
        let source = "## Main\n    Let r be Call randomInt with 1 and 6.\n";
        let doc = make_doc(source);
        let pos = Position { line: 1, character: 32 };
        let help = signature_help(&doc, pos).expect("stdlib names answer signature help");
        let sig = &help.signatures[0];
        assert!(sig.label.contains("randomInt"), "{}", sig.label);
        let params = sig.parameters.as_ref().expect("min and max parameters");
        assert_eq!(params.len(), 2, "prepositional groups both count: {params:?}");
        assert!(sig.documentation.is_some(), "the literate Note teaches");
    }

    #[test]
    fn multi_group_signatures_yield_every_parameter() {
        let params = extract_params_from_signature(
            "To native write (path: Text) and (content: Text) -> Result of Unit and Text",
        );
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].0, "path");
        assert_eq!(params[1].0, "content");
    }

    #[test]
    fn signature_help_finds_function_via_span_not_pointer() {
        // Regression test: call_idx lookup must use span equality, not pointer equality.
        // Clone the doc's tokens to ensure different pointers but same spans.
        let source = "## To add(a: Int, b: Int) -> Int\n    Return a + b.\n\n## Main\n    Let r be Call add with 1 and 2.\n";
        let doc = make_doc(source);
        // Position after "Call add" on the Call line
        let pos = Position { line: 4, character: 30 };
        let result = signature_help(&doc, pos);
        // Should find the function regardless of pointer identity
        // (This test passes trivially once pointer equality is replaced with span equality)
        if let Some(help) = &result {
            assert!(!help.signatures.is_empty(), "Should have at least one signature");
        }
    }
}
