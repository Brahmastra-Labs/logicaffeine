//! LOCK — an LGX lint finding and a refusal point at the author's own file, line and column.
//!
//! A finding is computed on the `View:` body, whose offsets start at the body; the parser maps it
//! into the file as it stores it. The parse seam then undoes its own edits — the `## Main` it wraps a
//! bare script in and the stdlib modules it splices in on demand — once, so a page that calls a
//! library function reports the same position as one that does not.

use logicaffeine_compile::ui_bridge::{lgx_findings_of, with_parsed_program_checked};

/// `(line, column)`, both 1-based, of byte `offset` in `src`.
fn line_col(src: &str, offset: usize) -> (usize, usize) {
    let before = &src[..offset.min(src.len())];
    (
        before.matches('\n').count() + 1,
        before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1,
    )
}

/// A page whose view carries an unlabelled image on its fifth line; `hole` is the text of the
/// paragraph above it.
fn page(hole: &str) -> String {
    format!(
        "## Page /\n\
         State:\n\
         \x20   Let name be \"ada\".\n\
         View:\n\
         \x20   <p>{hole}</p>\n\
         \x20   <img src=\"a.png\">\n"
    )
}

fn image_finding(src: &str) -> (usize, usize, String) {
    let findings = lgx_findings_of(src);
    let f = findings
        .iter()
        .find(|f| f.message.contains("`<img>` has no `alt`"))
        .unwrap_or_else(|| panic!("the image lint must fire: {findings:?}"));
    let (line, col) = line_col(src, f.span.start);
    (
        line,
        col,
        src.get(f.span.start..f.span.end).unwrap_or("").to_string(),
    )
}

#[test]
fn a_finding_points_at_its_element_in_the_file() {
    let src = page("{name}");
    let (line, col, covered) = image_finding(&src);
    assert_eq!((line, col), (6, 5), "the image is on line 6, column 5");
    assert_eq!(covered, "<img src=\"a.png\">");
}

#[test]
fn a_page_that_calls_the_library_reports_the_same_position() {
    // `upper` pulls strings.lg into the parsed text ahead of the page; the finding must not move.
    let src = page("{upper(name)}");
    let (line, col, covered) = image_finding(&src);
    assert_eq!((line, col), (6, 5));
    assert_eq!(covered, "<img src=\"a.png\">");
}

#[test]
fn a_refusal_through_the_checked_seam_points_into_the_file() {
    let src = "## Page /\nState:\n    Let name be \"ada\".\nView:\n    <p>{upper(nme)}</p>\n";
    let e = with_parsed_program_checked(src, |parsed, _| parsed.map(|_| ()))
        .expect_err("`nme` is refused");
    assert_eq!(src.get(e.span.start..e.span.end), Some("nme"), "{e:?}");
    assert_eq!(line_col(src, e.span.start), (5, 15));
}
