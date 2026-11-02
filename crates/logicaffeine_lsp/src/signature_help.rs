use lsp_types::{
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
    let call_token = doc
        .tokens
        .iter()
        .rev()
        .find(|t| t.span.end <= offset && matches!(t.kind, TokenType::Call))?;

    // Find the function name token (should be right after Call or after "with")
    let call_idx = doc
        .tokens
        .iter()
        .position(|t| t.span == call_token.span && t.kind == call_token.kind)?;

    // The function name should follow Call
    let func_name_token = doc.tokens.get(call_idx + 1)?;
    let func_name = doc
        .source
        .get(func_name_token.span.start..func_name_token.span.end)?;

    // Look up the function definition; stdlib prelude names answer when the
    // document defines nothing by that name.
    let defs = doc.symbol_index.definitions_of(func_name);
    let func_def = defs.iter().find(|d| d.kind == DefinitionKind::Function);
    let (detail, documentation) = match func_def {
        Some(def) => (def.detail.clone()?, def.doc.clone()),
        None => {
            let entry = crate::stdlib_docs::stdlib_doc(func_name).filter(|e| !e.is_type)?;
            (
                entry.signature.trim_start_matches("## ").to_string(),
                entry.doc.clone(),
            )
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
            documentation: Some(lsp_types::Documentation::String(format!(
                "{}: {}",
                name, ty
            ))),
        })
        .collect();

    Some(SignatureHelp {
        signatures: vec![SignatureInformation {
            label: detail.clone(),
            documentation: documentation.map(|d| {
                lsp_types::Documentation::MarkupContent(lsp_types::MarkupContent {
                    kind: lsp_types::MarkupKind::Markdown,
                    value: d,
                })
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
        let Some(close) = rest[open + 1..].find(')') else {
            break;
        };
        let group = &rest[open + 1..open + 1 + close];
        for part in group.split(',') {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }
            let mut split = part.splitn(2, ':');
            let Some(name) = split.next().map(|s| s.trim().to_string()) else {
                continue;
            };
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
#[path = "../../logicaffeine_tests/unit/logicaffeine_lsp/signature_help/tests.rs"]
mod tests;
