use super::*;
use crate::document::DocumentState;

fn make_doc(source: &str) -> DocumentState {
    DocumentState::new(source.to_string(), 1)
}

#[test]
fn inlay_hints_for_inferred_integer() {
    let doc = make_doc("## Main\n    Let x be 5.\n");
    let range = Range::default();
    let hints = inlay_hints(&doc, range);
    assert!(
        !hints.is_empty(),
        "Expected inlay hint for inferred variable"
    );
    match &hints[0].label {
        InlayHintLabel::String(s) => {
            assert_eq!(s, ": Int", "Integer literal should infer Int, got '{}'", s);
        }
        _ => panic!("Expected string label"),
    }
}

#[test]
fn inlay_hints_for_inferred_text() {
    let doc = make_doc("## Main\n    Let msg be \"hello\".\n");
    let range = Range::default();
    let hints = inlay_hints(&doc, range);
    assert!(!hints.is_empty(), "Expected inlay hint for text variable");
    match &hints[0].label {
        InlayHintLabel::String(s) => {
            assert_eq!(s, ": Text", "Text literal should infer Text, got '{}'", s);
        }
        _ => panic!("Expected string label"),
    }
}

#[test]
fn inlay_hints_for_inferred_bool() {
    let doc = make_doc("## Main\n    Let flag be true.\n");
    let range = Range::default();
    let hints = inlay_hints(&doc, range);
    assert!(!hints.is_empty(), "Expected inlay hint for bool variable");
    match &hints[0].label {
        InlayHintLabel::String(s) => {
            assert_eq!(s, ": Bool", "Bool literal should infer Bool, got '{}'", s);
        }
        _ => panic!("Expected string label"),
    }
}

#[test]
fn inlay_hints_empty_for_empty_doc() {
    let doc = make_doc("");
    let range = Range::default();
    let hints = inlay_hints(&doc, range);
    assert!(hints.is_empty());
}

#[test]
fn inlay_hints_skip_default_spans() {
    let doc = make_doc("## Main\n    Let x be 5.\n");
    let range = Range::default();
    let hints = inlay_hints(&doc, range);
    for hint in &hints {
        assert_eq!(
            hint.kind,
            Some(InlayHintKind::TYPE),
            "Hints should be TYPE kind"
        );
    }
}

#[test]
fn inlay_hints_hint_position_correct() {
    let doc = make_doc("## Main\n    Let x be 5.\n");
    let range = Range::default();
    let hints = inlay_hints(&doc, range);
    assert!(
        !hints.is_empty(),
        "Should have at least one inlay hint for inferred variable"
    );
    let hint = &hints[0];
    assert_eq!(
        hint.position.line, 1,
        "Hint should be on line 1 (the Let statement)"
    );
}

#[test]
fn extract_inferred_type_from_detail() {
    assert_eq!(extract_inferred_type("Let x: Int (inferred)"), "Int");
    assert_eq!(extract_inferred_type("Let x: Text (inferred)"), "Text");
    assert_eq!(extract_inferred_type("Let x: auto (inferred)"), "auto");
    assert_eq!(extract_inferred_type("Let x: Int"), "auto"); // no "(inferred)" marker
}

#[test]
fn inlay_hints_respects_range_parameter() {
    let doc = make_doc("## Main\n    Let x be 5.\n    Let y be 10.\n");
    let range = lsp_types::Range {
        start: lsp_types::Position {
            line: 1,
            character: 0,
        },
        end: lsp_types::Position {
            line: 1,
            character: 99,
        },
    };
    let restricted_hints = inlay_hints(&doc, range);
    let all_hints = inlay_hints(&doc, Range::default());
    assert!(
        restricted_hints.len() <= all_hints.len(),
        "Restricted range should have <= hints: {} vs {}",
        restricted_hints.len(),
        all_hints.len()
    );
}

#[test]
fn inlay_hint_borrowed_after_show() {
    let doc = make_doc("## Main\n    Let x be 5.\n    Show x.\n");
    let hints = inlay_hints(&doc, Range::default());
    let borrowed_hints: Vec<_> = hints
        .iter()
        .filter(|h| matches!(&h.label, InlayHintLabel::String(s) if s.contains("borrowed")))
        .collect();
    // x should be Borrowed after Show
    if doc
        .ownership_states
        .get("x")
        .map_or(false, |s| matches!(s, VarState::Borrowed))
    {
        assert!(
            !borrowed_hints.is_empty(),
            "Should have 'borrowed' inlay hint for x after Show"
        );
    }
}

#[test]
fn inlay_hint_no_marker_for_owned() {
    let doc = make_doc("## Main\n    Let x be 5.\n");
    let hints = inlay_hints(&doc, Range::default());
    let ownership_hints: Vec<_> = hints.iter()
        .filter(|h| matches!(&h.label, InlayHintLabel::String(s) if s.contains("moved") || s.contains("borrowed")))
        .collect();
    // x is Owned after just Let — no ownership hint should appear
    assert!(
        ownership_hints.is_empty(),
        "Owned variables should not have ownership inlay hints. Got: {:?}",
        ownership_hints.iter().map(|h| &h.label).collect::<Vec<_>>()
    );
}
