//! The pages the user has pointed at carry no em dash and no semicolon.
//!
//! The user (2026-10-01) wants copy written for readers who are not used to that punctuation:
//! short, plain sentences with periods, commas, colons and parentheses. The rewrite reaches only the
//! pages the user names ("only rewrite content i target specifically like pages i have pointed you
//! at like learn logic and pricing"), so the lock reads only [`TARGETED`] routes and never the legal
//! pages in [`EXEMPT`]. On each targeted page it reads what a visitor reads: the page's text; the
//! attributes a browser shows or speaks (tooltips, alt text, placeholders, accessible names); the
//! title and the search and social descriptions the page sets in its head; the JSON-LD that search
//! engines show; and the text the page shows only when the visitor acts, which its prerender never
//! carries.
//!
//! Code is not prose. Text inside `<code>`, `<pre>`, `<kbd>`, `<samp>`, `<textarea>`, `<math>`,
//! `<script>` and `<style>` is skipped, and so is everything inside an element whose class marks it
//! as an editor or as program output ([`NOT_PROSE_CLASSES`], each with its reason). A number range
//! keeps its en dash ("7–15", "2024–2026"). A dash between words fails however it is typed: an em
//! dash, an en dash or a hyphen with a space beside it, a horizontal bar.

#![cfg(feature = "web-tests")]

#[path = "web_common/page_html.rs"]
mod page_html;

use dioxus::document::{Document, Eval, MetaProps, NoOpDocument};
use dioxus::prelude::*;
use dioxus_history::{History, MemoryHistory};
use logicaffeine_web::content::{ContentEngine, LessonQuestion};
use logicaffeine_web::ui::pages::learn::LessonPanel;
use logicaffeine_web::ui::pages::contact::lead::{EmailError, PhoneError};
use logicaffeine_web::App;
use page_html::json_ld;
use std::cell::RefCell;
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::OnceLock;

// ---------------------------------------------------------------------------------------------
// What is read, and what is not
// ---------------------------------------------------------------------------------------------

/// A page the user has pointed at, and the text it shows only when the visitor acts.
struct Target {
    route: &'static str,
    shown_on_demand: Option<fn() -> Vec<Passage>>,
}

/// The pages whose copy is rewritten, and only those. The user (2026-10-01): "only rewrite content
/// i target specifically like pages i have pointed you at like learn logic and pricing".
const TARGETED: &[Target] = &[
    Target { route: "/contact", shown_on_demand: Some(pricing_form_messages) },
    Target { route: "/", shown_on_demand: Some(front_page_responses) },
    Target { route: "/learn", shown_on_demand: Some(learn_lessons) },
];

/// Pages the lock never reads, whatever is targeted. The user (2026-10-01): "dont rewrite the legal
/// pages", "legal pages can have emdashes".
const EXEMPT: &[(&str, &str)] = &[("/privacy", "a legal page"), ("/terms", "a legal page")];

/// What the pricing form says under a field it refuses: every message its email and phone checks
/// can show.
fn pricing_form_messages() -> Vec<Passage> {
    let email = EmailError::ALL.iter().map(|e| Passage { place: format!("the email field's {e:?} message"), text: e.message().to_string() });
    let phone = PhoneError::ALL.iter().map(|e| Passage { place: format!("the phone field's {e:?} message"), text: e.message().to_string() });
    email.chain(phone).collect()
}

/// What the front page says once the visitor acts: the copy button's confirmations. The agent
/// instructions are prose too, but sit in a `<pre>`, so `landing_page` reads them.
fn front_page_responses() -> Vec<Passage> {
    use logicaffeine_web::ui::pages::landing::content::said;
    said::all().into_iter().map(|(place, text)| Passage { place: place.to_string(), text: text.to_string() }).collect()
}

/// What /learn shows as the learner moves along the roadmap: every lesson (the page opens one at a
/// time), the hint after a wrong answer and the explanation after a settled one, and the practice
/// behind "More practice".
fn learn_lessons() -> Vec<Passage> {
    #[derive(Props, Clone, PartialEq)]
    struct LessonProps {
        id: String,
    }
    #[allow(non_snake_case)]
    fn Lesson(props: LessonProps) -> Element {
        rsx! { LessonPanel { lesson: props.id.clone(), done: false, on_next: move |_| {} } }
    }
    let said = |place: String, text: &str| Passage { place, text: collapse(text) };
    let mut out = Vec::new();
    for module in ContentEngine::shared().eras().iter().flat_map(|e| e.modules.iter()) {
        for section in &module.sections {
            let mut vdom = VirtualDom::new_with_props(Lesson, LessonProps { id: section.id.clone() });
            vdom.rebuild_in_place();
            let shown = passages(&dioxus_ssr::render(&vdom));
            out.extend(shown.into_iter().map(|p| Passage { place: format!("lesson {}, {}", section.id, p.place), text: p.text }));
            if let Some(LessonQuestion::Translate { hint, explanation, .. } | LessonQuestion::Choice { hint, explanation, .. }) = &section.question {
                out.push(said(format!("lesson {}'s hint", section.id), hint));
                out.push(said(format!("lesson {}'s explanation", section.id), explanation));
            }
        }
        for exercise in &module.exercises {
            let place = |what: &str| format!("practice {}'s {what}", exercise.id);
            out.push(said(place("prompt"), &exercise.prompt));
            out.extend(exercise.hint.iter().map(|t| said(place("hint"), t)));
            out.extend(exercise.explanation.iter().map(|t| said(place("explanation"), t)));
            out.extend(exercise.options.iter().flatten().map(|t| said(place("option"), t)));
        }
    }
    out
}

/// Elements whose text is code, formulas or program output rather than prose.
const NOT_PROSE_TAGS: &[&str] = &["code", "pre", "kbd", "samp", "textarea", "math", "script", "style", "template"];

/// Classes that mark an element as an editor or as what a program printed, each with why.
const NOT_PROSE_CLASSES: &[(&str, &str)] = &[
    ("code-editor", "the syntax-highlighted editor: its text is the program being edited"),
    ("guide-code-editor", "a Syntax Guide example's editor: its text is the example's source"),
    ("guide-code-output-content", "what a Syntax Guide example printed, or the FOL it produced"),
];

/// Attributes a browser shows (a tooltip, an image's stand-in, a field's hint, an option's
/// label) or speaks (an accessible name or description).
const READ_ATTRIBUTES: &[&str] = &["title", "alt", "placeholder", "aria-label", "aria-description", "label"];

/// Elements that run on inside a line of text. Every other element starts a new passage.
const INLINE_TAGS: &[&str] = &[
    "a", "abbr", "b", "bdi", "bdo", "bdt", "br", "cite", "code", "data", "dfn", "em", "i", "img", "kbd", "mark", "q", "s",
    "samp", "small", "span", "strong", "sub", "sup", "time", "u", "var", "wbr",
];

/// Elements that never have content or a closing tag.
const VOID_TAGS: &[&str] = &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"];

/// Elements whose content is raw text up to their own closing tag.
const RAW_TEXT_TAGS: &[&str] = &["script", "style", "textarea"];

// ---------------------------------------------------------------------------------------------
// The punctuation
// ---------------------------------------------------------------------------------------------

/// One piece of punctuation a visitor should never meet, at a char index of the text it is in.
#[derive(Debug, Clone, PartialEq)]
struct Fault {
    at: usize,
    what: &'static str,
}

/// Every em dash, semicolon and dash between words in `text`.
fn faults(text: &str) -> Vec<Fault> {
    let chars: Vec<char> = text.chars().collect();
    let spaced = |i: usize| i >= chars.len() || chars[i].is_whitespace();
    let spaced_before = |i: usize| i == 0 || chars[i - 1].is_whitespace();
    let mut found = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let what = match chars[i] {
            '\u{2014}' | '\u{FE58}' | '\u{FE31}' => Some("an em dash"),
            '\u{2015}' => Some("a horizontal bar (an em dash by another name)"),
            '\u{2E3A}' | '\u{2E3B}' => Some("a two- or three-em dash"),
            ';' | '\u{037E}' | '\u{FF1B}' | '\u{FE54}' => Some("a semicolon"),
            '\u{2013}' if spaced_before(i) || spaced(i + 1) => Some("an en dash between words (a range like 7–15 takes no space)"),
            '-' => {
                let start = i;
                while i + 1 < chars.len() && chars[i + 1] == '-' {
                    i += 1;
                }
                if spaced_before(start) && spaced(i + 1) {
                    found.push(Fault { at: start, what: "a hyphen standing in for a dash" });
                }
                None
            }
            _ => None,
        };
        if let Some(what) = what {
            found.push(Fault { at: i, what });
        }
        i += 1;
    }
    found
}

/// The 60 characters around a fault, with an ellipsis where the text goes on.
fn excerpt(text: &str, at: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    let start = at.saturating_sub(30);
    let end = (at + 30).min(chars.len());
    format!(
        "{}{}{}",
        if start > 0 { "…" } else { "" },
        chars[start..end].iter().collect::<String>(),
        if end < chars.len() { "…" } else { "" }
    )
}

// ---------------------------------------------------------------------------------------------
// Reading markup
// ---------------------------------------------------------------------------------------------

/// A run of text a visitor reads, and where it sits.
#[derive(Debug, Clone)]
struct Passage {
    place: String,
    text: String,
}

/// Named character references the pages write, with the characters they stand for.
const NAMED_REFERENCES: &[(&str, char)] = &[
    ("amp", '&'),
    ("lt", '<'),
    ("gt", '>'),
    ("quot", '"'),
    ("apos", '\''),
    ("nbsp", '\u{a0}'),
    ("mdash", '\u{2014}'),
    ("ndash", '\u{2013}'),
    ("hellip", '\u{2026}'),
    ("copy", '\u{a9}'),
    ("reg", '\u{ae}'),
    ("trade", '\u{2122}'),
    ("rarr", '\u{2192}'),
    ("larr", '\u{2190}'),
    ("harr", '\u{2194}'),
    ("middot", '\u{b7}'),
    ("bull", '\u{2022}'),
    ("lsquo", '\u{2018}'),
    ("rsquo", '\u{2019}'),
    ("ldquo", '\u{201c}'),
    ("rdquo", '\u{201d}'),
    ("laquo", '\u{ab}'),
    ("raquo", '\u{bb}'),
    ("times", '\u{d7}'),
    ("minus", '\u{2212}'),
    ("sect", '\u{a7}'),
    ("para", '\u{b6}'),
    ("deg", '\u{b0}'),
    ("shy", '\u{ad}'),
    ("ensp", '\u{2002}'),
    ("emsp", '\u{2003}'),
    ("thinsp", '\u{2009}'),
    ("zwnj", '\u{200c}'),
    ("zwj", '\u{200d}'),
];

/// Character references read back to the characters they stand for. A named reference outside
/// [`NAMED_REFERENCES`] stops the reader: left as written, its `;` would be counted as prose.
fn decode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp + 1..];
        let name_len = tail.find(|c: char| !(c.is_ascii_alphanumeric() || c == '#')).unwrap_or(tail.len());
        let name = &tail[..name_len];
        if name.is_empty() || !tail[name_len..].starts_with(';') {
            out.push('&');
            rest = tail;
            continue;
        }
        let decoded = if let Some(hex) = name.strip_prefix("#x").or_else(|| name.strip_prefix("#X")) {
            u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
        } else if let Some(dec) = name.strip_prefix('#') {
            dec.parse::<u32>().ok().and_then(char::from_u32)
        } else {
            let known = NAMED_REFERENCES.iter().find(|(n, _)| *n == name).map(|(_, c)| *c);
            Some(known.unwrap_or_else(|| panic!("the reader does not know the reference &{name}; (add it to NAMED_REFERENCES)")))
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &tail[name_len + 1..];
            }
            None => {
                out.push('&');
                rest = tail;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Whitespace runs collapsed to one space, as a browser lays text out.
fn collapse(text: &str) -> String {
    text.replace('\u{a0}', " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// One start tag: its name (lower case), its attributes as written (references not yet read),
/// and whether it closes itself.
struct StartTag {
    name: String,
    attributes: Vec<(String, String)>,
    self_closing: bool,
}

/// Reads the start tag whose body (after `<`, up to but not including `>`) is `body`.
fn start_tag(body: &str) -> StartTag {
    let chars: Vec<char> = body.chars().collect();
    let mut i = 0;
    let mut name = String::new();
    while i < chars.len() && !chars[i].is_whitespace() && chars[i] != '/' {
        name.push(chars[i].to_ascii_lowercase());
        i += 1;
    }
    let mut attributes = Vec::new();
    loop {
        while i < chars.len() && (chars[i].is_whitespace() || chars[i] == '/') {
            i += 1;
        }
        if i >= chars.len() {
            break;
        }
        let mut key = String::new();
        while i < chars.len() && !chars[i].is_whitespace() && chars[i] != '=' && chars[i] != '/' {
            key.push(chars[i].to_ascii_lowercase());
            i += 1;
        }
        while i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if i < chars.len() && chars[i] == '=' {
            i += 1;
            while i < chars.len() && chars[i].is_whitespace() {
                i += 1;
            }
            if i < chars.len() && (chars[i] == '"' || chars[i] == '\'') {
                let quote = chars[i];
                i += 1;
                while i < chars.len() && chars[i] != quote {
                    value.push(chars[i]);
                    i += 1;
                }
                i += 1;
            } else {
                while i < chars.len() && !chars[i].is_whitespace() {
                    value.push(chars[i]);
                    i += 1;
                }
            }
        }
        if !key.is_empty() {
            attributes.push((key, value));
        }
    }
    StartTag { self_closing: body.trim_end().ends_with('/') || VOID_TAGS.contains(&name.as_str()), name, attributes }
}

/// The index of the `>` that ends the tag starting at `from` (just after its `<`), skipping any
/// `>` inside a quoted attribute value.
fn tag_end(html: &str, from: usize) -> usize {
    let mut quote: Option<char> = None;
    for (i, c) in html[from..].char_indices() {
        match (quote, c) {
            (Some(q), c) if c == q => quote = None,
            (None, '"' | '\'') => quote = Some(c),
            (None, '>') => return from + i,
            _ => {}
        }
    }
    html.len()
}

/// An element the reader is inside.
struct Open {
    name: String,
    label: String,
    not_prose: bool,
}

/// `tag#id.class` for a start tag, to say where a passage sits.
fn label(tag: &StartTag) -> String {
    let attr = |k: &str| tag.attributes.iter().find(|(n, _)| n == k).map(|(_, v)| v.as_str());
    let mut out = tag.name.clone();
    if let Some(id) = attr("id").filter(|v| !v.is_empty()) {
        out.push('#');
        out.push_str(id);
    }
    if let Some(class) = attr("class").and_then(|c| c.split_whitespace().next()) {
        out.push('.');
        out.push_str(class);
    }
    out
}

fn is_not_prose(tag: &StartTag) -> bool {
    NOT_PROSE_TAGS.contains(&tag.name.as_str())
        || tag
            .attributes
            .iter()
            .filter(|(k, _)| k == "class")
            .flat_map(|(_, v)| v.split_whitespace())
            .any(|class| NOT_PROSE_CLASSES.iter().any(|(c, _)| *c == class))
}

/// Where the reader is: the last three elements it is inside.
fn path(stack: &[Open]) -> String {
    let labels: Vec<&str> = stack.iter().map(|o| o.label.as_str()).collect();
    let start = labels.len().saturating_sub(3);
    if labels.is_empty() { "the page".to_string() } else { labels[start..].join(" > ") }
}

/// Everything in `html` a visitor reads: each run of prose text, and each attribute a browser
/// shows or speaks, outside code, editors and program output.
fn passages(html: &str) -> Vec<Passage> {
    let mut out = Vec::new();
    let mut stack: Vec<Open> = Vec::new();
    let mut run = String::new();
    let mut run_place: Option<String> = None;
    let flush = |run: &mut String, run_place: &mut Option<String>, out: &mut Vec<Passage>| {
        let text = collapse(run);
        if !text.is_empty() {
            out.push(Passage { place: run_place.take().unwrap_or_default(), text });
        }
        run.clear();
        *run_place = None;
    };
    let mut i = 0;
    while i < html.len() {
        let Some(lt) = html[i..].find('<').map(|o| i + o) else {
            if !stack.last().is_some_and(|o| o.not_prose) {
                run_place.get_or_insert_with(|| path(&stack));
                run.push_str(&decode(&html[i..]));
            }
            break;
        };
        if lt > i && !stack.last().is_some_and(|o| o.not_prose) {
            run_place.get_or_insert_with(|| path(&stack));
            run.push_str(&decode(&html[i..lt]));
        }
        let rest = &html[lt..];
        if rest.starts_with("<!--") {
            i = rest.find("-->").map(|e| lt + e + 3).unwrap_or(html.len());
            continue;
        }
        if rest.starts_with("<!") || rest.starts_with("<?") {
            i = tag_end(html, lt + 1) + 1;
            continue;
        }
        if let Some(close) = rest.strip_prefix("</") {
            let name: String = close.chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-').collect::<String>().to_ascii_lowercase();
            if !INLINE_TAGS.contains(&name.as_str()) {
                flush(&mut run, &mut run_place, &mut out);
            }
            if let Some(at) = stack.iter().rposition(|o| o.name == name) {
                stack.truncate(at);
            }
            i = tag_end(html, lt + 1) + 1;
            continue;
        }
        if !rest[1..].starts_with(|c: char| c.is_ascii_alphabetic()) {
            if !stack.last().is_some_and(|o| o.not_prose) {
                run_place.get_or_insert_with(|| path(&stack));
                run.push('<');
            }
            i = lt + 1;
            continue;
        }
        let end = tag_end(html, lt + 1);
        let tag = start_tag(&html[lt + 1..end]);
        i = end + 1;
        let inside_not_prose = stack.last().is_some_and(|o| o.not_prose);
        let not_prose = inside_not_prose || is_not_prose(&tag);
        if !INLINE_TAGS.contains(&tag.name.as_str()) {
            flush(&mut run, &mut run_place, &mut out);
        } else if tag.name == "br" {
            run.push(' ');
        }
        // An element's own attributes are read unless it sits inside code: a `<textarea>`'s
        // placeholder is shown although its content is not prose.
        if !inside_not_prose {
            let shown_value = tag.name == "input"
                && tag.attributes.iter().any(|(k, v)| k == "type" && matches!(v.as_str(), "submit" | "button" | "reset"));
            for (key, value) in &tag.attributes {
                if READ_ATTRIBUTES.contains(&key.as_str()) || (shown_value && key == "value") {
                    let text = collapse(&decode(value));
                    if !text.is_empty() {
                        out.push(Passage { place: format!("the {key}= of <{}> in {}", label(&tag), path(&stack)), text });
                    }
                }
            }
        }
        if RAW_TEXT_TAGS.contains(&tag.name.as_str()) && !tag.self_closing {
            let closing = format!("</{}", tag.name);
            let content_end = html[i..].find(&closing).map(|e| i + e).unwrap_or(html.len());
            i = tag_end(html, content_end + 1).min(html.len()) + 1;
            continue;
        }
        if !tag.self_closing {
            stack.push(Open { label: label(&tag), name: tag.name, not_prose });
        }
    }
    flush(&mut run, &mut run_place, &mut out);
    out
}

/// Every string a JSON-LD value carries, with the path to it.
fn json_strings(value: &serde_json::Value, at: String, out: &mut Vec<Passage>) {
    match value {
        serde_json::Value::String(s) => out.push(Passage { place: at, text: collapse(s) }),
        serde_json::Value::Array(items) => {
            for (n, item) in items.iter().enumerate() {
                json_strings(item, format!("{at}[{n}]"), out);
            }
        }
        serde_json::Value::Object(fields) => {
            let kind = fields.get("@type").and_then(|t| t.as_str()).map(|t| format!(" ({t})")).unwrap_or_default();
            for (key, field) in fields {
                json_strings(field, format!("{at}{kind} → {key}"), out);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------------------------
// Rendering pages, head and all
// ---------------------------------------------------------------------------------------------

/// What the page set in its head: every title and every meta tag, as attribute pairs.
#[derive(Default)]
struct Head {
    titles: RefCell<Vec<String>>,
    metas: RefCell<Vec<Vec<(String, String)>>>,
}

/// A document that keeps what the page puts in its head. The test renderer's default document
/// drops it, and the SSG server writes it into the prerendered page.
struct HeadRecorder(Rc<Head>);

impl Document for HeadRecorder {
    fn eval(&self, js: String) -> Eval {
        NoOpDocument.eval(js)
    }

    fn set_title(&self, title: String) {
        self.0.titles.borrow_mut().push(title);
    }

    fn create_head_element(&self, _name: &str, _attributes: &[(&str, String)], _contents: Option<String>) {}

    fn create_meta(&self, props: MetaProps) {
        let attributes = props.attributes().into_iter().map(|(k, v)| (k.to_string(), v)).collect();
        self.0.metas.borrow_mut().push(attributes);
    }
}

/// One rendered route.
struct Page {
    route: String,
    html: String,
    titles: Vec<String>,
    metas: Vec<Vec<(String, String)>>,
}

fn render(route: &str) -> Page {
    let head = Rc::new(Head::default());
    let mut vdom = VirtualDom::new(App);
    vdom.provide_root_context(Rc::new(MemoryHistory::with_initial_path(route)) as Rc<dyn History>);
    vdom.provide_root_context(Rc::new(HeadRecorder(head.clone())) as Rc<dyn Document>);
    vdom.rebuild_in_place();
    let html = dioxus_ssr::render(&vdom);
    let titles = head.titles.borrow().clone();
    let metas = head.metas.borrow().clone();
    Page { route: route.to_string(), html, titles, metas }
}

/// Every targeted route, rendered.
fn pages() -> &'static [Page] {
    static PAGES: OnceLock<Vec<Page>> = OnceLock::new();
    PAGES.get_or_init(|| TARGETED.iter().map(|target| render(target.route)).collect())
}

// ---------------------------------------------------------------------------------------------
// The four surfaces of a page
// ---------------------------------------------------------------------------------------------

/// The page's text and the attributes a visitor reads.
fn body_passages(page: &Page) -> Vec<Passage> {
    passages(&page.html)
}

/// The meta tags a visitor reads: in a search result or a link preview.
fn is_read_meta(attributes: &[(String, String)]) -> Option<String> {
    let key = attributes.iter().find(|(k, _)| k == "name" || k == "property").map(|(_, v)| v.clone())?;
    (key.ends_with("title") || key.ends_with("description") || key.ends_with(":alt")).then_some(key)
}

/// The titles the page sets, and the descriptions and alt text its head carries.
fn head_passages(page: &Page) -> Vec<Passage> {
    let mut out: Vec<Passage> = page.titles.iter().map(|t| Passage { place: "the <title>".to_string(), text: collapse(t) }).collect();
    for meta in &page.metas {
        if let (Some(key), Some((_, content))) = (is_read_meta(meta), meta.iter().find(|(k, _)| k == "content")) {
            out.push(Passage { place: format!("<meta {key}>"), text: collapse(content) });
        }
    }
    out
}

/// Every string in the page's JSON-LD.
fn data_passages(page: &Page) -> Vec<Passage> {
    let mut out = Vec::new();
    for (n, block) in json_ld(&page.html).iter().enumerate() {
        json_strings(block, format!("JSON-LD block {n}"), &mut out);
    }
    out
}

// ---------------------------------------------------------------------------------------------
// Reporting
// ---------------------------------------------------------------------------------------------

struct Hit {
    route: String,
    line: String,
}

fn hits_in(route: &str, passages: &[Passage]) -> Vec<Hit> {
    passages
        .iter()
        .flat_map(|p| {
            faults(&p.text).into_iter().map(move |f| Hit {
                route: route.to_string(),
                line: format!("{} in {}: {:?}", f.what, p.place, excerpt(&p.text, f.at)),
            })
        })
        .collect()
}

/// The hits per route, with a count per route first.
fn report(hits: &[Hit]) -> String {
    let mut by_route: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for hit in hits {
        by_route.entry(hit.route.as_str()).or_default().push(hit.line.as_str());
    }
    let mut out = format!("{} hits on {} routes\n", hits.len(), by_route.len());
    for (route, lines) in &by_route {
        out.push_str(&format!("  {route}: {}\n", lines.len()));
    }
    for (route, lines) in &by_route {
        out.push_str(&format!("\n{route} ({}):\n", lines.len()));
        for line in lines {
            out.push_str(&format!("  · {line}\n"));
        }
    }
    out
}

fn surface_hits(surface: fn(&Page) -> Vec<Passage>) -> Vec<Hit> {
    pages().iter().flat_map(|p| hits_in(&p.route, &surface(p))).collect()
}

// ---------------------------------------------------------------------------------------------
// The lock
// ---------------------------------------------------------------------------------------------

#[test]
fn the_text_of_every_targeted_page_is_plain() {
    let hits = surface_hits(body_passages);
    assert!(hits.is_empty(), "visible text carries an em dash, a semicolon or a dash between words:\n{}", report(&hits));
}

#[test]
fn the_head_of_every_targeted_page_is_plain() {
    let hits = surface_hits(head_passages);
    assert!(hits.is_empty(), "a title or a description carries an em dash, a semicolon or a dash between words:\n{}", report(&hits));
}

#[test]
fn the_structured_data_of_every_targeted_page_is_plain() {
    let hits = surface_hits(data_passages);
    assert!(hits.is_empty(), "the JSON-LD search engines show carries an em dash, a semicolon or a dash between words:\n{}", report(&hits));
}

#[test]
fn what_a_targeted_page_shows_on_demand_is_plain() {
    let hits: Vec<Hit> = TARGETED.iter().filter_map(|t| t.shown_on_demand.map(|shown| hits_in(t.route, &shown()))).flatten().collect();
    assert!(hits.is_empty(), "text a page shows when the visitor acts carries an em dash, a semicolon or a dash between words:\n{}", report(&hits));
}

/// Every targeted route is a page the site prerenders, every exempt one too, and no route is both.
#[test]
fn the_targeted_pages_are_real_and_never_the_legal_ones() {
    let prerendered = logicaffeine_web::sitemap::prerender_routes();
    for target in TARGETED {
        assert!(prerendered.iter().any(|p| p == target.route), "TARGETED names {}, which the site does not prerender", target.route);
        assert!(!EXEMPT.iter().any(|(route, _)| *route == target.route), "{} is exempt, and the lock targets it", target.route);
    }
    for (route, why) in EXEMPT {
        assert!(prerendered.iter().any(|p| p == route), "EXEMPT names {route} ({why}), which the site does not prerender");
    }
}

// ---------------------------------------------------------------------------------------------
// The lock reads what it claims to read
// ---------------------------------------------------------------------------------------------

/// A page whose text the reader skipped would pass the lock by reading nothing.
#[test]
fn every_targeted_page_has_prose_to_read() {
    for page in pages() {
        let words: usize = body_passages(page).iter().map(|p| p.text.split_whitespace().count()).sum();
        assert!(words >= 5, "the reader found only {words} words of prose on {}", page.route);
    }
}

/// The head check reads the head each targeted page sets, not an empty recorder.
#[test]
fn every_targeted_page_sets_a_title_and_a_description() {
    for page in pages() {
        assert!(page.titles.iter().any(|t| !t.trim().is_empty()), "{} sets no title", page.route);
        let descriptions = page.metas.iter().filter(|m| is_read_meta(m).is_some_and(|k| k == "description")).count();
        assert!(descriptions > 0, "{} sets no meta description", page.route);
    }
}

/// The structured-data check reads FAQ answers.
#[test]
fn the_faq_answers_are_read() {
    let answers = pages().iter().flat_map(data_passages).filter(|p| p.place.contains("(Answer) → text")).count();
    assert!(answers > 0, "the reader found no FAQ answer in the targeted pages' JSON-LD");
}

/// The on-demand check reads every message the pricing form can show.
#[test]
fn the_pricing_form_messages_are_read() {
    assert_eq!(pricing_form_messages().len(), EmailError::ALL.len() + PhoneError::ALL.len());
    assert!(pricing_form_messages().iter().all(|p| p.text.split_whitespace().count() >= 3), "{:?}", pricing_form_messages());
}

/// The on-demand check reads every lesson on the Learn roadmap, not only the one the page opens, and
/// every practice exercise behind it.
#[test]
fn every_learn_lesson_and_exercise_is_read() {
    let read = learn_lessons();
    for module in ContentEngine::shared().eras().iter().flat_map(|e| e.modules.iter()) {
        for section in &module.sections {
            let place = format!("lesson {}, ", section.id);
            assert!(
                read.iter().any(|p| p.place.starts_with(&place) && p.text.contains(&section.title)),
                "the reader never saw lesson {} ({:?}) as the page shows it",
                section.id,
                section.title
            );
        }
        for exercise in &module.exercises {
            let place = format!("practice {}'s prompt", exercise.id);
            assert!(read.iter().any(|p| p.place == place), "the reader never saw practice {}", exercise.id);
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The reader, on markup written to test it
// ---------------------------------------------------------------------------------------------

fn faults_in_markup(html: &str) -> Vec<String> {
    passages(html).iter().flat_map(|p| faults(&p.text).into_iter().map(|f| format!("{} {:?}", f.what, excerpt(&p.text, f.at)))).collect()
}

#[test]
fn code_editors_and_output_are_not_prose() {
    let html = concat!(
        "<p>Run <code>Let x be 1; Show x.</code> and read.</p>",
        "<pre>a — b; c</pre><kbd>Ctrl;</kbd><samp>x; y</samp>",
        "<textarea placeholder=\"Type here\">a; b — c</textarea>",
        "<math><mi>a</mi><mo>;</mo></math>",
        "<script>let a = 1; let b = 2;</script><style>a { color: red; }</style>",
        "<div class=\"code-editor lang-logos\"><span class=\"tok\">x; y</span><div>a — b</div></div>",
        "<div class=\"guide-code-editor\"><div>Let x be 1; done.</div></div>",
        "<div class=\"guide-code-output-content fol\">∀x; P(x) — Q(x)</div>",
        "<template><p>a; b</p></template>",
        "<!-- a comment; with — punctuation -->",
    );
    assert!(faults_in_markup(html).is_empty(), "the reader counted code as prose: {:?}", faults_in_markup(html));
}

#[test]
fn prose_beside_code_is_read() {
    assert_eq!(faults_in_markup("<p>Use <code>x</code>; then stop.</p>").len(), 1);
    assert_eq!(faults_in_markup("<pre>a; b</pre><p>Then — stop.</p>").len(), 1);
    assert_eq!(faults_in_markup("<div class=\"code-editor\">a; b</div><p>after; this</p>").len(), 1);
}

#[test]
fn a_number_range_keeps_its_en_dash() {
    for plain in ["7–15 lessons", "from 2024–2026", "pages A–Z", "Mon–Fri", "a well-known word", "x-y", "−3 below"] {
        assert!(faults(plain).is_empty(), "{plain:?} was faulted: {:?}", faults(plain));
    }
}

#[test]
fn every_dash_between_words_is_caught() {
    for (text, what) in [
        ("fast — and plain", "an em dash"),
        ("fast—and plain", "an em dash"),
        ("fast ― and plain", "a horizontal bar (an em dash by another name)"),
        ("fast – and plain", "an en dash between words (a range like 7–15 takes no space)"),
        ("fast –and plain", "an en dash between words (a range like 7–15 takes no space)"),
        ("fast - and plain", "a hyphen standing in for a dash"),
        ("fast -- and plain", "a hyphen standing in for a dash"),
        ("fast --- and plain", "a hyphen standing in for a dash"),
        ("fast; and plain", "a semicolon"),
        ("fast ; and plain", "a semicolon"),
        ("fast\u{037E} and plain", "a semicolon"),
        ("fast\u{FF1B} and plain", "a semicolon"),
        ("fast ⸺ and plain", "a two- or three-em dash"),
    ] {
        let found = faults(text);
        assert_eq!(found.len(), 1, "{text:?} gave {found:?}");
        assert_eq!(found[0].what, what, "{text:?}");
    }
}

#[test]
fn a_flag_or_a_negative_number_is_not_a_dash() {
    for plain in ["run largo build --release now", "it was -3 degrees", "the -o flag"] {
        assert!(faults(plain).is_empty(), "{plain:?} was faulted: {:?}", faults(plain));
    }
}

#[test]
fn references_are_read_as_the_characters_they_stand_for() {
    assert!(faults_in_markup("<p>Tom &amp; Jerry &lt;3 &#39;hi&#39; &#x27;x&#x27; a&nbsp;b &quot;q&quot; &copy; 2026</p>").is_empty());
    assert_eq!(faults_in_markup("<p>a &mdash; b</p>").len(), 1);
    assert_eq!(faults_in_markup("<p>a &#8212; b</p>").len(), 1);
    assert_eq!(faults_in_markup("<p>a &#x2014; b</p>").len(), 1);
    assert_eq!(faults_in_markup("<p>a&#59; b</p>").len(), 1);
    assert_eq!(faults_in_markup("<p>a &ndash; b, 7&ndash;15</p>").len(), 1);
    assert!(faults_in_markup("<p>R&D and AT&T</p>").is_empty());
}

#[test]
#[should_panic(expected = "does not know the reference &frobnicate;")]
fn an_unknown_reference_stops_the_reader() {
    passages("<p>a &frobnicate; b</p>");
}

#[test]
fn shown_and_spoken_attributes_are_read() {
    assert_eq!(faults_in_markup("<button title=\"Copy; paste\">Copy</button>").len(), 1);
    assert_eq!(faults_in_markup("<img alt=\"a — b\" src=\"x.png\">").len(), 1);
    assert_eq!(faults_in_markup("<input placeholder=\"name; email\">").len(), 1);
    assert_eq!(faults_in_markup("<nav aria-label=\"Main — site\"></nav>").len(), 1);
    assert_eq!(faults_in_markup("<input type=\"submit\" value=\"Send; now\">").len(), 1);
    assert!(faults_in_markup("<div style=\"color: red; margin: 0\" data-x=\"a;b\" href=\"/a;b\" onclick=\"f(); g();\">ok</div>").is_empty());
    assert!(faults_in_markup("<input type=\"text\" value=\"typed; by them\">").is_empty());
    assert!(faults_in_markup("<pre><span title=\"in; code\">x</span></pre>").is_empty());
    assert_eq!(faults_in_markup("<textarea placeholder=\"Your message; optional\">a; b</textarea>").len(), 1);
    assert!(faults_in_markup("<iframe srcdoc=\"&lt;p&gt;a; b &frobnicate; c&lt;/p&gt;\"></iframe>").is_empty());
}

#[test]
fn a_quoted_angle_bracket_does_not_end_a_tag() {
    assert!(faults_in_markup("<div title=\"a > b\">plain text</div>").is_empty());
    assert_eq!(faults_in_markup("<div title=\"a > b\">x; y</div>").len(), 1);
}

#[test]
fn each_passage_says_where_it_sits() {
    let found = passages("<main class=\"guide\"><section id=\"basics\"><p>One; two</p></section></main>");
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].place, "main.guide > section#basics > p");
    assert_eq!(found[0].text, "One; two");
}

#[test]
fn inline_elements_run_on_and_blocks_start_anew() {
    let found = passages("<p>One <strong>two</strong> <a href=\"/\">three</a>.</p><p>Four</p>");
    let texts: Vec<&str> = found.iter().map(|p| p.text.as_str()).collect();
    assert_eq!(texts, ["One two three.", "Four"]);
}

#[test]
fn the_excerpt_is_sixty_characters_around_the_fault() {
    let text = format!("{}; {}", "a".repeat(50), "b".repeat(50));
    let found = faults(&text);
    assert_eq!(found.len(), 1);
    let shown = excerpt(&text, found[0].at);
    assert_eq!(shown, format!("…{}; {}…", "a".repeat(30), "b".repeat(28)));
}
