//! ★ LOCK — the LGX Socratic error catalog: every template, markup and stylesheet error
//! has a message that names the construct and the fix, never just the fact. ★

use logicaffeine_language::lgx::css;
use logicaffeine_language::lgx::html;
use logicaffeine_language::lgx::template;

/// (source, words the message must contain)
const TEMPLATE_CASES: &[(&str, &[&str])] = &[
    ("<p>{end}</p>", &["{end}", "open"]),
    ("{if a:}<p>x</p>", &["if", "{end}"]),
    ("<div><span></div>", &["</div>", "<span>"]),
    ("<div><p>x</p>", &["<div>", "</div>"]),
    ("</p>", &["</p>", "open"]),
    ("<p>x</p>{extends Site}", &["extends", "first"]),
    ("<p>{else:}x</p>", &["else", "if"]),
    (
        "{fro each x in xs:}<i>{x}</i>{end}",
        &["fro each", "for each"],
    ),
    ("<p>{if a:}{else:}{else:}{end}</p>", &["else", "one"]),
    ("<input>{x}</input>", &["input", "void"]),
    ("<p>{}</p>", &["{}", "expression"]),
    ("<button on-click=\"add\">x</button>", &["on-click", "{"]),
    ("<p>{block}</p>", &["block", "{block name:}"]),
    ("<p>{let x}</p>", &["let", "be"]),
];

#[test]
fn every_template_error_names_the_construct_and_the_fix() {
    for (src, words) in TEMPLATE_CASES {
        let err = template::parse(src).expect_err(&format!("{src:?} must be refused"));
        let msg = err.message();
        for w in *words {
            assert!(
                msg.contains(w),
                "{src:?}: message {msg:?} should mention {w:?}"
            );
        }
        assert!(
            err.span.end >= err.span.start && err.span.end <= src.len(),
            "{src:?}: span {:?} is inside the source",
            err.span
        );
    }
}

const HTML_CASES: &[(&str, &[&str])] = &[
    ("<div class=\"x\"", &["never closes", ">"]),
    ("<p title=\"x>y</p>", &["quote"]),
    ("<p>{count</p>", &["{", "}"]),
    ("<p>x}</p>", &["}", "closes nothing"]),
    ("<!-- never", &["<!--", "-->"]),
    ("<script>x</script>", &["<script>", "Actions"]),
    ("<style>.a{}</style>", &["<style>", "Style:"]),
    ("<!doctype html>", &["doctype", "layout"]),
    ("<p =x>", &["attribute"]),
    ("<1>", &["<", "tag name"]),
];

#[test]
fn every_markup_error_names_the_construct_and_the_fix() {
    for (src, words) in HTML_CASES {
        let err = html::tokenize(src).expect_err(&format!("{src:?} must be refused"));
        let msg = err.message();
        for w in *words {
            assert!(
                msg.contains(w),
                "{src:?}: message {msg:?} should mention {w:?}"
            );
        }
    }
}

#[test]
fn every_stylesheet_error_names_the_fix() {
    for src in [
        ".a { color: red",
        ".a { color: red } }",
        ".a { content: \"unterminated }",
    ] {
        let err = css::scope(src, "lgx-t").expect_err(&format!("{src:?} must be refused"));
        assert!(
            err.message().contains("brace") || err.message().contains("string"),
            "{src:?}: {}",
            err.message()
        );
    }
}
