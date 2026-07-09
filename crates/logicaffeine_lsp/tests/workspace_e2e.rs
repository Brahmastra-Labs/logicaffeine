//! Workspace-wide navigation: symbols across every file in the workspace and
//! goto-definition that crosses file boundaries — without opening the target.

mod harness;

use std::path::PathBuf;
use std::time::Duration;

use harness::Harness;
use tower_lsp::lsp_types::*;

fn fixture_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/proj_a")
}

/// Background indexing races the first request; poll briefly.
async fn workspace_symbols(harness: &mut Harness, query: &str) -> Vec<SymbolInformation> {
    for _ in 0..50 {
        #[allow(deprecated)]
        let response = harness
            .request::<request::WorkspaceSymbolRequest>(WorkspaceSymbolParams {
                query: query.to_string(),
                work_done_progress_params: Default::default(),
                partial_result_params: Default::default(),
            })
            .await;
        match response {
            Some(WorkspaceSymbolResponse::Flat(symbols)) if !symbols.is_empty() => {
                return symbols;
            }
            _ => tokio::time::sleep(Duration::from_millis(100)).await,
        }
    }
    Vec::new()
}

#[tokio::test]
async fn workspace_symbols_find_definitions_in_unopened_files() {
    let mut harness = Harness::start_with_workspace(&fixture_root()).await;

    let symbols = workspace_symbols(&mut harness, "triple").await;
    let hit = symbols
        .iter()
        .find(|s| s.name == "triple")
        .unwrap_or_else(|| panic!("'triple' lives in lib.lg (never opened), got: {symbols:#?}"));
    assert!(
        hit.location.uri.path().ends_with("lib.lg"),
        "the symbol's location is its defining file: {:?}",
        hit.location.uri
    );
    assert_eq!(hit.kind, SymbolKind::FUNCTION);

    let widgets = workspace_symbols(&mut harness, "widg").await;
    assert!(
        widgets.iter().any(|s| s.name == "Widget"),
        "matching is case-insensitive substring: {widgets:#?}"
    );
}

#[tokio::test]
async fn goto_definition_crosses_into_an_unopened_file() {
    let root = fixture_root();
    let mut harness = Harness::start_with_workspace(&root).await;

    // Warm the index (the symbol query polls until the background scan lands).
    let _ = workspace_symbols(&mut harness, "triple").await;

    let main_source = std::fs::read_to_string(root.join("src/main.lg")).unwrap();
    let uri = harness.open(&main_source).await;
    let _ = harness.recv_diagnostics(&uri).await;

    // `triple` in `Let result be triple(14).` — line 3, character 18.
    let response = harness
        .request::<request::GotoDefinition>(GotoDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: Position { line: 3, character: 18 },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        })
        .await;

    let location = match response {
        Some(GotoDefinitionResponse::Scalar(loc)) => loc,
        Some(GotoDefinitionResponse::Array(locs)) if !locs.is_empty() => locs[0].clone(),
        other => panic!("goto-def must cross files, got {other:?}"),
    };
    assert!(
        location.uri.path().ends_with("lib.lg"),
        "definition lives in lib.lg: {:?}",
        location.uri
    );
}

#[tokio::test]
async fn references_cross_file_find_callers_in_unopened_files() {
    let root = fixture_root();
    let mut harness = Harness::start_with_workspace(&root).await;
    let _ = workspace_symbols(&mut harness, "triple").await;

    // Open lib.lg (the DEFINING file) under its real URI; main.lg stays closed.
    let lib_path = root.join("src/lib.lg").canonicalize().unwrap();
    let lib_uri = Url::from_file_path(&lib_path).unwrap();
    let lib_source = std::fs::read_to_string(&lib_path).unwrap();
    harness.open_at(lib_uri.clone(), &lib_source).await;
    let _ = harness.recv_diagnostics(&lib_uri).await;

    // `triple` in `## To triple (n: Int) -> Int:` — line 2, character 6.
    let response = harness
        .request::<request::References>(ReferenceParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: lib_uri.clone() },
                position: Position { line: 2, character: 6 },
            },
            context: ReferenceContext { include_declaration: true },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        })
        .await
        .expect("references must answer");

    assert!(
        response.iter().any(|l| l.uri.path().ends_with("main.lg")),
        "the caller in UNOPENED main.lg must be found: {response:#?}"
    );
}

#[tokio::test]
async fn rename_updates_cross_file_callers() {
    let root = fixture_root();
    let mut harness = Harness::start_with_workspace(&root).await;
    let _ = workspace_symbols(&mut harness, "triple").await;

    let lib_path = root.join("src/lib.lg").canonicalize().unwrap();
    let lib_uri = Url::from_file_path(&lib_path).unwrap();
    let lib_source = std::fs::read_to_string(&lib_path).unwrap();
    harness.open_at(lib_uri.clone(), &lib_source).await;
    let _ = harness.recv_diagnostics(&lib_uri).await;

    let edit = harness
        .request::<request::Rename>(RenameParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: lib_uri.clone() },
                position: Position { line: 2, character: 6 },
            },
            new_name: "treble".to_string(),
            work_done_progress_params: Default::default(),
        })
        .await
        .expect("rename must answer");

    let changes = edit.changes.expect("rename returns changes");
    assert!(
        changes.keys().any(|u| u.path().ends_with("main.lg")),
        "rename must edit the UNOPENED caller too: {:?}",
        changes.keys().collect::<Vec<_>>()
    );
    assert!(
        changes.keys().any(|u| u.path().ends_with("lib.lg")),
        "rename must edit the defining file: {:?}",
        changes.keys().collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn open_document_edits_win_over_the_stale_disk_index() {
    let root = fixture_root();
    let mut harness = Harness::start_with_workspace(&root).await;
    let _ = workspace_symbols(&mut harness, "triple").await;

    let lib_path = root.join("src/lib.lg").canonicalize().unwrap();
    let lib_uri = Url::from_file_path(&lib_path).unwrap();
    let lib_source = std::fs::read_to_string(&lib_path).unwrap();
    harness.open_at(lib_uri.clone(), &lib_source).await;
    let _ = harness.recv_diagnostics(&lib_uri).await;

    // Open main.lg under its REAL uri too, then edit the call away in the
    // live buffer — the on-disk copy still calls `triple`.
    let main_path = root.join("src/main.lg").canonicalize().unwrap();
    let main_uri = Url::from_file_path(&main_path).unwrap();
    harness
        .open_at(main_uri.clone(), "## Main\nLet result be 42.\nShow result.\n")
        .await;
    let _ = harness.recv_diagnostics(&main_uri).await;

    let edit = harness
        .request::<request::Rename>(RenameParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: lib_uri.clone() },
                position: Position { line: 2, character: 6 },
            },
            new_name: "treble".to_string(),
            work_done_progress_params: Default::default(),
        })
        .await
        .expect("rename must answer");

    let changes = edit.changes.expect("rename returns changes");
    assert!(
        !changes.keys().any(|u| u.path().ends_with("main.lg")),
        "main.lg's LIVE buffer no longer calls triple — the stale disk index must not \
         produce an edit for it: {:?}",
        changes.keys().collect::<Vec<_>>()
    );
}
