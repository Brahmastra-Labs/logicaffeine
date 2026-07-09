//! Wave 7: bracket line-continuation + trailing commas. A collection literal,
//! call, or parenthesised expression may span lines — with the continuation
//! lines INDENTED — and may end with a trailing comma. Formatters emit both;
//! neither should be a syntax error.

mod common;
use common::assert_compiled_equals_interpreted_eq as eq;
use common::run_interpreter;

// ---- Trailing commas ----

#[test]
fn list_trailing_comma() {
    eq("## Main\nLet xs be [1, 2, 3,].\nShow xs.\n", "[1, 2, 3]");
}

#[test]
fn set_trailing_comma() {
    eq("## Main\nLet s be {1, 2, 3,}.\nShow s.\n", "{1, 2, 3}");
}

#[test]
fn map_trailing_comma() {
    eq("## Main\nLet m be {\"a\": 1, \"b\": 2,}.\nShow m.\n", "{a: 1, b: 2}");
}

// ---- Indented multi-line literals (the block style) ----

#[test]
fn list_indented_multiline() {
    let src = "## Main\nLet xs be [\n    1,\n    2,\n    3,\n].\nShow xs.\n";
    eq(src, "[1, 2, 3]");
}

#[test]
fn list_multiline_no_indent_still_works() {
    // The already-working flat continuation must keep working.
    eq("## Main\nLet xs be [1,\n2,\n3].\nShow xs.\n", "[1, 2, 3]");
}

#[test]
fn set_indented_multiline() {
    let src = "## Main\nLet s be {\n    1,\n    2,\n    3,\n}.\nShow s.\n";
    eq(src, "{1, 2, 3}");
}

#[test]
fn map_indented_multiline() {
    let src = "## Main\nLet m be {\n    \"a\": 1,\n    \"b\": 2,\n}.\nShow m.\n";
    eq(src, "{a: 1, b: 2}");
}

#[test]
fn paren_indented_multiline() {
    let src = "## Main\nLet n be (\n    1 + 2\n    + 3\n).\nShow n.\n";
    eq(src, "6");
}

#[test]
fn nested_indented_literals() {
    let src = "## Main\nLet xs be [\n    [1, 2],\n    [3, 4],\n].\nShow xs.\n";
    eq(src, "[[1, 2], [3, 4]]");
}

// ---- Regression: real indentation-significant blocks are untouched ----

#[test]
fn indented_blocks_still_structure_correctly() {
    let src = "## Main\nLet total be 0.\nRepeat for i from 1 to 3:\n    Set total to total + i.\n    Set total to total + 1.\nShow total.\n";
    let r = run_interpreter(src);
    assert!(r.success, "an indented loop body must still parse: {}", r.error);
    // 1+1 + 2+1 + 3+1 = 9
    assert_eq!(r.output.trim(), "9");
}

#[test]
fn nested_if_inside_loop_still_structures() {
    let src = "## Main\nLet total be 0.\nRepeat for i from 1 to 5:\n    If i is greater than 2:\n        Set total to total + i.\nShow total.\n";
    let r = run_interpreter(src);
    assert!(r.success, "a nested if/loop must still parse: {}", r.error);
    // 3 + 4 + 5 = 12
    assert_eq!(r.output.trim(), "12");
}
