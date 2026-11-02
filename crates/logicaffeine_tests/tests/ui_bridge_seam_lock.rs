//! ★ LOCK — `ui_bridge::parse_error_of` gives a caller the parse error located in the AUTHOR's own
//! file, with no `## Require` gate — the seam a caller that wants to render its OWN caret block
//! (`largo run --interpret`, matching what `largo check` already shows) takes instead of a
//! rendered advice string alone. ★
//!
//! The span is real only because the seam (`parse_seam_e`'s `UserText`) undoes BOTH edits it makes
//! to the text before parsing: the implicit `## Main\n` wrap for a bare script, and the stdlib
//! prelude splice (`## Route`, and any other construct a stdlib module answers). A program that
//! triggers the splice and still fails to parse is the one case that would show the old
//! bug — a span deep inside the spliced stdlib text rather than in the ~50 bytes the author wrote.

use logicaffeine_compile::ui_bridge::parse_error_of;

#[test]
fn a_program_that_parses_has_no_parse_error() {
    assert!(parse_error_of("## Main\nShow 1 + 1.\n").is_none());
}

#[test]
fn a_broken_program_locates_the_mistake_in_the_authors_own_text() {
    let src = "## Main\nIf 1 is 1\n    Show 1.\n";
    let err = parse_error_of(src).expect("an `If` with no `:` before its block is a parse error");
    assert!(
        err.span.start <= src.len() && err.span.end <= src.len(),
        "the span must fall inside the {}-byte source the author wrote, not the prelude-spliced \
         text the parser actually saw: {:?}",
        src.len(),
        err.span
    );
}

/// A `## Route` triggers the stdlib prelude splice (`matchRoute`, per the loader's own dependency
/// scan) — the exact shape diag's L1 finding measured refusing at "byte 271307 of a 150-byte file".
/// A syntax mistake in a program that ALSO uses a route must still locate inside the short source.
#[test]
fn a_mistake_in_a_program_that_splices_the_prelude_still_locates_in_the_short_source() {
    let src = "## Route GET /api/hello -> Text\n    Return \"hi\".\n";
    let err = parse_error_of(src).expect("a missing `:` after the route header is a parse error");
    assert!(
        err.span.start <= src.len() && err.span.end <= src.len(),
        "the located span must fall inside the {}-byte file the author wrote, not somewhere in \
         the spliced stdlib text: {:?}",
        src.len(),
        err.span
    );
}

/// The implicit `## Main` wrap (a bare script with no `##` headers) is ALSO undone: a mistake on
/// the bare script's own first line must not appear to be on line 2 (where the wrap's `## Main\n`
/// prefix would put it).
#[test]
fn a_mistake_in_a_bare_script_is_not_shifted_past_the_implicit_wrap() {
    let src = "If 1 is 1\n    Show 1.\n";
    let err = parse_error_of(src).expect("an `If` with no `:` before its block is a parse error");
    assert!(
        err.span.start <= src.len(),
        "the span must fall inside the {}-byte bare script, not past the `## Main\\n` wrap this \
         seam adds internally: {:?}",
        src.len(),
        err.span
    );
}
