//! A string literal whose content happens to be a keyword must be a plain
//! value — `Show "not".` prints `not`, not trip the `not` operator. The bug:
//! `check_word` matched by lexeme ignoring token kind, so a StringLiteral of
//! "not"/"and"/"if"/"in" was mistaken for the keyword.

mod common;
use common::assert_compiled_equals_interpreted_eq;

#[test]
fn show_string_not() {
    assert_compiled_equals_interpreted_eq("## Main\nShow \"not\".\n", "not");
}

#[test]
fn show_string_and() {
    assert_compiled_equals_interpreted_eq("## Main\nShow \"and\".\n", "and");
}

#[test]
fn show_string_if() {
    assert_compiled_equals_interpreted_eq("## Main\nShow \"if\".\n", "if");
}

#[test]
fn show_string_in() {
    assert_compiled_equals_interpreted_eq("## Main\nShow \"in\".\n", "in");
}

#[test]
fn string_or_in_a_variable() {
    assert_compiled_equals_interpreted_eq(
        "## Main\nLet w be \"or\".\nShow w.\n",
        "or",
    );
}

#[test]
fn keyword_string_in_otherwise_branch() {
    assert_compiled_equals_interpreted_eq(
        "## Main\nLet x be 5.\nIf x is even:\n    Show \"even\".\nOtherwise:\n    Show \"not\".\n",
        "not",
    );
}
