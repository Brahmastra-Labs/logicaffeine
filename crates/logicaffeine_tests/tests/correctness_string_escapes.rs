//! Part I correctness: string escapes DECODE — `"a\nb"` is two lines, not
//! the three characters `a n b`.
//!
//! The audit row: the lexer skipped the backslash and kept the next char
//! verbatim, silently corrupting every escaped string. The spec: decode
//! `\n \t \\ \" \u{…}`; a raw string `r"…"` keeps backslashes verbatim
//! (paths, regexes). Interpolation braces (`{x}`, `{{`) are untouched —
//! they live in their own in-string machinery.

mod common;
use common::assert_compiled_equals_interpreted_eq;

#[test]
fn newline_escape_decodes() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show "a\nb".
"#,
        "a\nb",
    );
}

#[test]
fn tab_escape_decodes() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show "col1\tcol2".
"#,
        "col1\tcol2",
    );
}

#[test]
fn escaped_quote_decodes() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show "say \"hi\"".
"#,
        r#"say "hi""#,
    );
}

#[test]
fn escaped_backslash_decodes() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show "back\\slash".
"#,
        r"back\slash",
    );
}

#[test]
fn unicode_escape_decodes() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show "wave \u{1F44B}".
"#,
        "wave \u{1F44B}",
    );
}

#[test]
fn raw_string_keeps_backslashes() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Show r"C:\temp\new".
"#,
        r"C:\temp\new",
    );
}

#[test]
fn interpolation_still_works_beside_escapes() {
    assert_compiled_equals_interpreted_eq(
        r#"## Main
Let x be 7.
Show "x is {x}\non line two".
"#,
        "x is 7\non line two",
    );
}
