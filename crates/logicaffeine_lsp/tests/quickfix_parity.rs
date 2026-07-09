//! The decision-table ↔ code-actions parity ratchet: every quickfix the
//! decision table PROMISES (`Quickfix::Provided(title)`) must actually be
//! produced by `code_actions` on a real broken program. A promised fix with
//! no producer, or a battery snippet that stops producing it, fails here —
//! `decision_for` and `code_actions.rs` can never drift apart silently.

#[path = "harness/error_kinds.rs"]
mod error_kinds;

use error_kinds::{all_parse_error_kinds, kind_label};
use logicaffeine_lsp::diagnostics::{decision_for, Quickfix};
use logicaffeine_lsp::document::DocumentState;
use tower_lsp::lsp_types::{CodeActionOrCommand, Range, Url};

/// One broken program per promised quickfix, keyed by the kind's label.
const BATTERY: &[(&str, &str)] = &[
    ("IsValueEquality", "## Main\nLet x be 5.\nx is 5.\n"),
    ("ZeroIndex", "## Main\nLet xs be [1, 2, 3].\nShow item 0 of xs.\n"),
    ("UseAfterMove", "## Main\nLet x be 5.\nLet a be 0.\nGive x to a.\nShow x.\n"),
];

/// Promised quickfixes with NO producing program today — each a recorded,
/// dated gap this ratchet found, not a silent one. Both directions: the
/// moment a battery snippet CAN produce the fix, the entry must move up.
const NOT_YET_PRODUCIBLE: &[(&str, &str)] = &[(
    "UndefinedVariable",
    "the analysis pipeline does not yet emit undefined-variable for any program \
     (typo'd names pass silently — `Show contu.` reports nothing); unresolved-reference \
     detection is the warnings-layer work that makes this promise real",
)];

/// Match a decision title template against a produced action title:
/// `…` and `<placeholder>` segments match any text.
fn title_matches(template: &str, actual: &str) -> bool {
    let (open, close) = match (template.find('…'), template.find('<')) {
        (Some(i), _) => (i, i + '…'.len_utf8()),
        (None, Some(i)) => match template[i..].find('>') {
            Some(j) => (i, i + j + 1),
            None => return template == actual,
        },
        (None, None) => return template == actual,
    };
    let (prefix, suffix) = (&template[..open], &template[close..]);
    actual.starts_with(prefix) && actual.ends_with(suffix) && actual.len() >= prefix.len() + suffix.len()
}

#[test]
fn every_promised_quickfix_is_produced_by_a_real_program() {
    let uri = Url::parse("file:///battery.lg").unwrap();
    for kind in all_parse_error_kinds() {
        let label = kind_label(&kind);
        let decision = decision_for(&kind);
        let Quickfix::Provided(template) = decision.quickfix else { continue };

        if let Some((_, reason)) = NOT_YET_PRODUCIBLE.iter().find(|(name, _)| *name == label) {
            assert!(!reason.is_empty(), "{label}: recorded gaps carry their reason");
            continue;
        }

        let (_, snippet) = BATTERY
            .iter()
            .find(|(name, _)| *name == label)
            .unwrap_or_else(|| {
                panic!(
                    "{label}: promises quickfix {template:?} but has no battery snippet — \
                     add one here so the promise stays live"
                )
            });

        let doc = DocumentState::new(snippet.to_string(), 1);
        let titles: Vec<String> = logicaffeine_lsp::code_actions::code_actions(
            &doc,
            Range::default(),
            &uri,
        )
        .into_iter()
        .filter_map(|action| match action {
            CodeActionOrCommand::CodeAction(ca) => Some(ca.title),
            CodeActionOrCommand::Command(c) => Some(c.title),
        })
        .collect();

        assert!(
            titles.iter().any(|t| title_matches(template, t)),
            "{label}: the decision table promises {template:?}, but the battery program \
             produced only {titles:?}"
        );
    }

    for (name, _) in BATTERY {
        let still_promised = all_parse_error_kinds().iter().any(|kind| {
            kind_label(kind) == *name
                && matches!(decision_for(kind).quickfix, Quickfix::Provided(_))
        });
        assert!(
            still_promised,
            "{name}: battery snippet for a kind that no longer promises a quickfix — remove it"
        );
    }
}
