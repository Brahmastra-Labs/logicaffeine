//! The teaching-corpus golden: everything the server TEACHES over one
//! representative document — hover markdown verbatim, completion teaching
//! text, signature-help documentation, code-lens titles, quickfix titles
//! over a battery of broken snippets, and the diagnostic docs-links — all
//! rendered through the REAL pipeline and committed. A wording change
//! anywhere in the teaching surface shows up here as a reviewable diff.
//!
//! (The socratic explanation PROSE churns in `socratic_corpus` — here the
//! diagnostics section pins codes and hrefs only, so no text churns twice.)

use logicaffeine_lsp::document::DocumentState;
use tower_lsp::lsp_types::{CompletionResponse, HoverContents, Position, Range, Url};

const CORPUS: &str = "\
## Note
Doubles a number.

## To double (n: Int) -> Int:
    Return n * 2.

## A Point has:
    An x: Int.
    A y: Int.

## Main
Let answer be double(21).
Let mutable count be 0.
Set count to answer.
Let hash be md5([1, 2, 3]).
Call double with 7.
Show answer.

## Theorem: Socrates
Given: All men are mortal. Socrates is a man.
Prove: Socrates is mortal.
Proof: Auto.
";

/// (line, character, label) — where hover teaches, and what we call it.
const HOVER_PROBES: &[(u32, u32, &str)] = &[
    (0, 0, "the ## Note header"),
    (11, 0, "the Let keyword"),
    (11, 14, "the documented call to double"),
    (13, 0, "the Set keyword"),
    (14, 12, "the stdlib name md5"),
    (6, 5, "the Point struct name"),
    (18, 0, "the ## Theorem header"),
];

/// Completion labels whose teaching text the golden pins, per context.
const STATEMENT_COMPLETIONS: &[&str] =
    &["Let", "Set", "If", "While", "Repeat", "Return", "Show", "Give", "Push", "Call", "Inspect"];
const EXPRESSION_COMPLETIONS: &[&str] = &["double", "answer", "md5", "Message"];

/// Broken snippets that must produce the decision table's quickfixes.
const QUICKFIX_BATTERY: &[(&str, &str)] = &[
    ("zero-index", "## Main\nLet xs be [1, 2, 3].\nShow item 0 of xs.\n"),
    ("use-after-move", "## Main\nLet x be 5.\nLet a be 0.\nGive x to a.\nShow x.\n"),
    ("unused-variable", "## Main\nLet unused be 5.\nShow 1.\n"),
];

fn render() -> String {
    let doc = DocumentState::new(CORPUS.to_string(), 1);
    let uri = Url::parse("file:///corpus.lg").unwrap();
    let mut out = String::new();

    out.push_str("# Hover\n");
    for (line, character, label) in HOVER_PROBES {
        out.push_str(&format!("\n## {line}:{character} — {label}\n"));
        match logicaffeine_lsp::hover::hover(&doc, Position { line: *line, character: *character })
        {
            Some(hover) => {
                let HoverContents::Markup(markup) = hover.contents else {
                    panic!("hover is always markdown");
                };
                for l in markup.value.lines() {
                    out.push_str("    ");
                    out.push_str(l);
                    out.push('\n');
                }
            }
            None => out.push_str("    (no hover)\n"),
        }
    }

    out.push_str("\n# Completions — statement context (after Show answer.)\n");
    let items = completion_items(&doc, Position { line: 16, character: 12 });
    for label in STATEMENT_COMPLETIONS {
        out.push_str(&render_item(&items, label));
    }

    out.push_str("\n# Completions — expression context (after be)\n");
    let items = completion_items(&doc, Position { line: 11, character: 14 });
    for label in EXPRESSION_COMPLETIONS {
        out.push_str(&render_item(&items, label));
    }

    out.push_str("\n# Signature help — Call double with 7\n");
    match logicaffeine_lsp::signature_help::signature_help(
        &doc,
        Position { line: 15, character: 17 },
    ) {
        Some(help) => {
            let sig = &help.signatures[0];
            out.push_str(&format!(
                "label: {} | active: {} | doc: {}\n",
                sig.label,
                help.active_parameter.unwrap_or(99),
                match &sig.documentation {
                    Some(tower_lsp::lsp_types::Documentation::MarkupContent(c)) =>
                        c.value.as_str(),
                    Some(_) => "(plain)",
                    None => "(none)",
                }
            ));
        }
        None => out.push_str("(no signature help)\n"),
    }

    out.push_str("\n# Code lenses\n");
    for lens in logicaffeine_lsp::code_lens::code_lenses(&doc, &uri) {
        if let Some(command) = lens.command {
            out.push_str(&format!(
                "- line {}: {} ({})\n",
                lens.range.start.line, command.title, command.command
            ));
        }
    }

    out.push_str("\n# Quickfixes\n");
    for (code, snippet) in QUICKFIX_BATTERY {
        let broken = DocumentState::new(snippet.to_string(), 1);
        out.push_str(&format!("\n## {code}\n"));
        for action in
            logicaffeine_lsp::code_actions::code_actions(&broken, Range::default(), &uri)
        {
            if let tower_lsp::lsp_types::CodeActionOrCommand::CodeAction(action) = action {
                out.push_str(&format!("- {}\n", action.title));
            }
        }
    }

    out.push_str("\n# Diagnostic docs links (code → quickguide anchor)\n");
    for kind in error_kinds::all_parse_error_kinds() {
        let decision = logicaffeine_lsp::diagnostics::decision_for(&kind);
        if let logicaffeine_lsp::diagnostics::DocsLink::Anchor(anchor) = decision.docs {
            out.push_str(&format!(
                "- {} → #{anchor}\n",
                decision.code.expect("anchors require codes")
            ));
        }
    }

    out
}

fn completion_items(
    doc: &DocumentState,
    position: Position,
) -> Vec<tower_lsp::lsp_types::CompletionItem> {
    match logicaffeine_lsp::completion::completions(doc, position) {
        Some(CompletionResponse::Array(items)) => items,
        other => panic!("expected array completions, got {other:?}"),
    }
}

fn render_item(items: &[tower_lsp::lsp_types::CompletionItem], label: &str) -> String {
    match items.iter().find(|i| i.label == label) {
        Some(item) => format!(
            "- {} [{}] — {} — doc:{}\n",
            item.label,
            item.kind.map(|k| format!("{k:?}")).unwrap_or_default(),
            item.detail.as_deref().unwrap_or("(no detail)"),
            if item.documentation.is_some() { "yes" } else { "NO" },
        ),
        None => format!("- {label} MISSING\n"),
    }
}

#[path = "harness/error_kinds.rs"]
mod error_kinds;

#[test]
fn the_teaching_surface_renders_exactly_this() {
    let actual = render();
    if std::env::var("UPDATE_TEACHING_GOLDEN").is_ok() {
        // Deliberate, reviewed regeneration only: the diff of the committed
        // file IS the review surface.
        std::fs::write(
            concat!(env!("CARGO_MANIFEST_DIR"), "/tests/goldens/teaching_corpus.md"),
            &actual,
        )
        .expect("write golden");
    }
    let golden = include_str!("goldens/teaching_corpus.md");
    assert_eq!(
        actual, golden,
        "\n--- actual teaching corpus (regenerate with UPDATE_TEACHING_GOLDEN=1 ONLY as a \
         deliberate, reviewed teaching change) ---\n{actual}"
    );
}
