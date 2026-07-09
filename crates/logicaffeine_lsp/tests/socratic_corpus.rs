//! The socratic-explanation corpus golden: one representative instance of
//! EVERY `ParseErrorKind`, rendered through the real `socratic_explanation`
//! and committed. When the teaching prose changes — deliberately or by
//! accident — the diff IS the review.
//!
//! The companion contract lock makes the socratic shape mechanical: every
//! explanation must genuinely guide (a `?` question, or an explicit
//! `Tip:`/`Try `), with a wildcard-free exemption table for the kinds that
//! deliberately don't (and a both-directions check so a kind that starts
//! teaching leaves the table).

#[path = "harness/error_kinds.rs"]
mod error_kinds;

use error_kinds::{all_parse_error_kinds, kind_label, parse_error_kind_guard};
use logicaffeine_base::Interner;
use logicaffeine_language::error::{socratic_explanation, ParseError};
use logicaffeine_language::token::Span;

fn render_corpus() -> String {
    let interner = Interner::new();
    let mut out = String::new();
    for kind in all_parse_error_kinds() {
        parse_error_kind_guard(&kind);
        out.push_str(&format!("── {}\n", kind_label(&kind)));
        let error = ParseError { kind, span: Span::new(0, 1) };
        out.push_str(&socratic_explanation(&error, &interner));
        out.push_str("\n\n");
    }
    out
}

#[test]
fn every_socratic_explanation_matches_the_committed_golden() {
    let actual = render_corpus();
    let golden = include_str!("goldens/socratic_explanations.txt");
    assert_eq!(
        actual, golden,
        "\n--- actual corpus (update goldens/socratic_explanations.txt \
         ONLY as a deliberate, reviewed teaching change) ---\n{actual}"
    );
}

/// Kinds exempt from the guiding-question contract, each with its reason.
/// Both directions: an exempt kind that now teaches must leave this table.
const CONTRACT_EXEMPT: &[(&str, &str)] = &[
    ("Custom", "carries caller-authored prose verbatim; the caller owns the contract"),
];

#[test]
fn every_explanation_guides_a_question_or_a_tip() {
    let interner = Interner::new();
    for kind in all_parse_error_kinds() {
        let label = kind_label(&kind);
        let exempt = CONTRACT_EXEMPT.iter().any(|(name, _)| *name == label);
        let error = ParseError { kind, span: Span::new(0, 1) };
        let explanation = socratic_explanation(&error, &interner);
        let teaches = explanation.contains('?')
            || explanation.contains("Tip:")
            || explanation.contains("Try ");

        if exempt {
            assert!(
                !teaches,
                "{label}: is CONTRACT_EXEMPT but now teaches — remove it from the table"
            );
        } else {
            assert!(
                teaches,
                "{label}: the explanation must guide (a '?' question, a 'Tip:', or a \
                 'Try …') — it currently only reports:\n{explanation}"
            );
        }
    }
    for (name, reason) in CONTRACT_EXEMPT {
        assert!(!reason.is_empty(), "{name}: exemptions record their reason");
        assert!(
            all_parse_error_kinds().iter().any(|k| kind_label(k) == *name),
            "{name}: stale exemption — no such ParseErrorKind"
        );
    }
}

/// The editor points at spans; prose that recites byte offsets ("at position
/// 47") is noise the reader cannot use. The corpus renders every kind at
/// span 0..1 — no explanation may leak the offset.
#[test]
fn explanations_never_recite_byte_positions() {
    let corpus = render_corpus();
    assert!(
        !corpus.to_lowercase().contains("at position"),
        "an explanation recites a byte offset — the span already carries the location:\n{corpus}"
    );
}
