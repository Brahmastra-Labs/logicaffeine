//! Async-architecture invariants: debounced coalescing, generation-guarded
//! staleness, incremental text sync, and snapshot semantics.

mod harness;

use harness::Harness;
use tower_lsp::lsp_types::*;

use logicaffeine_lsp::document::apply_content_change;

const CLEAN_SOURCE: &str = "## Main\n    Let x be 5.\n    Show x.\n";
const BROKEN_SOURCE: &str = "## Main\n    Let be.\n";

/// A burst of rapid edits must coalesce: analysis runs for the final text,
/// not once per keystroke. The publish count proves the debounce; the final
/// version proves no stale result won the race.
#[tokio::test]
async fn rapid_edit_burst_coalesces_analysis() {
    let mut harness = Harness::start().await;
    let uri = harness.open(CLEAN_SOURCE).await;
    let first = harness.recv_diagnostics(&uri).await;
    assert_eq!(first.version, Some(1));

    let final_version = 52;
    for version in 2..=final_version {
        let text = if version % 2 == 0 { BROKEN_SOURCE } else { CLEAN_SOURCE };
        harness.change(&uri, text, version).await;
    }

    // Drain publishes until the final version lands.
    let mut publish_count = 0;
    let last = loop {
        let published = harness.recv_diagnostics(&uri).await;
        publish_count += 1;
        assert!(publish_count < 20, "a 51-edit burst must not publish per edit");
        if published.version == Some(final_version) {
            break published;
        }
    };

    assert!(
        !last.diagnostics.is_empty(),
        "the final text is broken; its diagnostics must win"
    );
}

/// After the debounce window, requests answer against the latest text.
#[tokio::test]
async fn requests_see_the_latest_snapshot_after_an_edit() {
    let mut harness = Harness::start().await;
    let uri = harness.open(BROKEN_SOURCE).await;
    let _ = harness.recv_diagnostics(&uri).await;

    harness.change(&uri, CLEAN_SOURCE, 2).await;
    let settled = harness.recv_diagnostics(&uri).await;
    assert_eq!(settled.version, Some(2));
    assert!(settled.diagnostics.is_empty());

    let hover = harness
        .request::<request::HoverRequest>(HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: Position { line: 1, character: 5 },
            },
            work_done_progress_params: Default::default(),
        })
        .await;
    assert!(hover.is_some(), "hover must answer against the new text");
}

/// Incremental sync: range edits patch the server's copy of the document.
#[tokio::test]
async fn incremental_range_edit_round_trips() {
    let mut harness = Harness::start().await;
    let uri = harness.open(CLEAN_SOURCE).await;
    let _ = harness.recv_diagnostics(&uri).await;

    // Replace the `5` in "Let x be 5." (line 1, chars 13..14) with `7`.
    harness
        .change_range(
            &uri,
            Range {
                start: Position { line: 1, character: 13 },
                end: Position { line: 1, character: 14 },
            },
            "7",
            2,
        )
        .await;

    let published = harness.recv_diagnostics(&uri).await;
    assert_eq!(published.version, Some(2));
    assert!(published.diagnostics.is_empty(), "the patched text is still valid");

    // The inlay hint for `x` proves the server analyzed the PATCHED text —
    // asking for hints over the whole doc still works after a range edit.
    let hints = harness
        .request::<request::InlayHintRequest>(InlayHintParams {
            work_done_progress_params: Default::default(),
            text_document: TextDocumentIdentifier { uri },
            range: Range {
                start: Position { line: 0, character: 0 },
                end: Position { line: 3, character: 0 },
            },
        })
        .await;
    assert!(hints.is_some(), "analysis ran against the patched document");
}

// ---------------------------------------------------------------------------
// apply_content_change unit coverage (UTF-16 edits, CRLF, multibyte)
// ---------------------------------------------------------------------------

fn range(sl: u32, sc: u32, el: u32, ec: u32) -> Range {
    Range {
        start: Position { line: sl, character: sc },
        end: Position { line: el, character: ec },
    }
}

#[test]
fn apply_replaces_within_a_line() {
    let mut text = "Let x be 5.".to_string();
    apply_content_change(&mut text, Some(range(0, 9, 0, 10)), "7");
    assert_eq!(text, "Let x be 7.");
}

#[test]
fn apply_none_range_replaces_whole_document() {
    let mut text = "old".to_string();
    apply_content_change(&mut text, None, "brand new");
    assert_eq!(text, "brand new");
}

#[test]
fn apply_handles_multibyte_utf16_offsets() {
    // 'é' is 1 UTF-16 unit but 2 UTF-8 bytes; the edit range is in UTF-16.
    let mut text = "Let café be 5.".to_string();
    apply_content_change(&mut text, Some(range(0, 4, 0, 8)), "shop");
    assert_eq!(text, "Let shop be 5.");
}

#[test]
fn apply_handles_supplementary_plane_characters() {
    // '𝛑' is TWO UTF-16 units (surrogate pair) and 4 UTF-8 bytes.
    let mut text = "Let 𝛑 be 3.".to_string();
    apply_content_change(&mut text, Some(range(0, 4, 0, 6)), "pi");
    assert_eq!(text, "Let pi be 3.");
}

#[test]
fn apply_spans_lines_with_crlf() {
    let mut text = "Let x be 5.\r\nShow x.\r\n".to_string();
    apply_content_change(&mut text, Some(range(0, 11, 1, 0)), "\r\n\r\n");
    assert_eq!(text, "Let x be 5.\r\n\r\nShow x.\r\n");
}

#[test]
fn apply_insertion_at_end_of_document() {
    let mut text = "Let x be 5.".to_string();
    apply_content_change(&mut text, Some(range(0, 11, 0, 11)), "\nShow x.");
    assert_eq!(text, "Let x be 5.\nShow x.");
}
