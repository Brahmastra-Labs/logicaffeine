//! Where a problem is underlined is the problem's address: the name used after it was given
//! away, the value that does not fit its declared type, the statement at fault — never the
//! period that ends it, the header of the function around it, or the first line of the file.
use crate::document::DocumentState;
use lsp_types::NumberOrString;

/// Where each error diagnostic whose code (or message) matches `which` starts, and the text
/// it underlines.
fn underlined_at(source: &str, which: &str) -> Vec<(usize, String)> {
    let doc = DocumentState::new(source.to_string(), 1);
    doc.diagnostics
        .iter()
        .filter(|d| {
            matches!(&d.code, Some(NumberOrString::String(c)) if c == which) || d.message.contains(which)
        })
        .map(|d| {
            let (start, end) = (doc.line_index.offset(d.range.start), doc.line_index.offset(d.range.end));
            (start, source[start..end].to_string())
        })
        .collect()
}

/// The text each error diagnostic whose code (or message) matches `which` underlines.
fn underlined(source: &str, which: &str) -> Vec<String> {
    underlined_at(source, which).into_iter().map(|(_, text)| text).collect()
}

/// The byte offset of `needle` in `source`, which must occur exactly once.
fn only(source: &str, needle: &str) -> usize {
    assert_eq!(source.matches(needle).count(), 1, "{needle:?} must occur once in {source:?}");
    source.find(needle).unwrap()
}

#[test]
fn a_use_after_move_underlines_the_name_used() {
    let source = "## Main\nLet s be a new Seq of Int.\nPush 1 to s.\nGive s to keeper.\nShow s.\n";
    let use_at = only(source, "Show s.") + "Show ".len();
    assert_eq!(underlined_at(source, "use-after-move"), vec![(use_at, "s".to_string())]);
}

#[test]
fn a_use_after_move_in_a_function_underlines_the_name_used() {
    let source = "## To f () -> Int:\n    Let s be a new Seq of Int.\n    Give s to keeper.\n    Show s.\n    Return 0.\n\n## Main\nShow f().\n";
    let use_at = only(source, "Show s.") + "Show ".len();
    assert_eq!(underlined_at(source, "use-after-move"), vec![(use_at, "s".to_string())]);
}

#[test]
fn a_maybe_moved_use_underlines_the_name_used() {
    let source = "## Main\nLet s be a new Seq of Int.\nLet c be true.\nIf c:\n    Give s to keeper.\nShow s.\n";
    let use_at = only(source, "Show s.") + "Show ".len();
    assert_eq!(underlined_at(source, "maybe-moved"), vec![(use_at, "s".to_string())]);
}

#[test]
fn a_maybe_moved_use_in_a_function_underlines_the_name_used() {
    let source = "## To f (c: Bool) -> Int:\n    Let s be a new Seq of Int.\n    If c:\n        Give s to keeper.\n    Show s.\n    Return 0.\n\n## Main\nShow f(true).\n";
    let use_at = only(source, "Show s.") + "Show ".len();
    assert_eq!(underlined_at(source, "maybe-moved"), vec![(use_at, "s".to_string())]);
}

#[test]
fn an_escape_underlines_the_name_that_escapes() {
    let source = "## Main\nLet outer be 0.\nInside a new zone called \"Scratch\":\n    Let x be 5.\n    Set outer to x.\nShow outer.\n";
    let escape_at = only(source, "Set outer to x.") + "Set outer to ".len();
    assert_eq!(underlined_at(source, "escape-assignment"), vec![(escape_at, "x".to_string())]);
}

#[test]
fn an_escape_in_a_function_underlines_the_name_that_escapes() {
    let source = "## To f () -> Int:\n    Let outer be 0.\n    Inside a new zone called \"Scratch\":\n        Let x be 5.\n        Set outer to x.\n    Return outer.\n\n## Main\nShow f().\n";
    let escape_at = only(source, "Set outer to x.") + "Set outer to ".len();
    assert_eq!(underlined_at(source, "escape-assignment"), vec![(escape_at, "x".to_string())]);
}

#[test]
fn a_declared_type_mismatch_underlines_the_value() {
    let source = "## Main\nLet x: Int be \"hello\".\nShow x.\n";
    assert_eq!(underlined(source, "type-mismatch"), vec!["\"hello\""]);
}

#[test]
fn an_interpolation_error_underlines_its_string() {
    let source = "## Main\nShow \"{\\\"a\\\": 1}\".\n";
    assert_eq!(underlined(source, "interpolation hole"), vec!["\"{\\\"a\\\": 1}\""]);
}

#[test]
fn a_quantity_mismatch_underlines_its_statement() {
    let source = "## Main\nLet len be quantity(3, \"meter\").\nLet t be quantity(2, \"second\").\nShow len + t.\n";
    assert_eq!(underlined(source, "different dimensions"), vec!["Show len + t."]);
}

#[test]
fn an_error_inside_a_function_underlines_its_statement() {
    let source = "## To f () -> Int:\n    Return \"oops\".\n\n## To g () -> Int:\n    Return \"worse\".\n\n## Main\nShow 1.\n";
    assert_eq!(underlined(source, "type-mismatch"), vec!["Return \"oops\".", "Return \"worse\"."]);
}
