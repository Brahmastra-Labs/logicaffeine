use lsp_types::{CodeLens, Command, Range};

use logicaffeine_language::token::BlockType;

use crate::document::DocumentState;

/// Handle code lens request.
///
/// Shows "Run" above ## Main blocks and "Verify" above ## Theorem blocks.
/// Commands include the block name as an argument for the editor extension.
pub fn code_lenses(doc: &DocumentState, uri: &lsp_types::Url) -> Vec<CodeLens> {
    let mut lenses = Vec::new();

    for (name, block_type, span) in &doc.symbol_index.block_spans {
        let range = Range {
            start: doc.line_index.position(span.start),
            end: doc.line_index.position(span.start),
        };

        let args = Some(vec![
            serde_json::Value::String(uri.to_string()),
            serde_json::Value::String(name.clone()),
        ]);

        match block_type {
            BlockType::Main => {
                lenses.push(CodeLens {
                    range,
                    command: Some(Command {
                        title: "Run".to_string(),
                        command: "logicaffeine.run".to_string(),
                        arguments: args,
                    }),
                    data: None,
                });
            }
            BlockType::Theorem => {
                lenses.push(CodeLens {
                    range,
                    command: Some(Command {
                        title: "Verify".to_string(),
                        command: "logicaffeine.verify".to_string(),
                        arguments: args.clone(),
                    }),
                    data: None,
                });
                lenses.push(CodeLens {
                    range,
                    command: Some(Command {
                        title: "Prove".to_string(),
                        command: "logicaffeine.prove".to_string(),
                        arguments: args,
                    }),
                    data: None,
                });
            }
            BlockType::Proof => {
                lenses.push(CodeLens {
                    range,
                    command: Some(Command {
                        title: "Check Proof".to_string(),
                        command: "logicaffeine.checkProof".to_string(),
                        arguments: args,
                    }),
                    data: None,
                });
            }
            _ => {}
        }
    }

    lenses
}

#[cfg(test)]
#[path = "../../logicaffeine_tests/unit/logicaffeine_lsp/code_lens/tests.rs"]
mod tests;
