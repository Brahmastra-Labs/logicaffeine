//! LOCK — a broken rule on line N of a `Style:` block is refused on line N, whatever precedes it.
//!
//! The scoper reports a byte offset into the stylesheet; the stylesheet is the section's body with
//! each line's indentation stripped; the parser maps the offset through the same `RawMap` a
//! template's positions go through. Comments are blanked in place rather than cut out, and a
//! group at-rule's body is parsed at its own position, so neither moves an offset.

#[path = "language_common/mod.rs"]
mod common;

use common::{line_col, refusal};
use logicaffeine_language::lgx::css;

/// A component whose `Style:` block is `lines`, one per line.
fn page(lines: &[&str]) -> String {
    let body: String = lines.iter().map(|l| format!("    {l}\n")).collect();
    format!(
        "## Component App\nView:\n    <p class=\"a\">x</p>\nStyle:\n{body}\n## Main\nMount App.\n"
    )
}

/// The line and column the refusal of `src` points at, and the character there.
fn refused_at(src: &str) -> (usize, usize, String) {
    let parsed = common::parse(src);
    let Err(e) = parsed else {
        panic!("this stylesheet must be refused:\n{src}")
    };
    let (line, col) = line_col(src, e.span.start);
    (
        line,
        col,
        src.get(e.span.start..e.span.end).unwrap_or("").to_string(),
    )
}

#[test]
fn a_bad_rule_on_the_third_line_is_refused_on_the_third_line() {
    let src = page(&[
        ".a { color: red; }",
        ".b { color: blue; }",
        ".c { color: green;",
    ]);
    let (line, col, at) = refused_at(&src);
    // `Style:` is line 4, so the stylesheet's third line is line 7.
    assert_eq!((line, col, at.as_str()), (7, 8, "{"), "{src}");
}

#[test]
fn a_comment_before_the_mistake_does_not_move_it() {
    let src = page(&[
        "/* the palette",
        "   spans two lines */",
        ".a { color: red; }",
        ".c { color: green;",
    ]);
    let (line, col, _) = refused_at(&src);
    assert_eq!((line, col), (8, 8), "{src}");
}

#[test]
fn a_mistake_inside_media_is_refused_where_it_is() {
    // `stray` is a prelude with no block — an error only the media block's own parse can see.
    let src = page(&[
        ".a { color: red; }",
        "@media (max-width: 40em) {",
        "  .b { color: blue; }",
        "  stray",
        "}",
    ]);
    let (line, col, at) = refused_at(&src);
    assert_eq!((line, col, at.as_str()), (8, 7, "s"), "{src}");
}

#[test]
fn a_stray_close_brace_is_refused_where_it_is() {
    let src = page(&[".a { color: red; }", "}"]);
    let (line, col, at) = refused_at(&src);
    assert_eq!((line, col, at.as_str()), (6, 5, "}"), "{src}");
}

#[test]
fn the_refusal_names_the_section_and_the_component() {
    let (message, _) = refusal(&page(&[".c { color: green;"]));
    assert!(
        message.contains("The `Style:` of `App` is not valid CSS"),
        "{message}"
    );
    let (message, _) =
        refusal("## Page /about\nView:\n    <p>x</p>\nStyle:\n    p { color: red;\n");
    assert!(
        message.contains("The `Style:` of `## Page /about` is not valid CSS"),
        "{message}"
    );
}

#[test]
fn comments_are_still_removed_from_what_is_emitted() {
    let scoped = css::scope(
        "/* a */ .a /* b */ { color: /* c */ red; } /* tail */",
        "lgx-x",
    )
    .unwrap();
    assert!(
        !scoped.contains("/*") && !scoped.contains('\0'),
        "{scoped:?}"
    );
    assert_eq!(scoped, ".a.lgx-x { color:  red; }");
}

#[test]
fn an_offset_inside_media_counts_from_the_start_of_the_stylesheet() {
    let sheet = "@media screen {\n  .a { color: red; }\n  garbage\n}\n";
    let e = css::scope(sheet, "c").expect_err("a prelude with no block");
    assert!(
        sheet[e.at..].starts_with("garbage"),
        "offset {} points at {:?}",
        e.at,
        &sheet[e.at..]
    );
    let nested =
        "@supports (display: grid) {\n  @media print {\n    .a { color: red; }\n    oops\n  }\n}\n";
    let e = css::scope(nested, "c").expect_err("a prelude with no block, two groups deep");
    assert!(
        nested[e.at..].starts_with("oops"),
        "offset {} points at {:?}",
        e.at,
        &nested[e.at..]
    );
}

#[test]
fn an_offset_after_a_comment_counts_the_comment() {
    let sheet = "/* héllo */ .a { color: red;";
    let e = css::scope(sheet, "c").expect_err("an unbalanced brace");
    assert_eq!(
        &sheet[e.at..e.at + 1],
        "{",
        "offset {} points at {:?}",
        e.at,
        &sheet[e.at..]
    );
}
