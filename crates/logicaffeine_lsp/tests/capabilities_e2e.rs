//! The remaining capability set: document highlights (read/write kinds),
//! selection ranges (word → sentence → block → document), on-type
//! formatting, call hierarchy, and pull diagnostics.

mod harness;

use harness::Harness;
use tower_lsp::lsp_types::*;

const MUTATION_SOURCE: &str = "## Main\nLet mutable x be 5.\nSet x to 6.\nShow x.\n";

fn col(source: &str, line: u32, needle: &str) -> u32 {
    source.split('\n').nth(line as usize).unwrap().find(needle).unwrap() as u32
}

#[tokio::test]
async fn document_highlights_mark_reads_and_writes() {
    let mut harness = Harness::start().await;
    let uri = harness.open(MUTATION_SOURCE).await;
    let _ = harness.recv_diagnostics(&uri).await;

    let highlights = harness
        .request::<request::DocumentHighlightRequest>(DocumentHighlightParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: Position {
                    line: 3,
                    character: col(MUTATION_SOURCE, 3, "x"),
                },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        })
        .await
        .expect("x has highlights");

    assert!(
        highlights.len() >= 3,
        "declaration, Set target, and read all highlight: {highlights:#?}"
    );
    let write_lines: Vec<u32> = highlights
        .iter()
        .filter(|h| h.kind == Some(DocumentHighlightKind::WRITE))
        .map(|h| h.range.start.line)
        .collect();
    assert!(
        write_lines.contains(&1) && write_lines.contains(&2),
        "the Let and the Set are writes, got writes on {write_lines:?}"
    );
    assert!(
        highlights
            .iter()
            .any(|h| h.range.start.line == 3 && h.kind == Some(DocumentHighlightKind::READ)),
        "the Show is a read: {highlights:#?}"
    );
}

#[tokio::test]
async fn selection_ranges_expand_word_sentence_block_document() {
    let source = "## Main\nLet total be 1 + 2.\nShow total.\n";
    let mut harness = Harness::start().await;
    let uri = harness.open(source).await;
    let _ = harness.recv_diagnostics(&uri).await;

    let ranges = harness
        .request::<request::SelectionRangeRequest>(SelectionRangeParams {
            text_document: TextDocumentIdentifier { uri },
            positions: vec![Position {
                line: 1,
                character: col(source, 1, "total"),
            }],
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        })
        .await
        .expect("selection ranges exist");

    assert_eq!(ranges.len(), 1);
    let word = &ranges[0];
    assert_eq!(word.range.start.line, 1, "innermost = the word under the cursor");

    let sentence = word.parent.as_ref().expect("word expands to its sentence");
    assert_eq!(sentence.range.start.line, 1);
    assert!(
        sentence.range.end.character > word.range.end.character
            || sentence.range.end.line > word.range.end.line,
        "the sentence is wider than the word"
    );

    let block = sentence.parent.as_ref().expect("sentence expands to its block");
    assert_eq!(block.range.start.line, 0, "the block starts at ## Main");
    assert!(block.range.end.line >= 2, "the block spans its statements");
}

#[tokio::test]
async fn typing_a_period_normalizes_the_line() {
    // A tab-indented sentence: the canonical form is 4-space indentation
    // (the exact `largo fmt` rule), applied the moment the sentence closes.
    let source = "## Main\n\tLet x be 5.\nShow x.\n";
    let mut harness = Harness::start().await;
    let uri = harness.open(source).await;
    let _ = harness.recv_diagnostics(&uri).await;

    let edits = harness
        .request::<request::OnTypeFormatting>(DocumentOnTypeFormattingParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: Position {
                    line: 1,
                    character: "\tLet x be 5.".len() as u32,
                },
            },
            ch: ".".to_string(),
            options: FormattingOptions {
                tab_size: 4,
                insert_spaces: true,
                ..Default::default()
            },
        })
        .await
        .expect("the tab-indented line needs normalizing");

    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].range.start.line, 1);
    assert_eq!(
        edits[0].new_text, "    Let x be 5.",
        "tab indent becomes 4 spaces, content untouched"
    );
}

const CALL_SOURCE: &str = "\
## To double (n: Int) -> Int:
    Return n * 2.

## To quadruple (n: Int) -> Int:
    Return double(double(n)).

## Main
Show quadruple(10).
";

#[tokio::test]
async fn call_hierarchy_walks_both_directions() {
    let mut harness = Harness::start().await;
    let uri = harness.open(CALL_SOURCE).await;
    let _ = harness.recv_diagnostics(&uri).await;

    let items = harness
        .request::<request::CallHierarchyPrepare>(CallHierarchyPrepareParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position {
                    line: 0,
                    character: col(CALL_SOURCE, 0, "double"),
                },
            },
            work_done_progress_params: Default::default(),
        })
        .await
        .expect("preparing on a function name yields an item");
    assert_eq!(items.len(), 1);
    assert_eq!(items[0].name, "double");
    assert_eq!(items[0].kind, SymbolKind::FUNCTION);

    let incoming = harness
        .request::<request::CallHierarchyIncomingCalls>(CallHierarchyIncomingCallsParams {
            item: items[0].clone(),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        })
        .await
        .expect("double has callers");
    assert_eq!(incoming.len(), 1, "one calling function: {incoming:#?}");
    assert_eq!(incoming[0].from.name, "quadruple");
    assert_eq!(
        incoming[0].from_ranges.len(),
        2,
        "quadruple calls double twice"
    );

    let quadruple = incoming[0].from.clone();
    let outgoing = harness
        .request::<request::CallHierarchyOutgoingCalls>(CallHierarchyOutgoingCallsParams {
            item: quadruple,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        })
        .await
        .expect("quadruple calls out");
    assert_eq!(outgoing.len(), 1);
    assert_eq!(outgoing[0].to.name, "double");
}

#[tokio::test]
async fn pull_diagnostics_report_full_then_unchanged() {
    let mut harness = Harness::start().await;
    let uri = harness.open("## Main\n    Let be.\n").await;
    let _ = harness.recv_diagnostics(&uri).await;

    let report = harness
        .request::<request::DocumentDiagnosticRequest>(DocumentDiagnosticParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            identifier: None,
            previous_result_id: None,
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        })
        .await;

    let (result_id, item_count) = match report {
        DocumentDiagnosticReportResult::Report(DocumentDiagnosticReport::Full(full)) => (
            full.full_document_diagnostic_report.result_id.clone(),
            full.full_document_diagnostic_report.items.len(),
        ),
        other => panic!("first pull is a full report, got {other:?}"),
    };
    assert!(item_count > 0, "broken source has diagnostics");
    let result_id = result_id.expect("full reports carry a result id");

    let second = harness
        .request::<request::DocumentDiagnosticRequest>(DocumentDiagnosticParams {
            text_document: TextDocumentIdentifier { uri },
            identifier: None,
            previous_result_id: Some(result_id),
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        })
        .await;

    assert!(
        matches!(
            second,
            DocumentDiagnosticReportResult::Report(DocumentDiagnosticReport::Unchanged(_))
        ),
        "an unedited document answers a known result id with Unchanged, got {second:?}"
    );
}


#[tokio::test]
async fn hover_over_a_call_shows_the_function_signature() {
    let mut harness = Harness::start().await;
    let uri = harness.open(CALL_SOURCE).await;
    let _ = harness.recv_diagnostics(&uri).await;

    // The `double` inside quadruple's body.
    let hover = harness
        .request::<request::HoverRequest>(HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                position: Position {
                    line: 4,
                    character: col(CALL_SOURCE, 4, "double"),
                },
            },
            work_done_progress_params: Default::default(),
        })
        .await
        .expect("hovering a call documents the callee");

    let text = match hover.contents {
        HoverContents::Markup(m) => m.value,
        HoverContents::Scalar(MarkedString::String(s)) => s,
        other => format!("{other:?}"),
    };
    assert!(
        text.contains("double") && text.contains("Int"),
        "the hover shows the signature: {text}"
    );
}

#[tokio::test]
async fn untyped_let_gets_an_inferred_type_hint() {
    let mut harness = Harness::start().await;
    let uri = harness.open("## Main\nLet x be 5.\nShow x.\n").await;
    let _ = harness.recv_diagnostics(&uri).await;

    let hints = harness
        .request::<request::InlayHintRequest>(InlayHintParams {
            work_done_progress_params: Default::default(),
            text_document: TextDocumentIdentifier { uri },
            range: Range {
                start: Position { line: 0, character: 0 },
                end: Position { line: 3, character: 0 },
            },
        })
        .await
        .expect("an untyped Let earns a type hint");

    let rendered: Vec<String> = hints
        .iter()
        .map(|h| match &h.label {
            InlayHintLabel::String(s) => s.clone(),
            InlayHintLabel::LabelParts(parts) => {
                parts.iter().map(|p| p.value.clone()).collect()
            }
        })
        .collect();
    assert!(
        rendered.iter().any(|l| l.contains("Int")),
        "the inferred type shows inline: {rendered:?}"
    );
}

#[tokio::test]
async fn completion_after_a_colon_offers_types() {
    let source = "## Main\nLet x: \n";
    let mut harness = Harness::start().await;
    let uri = harness.open(source).await;
    let _ = harness.recv_diagnostics(&uri).await;

    let completions = harness
        .request::<request::Completion>(CompletionParams {
            text_document_position: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri },
                // Right after "Let x: " — inside the annotation position.
                position: Position { line: 1, character: 7 },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
            context: None,
        })
        .await
        .expect("a type annotation position offers completions");

    let items = match completions {
        CompletionResponse::Array(items) => items,
        CompletionResponse::List(list) => list.items,
    };
    assert!(
        items.iter().any(|i| i.label == "Int"),
        "types complete after ':': {:?}",
        items.iter().map(|i| &i.label).collect::<Vec<_>>()
    );
}

#[tokio::test]
async fn range_formatting_touches_only_intersecting_lines() {
    let mut harness = Harness::start().await;
    // BOTH lines are mis-indented (tabs); only line 2 is in the range.
    let uri = harness.open("## Main\n\tLet a be 1.\n\tLet b be 2.\n").await;
    let _ = harness.recv_diagnostics(&uri).await;

    let edits = harness
        .request::<request::RangeFormatting>(DocumentRangeFormattingParams {
            text_document: TextDocumentIdentifier { uri },
            range: Range {
                start: Position { line: 2, character: 0 },
                end: Position { line: 2, character: 12 },
            },
            options: FormattingOptions::default(),
            work_done_progress_params: Default::default(),
        })
        .await
        .expect("the mis-indented line in range must format");

    assert!(!edits.is_empty());
    for edit in &edits {
        assert_eq!(
            edit.range.start.line, 2,
            "range formatting must not touch line 1 (outside the range): {edits:#?}"
        );
    }
    assert_eq!(edits[0].new_text, "    Let b be 2.", "tab reindents to 4 spaces");
}
