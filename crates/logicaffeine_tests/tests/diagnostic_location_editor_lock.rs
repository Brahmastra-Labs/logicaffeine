//! ★ LOCK — the editor reports every LGX mistake where the author made it. ★
//!
//! The editor half of `crates/logicaffeine_tests/tests/diagnostic_location_lock.rs`: the same
//! census table, every row opened as a document, and the language server's published diagnostic
//! for it held to the construct the row names — its range must start inside that construct, on
//! line 3 or later of the file.
//!
//! The server parses a document WITHOUT the stdlib prelude, so a span here is a file offset as the
//! parser made it: a row mislocated in the editor is mislocated by the code that raised it. Rows
//! pending on another lane name the `Editor` surface in their `pending` list, which only shrinks.

#[path = "diagnostic_location/rows.rs"]
mod rows;

use logicaffeine_lsp::document::DocumentState;
use rows::{Reach, Severity, Surface};
use tower_lsp::lsp_types::DiagnosticSeverity;

#[test]
fn the_editor_reports_every_mistake_where_it_is() {
    let mut wrong = Vec::new();
    let mut checked = 0;
    for row in rows::rows() {
        let Reach::Page {
            source,
            at,
            says,
            severity,
            pending,
        } = &row.reach
        else {
            continue;
        };
        let doc = DocumentState::new(source.clone(), 1);
        let want = match severity {
            Severity::Error => DiagnosticSeverity::ERROR,
            Severity::Warning => DiagnosticSeverity::WARNING,
        };
        let found = doc
            .diagnostics
            .iter()
            .find(|d| d.message.contains(says) && d.severity == Some(want));
        let from = source
            .find(at)
            .unwrap_or_else(|| panic!("{:?}: the fixture must contain {at:?}", row.member));
        let (located, reported) = match found {
            Some(d) => {
                checked += 1;
                let start = doc.line_index.offset(d.range.start);
                (
                    start >= from && start < from + at.len(),
                    format!("{}:{}", d.range.start.line + 1, d.range.start.character + 1),
                )
            }
            None => (
                false,
                format!(
                    "nowhere — no {want:?} carries {says:?}; published: {:?}",
                    doc.diagnostics
                        .iter()
                        .map(|d| &d.message)
                        .collect::<Vec<_>>()
                ),
            ),
        };
        match (pending.iter().find(|(s, _)| *s == Surface::Editor), located) {
            (None, true) | (Some(_), false) => {}
            (None, false) => wrong.push(format!(
                "{:?}: the editor reports it at {reported}, not inside `{at}`",
                row.member
            )),
            (Some((_, waits_on)), true) => wrong.push(format!(
                "{:?}: the editor now reports it inside `{at}` — the work it waited on ({waits_on}) \
                 has landed, so remove `Surface::Editor` from its `pending` list",
                row.member
            )),
        }
    }
    assert!(
        checked >= 60,
        "the census reached only {checked} rows — the table shrank or the editor stopped answering"
    );
    assert!(
        wrong.is_empty(),
        "the editor's diagnostics:\n{}",
        wrong.join("\n")
    );
}

#[test]
fn the_editor_reports_nothing_but_the_mistake() {
    // A second error on a line that holds no mistake sends the author to fix code that is fine.
    // Warnings are left out: a fixture's own lint findings are real.
    let mut wrong = Vec::new();
    for row in rows::rows() {
        let Reach::Page { source, says, .. } = &row.reach else {
            continue;
        };
        let doc = DocumentState::new(source.clone(), 1);
        for d in doc
            .diagnostics
            .iter()
            .filter(|d| d.severity == Some(DiagnosticSeverity::ERROR))
        {
            if d.message.contains(says) {
                continue;
            }
            wrong.push(format!(
                "{:?}: the editor also reports {:?}",
                row.member, d.message
            ));
        }
    }
    assert!(
        wrong.is_empty(),
        "errors about constructs that are fine:\n{}",
        wrong.join("\n")
    );
}

/// L11 (lang2's `fragment_mode`, `pipeline.rs::try_parse_block`): when a whole-file parse fails, the
/// editor falls back to parsing each `##` block on its own for recovery — and a fragment naming
/// `Mount App` has no OTHER block in view to confirm `App` is real. Before `fragment_mode`, the
/// fragment ran the parser's WHOLE-FILE checks anyway (`check_mounts_name_something` among them) and
/// invented `` `Mount App`: no component named `App` `` for a component the file plainly declares a
/// few lines up — on every page with any unrelated mistake, not just this one.
///
/// This only asserts the FALSE refusal is gone. It does NOT assert a fragment can catch a REAL
/// cross-block mistake (a store write reaching into another block's store, a fetch answering the
/// wrong type) — `fragment_mode`'s own doc comment is explicit that cross-block LGX checks stay
/// ABSENT from a fragment's own reparse, not wrong, until the next whole-file reparse; that gap is
/// still open (see this lane's progress file).
#[test]
fn a_fragment_reparse_does_not_invent_a_missing_component() {
    // `## To broken`'s own incomplete sentence breaks ONLY that block, forcing per-block recovery —
    // `## Component App` and `## Main` are each syntactically fine on their own, so `## Main`'s
    // fragment must not doubt a component the FILE plainly declares, even though its own isolated
    // reparse cannot see that declaration confirmed.
    let source = "## Component App\nState:\n    Let n be 0.\nView:\n    <p>{n}</p>\n\n\
                  ## To broken\n    Let x be\n\n\
                  ## Main\nMount App.\n";
    let doc = DocumentState::new(source.to_string(), 1);
    let invented: Vec<_> = doc
        .diagnostics
        .iter()
        .filter(|d| d.message.contains("no component named `App`"))
        .map(|d| &d.message)
        .collect();
    assert!(
        invented.is_empty(),
        "a per-block fragment invented a missing component `App`, which the file plainly declares: {invented:?}\n\
         all diagnostics: {:?}",
        doc.diagnostics.iter().map(|d| &d.message).collect::<Vec<_>>()
    );
}
