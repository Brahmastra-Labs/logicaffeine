//! Flycheck behavior over the real server loop, with a mock runner so no
//! test spawns cargo: findings merge with interactive diagnostics under
//! their own source, unavailable toolchains degrade silently, edits clear
//! stale findings, and a newer save always beats a slower older one.

mod harness;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use harness::Harness;
use tower_lsp::lsp_types::*;
use tower_lsp::LspService;

use logicaffeine_language::token::Span;
use logicaffeine_lsp::flycheck::{FlycheckFinding, FlycheckRunner, FLYCHECK_SOURCE};
use logicaffeine_lsp::server::LogicAffeineServer;

const CLEAN_SOURCE: &str = "## Main\n    Let x be 5.\n    Show x.\n";

/// Span of `Let x be 5.` inside CLEAN_SOURCE.
fn let_span() -> Span {
    let start = CLEAN_SOURCE.find("Let x").unwrap();
    Span::new(start, start + "Let x be 5.".len())
}

enum MockBehavior {
    Findings(Vec<FlycheckFinding>),
    Unavailable,
    /// First call sleeps then reports "OLD"; later calls report "NEW".
    SlowFirst,
}

struct MockRunner {
    behavior: MockBehavior,
    calls: Arc<AtomicUsize>,
}

impl FlycheckRunner for MockRunner {
    fn check(&self, _source: &str, _workspace_key: &str) -> Option<Vec<FlycheckFinding>> {
        let call = self.calls.fetch_add(1, Ordering::SeqCst);
        match &self.behavior {
            MockBehavior::Unavailable => None,
            MockBehavior::Findings(findings) => Some(
                findings
                    .iter()
                    .map(|f| FlycheckFinding {
                        message: f.message.clone(),
                        suggestion: f.suggestion.clone(),
                        span: f.span,
                    })
                    .collect(),
            ),
            MockBehavior::SlowFirst => {
                if call == 0 {
                    std::thread::sleep(Duration::from_millis(400));
                    Some(vec![FlycheckFinding {
                        message: "OLD".to_string(),
                        suggestion: None,
                        span: Some(let_span()),
                    }])
                } else {
                    Some(vec![FlycheckFinding {
                        message: "NEW".to_string(),
                        suggestion: None,
                        span: Some(let_span()),
                    }])
                }
            }
        }
    }
}

async fn start_with(behavior: MockBehavior) -> (Harness, Arc<AtomicUsize>) {
    let calls = Arc::new(AtomicUsize::new(0));
    let runner_calls = Arc::clone(&calls);
    let harness = Harness::start_with_service(move || {
        LspService::new(move |client| {
            LogicAffeineServer::with_flycheck(
                client,
                Box::new(MockRunner {
                    behavior,
                    calls: runner_calls,
                }),
            )
        })
    })
    .await;
    (harness, calls)
}

fn rustc_diags(published: &PublishDiagnosticsParams) -> Vec<&Diagnostic> {
    published
        .diagnostics
        .iter()
        .filter(|d| d.source.as_deref() == Some(FLYCHECK_SOURCE))
        .collect()
}

#[tokio::test]
async fn save_publishes_rustc_findings_under_their_own_source() {
    let (mut harness, _calls) = start_with(MockBehavior::Findings(vec![FlycheckFinding {
        message: "Cannot use 'x' after giving it away.".to_string(),
        suggestion: Some("Give 'a copy of x' instead.".to_string()),
        span: Some(let_span()),
    }]))
    .await;

    let uri = harness.open(CLEAN_SOURCE).await;
    let first = harness.recv_diagnostics(&uri).await;
    assert!(rustc_diags(&first).is_empty(), "nothing before the first save");

    harness.save(&uri).await;
    let published = harness.recv_diagnostics(&uri).await;
    let rustc = rustc_diags(&published);
    assert_eq!(rustc.len(), 1, "the finding surfaces: {published:#?}");
    assert_eq!(rustc[0].range.start.line, 1, "the span maps to the Let line");
    assert!(
        rustc[0].message.contains("giving it away"),
        "speaks English: {}",
        rustc[0].message
    );
    assert!(
        rustc[0].message.contains("a copy of x"),
        "the suggestion travels with the finding: {}",
        rustc[0].message
    );
}

#[tokio::test]
async fn missing_toolchain_degrades_silently() {
    let (mut harness, calls) = start_with(MockBehavior::Unavailable).await;

    let uri = harness.open(CLEAN_SOURCE).await;
    let _ = harness.recv_diagnostics(&uri).await;

    harness.save(&uri).await;
    // Round-trip through a request to let any (wrong) publish land first.
    let _ = harness
        .request::<request::HoverRequest>(HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position { line: 1, character: 5 },
            },
            work_done_progress_params: Default::default(),
        })
        .await;
    tokio::time::sleep(Duration::from_millis(200)).await;

    assert!(calls.load(Ordering::SeqCst) >= 1, "the runner was consulted");
    for published in harness.drain_pending_diagnostics(&uri) {
        assert!(
            rustc_diags(&published).is_empty(),
            "an unavailable toolchain must publish nothing: {published:#?}"
        );
    }
}

#[tokio::test]
async fn an_edit_clears_stale_flycheck_findings() {
    let (mut harness, _calls) = start_with(MockBehavior::Findings(vec![FlycheckFinding {
        message: "stale candidate".to_string(),
        suggestion: None,
        span: Some(let_span()),
    }]))
    .await;

    let uri = harness.open(CLEAN_SOURCE).await;
    let _ = harness.recv_diagnostics(&uri).await;

    harness.save(&uri).await;
    let after_save = harness.recv_diagnostics(&uri).await;
    assert_eq!(rustc_diags(&after_save).len(), 1);

    // Any edit invalidates the positions the findings were computed against.
    harness
        .change(&uri, "## Main\n    Let y be 7.\n    Show y.\n", 2)
        .await;
    let after_edit = harness.recv_diagnostics(&uri).await;
    assert!(
        rustc_diags(&after_edit).is_empty(),
        "stale rustc findings must not survive an edit: {after_edit:#?}"
    );
}

#[tokio::test]
async fn a_newer_save_always_beats_a_slower_older_one() {
    let (mut harness, calls) = start_with(MockBehavior::SlowFirst).await;

    let uri = harness.open(CLEAN_SOURCE).await;
    let _ = harness.recv_diagnostics(&uri).await;

    harness.save(&uri).await;
    // Give the first (slow) run a moment to actually start.
    tokio::time::sleep(Duration::from_millis(50)).await;
    harness.save(&uri).await;

    // NEW must arrive; OLD must never appear, before or after.
    let published = harness.recv_diagnostics(&uri).await;
    let rustc = rustc_diags(&published);
    assert_eq!(rustc.len(), 1, "{published:#?}");
    assert!(rustc[0].message.contains("NEW"), "got: {}", rustc[0].message);

    // Wait past the slow run's completion; its stale result must be dropped.
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(calls.load(Ordering::SeqCst), 2);
    for published in harness.drain_pending_diagnostics(&uri) {
        for diagnostic in rustc_diags(&published) {
            assert!(
                !diagnostic.message.contains("OLD"),
                "a stale slow result clobbered a newer save: {published:#?}"
            );
        }
    }
}

#[tokio::test]
async fn the_flycheck_setting_disables_and_reenables_the_pass() {
    let (mut harness, calls) = start_with(MockBehavior::Findings(vec![FlycheckFinding {
        message: "finding".to_string(),
        suggestion: None,
        span: Some(let_span()),
    }]))
    .await;

    let uri = harness.open(CLEAN_SOURCE).await;
    let _ = harness.recv_diagnostics(&uri).await;

    // Enabled by default.
    harness.save(&uri).await;
    let published = harness.recv_diagnostics(&uri).await;
    assert_eq!(rustc_diags(&published).len(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 1);

    // Disable via configuration: saves stop consulting the runner AND the
    // stale findings clear.
    harness
        .notify::<notification::DidChangeConfiguration>(DidChangeConfigurationParams {
            settings: serde_json::json!({
                "logicaffeine": { "flycheck": { "enable": false } }
            }),
        })
        .await;
    harness.save(&uri).await;
    // Round-trip to let any (wrong) work land.
    let _ = harness
        .request::<request::HoverRequest>(HoverParams {
            text_document_position_params: TextDocumentPositionParams {
                text_document: TextDocumentIdentifier { uri: uri.clone() },
                position: Position { line: 1, character: 5 },
            },
            work_done_progress_params: Default::default(),
        })
        .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        calls.load(Ordering::SeqCst),
        1,
        "a disabled flycheck must not consult the runner"
    );
    let cleared = harness.drain_pending_diagnostics(&uri);
    assert!(
        cleared
            .last()
            .map(|p| rustc_diags(p).is_empty())
            .unwrap_or(true),
        "disabling clears previously published findings: {cleared:#?}"
    );

    // Re-enable: the pass comes back.
    harness
        .notify::<notification::DidChangeConfiguration>(DidChangeConfigurationParams {
            settings: serde_json::json!({
                "logicaffeine": { "flycheck": { "enable": true } }
            }),
        })
        .await;
    harness.save(&uri).await;
    // The disable-time retraction publish may still be in flight; take
    // publishes until the flycheck one lands.
    let mut attempts = 0;
    let published = loop {
        let published = harness.recv_diagnostics(&uri).await;
        if !rustc_diags(&published).is_empty() {
            break published;
        }
        attempts += 1;
        assert!(attempts < 5, "the re-enabled flycheck must publish its finding");
    };
    assert_eq!(rustc_diags(&published).len(), 1);
    assert_eq!(calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn findings_overlapping_interactive_errors_are_deduplicated() {
    // The interactive pipeline already reports use-after-move on this source;
    // a rustc finding on the same spot must not double-report.
    let source = "## Main\n    Let x be 5.\n    Let y be 0.\n    Give x to y.\n    Show x.\n";
    let show_start = source.find("Show x").unwrap();

    let (mut harness, _calls) = start_with(MockBehavior::Findings(vec![FlycheckFinding {
        message: "duplicate of the interactive finding".to_string(),
        suggestion: None,
        span: Some(Span::new(show_start, show_start + "Show x.".len())),
    }]))
    .await;

    let uri = harness.open(source).await;
    let first = harness.recv_diagnostics(&uri).await;
    assert!(
        !first.diagnostics.is_empty(),
        "the interactive pipeline reports use-after-move here"
    );

    harness.save(&uri).await;
    let published = harness.recv_diagnostics(&uri).await;
    assert!(
        rustc_diags(&published).is_empty(),
        "rustc findings overlapping interactive errors are noise: {published:#?}"
    );
}
