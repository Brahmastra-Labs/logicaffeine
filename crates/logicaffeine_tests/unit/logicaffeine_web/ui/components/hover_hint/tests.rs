use super::*;

const PROGRAM: &str = "## Main\nLet n be 10.\nShow n.\n";

fn doc() -> DocumentState {
    DocumentState::new(PROGRAM.to_string(), 0)
}

fn offset_of(needle: &str, nth: usize) -> usize {
    PROGRAM.match_indices(needle).nth(nth).map(|(i, _)| i).expect("needle in program")
}

#[test]
fn a_keyword_explains_itself() {
    let md = hover_markdown(&doc(), offset_of("Let", 0)).expect("a hover for Let");
    assert!(md.contains("Let") && md.contains("variable"), "{md}");
}

#[test]
fn a_variable_is_described() {
    let md = hover_markdown(&doc(), offset_of("n.", 0) ).expect("a hover for n");
    assert!(md.contains('n'), "{md}");
}

#[test]
fn blank_space_has_no_hint() {
    assert_eq!(hover_markdown(&doc(), offset_of("\n", 1)), None);
}

#[test]
fn markdown_paragraphs_and_rules() {
    assert_eq!(markdown_to_html("one\n\n---\n\ntwo"), "<p>one</p><hr><p>two</p>");
}

#[test]
fn markdown_bold_emphasis_and_inline_code() {
    assert_eq!(
        markdown_to_html("**Let** declares `x` _once_"),
        "<p><strong>Let</strong> declares <code>x</code> <em>once</em></p>"
    );
}

#[test]
fn markdown_fenced_code_keeps_its_lines() {
    assert_eq!(
        markdown_to_html("```logos\nLet x be 1.\nShow x.\n```"),
        "<pre><code>Let x be 1.\nShow x.</code></pre>"
    );
}

#[test]
fn markdown_bullets() {
    assert_eq!(markdown_to_html("- a\n- b"), "<ul><li>a</li><li>b</li></ul>");
}

#[test]
fn markdown_line_breaks_inside_a_paragraph() {
    assert_eq!(markdown_to_html("first\nsecond"), "<p>first<br>second</p>");
}

#[test]
fn markdown_escapes_html() {
    let html = markdown_to_html("<script>alert(1)</script> & `<b>`");
    assert!(!html.contains("<script>") && !html.contains("<b>"), "{html}");
    assert!(html.contains("&lt;script&gt;") && html.contains("&amp;") && html.contains("<code>&lt;b&gt;</code>"), "{html}");
}

#[test]
fn markdown_links_open_in_a_new_tab() {
    assert_eq!(
        markdown_to_html("see [Quick Guide](https://example.com/g#5) now"),
        "<p>see <a href=\"https://example.com/g#5\" target=\"_blank\" rel=\"noopener noreferrer\">Quick Guide</a> now</p>"
    );
}

#[test]
fn markdown_refuses_links_that_are_not_web_addresses() {
    let html = markdown_to_html("[click](javascript:alert(1))");
    assert!(!html.contains("<a"), "{html}");
    assert!(html.contains("click"), "{html}");
}

#[test]
fn markdown_leaves_unclosed_markers_as_text() {
    assert_eq!(markdown_to_html("2 ** 3 and `open"), "<p>2 ** 3 and `open</p>");
}
