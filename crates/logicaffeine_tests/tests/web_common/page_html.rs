//! Reading a prerendered page the way a reader and a crawler do: rendering a route natively, the
//! text a reader sees, the structured data a crawler reads, and the elements, links and attributes
//! a suite asserts on. The markup is the SSR output of `dioxus_ssr`, so these readers know its
//! habits: attributes in double quotes, the five escaped characters written as numeric references.

#![allow(dead_code)]

use dioxus::prelude::*;
use dioxus_history::{History, MemoryHistory};
use std::collections::BTreeMap;
use std::rc::Rc;

/// The route as the SSG prerender writes it.
pub fn render_route(path: &str) -> String {
    let mut vdom = VirtualDom::new(logicaffeine_web::App);
    vdom.provide_root_context(Rc::new(MemoryHistory::with_initial_path(path)) as Rc<dyn History>);
    vdom.rebuild_in_place();
    dioxus_ssr::render(&vdom)
}

/// Text as the prerender writes it: the five characters HTML escapes, as numeric references.
pub fn escaped(text: &str) -> String {
    text.chars()
        .map(|c| match c {
            '"' => "&#34;".to_string(),
            '&' => "&#38;".to_string(),
            '\'' => "&#39;".to_string(),
            '<' => "&#60;".to_string(),
            '>' => "&#62;".to_string(),
            c => c.to_string(),
        })
        .collect()
}

/// Character references read back to the characters they stand for.
pub fn decode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let tail = &rest[amp..];
        let Some(semi) = tail.find(';').filter(|&i| i <= 10) else {
            out.push('&');
            rest = &tail[1..];
            continue;
        };
        let name = &tail[1..semi];
        let decoded = match name {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            "nbsp" => Some('\u{a0}'),
            _ if name.starts_with("#x") || name.starts_with("#X") => u32::from_str_radix(&name[2..], 16).ok().and_then(char::from_u32),
            _ if name.starts_with('#') => name[1..].parse::<u32>().ok().and_then(char::from_u32),
            _ => None,
        };
        match decoded {
            Some(c) => {
                out.push(c);
                rest = &tail[semi + 1..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The markup with its `<script>` and `<style>` elements removed.
pub fn without_raw_text(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    loop {
        let next = ["<script", "<style"].iter().filter_map(|open| rest.find(open).map(|i| (i, *open))).min();
        let Some((at, open)) = next else { break };
        out.push_str(&rest[..at]);
        let close = if open == "<script" { "</script>" } else { "</style>" };
        rest = match rest[at..].find(close) {
            Some(end) => &rest[at + end + close.len()..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// Elements that sit inside a line of text: a browser runs their text on with the text around
/// them, where every other element starts a new run.
const INLINE_TAGS: &[&str] = &["a", "abbr", "b", "cite", "code", "em", "i", "kbd", "q", "samp", "small", "span", "strong", "sub", "sup"];

/// What a reader sees: tags, scripts and styles removed, references decoded, whitespace collapsed.
pub fn visible_text(html: &str) -> String {
    let markup = without_raw_text(html);
    let mut text = String::with_capacity(markup.len());
    let mut rest = markup.as_str();
    while let Some(lt) = rest.find('<') {
        text.push_str(&rest[..lt]);
        let tag = &rest[lt + 1..];
        let name: String = tag.trim_start_matches('/').chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
        if !INLINE_TAGS.contains(&name.to_ascii_lowercase().as_str()) {
            text.push(' ');
        }
        rest = match tag.find('>') {
            Some(gt) => &tag[gt + 1..],
            None => "",
        };
    }
    text.push_str(rest);
    decode(&text).replace('\u{a0}', " ").split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Every JSON-LD block the markup carries, parsed.
pub fn json_ld(html: &str) -> Vec<serde_json::Value> {
    html.split("<script type=\"application/ld+json\">")
        .skip(1)
        .map(|tail| {
            let json = &tail[..tail.find("</script>").expect("a closed JSON-LD script")];
            serde_json::from_str(json).unwrap_or_else(|e| panic!("a JSON-LD block does not parse ({e}):\n{json}"))
        })
        .collect()
}

/// Every string a JSON value holds, keys and values alike, in document order.
pub fn json_strings(value: &serde_json::Value) -> Vec<String> {
    let mut out = Vec::new();
    fn walk(value: &serde_json::Value, out: &mut Vec<String>) {
        match value {
            serde_json::Value::String(s) => out.push(s.clone()),
            serde_json::Value::Array(items) => items.iter().for_each(|v| walk(v, out)),
            serde_json::Value::Object(map) => map.iter().for_each(|(k, v)| {
                out.push(k.clone());
                walk(v, out);
            }),
            _ => {}
        }
    }
    walk(value, &mut out);
    out
}

/// The whole element that carries `id="{id}"`: its start tag through the end tag that closes it.
pub fn element_by_id<'a>(html: &'a str, id: &str) -> &'a str {
    let needle = format!(" id=\"{id}\"");
    let at = html.find(&needle).unwrap_or_else(|| panic!("the page renders no element with id={id:?}"));
    let start = html[..at].rfind('<').expect("the id sits inside a tag");
    let name: String = html[start + 1..].chars().take_while(|c| c.is_ascii_alphanumeric()).collect();
    &html[start..start + closing(&html[start..], &name)]
}

/// The byte length of the element at the start of `html`, named `name`, through its end tag.
fn closing(html: &str, name: &str) -> usize {
    let open = format!("<{name}");
    let close = format!("</{name}>");
    let opens_here = |i: usize| html[i + open.len()..].chars().next().is_some_and(|c| c.is_whitespace() || c == '>' || c == '/');
    let mut depth = 0usize;
    let mut at = 0usize;
    loop {
        let mut from = at;
        let next_open = loop {
            match html[from..].find(&open).map(|i| from + i) {
                Some(i) if opens_here(i) => break Some(i),
                Some(i) => from = i + open.len(),
                None => break None,
            }
        };
        let next_close = html[at..].find(&close).map(|i| at + i).unwrap_or_else(|| panic!("<{name}> never closes"));
        match next_open {
            Some(o) if o < next_close => {
                depth += 1;
                at = o + open.len();
            }
            _ => {
                depth -= 1;
                at = next_close + close.len();
                if depth == 0 {
                    return at;
                }
            }
        }
    }
}

/// The attributes of one start tag (`<input type="email" required=true …>`), by name.
pub fn attributes(tag: &str) -> BTreeMap<String, String> {
    let body = tag.trim_start_matches('<');
    let body = body.trim_end_matches('>').trim_end_matches('/');
    let mut attrs = BTreeMap::new();
    let mut chars = body.char_indices().peekable();
    while let Some(&(_, c)) = chars.peek() {
        if c.is_whitespace() {
            break;
        }
        chars.next();
    }
    loop {
        while chars.peek().is_some_and(|&(_, c)| c.is_whitespace()) {
            chars.next();
        }
        let Some(&(name_start, _)) = chars.peek() else { break };
        let mut name_end = body.len();
        while let Some(&(i, c)) = chars.peek() {
            if c == '=' || c.is_whitespace() {
                name_end = i;
                break;
            }
            chars.next();
        }
        let name = body[name_start..name_end.min(body.len())].to_string();
        if chars.peek().is_some_and(|&(_, c)| c == '=') {
            chars.next();
            let value = match chars.peek().map(|&(_, c)| c) {
                Some(q @ ('"' | '\'')) => {
                    chars.next();
                    let mut v = String::new();
                    for (_, c) in chars.by_ref() {
                        if c == q {
                            break;
                        }
                        v.push(c);
                    }
                    v
                }
                _ => {
                    let mut v = String::new();
                    while let Some(&(_, c)) = chars.peek() {
                        if c.is_whitespace() {
                            break;
                        }
                        v.push(c);
                        chars.next();
                    }
                    v
                }
            };
            attrs.insert(name, decode(&value));
        } else if !name.is_empty() {
            attrs.insert(name, String::new());
        }
    }
    attrs
}

/// Every start tag named `name` in the markup, as its attributes.
pub fn tags(html: &str, name: &str) -> Vec<BTreeMap<String, String>> {
    let open = format!("<{name}");
    let mut found = Vec::new();
    let mut rest = html;
    while let Some(at) = rest.find(&open) {
        let tail = &rest[at..];
        let after = tail[open.len()..].chars().next();
        let end = tail.find('>').expect("a closed start tag");
        if matches!(after, Some(c) if c.is_whitespace() || c == '>' || c == '/') {
            found.push(attributes(&tail[..=end]));
        }
        rest = &tail[end + 1..];
    }
    found
}

/// Every element named `name` in the markup, whole (start tag, content and end tag), in document
/// order, the ones nested inside another of the same name included.
pub fn elements<'a>(html: &'a str, name: &str) -> Vec<&'a str> {
    let open = format!("<{name}");
    let mut found = Vec::new();
    let mut at = 0usize;
    while let Some(i) = html[at..].find(&open) {
        let start = at + i;
        if html[start + open.len()..].chars().next().is_some_and(|c| c.is_whitespace() || c == '>') {
            let len = closing(&html[start..], name);
            found.push(&html[start..start + len]);
        }
        at = start + open.len();
    }
    found
}

/// The markup between an element's start tag and its end tag.
pub fn inner(element: &str) -> &str {
    let start = element.find('>').expect("a closed start tag") + 1;
    let end = element.rfind("</").expect("an end tag");
    &element[start..end]
}

/// The attributes of an element's own start tag.
pub fn own_attributes(element: &str) -> BTreeMap<String, String> {
    attributes(&element[..=element.find('>').expect("a closed start tag")])
}

/// Whether the element's class list holds `class`.
pub fn has_class(attrs: &BTreeMap<String, String>, class: &str) -> bool {
    attrs.get("class").is_some_and(|c| c.split_whitespace().any(|w| w == class))
}

/// Every link in the markup: its attributes and the text it shows.
pub fn links(html: &str) -> Vec<(BTreeMap<String, String>, String)> {
    elements(html, "a").into_iter().map(|a| (own_attributes(a), visible_text(inner(a)))).collect()
}

/// The text of every `<tag>` element in the markup, in order, as a reader sees it.
pub fn element_texts(html: &str, tag: &str) -> Vec<String> {
    elements(html, tag).into_iter().map(|e| visible_text(inner(e))).collect()
}

/// An element's text with its line breaks and indentation kept: tags removed, references decoded.
/// What a `<pre>` shows, and what a copy of it holds.
pub fn preformatted_text(element: &str) -> String {
    let content = inner(element);
    let mut text = String::new();
    let mut rest = content;
    while let Some(lt) = rest.find('<') {
        text.push_str(&rest[..lt]);
        rest = &rest[lt + rest[lt..].find('>').expect("a closed tag") + 1..];
    }
    text.push_str(rest);
    decode(&text)
}

/// A boolean attribute that is present and not set false.
pub fn is_set(attrs: &BTreeMap<String, String>, name: &str) -> bool {
    attrs.get(name).is_some_and(|v| v != "false")
}
