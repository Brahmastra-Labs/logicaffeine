//! No code view asks the DOM channel to move more nodes at once than the channel can count.
//!
//! dioxus-interpreter sends the size of every append, replace and insert as a `u16` (`m as u16`
//! in its `write_native_mutations`). The Studio's highlighters draw a node for every token, all
//! siblings under one parent, so a long enough text — a big program pasted into an editor, or
//! Compile's Rust for a theorem with some fifteen hundred premises — asked for one edit of more
//! than 65,535 nodes. The count wrapped, the interpreter went looking for a node it no longer had
//! (`Cannot read properties of null (reading 'replaceWith')`), and the Studio stopped answering.
//! These render such texts — as a first render and as an update — and check every edit fits, and
//! that the highlighted text still reads exactly as the source.

#![cfg(feature = "web-tests")]

use dioxus::dioxus_core::{Mutation, Mutations, ScopeId};
use dioxus::prelude::*;
use logicaffeine_web::ui::components::code_editor::{CodeEditor, CodeView, Language};
use std::cell::RefCell;

/// The most nodes one edit can move: its count travels as a `u16`.
const CHANNEL_MAX: usize = u16::MAX as usize;

/// The largest number of nodes any single edit in `mutations` moves.
fn widest_move(mutations: &Mutations) -> usize {
    mutations
        .edits
        .iter()
        .map(|edit| match edit {
            Mutation::AppendChildren { m, .. }
            | Mutation::ReplaceWith { m, .. }
            | Mutation::ReplacePlaceholder { m, .. }
            | Mutation::InsertAfter { m, .. }
            | Mutation::InsertBefore { m, .. } => *m,
            _ => 0,
        })
        .max()
        .unwrap_or(0)
}

/// A program of `lines` short statements — about nine highlighted nodes each.
fn program(lines: usize) -> String {
    "Let x be 1.\n".repeat(lines)
}

thread_local! {
    static SOURCE: RefCell<String> = const { RefCell::new(String::new()) };
}

fn source() -> String {
    SOURCE.with(|s| s.borrow().clone())
}

#[component]
fn ViewHost() -> Element {
    rsx! { CodeView { code: source(), language: Language::Logos } }
}

#[component]
fn EditorHost() -> Element {
    rsx! {
        CodeEditor {
            value: source(),
            on_change: move |_| {},
            language: Language::Logos,
        }
    }
}

/// The first render of `host` over `text`, and the update from an empty text to `text`.
fn render_both_ways(host: fn() -> Element, text: &str) -> (Mutations, Mutations) {
    SOURCE.with(|s| *s.borrow_mut() = text.to_string());
    let first = VirtualDom::new(host).rebuild_to_vec();

    SOURCE.with(|s| s.borrow_mut().clear());
    let mut dom = VirtualDom::new(host);
    dom.rebuild_in_place();
    SOURCE.with(|s| *s.borrow_mut() = text.to_string());
    dom.mark_dirty(ScopeId::APP);
    let update = dom.render_immediate_to_vec();
    (first, update)
}

fn assert_fits(host: fn() -> Element, what: &str) {
    let text = program(30_000);
    let (first, update) = render_both_ways(host, &text);
    assert!(
        widest_move(&first) <= CHANNEL_MAX,
        "{what}: the first render moves {} nodes in one edit; the channel counts to {CHANNEL_MAX}",
        widest_move(&first)
    );
    assert!(
        widest_move(&update) <= CHANNEL_MAX,
        "{what}: growing from empty moves {} nodes in one edit; the channel counts to {CHANNEL_MAX}",
        widest_move(&update)
    );
}

#[test]
fn a_huge_read_only_view_fits_the_channel() {
    assert_fits(ViewHost, "CodeView");
}

#[test]
fn a_huge_program_in_an_editor_fits_the_channel() {
    assert_fits(EditorHost, "CodeEditor");
}

/// The text of the highlight layer in rendered HTML: every tag dropped, entities decoded.
fn highlighted_text(html: &str) -> String {
    let start = html.find("class=\"code-editor-highlight\"").expect("a highlight layer");
    let open_end = start + html[start..].find('>').expect("its opening tag ends");
    let mut depth = 1usize;
    let mut text = String::new();
    let mut rest = &html[open_end + 1..];
    while depth > 0 {
        let lt = rest.find('<').expect("the highlight layer closes");
        text.push_str(&rest[..lt]);
        let gt = lt + rest[lt..].find('>').expect("a tag ends");
        let tag = &rest[lt..=gt];
        if tag.starts_with("</div") {
            depth -= 1;
        } else if tag.starts_with("<div") && !tag.ends_with("/>") {
            depth += 1;
        }
        rest = &rest[gt + 1..];
    }
    decode_entities(&text)
}

/// HTML text with its character references read back: numeric (`&#34;`, `&#x22;`) and the named
/// ones an escaper writes (`&lt;`, `&gt;`, `&quot;`, `&apos;`, `&amp;`).
fn decode_entities(html: &str) -> String {
    let mut out = String::with_capacity(html.len());
    let mut rest = html;
    while let Some(amp) = rest.find('&') {
        out.push_str(&rest[..amp]);
        let after = &rest[amp + 1..];
        let semi = after.find(';').expect("a character reference ends with `;`");
        let name = &after[..semi];
        let decoded = match name {
            "lt" => '<',
            "gt" => '>',
            "quot" => '"',
            "apos" => '\'',
            "amp" => '&',
            _ if name.starts_with("#x") || name.starts_with("#X") => {
                char::from_u32(u32::from_str_radix(&name[2..], 16).expect("a hex reference")).expect("a character")
            }
            _ if name.starts_with('#') => char::from_u32(name[1..].parse().expect("a decimal reference")).expect("a character"),
            other => panic!("an unexpected character reference &{other};"),
        };
        out.push(decoded);
        rest = &after[semi + 1..];
    }
    out.push_str(rest);
    out
}

#[test]
fn the_highlighted_text_reads_exactly_as_the_source() {
    for lines in [0, 1, 455, 456, 457, 3_000, 30_000] {
        let text = program(lines) + "Show \"a & b <c>\".\n";
        SOURCE.with(|s| *s.borrow_mut() = text.clone());
        for (host, what) in [(ViewHost as fn() -> Element, "CodeView"), (EditorHost, "CodeEditor")] {
            let mut dom = VirtualDom::new(host);
            dom.rebuild_in_place();
            let shown = highlighted_text(&dioxus_ssr::render(&dom));
            assert!(shown == text, "{what} over {lines} lines shows other text than its source ({} vs {} bytes)", shown.len(), text.len());
        }
    }
}
