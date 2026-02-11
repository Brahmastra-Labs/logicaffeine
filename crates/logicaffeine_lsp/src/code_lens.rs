use tower_lsp::lsp_types::{CodeLens, Command, Range};

use logicaffeine_language::token::BlockType;

use crate::document::DocumentState;

/// Handle code lens request.
///
/// Shows "Run" above ## Main blocks and "Verify" above ## Theorem blocks.
/// Commands include the block name as an argument for the editor extension.
pub fn code_lenses(doc: &DocumentState, uri: &tower_lsp::lsp_types::Url) -> Vec<CodeLens> {
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
    fn code_lens_for_main_block() {
        let doc = make_doc("## Main\n    Let x be 5.\n");
        let lenses = code_lenses(&doc, &test_uri());
        let run_lenses: Vec<_> = lenses.iter().filter(|l| {
            l.command.as_ref().map(|c| c.command == "logicaffeine.run").unwrap_or(false)
        }).collect();
        assert_eq!(run_lenses.len(), 1, "Expected exactly one 'Run' lens for ## Main");
        assert_eq!(run_lenses[0].command.as_ref().unwrap().title, "Run");
    }

    #[test]
    fn code_lens_empty_for_no_blocks() {
        let doc = make_doc("");
        let lenses = code_lenses(&doc, &test_uri());
        assert!(lenses.is_empty(), "Empty doc should have no lenses");
    }

    #[test]
    fn code_lens_note_block_no_lenses() {
        let doc = make_doc("## Note: readme\n    Some docs here.\n");
        let lenses = code_lenses(&doc, &test_uri());
        let run_or_verify: Vec<_> = lenses.iter().filter(|l| {
            l.command.as_ref().map(|c|
                c.command == "logicaffeine.run" || c.command == "logicaffeine.verify"
            ).unwrap_or(false)
        }).collect();
        assert!(run_or_verify.is_empty(),
            "Note block should not have Run or Verify lenses, got {:?}",
            run_or_verify.iter().map(|l| &l.command).collect::<Vec<_>>());
    }

    #[test]
    fn code_lens_positioned_at_block_start() {
        let doc = make_doc("## Main\n    Let x be 5.\n");
        let lenses = code_lenses(&doc, &test_uri());
        assert!(!lenses.is_empty());
        assert_eq!(lenses[0].range.start.line, 0, "Lens should be on the block header line");
    }

    #[test]
    fn code_lens_theorem_has_verify_and_prove() {
        let doc = make_doc("## Theorem: all humans are mortal\n    All humans are mortal.\n");
        let lenses = code_lenses(&doc, &test_uri());
        let verify_lenses: Vec<_> = lenses.iter().filter(|l| {
            l.command.as_ref().map(|c| c.command == "logicaffeine.verify").unwrap_or(false)
        }).collect();
        let prove_lenses: Vec<_> = lenses.iter().filter(|l| {
            l.command.as_ref().map(|c| c.command == "logicaffeine.prove").unwrap_or(false)
        }).collect();
        assert_eq!(verify_lenses.len(), 1, "Theorem should have 'Verify' lens");
        assert_eq!(prove_lenses.len(), 1, "Theorem should have 'Prove' lens");
    }

    #[test]
    fn code_lens_proof_block_has_check() {
        let doc = make_doc("## Proof\n    By assumption.\n");
        let lenses = code_lenses(&doc, &test_uri());
        let check_lenses: Vec<_> = lenses.iter().filter(|l| {
            l.command.as_ref().map(|c| c.command == "logicaffeine.checkProof").unwrap_or(false)
        }).collect();
        assert_eq!(check_lenses.len(), 1, "Proof block should have 'Check Proof' lens");
        assert_eq!(
            check_lenses[0].command.as_ref().unwrap().title,
            "Check Proof"
        );
    }

    #[test]
    fn run_lens_has_arguments() {
        let doc = make_doc("## Main\n    Let x be 5.\n");
        let uri = test_uri();
        let lenses = code_lenses(&doc, &uri);
        let run_lens = lenses.iter().find(|l| {
            l.command.as_ref().map(|c| c.command == "logicaffeine.run").unwrap_or(false)
        }).expect("Should have Run lens");
        let args = run_lens.command.as_ref().unwrap().arguments.as_ref();
        assert!(args.is_some(), "Run lens should have arguments");
        let args = args.unwrap();
        assert_eq!(args.len(), 2, "Should have URI and block name arguments");
        assert_eq!(args[0].as_str().unwrap(), uri.as_str());
    }
}
