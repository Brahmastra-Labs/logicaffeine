use super::*;
use crate::document::DocumentState;

fn make_doc(source: &str) -> DocumentState {
    DocumentState::new(source.to_string(), 1)
}

fn test_uri() -> Url {
    Url::parse("file:///test.logos").unwrap()
}

#[test]
fn rename_variable_produces_edits() {
    let doc = make_doc("## Main\n    Let x be 5.\n    Show x.\n");
    // Position on "x" in "Show x."
    let pos = Position {
        line: 2,
        character: 9,
    };
    let result = rename(&doc, pos, "y".to_string(), &test_uri());
    assert!(result.is_some(), "Expected rename edits");
    let ws_edit = result.unwrap();
    let changes = ws_edit.changes.unwrap();
    let edits = changes.get(&test_uri()).unwrap();
    assert!(
        edits.len() >= 2,
        "Expected edits for definition and reference, got {}",
        edits.len()
    );
    for edit in edits {
        assert_eq!(edit.new_text, "y");
    }
}

#[test]
fn rename_unknown_returns_none() {
    let doc = make_doc("## Main\n    Let x be 5.\n");
    // Position in whitespace
    let pos = Position {
        line: 0,
        character: 50,
    };
    let result = rename(&doc, pos, "y".to_string(), &test_uri());
    assert!(result.is_none());
}

#[test]
fn prepare_rename_for_variable() {
    let doc = make_doc("## Main\n    Let x be 5.\n    Show x.\n");
    // Position on "x" in "Show x."
    let pos = Position {
        line: 2,
        character: 9,
    };
    let result = prepare_rename(&doc, pos);
    assert!(
        result.is_some(),
        "Expected prepare_rename to succeed for variable 'x'"
    );
    let (range, text) = result.unwrap();
    assert_eq!(text, "x");
    assert_eq!(range.start.line, 2);
}

#[test]
fn rename_edits_at_correct_positions() {
    let source = "## Main\n    Let x be 5.\n    Show x.\n";
    let doc = make_doc(source);
    let pos = Position {
        line: 2,
        character: 9,
    };
    let result = rename(&doc, pos, "y".to_string(), &test_uri());
    assert!(result.is_some());
    let ws_edit = result.unwrap();
    let changes = ws_edit.changes.unwrap();
    let edits = changes.get(&test_uri()).unwrap();
    for edit in edits {
        let start = doc.line_index.offset(edit.range.start);
        let end = doc.line_index.offset(edit.range.end);
        let old_text = &source[start..end];
        assert_eq!(
            old_text, "x",
            "Edit range should point to 'x', got '{}'",
            old_text
        );
    }
}

#[test]
fn rename_includes_definition_and_references() {
    let doc = make_doc("## Main\n    Let x be 5.\n    Show x.\n");
    let pos = Position {
        line: 2,
        character: 9,
    };
    let result = rename(&doc, pos, "y".to_string(), &test_uri());
    assert!(result.is_some());
    let ws_edit = result.unwrap();
    let changes = ws_edit.changes.unwrap();
    let edits = changes.get(&test_uri()).unwrap();
    // Should include at least: 1 definition + 1+ references
    assert!(
        edits.len() >= 2,
        "Expected at least 2 edits (def + ref), got {}",
        edits.len()
    );
}

#[test]
fn prepare_rename_rejects_more_tokens() {
    let doc = make_doc("## Main\n    Let x be 5.\n");
    // Position on "be" (a keyword)
    let pos = Position {
        line: 1,
        character: 10,
    };
    let result = prepare_rename(&doc, pos);
    assert!(result.is_none(), "'Be' keyword should not be renameable");
    // Position on "5" (a Number)
    let pos2 = Position {
        line: 1,
        character: 14,
    };
    let result2 = prepare_rename(&doc, pos2);
    assert!(result2.is_none(), "Number should not be renameable");
}

#[test]
fn rename_rejects_empty_name() {
    let doc = make_doc("## Main\n    Let x be 5.\n    Show x.\n");
    let pos = Position {
        line: 2,
        character: 9,
    };
    let result = rename(&doc, pos, "".to_string(), &test_uri());
    assert!(result.is_none(), "Empty name should be rejected");
}

#[test]
fn rename_rejects_whitespace_name() {
    let doc = make_doc("## Main\n    Let x be 5.\n    Show x.\n");
    let pos = Position {
        line: 2,
        character: 9,
    };
    let result = rename(&doc, pos, "foo bar".to_string(), &test_uri());
    assert!(result.is_none(), "Name with spaces should be rejected");
}

#[test]
fn rename_rejects_reserved_keyword() {
    let doc = make_doc("## Main\n    Let x be 5.\n    Show x.\n");
    let pos = Position {
        line: 2,
        character: 9,
    };
    let result = rename(&doc, pos, "Let".to_string(), &test_uri());
    assert!(result.is_none(), "Reserved keyword should be rejected");
}

#[test]
fn rename_rejects_numeric_start() {
    let doc = make_doc("## Main\n    Let x be 5.\n    Show x.\n");
    let pos = Position {
        line: 2,
        character: 9,
    };
    let result = rename(&doc, pos, "3abc".to_string(), &test_uri());
    assert!(
        result.is_none(),
        "Name starting with digit should be rejected"
    );
}

#[test]
fn rename_accepts_valid_name() {
    assert!(validate_new_name("myVar").is_none());
    assert!(validate_new_name("_private").is_none());
    assert!(validate_new_name("x").is_none());
}

#[test]
fn prepare_rename_rejects_keywords() {
    let doc = make_doc("## Main\n    Let x be 5.\n");
    // Position on "Let" keyword (line 1, character 4)
    let pos = Position {
        line: 1,
        character: 4,
    };
    let result = prepare_rename(&doc, pos);
    assert!(result.is_none(), "Keywords should not be renameable");
}
