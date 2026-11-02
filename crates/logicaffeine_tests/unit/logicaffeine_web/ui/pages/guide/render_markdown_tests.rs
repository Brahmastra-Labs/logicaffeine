use super::{render_markdown, SECTIONS};

#[path = "../../inline_markup/shown_as_text.rs"]
mod shown_as_text;
use shown_as_text::markup_shown_as_text;

/// A fenced block becomes one copyable `<pre><code>` unit — never a paragraph
/// of mangled inline code (the install-command regression).
#[test]
fn fenced_block_renders_as_pre_code_with_copy_button() {
    let md =
        "Install it:\n\n```bash\ncurl -fsSL https://logicaffeine.com/install.sh | sh\n```\n\nDone.";
    let html = render_markdown(md);
    assert!(
        html.contains(
            "<pre><code>curl -fsSL https://logicaffeine.com/install.sh | sh</code></pre>"
        ),
        "{html}"
    );
    assert!(html.contains("guide-code-copy"), "{html}");
    assert!(
        !html.contains("```"),
        "fence markers must never leak into the page: {html}"
    );
}

/// Blank lines and indentation inside a fence are content, not structure.
#[test]
fn fenced_block_preserves_blank_lines_and_indentation() {
    let md = "```logos\n## Main\n    Show 1.\n\n    Show 2.\n```";
    let html = render_markdown(md);
    assert!(
        html.contains("## Main\n    Show 1.\n\n    Show 2."),
        "{html}"
    );
}

/// Code is escaped, and the fence's language tag is not rendered as text.
#[test]
fn fenced_block_escapes_html_and_drops_language_tag() {
    let html = render_markdown("```powershell\nirm https://x | iex && a < b\n```");
    assert!(html.contains("&amp;&amp; a &lt; b"), "{html}");
    assert!(
        !html.contains("powershell"),
        "language tag must not render: {html}"
    );
}

/// The whole shipped guide renders with zero leaked fence markers.
#[test]
fn no_section_leaks_fence_markers() {
    for section in SECTIONS.iter() {
        let html = render_markdown(section.content);
        assert!(
            !html.contains("```"),
            "section '{}' leaks fence markers",
            section.id
        );
    }
}

/// A plain fence (a shell command, a config file) is not a live example: it carries its own
/// class, so neither one's styling reaches the other, and a selector for examples finds only
/// examples.
#[test]
fn a_plain_fence_is_not_a_live_example() {
    let html = render_markdown("```bash\nlargo new app\n```");
    assert!(html.contains("class=\"guide-fence\""), "{html}");
    assert!(
        !html.contains("guide-code-block"),
        "a plain fence wears the live example's class: {html}"
    );
    assert!(
        !super::GUIDE_STYLE.contains(".guide-code-block"),
        "the guide's stylesheet restyles the live example component"
    );
}

#[test]
fn guide_prose_renders_emphasis_and_rules() {
    assert_eq!(
        render_markdown("*Try it:* change the *second* line.\n\n---\n\nNext."),
        "<p><em>Try it:</em> change the <em>second</em> line.</p><hr/><p>Next.</p>"
    );
}

/// Whatever Markdown a section is written in, the reader sees it rendered — never its markers.
#[test]
fn no_section_shows_markup_as_text() {
    let mut shown = Vec::new();
    for section in SECTIONS.iter() {
        for marker in markup_shown_as_text(&render_markdown(section.content)) {
            shown.push(format!("{}: {marker}", section.id));
        }
    }
    assert!(
        shown.is_empty(),
        "{} marker(s) shown as text:\n{}",
        shown.len(),
        shown.join("\n")
    );
}
