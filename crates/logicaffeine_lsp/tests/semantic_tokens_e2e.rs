//! Resolution-aware semantic highlighting: parts of speech are the syntax,
//! but resolution decides what an identifier IS — parameter, function, type,
//! field, variant, or variable — and modifiers carry declaration/mutability/
//! mutation/stdlib facts. Prose in `## Note`/`## Example` recedes to comment.

mod harness;

use harness::Harness;
use tower_lsp::lsp_types::*;

use logicaffeine_lsp::document::DocumentState;
use logicaffeine_lsp::line_index::LineIndex;
use logicaffeine_lsp::semantic_tokens::{
    encode_document_tokens, MOD_DECLARATION, MOD_DEFAULT_LIBRARY, MOD_MODIFICATION, MOD_READONLY,
    TYPE_COMMENT, TYPE_ENUM_MEMBER, TYPE_FUNCTION, TYPE_PARAMETER, TYPE_PROPERTY, TYPE_VARIABLE,
};

/// One decoded semantic token: absolute position + classification.
#[derive(Debug, PartialEq)]
struct Decoded {
    line: u32,
    character: u32,
    length: u32,
    token_type: u32,
    modifiers: u32,
}

fn decode(source: &str) -> Vec<Decoded> {
    let doc = DocumentState::new(source.to_string(), 1);
    let raw = encode_document_tokens(&doc);
    let mut decoded = Vec::with_capacity(raw.len());
    let (mut line, mut character) = (0u32, 0u32);
    for token in raw {
        line += token.delta_line;
        if token.delta_line > 0 {
            character = token.delta_start;
        } else {
            character += token.delta_start;
        }
        decoded.push(Decoded {
            line,
            character,
            length: token.length,
            token_type: token.token_type,
            modifiers: token.token_modifiers_bitset,
        });
    }
    decoded
}

/// The token covering (line, character) — panics loudly if none.
fn at<'a>(tokens: &'a [Decoded], line: u32, character: u32) -> &'a Decoded {
    tokens
        .iter()
        .find(|t| {
            t.line == line && t.character <= character && character < t.character + t.length
        })
        .unwrap_or_else(|| panic!("no token at {line}:{character} in {tokens:#?}"))
}

fn line_char(source: &str, line: u32, needle: &str) -> u32 {
    let index = LineIndex::new(source);
    let text: Vec<&str> = source.split('\n').collect();
    let col = text[line as usize]
        .find(needle)
        .unwrap_or_else(|| panic!("{needle:?} not on line {line}"));
    // Test sources are ASCII, so byte column == UTF-16 column.
    let _ = index;
    col as u32
}

#[test]
fn parameter_resolves_at_declaration_and_reference() {
    let source = "## To greet (name: Text):\n    Show name.\n";
    let tokens = decode(source);

    let decl = at(&tokens, 0, line_char(source, 0, "name"));
    assert_eq!(decl.token_type, TYPE_PARAMETER, "declaration site is a parameter");
    assert_ne!(decl.modifiers & MOD_DECLARATION, 0, "definition site carries declaration");

    let reference = at(&tokens, 1, line_char(source, 1, "name"));
    assert_eq!(reference.token_type, TYPE_PARAMETER, "reference resolves to the parameter");
    assert_eq!(
        reference.modifiers & MOD_DECLARATION,
        0,
        "references are not declarations"
    );
}

#[test]
fn declaration_modifier_fires_only_at_the_definition_site() {
    let source = "## Main\n    Let x be 5.\n    Show x.\n";
    let tokens = decode(source);

    let decl = at(&tokens, 1, line_char(source, 1, "x be") /* the bound x */);
    assert_eq!(decl.token_type, TYPE_VARIABLE);
    assert_ne!(decl.modifiers & MOD_DECLARATION, 0);
    assert_ne!(decl.modifiers & MOD_READONLY, 0, "an immutable Let is readonly");

    let reference = at(&tokens, 2, line_char(source, 2, "x."));
    assert_eq!(reference.token_type, TYPE_VARIABLE);
    assert_eq!(reference.modifiers & MOD_DECLARATION, 0);
}

#[test]
fn mutable_let_is_not_readonly_and_set_target_is_a_modification() {
    let source = "## Main\n    Let mutable x be 5.\n    Set x to 6.\n    Show x.\n";
    let tokens = decode(source);

    let decl = at(&tokens, 1, line_char(source, 1, "x be"));
    assert_eq!(decl.modifiers & MOD_READONLY, 0, "a mutable Let is not readonly");

    let target = at(&tokens, 2, line_char(source, 2, "x to"));
    assert_ne!(target.modifiers & MOD_MODIFICATION, 0, "Set target is a write site");

    let read = at(&tokens, 3, line_char(source, 3, "x."));
    assert_eq!(read.modifiers & MOD_MODIFICATION, 0, "a read is not a write");
}

#[test]
fn function_call_resolves_to_function() {
    let source = "## To double (n: Int) -> Int:\n    Return n * 2.\n\n## Main\nShow double(21).\n";
    let tokens = decode(source);
    let call = at(&tokens, 4, line_char(source, 4, "double"));
    assert_eq!(call.token_type, TYPE_FUNCTION, "call site resolves to the function");
}

#[test]
fn field_and_variant_references_resolve() {
    let source = "\
## A Point has:
    An x: Int.
    A y: Int.

## A Shape is one of:
    A Circle with radius Int.

## Main
Let p be a new Point.
Show p's x.
";
    let tokens = decode(source);
    let field = at(&tokens, 9, line_char(source, 9, "x."));
    assert_eq!(field.token_type, TYPE_PROPERTY, "p's x resolves to the field");

    // The variant name at its declaration inside the enum block.
    let variant = at(&tokens, 5, line_char(source, 5, "Circle"));
    assert_eq!(variant.token_type, TYPE_ENUM_MEMBER, "variants are enum members");
}

#[test]
fn note_prose_recedes_to_comment_but_its_header_does_not() {
    let source = "\
## Note

The quick brown fox jumps over the lazy dog and should not light up.

## Main
Let x be 5.
";
    let tokens = decode(source);

    let prose = at(&tokens, 2, line_char(source, 2, "quick"));
    assert_eq!(prose.token_type, TYPE_COMMENT, "Note prose is documentation");
    let connective = at(&tokens, 2, line_char(source, 2, "and should"));
    assert_eq!(connective.token_type, TYPE_COMMENT, "even 'and' recedes inside a Note");

    let header = at(&tokens, 0, 3);
    assert_ne!(header.token_type, TYPE_COMMENT, "the ## Note header stays a header");

    let code = at(&tokens, 5, line_char(source, 5, "x be"));
    assert_eq!(code.token_type, TYPE_VARIABLE, "code after the Note is unaffected");
}

#[test]
fn stdlib_prelude_names_carry_default_library() {
    let source = "## Main\n    Let h be md5 of \"abc\".\n";
    let tokens = decode(source);
    let call = at(&tokens, 1, line_char(source, 1, "md5"));
    assert_ne!(
        call.modifiers & MOD_DEFAULT_LIBRARY,
        0,
        "md5 is stdlib prelude vocabulary"
    );
}

// ---------------------------------------------------------------------------
// Range + delta over the real server loop
// ---------------------------------------------------------------------------

const RANGE_SOURCE: &str = "## Main\n    Let x be 5.\n    Let y be 6.\n    Show x.\n";

#[tokio::test]
async fn semantic_tokens_range_returns_only_the_requested_lines() {
    let mut harness = Harness::start().await;
    let uri = harness.open(RANGE_SOURCE).await;
    let _ = harness.recv_diagnostics(&uri).await;

    let full = match harness
        .request::<request::SemanticTokensFullRequest>(SemanticTokensParams {
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            text_document: TextDocumentIdentifier { uri: uri.clone() },
        })
        .await
    {
        Some(SemanticTokensResult::Tokens(tokens)) => tokens,
        other => panic!("expected full tokens, got {other:?}"),
    };

    let ranged = match harness
        .request::<request::SemanticTokensRangeRequest>(SemanticTokensRangeParams {
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            text_document: TextDocumentIdentifier { uri },
            range: Range {
                start: Position { line: 2, character: 0 },
                end: Position { line: 3, character: 0 },
            },
        })
        .await
    {
        Some(SemanticTokensRangeResult::Tokens(tokens)) => tokens,
        other => panic!("expected ranged tokens, got {other:?}"),
    };

    assert!(!ranged.data.is_empty(), "line 2 has tokens");
    assert!(
        ranged.data.len() < full.data.len(),
        "a one-line range must return fewer tokens than the full document"
    );
    assert_eq!(
        ranged.data[0].delta_line, 2,
        "the first ranged token is absolute from the document start"
    );
}

#[tokio::test]
async fn semantic_tokens_delta_returns_a_single_splice_after_an_edit() {
    let mut harness = Harness::start().await;
    let uri = harness.open(RANGE_SOURCE).await;
    let _ = harness.recv_diagnostics(&uri).await;

    let first = match harness
        .request::<request::SemanticTokensFullRequest>(SemanticTokensParams {
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            text_document: TextDocumentIdentifier { uri: uri.clone() },
        })
        .await
    {
        Some(SemanticTokensResult::Tokens(tokens)) => tokens,
        other => panic!("expected full tokens, got {other:?}"),
    };
    let result_id = first.result_id.expect("full response carries a result id");

    harness
        .change(&uri, "## Main\n    Let x be 5.\n    Let z be 7.\n    Show x.\n", 2)
        .await;
    let _ = harness.recv_diagnostics(&uri).await;

    let delta = harness
        .request::<request::SemanticTokensFullDeltaRequest>(SemanticTokensDeltaParams {
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            text_document: TextDocumentIdentifier { uri },
            previous_result_id: result_id,
        })
        .await;

    match delta {
        Some(SemanticTokensFullDeltaResult::TokensDelta(delta)) => {
            assert!(
                delta.edits.len() <= 1,
                "a one-line edit is a single splice, got {:?}",
                delta.edits
            );
        }
        Some(SemanticTokensFullDeltaResult::Tokens(_)) => {
            panic!("server should answer a known result id with a delta, not full tokens");
        }
        other => panic!("expected a delta, got {other:?}"),
    }
}
