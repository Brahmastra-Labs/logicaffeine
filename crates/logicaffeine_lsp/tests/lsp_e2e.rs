//! End-to-end tests: a simulated editor talking to the real server loop.

mod harness;

use harness::Harness;
use tower_lsp::lsp_types::*;

const CLEAN_SOURCE: &str = "## Main\n    Let x be 5.\n    Show x.\n";
const BROKEN_SOURCE: &str = "## Main\n    Let be.\n";
const USE_AFTER_MOVE_SOURCE: &str =
    "## Main\n    Let x be 5.\n    Let y be 0.\n    Give x to y.\n    Show x.\n";

#[tokio::test]
async fn initialize_reports_server_info() {
    let harness = Harness::start().await;
    let info = harness.init.server_info.as_ref().expect("server info");
    assert_eq!(info.name, "logicaffeine-lsp");
    assert!(info.version.is_some(), "server should report its version");
}

#[tokio::test]
async fn did_open_clean_source_publishes_empty_diagnostics() {
    let mut harness = Harness::start().await;
    let uri = harness.open(CLEAN_SOURCE).await;
    let published = harness.recv_diagnostics(&uri).await;
    assert_eq!(
        published.diagnostics,
        vec![],
        "clean source must publish an explicit empty diagnostic set"
    );
    assert_eq!(published.version, Some(1));
}

#[tokio::test]
async fn did_open_broken_source_publishes_socratic_diagnostics() {
    let mut harness = Harness::start().await;
    let uri = harness.open(BROKEN_SOURCE).await;
    let published = harness.recv_diagnostics(&uri).await;
    assert!(
        !published.diagnostics.is_empty(),
        "broken source must produce diagnostics"
    );
    for diagnostic in &published.diagnostics {
        assert_eq!(diagnostic.source.as_deref(), Some("logicaffeine"));
        assert!(
            !diagnostic.message.is_empty(),
            "every diagnostic carries an explanation"
        );
    }
}

#[tokio::test]
async fn use_after_move_round_trip_carries_related_information() {
    let mut harness = Harness::start().await;
    let uri = harness.open(USE_AFTER_MOVE_SOURCE).await;
    let published = harness.recv_diagnostics(&uri).await;

    let move_diag = published
        .diagnostics
        .iter()
        .find(|d| {
            matches!(&d.code, Some(NumberOrString::String(c)) if c == "use-after-move")
        })
        .expect("Give x then Show x must produce a use-after-move diagnostic");
    assert!(
        move_diag.message.contains("giving it away"),
        "message should speak English, got: {}",
        move_diag.message
    );
    let related = move_diag
        .related_information
        .as_ref()
        .expect("use-after-move links to its cause");
    assert_eq!(related[0].location.uri, uri);
    assert_eq!(
        related[0].location.range.start.line, 3,
        "cause should point at the Give statement"
    );
}

#[tokio::test]
async fn did_change_updates_diagnostics() {
    let mut harness = Harness::start().await;
    let uri = harness.open(CLEAN_SOURCE).await;
    let first = harness.recv_diagnostics(&uri).await;
    assert!(first.diagnostics.is_empty());

    harness.change(&uri, BROKEN_SOURCE, 2).await;
    let second = harness.recv_diagnostics(&uri).await;
    assert!(
        !second.diagnostics.is_empty(),
        "edit that breaks the program must produce fresh diagnostics"
    );
    assert_eq!(second.version, Some(2));

    harness.change(&uri, CLEAN_SOURCE, 3).await;
    let third = harness.recv_diagnostics(&uri).await;
    assert!(
        third.diagnostics.is_empty(),
        "fixing the program must clear diagnostics"
    );
    assert_eq!(third.version, Some(3));
}

#[tokio::test]
async fn did_close_clears_diagnostics() {
    let mut harness = Harness::start().await;
    let uri = harness.open(BROKEN_SOURCE).await;
    let published = harness.recv_diagnostics(&uri).await;
    assert!(!published.diagnostics.is_empty());

    harness.close(&uri).await;
    let cleared = harness.recv_diagnostics(&uri).await;
    assert!(
        cleared.diagnostics.is_empty(),
        "closing a document must clear its diagnostics"
    );
}

#[tokio::test]
async fn semantic_tokens_full_returns_tokens() {
    let mut harness = Harness::start().await;
    let uri = harness.open(CLEAN_SOURCE).await;
    let _ = harness.recv_diagnostics(&uri).await;

    let result = harness
        .request::<request::SemanticTokensFullRequest>(SemanticTokensParams {
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            text_document: TextDocumentIdentifier { uri },
        })
        .await;

    match result {
        Some(SemanticTokensResult::Tokens(tokens)) => {
            assert!(
                !tokens.data.is_empty(),
                "clean source must produce semantic tokens"
            );
        }
        other => panic!("expected full semantic tokens, got {other:?}"),
    }
}

#[tokio::test]
async fn hover_over_let_keyword_documents_it() {
    let mut harness = Harness::start().await;
    let uri = harness.open(CLEAN_SOURCE).await;
    let _ = harness.recv_diagnostics(&uri).await;

    let hover = harness
        .request::<request::HoverRequest>(HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: Position {
                    line: 1,
                    character: 5,
                },
            },
            work_done_progress_params: Default::default(),
        })
        .await;

    assert!(hover.is_some(), "hovering 'Let' must document the keyword");
}

#[tokio::test]
async fn goto_definition_resolves_local_variable() {
    let mut harness = Harness::start().await;
    let uri = harness.open(CLEAN_SOURCE).await;
    let _ = harness.recv_diagnostics(&uri).await;

    // "Show x." — x is at line 2, character 9.
    let response = harness
        .request::<request::GotoDefinition>(GotoDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position {
                    line: 2,
                    character: 9,
                },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        })
        .await;

    let location = match response {
        Some(GotoDefinitionResponse::Scalar(loc)) => loc,
        Some(GotoDefinitionResponse::Array(locs)) if !locs.is_empty() => locs[0].clone(),
        other => panic!("expected a definition location, got {other:?}"),
    };
    assert_eq!(location.uri, uri);
    assert_eq!(
        location.range.start.line, 1,
        "definition of x is the Let on line 1"
    );
}

#[tokio::test]
async fn requests_against_unopened_documents_return_null_not_errors() {
    let mut harness = Harness::start().await;
    let ghost = Url::parse("file:///harness/never-opened.lg").unwrap();

    let hover = harness
        .request::<request::HoverRequest>(HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: ghost },
                position: Position::default(),
            },
            work_done_progress_params: Default::default(),
        })
        .await;
    assert!(hover.is_none(), "unopened documents yield null, not errors");
}
