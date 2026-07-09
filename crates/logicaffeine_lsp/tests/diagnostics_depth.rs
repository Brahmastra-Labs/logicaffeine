//! Socratic diagnostics depth: the typechecker's findings surface as LSP
//! diagnostics with REAL statement spans (via the parser's stmt_spans
//! side-table), every failing statement reports (not just the first), and
//! field errors know which fields exist.

mod harness;

use harness::Harness;
use tower_lsp::lsp_types::*;

/// Two functions each returning Text from `-> Int`: pure checker errors —
/// the parser is happy with this program, so before the typechecker was
/// surfaced the editor showed NOTHING here.
const TWO_BAD_FUNCTIONS: &str = "\
## To f () -> Int:
    Return \"oops\".

## To g () -> Int:
    Return \"worse\".

## Main
Show 1.
";

#[tokio::test]
async fn every_failing_statement_reports_with_its_own_span() {
    let mut harness = Harness::start().await;
    let uri = harness.open(TWO_BAD_FUNCTIONS).await;
    let published = harness.recv_diagnostics(&uri).await;

    let type_errors: Vec<&Diagnostic> = published
        .diagnostics
        .iter()
        .filter(|d| {
            matches!(&d.code, Some(NumberOrString::String(c)) if c == "type-mismatch")
        })
        .collect();
    assert_eq!(
        type_errors.len(),
        2,
        "both bad functions must report, got: {:#?}",
        published.diagnostics
    );

    assert_eq!(
        type_errors[0].range.start.line, 0,
        "first error anchors on the first function"
    );
    assert!(
        type_errors[1].range.start.line >= 3,
        "second error anchors on the second function, got {:?}",
        type_errors[1].range
    );
    for diagnostic in &type_errors {
        assert!(
            diagnostic.message.contains("Int") && diagnostic.message.contains("Text"),
            "the socratic message names both types: {}",
            diagnostic.message
        );
    }
}

#[tokio::test]
async fn use_after_move_cause_points_at_the_give_that_moved_it() {
    // `x` appears as the RECIPIENT of the first Give (line 4) and the OBJECT
    // of the second (line 5). The cause link must point at the Give that
    // moved x — a proximity heuristic that matches any Give near an `x`
    // picks line 4 and lies.
    let source = "\
## Main
Let y be 6.
Let x be 5.
Let a be 0.
Give y to x.
Give x to a.
Show x.
";
    let mut harness = Harness::start().await;
    let uri = harness.open(source).await;
    let published = harness.recv_diagnostics(&uri).await;

    let move_diag = published
        .diagnostics
        .iter()
        .find(|d| {
            matches!(&d.code, Some(NumberOrString::String(c)) if c == "use-after-move")
        })
        .unwrap_or_else(|| {
            panic!("Show x after Give x must error, got: {:#?}", published.diagnostics)
        });
    let related = move_diag
        .related_information
        .as_ref()
        .expect("use-after-move links its cause");
    assert_eq!(
        related[0].location.range.start.line, 5,
        "the cause is `Give x to a.` on line 5, not the Give that RECEIVED x"
    );
}

#[tokio::test]
async fn coded_diagnostics_link_the_quickguide_via_code_description() {
    // A use-after-move must carry a codeDescription that opens the guide —
    // the diagnostic doesn't just report, it hands the reader the lesson.
    let source = "\
## Main
Let x be 5.
Let a be 0.
Give x to a.
Show x.
";
    let mut harness = Harness::start().await;
    let uri = harness.open(source).await;
    let published = harness.recv_diagnostics(&uri).await;

    let move_diag = published
        .diagnostics
        .iter()
        .find(|d| {
            matches!(&d.code, Some(NumberOrString::String(c)) if c == "use-after-move")
        })
        .unwrap_or_else(|| {
            panic!("Show x after Give x must error, got: {:#?}", published.diagnostics)
        });
    let description = move_diag
        .code_description
        .as_ref()
        .expect("use-after-move carries a docs link");
    assert!(
        description.href.as_str().contains("LOGOS_QUICKGUIDE.md#"),
        "the docs link opens a real quickguide anchor: {}",
        description.href
    );
}

#[tokio::test]
async fn multiple_broken_sentences_each_report_and_good_code_stays_alive() {
    // Two independently broken sentences among four good ones. The old
    // recovery stopped at the first failure per block: one error, dead index.
    let source = "## Main\nLet a be 1.\nLet be.\nLet b be 2.\nSet to.\nShow a.\nShow b.\n";
    let mut harness = Harness::start().await;
    let uri = harness.open(source).await;
    let published = harness.recv_diagnostics(&uri).await;

    let error_lines: Vec<u32> = published
        .diagnostics
        .iter()
        .filter(|d| d.severity == Some(DiagnosticSeverity::ERROR))
        .map(|d| d.range.start.line)
        .collect();
    assert!(
        error_lines.contains(&2) && error_lines.contains(&4),
        "both broken sentences must report (lines 2 and 4), got errors on {error_lines:?}: {:#?}",
        published.diagnostics
    );

    // The good statements around the breakage stay indexed: goto-def on the
    // `b` in `Show b.` still lands on its Let.
    let response = harness
        .request::<request::GotoDefinition>(GotoDefinitionParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position { line: 6, character: 5 },
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        })
        .await;
    let location = match response {
        Some(GotoDefinitionResponse::Scalar(loc)) => loc,
        Some(GotoDefinitionResponse::Array(locs)) if !locs.is_empty() => locs[0].clone(),
        other => panic!("goto-def must survive broken neighbors, got {other:?}"),
    };
    assert_eq!(location.range.start.line, 3, "b is defined on line 3");
}

#[tokio::test]
async fn unused_variable_is_a_faded_hint_with_a_removal_fix() {
    let source = "## Main\nLet unused be 5.\nLet used be 6.\nShow used.\n";
    let mut harness = Harness::start().await;
    let uri = harness.open(source).await;
    let published = harness.recv_diagnostics(&uri).await;

    let hint = published
        .diagnostics
        .iter()
        .find(|d| {
            matches!(&d.code, Some(NumberOrString::String(c)) if c == "unused-variable")
        })
        .unwrap_or_else(|| {
            panic!("'unused' is never read — expected a hint, got: {:#?}", published.diagnostics)
        });

    assert_eq!(hint.severity, Some(DiagnosticSeverity::HINT));
    assert_eq!(
        hint.tags.as_deref(),
        Some(&[DiagnosticTag::UNNECESSARY][..]),
        "editors fade UNNECESSARY-tagged ranges"
    );
    assert_eq!(hint.range.start.line, 1, "the hint anchors on the unused Let");

    // No hint for the used variable.
    assert!(
        !published.diagnostics.iter().any(|d| {
            matches!(&d.code, Some(NumberOrString::String(c)) if c == "unused-variable")
                && d.range.start.line == 2
        }),
        "'used' is read by Show and must not be flagged"
    );

    // The quickfix removes the whole statement.
    let actions = harness
        .request::<request::CodeActionRequest>(CodeActionParams {
            text_document: TextDocumentIdentifier { uri: uri.clone() },
            range: hint.range,
            context: CodeActionContext {
                diagnostics: vec![hint.clone()],
                ..Default::default()
            },
            work_done_progress_params: Default::default(),
            partial_result_params: Default::default(),
        })
        .await
        .unwrap_or_default();

    let removal = actions
        .iter()
        .find_map(|a| match a {
            CodeActionOrCommand::CodeAction(action) if action.title.contains("Remove") => {
                Some(action)
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("expected a Remove quickfix, got: {actions:#?}"));

    let edits = removal
        .edit
        .as_ref()
        .and_then(|e| e.changes.as_ref())
        .and_then(|c| c.get(&uri))
        .expect("the removal edits this document");
    assert_eq!(edits.len(), 1);
    assert_eq!(edits[0].new_text, "", "removal deletes, not rewrites");
    assert_eq!(
        edits[0].range.start.line, 1,
        "the deletion covers the unused statement"
    );
}

#[tokio::test]
async fn field_not_found_reports_the_available_fields() {
    let source = "\
## A Point has:
    An x: Int.
    A y: Int.

## Main
Let p be a new Point.
Let q be p's z.
Show q.
";
    let mut harness = Harness::start().await;
    let uri = harness.open(source).await;
    let published = harness.recv_diagnostics(&uri).await;

    let field_error = published
        .diagnostics
        .iter()
        .find(|d| {
            matches!(&d.code, Some(NumberOrString::String(c)) if c == "field-not-found")
        })
        .unwrap_or_else(|| {
            panic!(
                "a field-not-found diagnostic must surface, got: {:#?}",
                published.diagnostics
            )
        });

    assert!(
        field_error.message.contains('x') && field_error.message.contains('y'),
        "the message lists the fields that DO exist: {}",
        field_error.message
    );
}
