use super::markdown_to_html;

#[path = "../../../inline_markup/shown_as_text.rs"]
mod shown_as_text;
use shown_as_text::markup_shown_as_text;

#[test]
fn a_paragraph_wrapped_across_lines_renders_as_one() {
    assert_eq!(
        markdown_to_html("Prebuilt binaries for\nevery platform, no sudo."),
        "<p>Prebuilt binaries for every platform, no sudo.</p>\n"
    );
}

#[test]
fn a_line_that_starts_with_a_number_is_kept() {
    assert_eq!(markdown_to_html("16 new test files."), "<p>16 new test files.</p>\n");
}

#[test]
fn a_numbered_list_counts_past_nine() {
    assert_eq!(markdown_to_html("9. a\n10. b"), "<ol>\n<li>a</li>\n<li>b</li>\n</ol>\n");
}

#[test]
fn a_wrapped_list_item_stays_in_its_item() {
    assert_eq!(markdown_to_html("- one\n  more\n- two"), "<ul>\n<li>one more</li>\n<li>two</li>\n</ul>\n");
}

#[test]
fn a_wrapped_quote_is_one_quote() {
    assert_eq!(markdown_to_html("> said\n> **twice**"), "<blockquote>said <strong>twice</strong></blockquote>\n");
}

#[test]
fn a_code_block_keeps_its_indentation() {
    assert_eq!(
        markdown_to_html("```logos\n## Main\n    Show 1 < 2.\n```"),
        "<pre><code>## Main\n    Show 1 &lt; 2.</code></pre>\n"
    );
}

#[test]
fn a_table_renders_as_a_table() {
    assert_eq!(
        markdown_to_html("| Before | After |\n|---|---|\n| 3 s | 1 s |"),
        "<div class=\"article-table\"><table><thead><tr><th>Before</th><th>After</th></tr></thead>\
             <tbody><tr><td>3 s</td><td>1 s</td></tr></tbody></table></div>\n"
    );
}

#[test]
fn a_heading_escapes_its_text() {
    assert_eq!(markdown_to_html("### Waveform & Consistency"), "<h3>Waveform &amp; Consistency</h3>\n");
}

#[test]
fn an_article_renders_emphasis_links_and_rules() {
    assert_eq!(
        markdown_to_html("An *italic* word and [the guide](/guide).\n\n---"),
        "<p>An <em>italic</em> word and <a href=\"/guide\">the guide</a>.</p>\n<hr/>\n"
    );
}

/// Whatever Markdown an article is written in, the reader sees it rendered — never its markers.
#[test]
fn no_article_shows_markup_as_text() {
    let mut shown = Vec::new();
    for article in crate::ui::pages::news::get_articles() {
        for marker in markup_shown_as_text(&markdown_to_html(article.content)) {
            shown.push(format!("{}: {marker}", article.slug));
        }
    }
    assert!(shown.is_empty(), "{} marker(s) shown as text:\n{}", shown.len(), shown.join("\n"));
}
