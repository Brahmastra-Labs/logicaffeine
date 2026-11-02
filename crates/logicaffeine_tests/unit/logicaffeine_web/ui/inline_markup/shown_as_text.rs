//! The Markdown a reader would see as text in rendered HTML — shared by the unit tests of every
//! page that renders Markdown: inline markup itself, the guide, and news articles.

/// The Markdown a reader would see as text in rendered `html`: a `**`, an emphasis-shaped
/// `*…*`, an unrendered `[text](url)`, or a paragraph that is only a `---` rule. Code is left
/// out — there such characters are the code's own.
pub(crate) fn markup_shown_as_text(html: &str) -> Vec<String> {
    let mut prose = String::new();
    let mut rest = html;
    while let Some((at, close)) = [("<code", "</code>"), ("<pre", "</pre>")]
        .iter()
        .filter_map(|(open, close)| rest.find(open).map(|at| (at, *close)))
        .min_by_key(|(at, _)| *at)
    {
        prose.push_str(&rest[..at]);
        prose.push(' ');
        rest = rest[at..].find(close).map_or("", |end| &rest[at + end + close.len()..]);
    }
    prose.push_str(rest);

    let decode = |t: &str| {
        t.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&#39;", "'").replace("&amp;", "&")
    };
    let mut found = Vec::new();
    for paragraph in prose.split("<p>").skip(1) {
        let text = paragraph.split("</p>").next().unwrap_or("").trim();
        if text.len() >= 3 && text.chars().all(|c| matches!(c, '-' | '*' | '_' | ' ')) {
            found.push(format!("a rule shown as text: {text:?}"));
        }
    }
    for node in prose.split('<').map(|piece| piece.split_once('>').map_or(piece, |(_, text)| text)) {
        let text = decode(node);
        if text.contains("**") {
            found.push(format!("`**` shown as text: {:?}", text.trim()));
        } else if text.contains("](") {
            found.push(format!("a link shown as text: {:?}", text.trim()));
        } else if crate::ui::inline_markup::emphasis_html(&text).contains("<em>") {
            found.push(format!("`*emphasis*` shown as text: {:?}", text.trim()));
        }
    }
    found
}
