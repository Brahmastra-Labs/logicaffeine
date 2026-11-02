//! The front page and `/studio` draw one Studio.
//!
//! The user (2026-10-15): "The demo showing each example should be more modeled lke the file tree and
//! IDE in the Studio so when people get to the actual studio it looks basically the same. Can we not
//! use the Studio as a component?" The Studio's working area is one component, the workbench
//! (`ui/pages/studio_workbench.rs`): its toolbar, Files tree, editor and output columns and status
//! bar, with every mode's handlers. The Studio page renders it with the page's own head, nav, the
//! reader's store and the address bar around it, and the front page renders it in a frame. This lock
//! holds that shape, so the two pages can never grow two Studios again.

#![cfg(feature = "web-tests")]

#[path = "web_common/page_html.rs"]
mod page_html;

use dioxus::prelude::*;
use dioxus_history::{History, MemoryHistory};
use logicaffeine_compile::web_host::assemble_preview_painted;
use logicaffeine_web::ui::examples::{compiled_runtime_notice, planted_bug_notice, seeded_examples};
use logicaffeine_web::ui::pages::landing::content::shelf;
use logicaffeine_web::ui::pages::studio::STUDIO_STYLE;
use logicaffeine_web::ui::pages::studio_workbench::{preview_document, use_shelf, Shelf, StudioFrame, FRAME_STYLE, PREVIEW_CENTERING_STYLE, PREVIEW_TITLE};
use logicaffeine_web::ui::router::studio_file_url;
use page_html::{attributes, element_by_id, elements, has_class, inner, links, own_attributes, render_route, tags, visible_text, without_raw_text};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::OnceLock;

fn web_ui() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/logicaffeine_web/src/ui")
}

const WORKBENCH: &str = "pages/studio_workbench.rs";

/// The classes of the Studio's working area: its toolbar, its row of files and panels, and each
/// panel's header. The page's own frame (`studio-container`) is the page's to draw.
const WORKBENCH_CLASSES: &[&str] = &["studio-toolbar", "studio-content", "studio-sidebar", "studio-main", "studio-panel", "panel-header"];

/// What only the Studio page may reach: the reader's store, the address bar, the page's head and nav,
/// and the guard its prerendered copy carries.
const PAGE_ONLY: &[&str] = &[
    "get_platform_vfs",
    "replace_bar_url",
    "sync_studio_url",
    "retire_old_copies",
    "open_store",
    "PageHead",
    "MainNav",
    "JsonLd",
    "prerender_guard",
];

/// A Rust source with its comments, raw strings (the stylesheets) and char literals blanked out, so
/// what is left is code and its ordinary string literals.
fn code_of(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut out = String::with_capacity(source.len());
    let mut i = 0;
    while i < bytes.len() {
        let rest = &source[i..];
        if rest.starts_with("//") {
            let end = rest.find('\n').unwrap_or(rest.len());
            out.push_str(&blank(&rest[..end]));
            i += end;
        } else if rest.starts_with("/*") {
            let end = rest.find("*/").map_or(rest.len(), |at| at + 2);
            out.push_str(&blank(&rest[..end]));
            i += end;
        } else if let Some(hashes) = raw_string_opening(rest) {
            let close = format!("\"{}", "#".repeat(hashes));
            let body = 2 + hashes;
            let end = rest[body..].find(&close).map_or(rest.len(), |at| body + at + close.len());
            out.push_str(&blank(&rest[..end]));
            i += end;
        } else if rest.starts_with('"') {
            let mut j = 1;
            while j < rest.len() && !rest[j..].starts_with('"') {
                j += if rest[j..].starts_with('\\') { 2 } else { rest[j..].chars().next().unwrap().len_utf8() };
            }
            let end = (j + 1).min(rest.len());
            out.push_str(&rest[..end]);
            i += end;
        } else if rest.starts_with('\'') && rest.len() > 2 && (rest.as_bytes()[2] == b'\'' || rest.starts_with("'\\")) {
            let end = rest[1..].find('\'').map_or(1, |at| at + 2);
            out.push_str(&" ".repeat(end));
            i += end;
        } else {
            let ch = rest.chars().next().unwrap();
            out.push(ch);
            i += ch.len_utf8();
        }
    }
    out
}

/// `text` with everything but its line breaks blanked, so the lines of what is left keep their numbers.
fn blank(text: &str) -> String {
    text.chars().map(|c| if c == '\n' { '\n' } else { ' ' }).collect()
}

/// How many `#`s open a raw string at the start of `text`, if one opens there (`r"`, `r#"`, …).
fn raw_string_opening(text: &str) -> Option<usize> {
    let after_r = text.strip_prefix('r')?;
    let hashes = after_r.chars().take_while(|c| *c == '#').count();
    after_r[hashes..].starts_with('"').then_some(hashes)
}

/// The ordinary string literals in `code`, unescaped no further than the quotes.
fn literals(code: &str) -> Vec<&str> {
    let mut found = Vec::new();
    let mut rest = code;
    while let Some(open) = rest.find('"') {
        let after = &rest[open + 1..];
        let mut j = 0;
        while j < after.len() && !after[j..].starts_with('"') {
            j += if after[j..].starts_with('\\') { 2 } else { after[j..].chars().next().unwrap().len_utf8() };
        }
        found.push(&after[..j.min(after.len())]);
        rest = &after[(j + 1).min(after.len())..];
    }
    found
}

/// The workbench classes a source draws: those a string literal names as a whole class token.
fn drawn_classes(source: &str) -> Vec<&'static str> {
    let code = code_of(source);
    let tokens: Vec<String> = literals(&code).iter().flat_map(|l| l.split_whitespace().map(str::to_string)).collect();
    WORKBENCH_CLASSES.iter().copied().filter(|class| tokens.iter().any(|t| t == class)).collect()
}

fn sources(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap_or_else(|e| panic!("read {}: {e}", dir.display())) {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

fn relative(path: &Path) -> String {
    path.strip_prefix(web_ui()).unwrap_or(path).to_string_lossy().into_owned()
}

#[test]
fn the_reader_sees_classes_and_skips_stylesheets_comments_and_selectors() {
    let source = r##"
        // class: "studio-toolbar" in a comment is not drawn
        /* "studio-main" in a block comment is not drawn */
        const STYLE: &str = r#".studio-panel { color: red; } "studio-content""#;
        fn find() { query(".studio-sidebar > .studio-main"); }
        fn view() -> Element { rsx! { div { class: "panel-header wide", "Code \"Editor\"" } } }
        fn char_is_not_a_string() -> char { '"' }
        fn more() -> &'static str { "studio-toolbar" }
    "##;
    assert_eq!(drawn_classes(source), vec!["studio-toolbar", "panel-header"]);
}

/// Only the workbench draws the Studio's working area. The Studio page wraps it, and the front page
/// frames it; neither draws a toolbar or a panel of its own.
#[test]
fn only_the_workbench_draws_the_studio() {
    let ui = web_ui();
    let workbench = ui.join(WORKBENCH);
    assert!(workbench.is_file(), "the Studio's workbench has no module of its own yet: {}", workbench.display());
    let drawn = drawn_classes(&std::fs::read_to_string(&workbench).unwrap());
    for class in WORKBENCH_CLASSES {
        assert!(drawn.contains(class), "{WORKBENCH} draws no {class}, so this lock would prove nothing about it");
    }
    let elsewhere: Vec<String> = sources(&ui)
        .into_iter()
        .filter(|path| relative(path) != WORKBENCH)
        .filter_map(|path| {
            let drawn = drawn_classes(&std::fs::read_to_string(&path).unwrap());
            (!drawn.is_empty()).then(|| format!("{} draws {}", relative(&path), drawn.join(", ")))
        })
        .collect();
    assert!(elsewhere.is_empty(), "only {WORKBENCH} may draw the Studio's working area:\n{}", elsewhere.join("\n"));
}

/// The workbench reaches no store, no address bar and no page head: those are the Studio page's, so
/// a workbench on another page can never write a visitor's files or move their address.
#[test]
fn the_workbench_reaches_no_store_no_address_and_no_head() {
    let workbench = web_ui().join(WORKBENCH);
    assert!(workbench.is_file(), "the Studio's workbench has no module of its own yet: {}", workbench.display());
    let code = code_of(&std::fs::read_to_string(&workbench).unwrap());
    let reached: Vec<&str> = PAGE_ONLY.iter().copied().filter(|name| code.contains(name)).collect();
    assert!(reached.is_empty(), "{WORKBENCH} reaches what only the Studio page may: {}", reached.join(", "));
}

/// A Web page's preview is painted from the first frame: the server's render already carries the page's
/// first paint, built from the file the Studio opens, in a frame a screen reader can name. Before, the
/// preview started from an empty source, so the first frame showed a build error over a blank frame,
/// and every rebuild flashed blank before the page painted.
#[test]
fn a_web_page_prerenders_its_preview_painted() {
    let path = "/examples/code/web/counter.logos";
    let source = seeded_examples().find(|spec| spec.vfs_path == path).unwrap_or_else(|| panic!("{path} is not a shipped example")).source;
    let html = render_route(&studio_file_url(path));
    let frames = tags(&html, "iframe");
    assert_eq!(frames.len(), 1, "the Studio renders {} previews for {path}", frames.len());
    let painted = assemble_preview_painted(source).unwrap_or_else(|e| panic!("{path} does not build: {e:?}"));
    let want = preview_document(source);
    assert!(frames[0].get("srcdoc") == Some(&want), "the Studio's first frame of {path} is not the Studio's preview document");
    let body = &painted[painted.find("<body").unwrap()..];
    assert!(want.ends_with(body), "the Studio's preview of {path} does not carry the page's first paint");
    assert_eq!(want.matches(PREVIEW_CENTERING_STYLE).count(), 1, "the Studio's preview of {path} does not center the page in its frame");
    assert!(want.find(PREVIEW_CENTERING_STYLE).unwrap() < want.find("</head>").unwrap(), "the centering style is not in the preview's head");
    assert_eq!(frames[0].get("title").map(String::as_str), Some(PREVIEW_TITLE), "the preview has no name for a screen reader");
    assert_eq!(frames[0].get("sandbox").map(String::as_str), Some("allow-scripts"), "the preview is not sandboxed");
}

// ---------------------------------------------------------------------------------------------
// The Studio's words are plain
// ---------------------------------------------------------------------------------------------

/// Modules the Studio reaches that are the site's, not the Studio's, so the Studio's sources stop at
/// them, each with why.
const THE_SITES: &[(&str, &str)] = &[
    ("seo.rs", "every page's head and structured data"),
    ("router.rs", "the site's routes"),
    ("pages/mod.rs", "the list of the site's pages"),
    ("components/main_nav.rs", "the site's nav, which every page draws and the punctuation lock reads on the pages it names"),
    ("examples.rs", "the registry: the examples' own text and what they print, which the Studio shows as code; its two notices are read through their functions"),
];

/// The module file a path from `crate::ui` names, if any: the longest leading part that is a module.
fn module_file(segments: &[&str]) -> Option<PathBuf> {
    let ui = web_ui();
    (1..=segments.len()).rev().find_map(|n| {
        let stem = ui.join(segments[..n].join("/"));
        [stem.with_extension("rs"), stem.join("mod.rs")].into_iter().find(|f| f.is_file())
    })
}

/// The paths a source names after `prefix`, as their segments.
fn named_paths<'a>(code: &'a str, prefix: &str) -> Vec<Vec<&'a str>> {
    code.match_indices(prefix)
        .filter(|(at, _)| !code[..*at].ends_with(|c: char| c.is_ascii_alphanumeric() || c == '_'))
        .map(|(at, _)| {
            let rest = &code[at + prefix.len()..];
            let end = rest.find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == ':')).unwrap_or(rest.len());
            rest[..end].split("::").filter(|s| !s.is_empty()).collect()
        })
        .collect()
}

/// The sources the Studio draws from: the page, the workbench, and every module of the app they reach,
/// through `crate::ui::` and `super::` paths, up to the site's own modules ([`THE_SITES`]).
fn the_studios_sources() -> Vec<PathBuf> {
    let ui = web_ui();
    let sites: Vec<PathBuf> = THE_SITES.iter().map(|(file, _)| ui.join(file)).collect();
    let mut found: Vec<PathBuf> = vec![ui.join("pages/studio.rs"), ui.join(WORKBENCH)];
    let mut at = 0;
    while at < found.len() {
        let file = found[at].clone();
        at += 1;
        let code = code_of(&std::fs::read_to_string(&file).unwrap());
        let parent: Vec<String> = file.parent().unwrap().strip_prefix(&ui).unwrap().iter().map(|s| s.to_string_lossy().into_owned()).collect();
        let mut reached: Vec<Vec<String>> = named_paths(&code, "crate::ui::").into_iter().map(|p| p.into_iter().map(str::to_string).collect()).collect();
        reached.extend(named_paths(&code, "super::").into_iter().map(|p| parent.iter().cloned().chain(p.into_iter().map(str::to_string)).collect()));
        for path in reached {
            let segments: Vec<&str> = path.iter().map(String::as_str).collect();
            if let Some(module) = module_file(&segments).filter(|m| !found.contains(m) && !sites.contains(m)) {
                found.push(module);
            }
        }
    }
    found.sort();
    found
}

/// A string literal as the program holds it: its escapes read.
fn unescaped(literal: &str) -> String {
    let mut out = String::new();
    let mut chars = literal.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('u') => {
                let hex: String = chars.by_ref().skip(1).take_while(|c| *c != '}').collect();
                out.extend(u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32));
            }
            Some('\n') => {
                while chars.peek().is_some_and(|c| c.is_whitespace()) {
                    chars.next();
                }
            }
            Some(other) => out.push(other),
            None => {}
        }
    }
    out
}

/// The text a literal shows a reader: the literal itself, or the text between the tags of the markup
/// it builds.
fn shown(literal: &str) -> String {
    let text = unescaped(literal);
    if !(text.contains('<') && text.contains('>')) {
        return text;
    }
    let mut out = String::new();
    let mut inside = false;
    for c in text.chars() {
        match c {
            '<' => inside = true,
            '>' if inside => {
                inside = false;
                out.push(' ');
            }
            c if !inside => out.push(c),
            _ => {}
        }
    }
    out
}

/// The punctuation the user keeps off the site's pages that `text` uses: an em dash or a horizontal
/// bar, a dash typed between words, and a semicolon between words. A semicolon in code or in a
/// stylesheet (`color: red; font-weight: 600`, `a = 1; b = 2`) is not between words.
fn unplain(text: &str) -> Vec<&'static str> {
    let chars: Vec<char> = text.chars().collect();
    let mut found = Vec::new();
    for (i, c) in chars.iter().enumerate() {
        let before = i.checked_sub(1).map(|j| chars[j]);
        let after = chars.get(i + 1).copied();
        match c {
            '\u{2014}' | '\u{2015}' => found.push("an em dash"),
            '\u{2013}' | '-' if before.is_some_and(char::is_whitespace) && after.is_none_or(char::is_whitespace) => found.push("a dash between words"),
            ';' if before.is_some_and(char::is_alphabetic) && after == Some(' ') => {
                let word: String = chars[i + 2..].iter().take_while(|c| c.is_alphabetic()).collect();
                if !word.is_empty() && chars.get(i + 2 + word.chars().count()) == Some(&' ') {
                    found.push("a semicolon");
                }
            }
            _ => {}
        }
    }
    found
}

#[test]
fn the_plain_reader_finds_what_the_site_keeps_off_its_pages() {
    assert_eq!(unplain("Nothing printed yet \u{2014} what the program shows appears here."), ["an em dash"]);
    assert_eq!(unplain("The sweep line is the program counter; each register lane lights up"), ["a semicolon"]);
    assert_eq!(unplain("a step - and another"), ["a dash between words"]);
    assert!(unplain("padding: 0; overflow: hidden; color: red; font-weight: 600;").is_empty());
    assert!(unplain("let a = 1; let b = 2; el.focus(); window.scrollTo(0, 0);").is_empty());
    assert!(unplain("k-induction, 7\u{2013}15 steps, a -1 offset").is_empty());
    assert_eq!(shown(r#"<div style=\"font-size:12px;\">Solved grid \u{2014} every filled cell certified, no Z3:</div>"#), " Solved grid \u{2014} every filled cell certified, no Z3: ");
    assert_eq!(unescaped(r"one \
            two\n\u{2717}"), "one two\n\u{2717}");
}

/// Sentences the lock lets stand while a pre-existing test pins their words, each with that test. Each
/// goes, and its test changes, once the user approves the change.
const PINNED_BY_A_TEST: &[(&str, &str)] = &[(
    "\u{2713} Certified UNSAT \u{2014} {} cannot fit {}.",
    "pigeonhole_viz's the_verdict_reads_as_a_certified_success reads the verdict word for word",
)];

/// The Studio says nothing with the punctuation the user keeps off the site (2026-10-01, and D1 of the
/// homepage plan, approved 2026-10-15: "the Studio's words go plain"): no string any of its sources
/// holds uses an em dash, a dash between words or a semicolon between words. Comments and
/// stylesheets are not read.
#[test]
fn the_studio_says_nothing_with_an_em_dash_or_a_semicolon() {
    let sources = the_studios_sources();
    for expected in ["pages/studio_actions.rs", "pages/hardware_report.rs", "pages/register_alloc_viz.rs", "components/socratic_guide.rs", "components/debug_drawer.rs", "fault.rs"] {
        assert!(sources.contains(&web_ui().join(expected)), "the Studio's sources miss {expected}: {:?}", sources.iter().map(|p| relative(p)).collect::<Vec<_>>());
    }
    let mut found = Vec::new();
    for path in &sources {
        let source = std::fs::read_to_string(path).unwrap();
        let code = code_of(&source);
        for literal in literals(&code).into_iter().filter(|literal| !PINNED_BY_A_TEST.iter().any(|(pinned, _)| *pinned == unescaped(literal))) {
            let text = shown(literal);
            let faults = unplain(&text);
            if !faults.is_empty() {
                let line = code[..literal.as_ptr() as usize - code.as_ptr() as usize].lines().count();
                found.push(format!("{}:{line} has {}: {:?}", relative(path), faults.join(", "), text.trim().chars().take(110).collect::<String>()));
            }
        }
    }
    for spec in seeded_examples() {
        let notices = [compiled_runtime_notice(spec.vfs_path), planted_bug_notice(spec.vfs_path)];
        for notice in notices.into_iter().flatten() {
            let faults = unplain(&notice);
            if !faults.is_empty() {
                found.push(format!("the notice opening {} has {}: {notice:?}", spec.vfs_path, faults.join(", ")));
            }
        }
    }
    assert!(found.is_empty(), "the Studio says {} things in punctuation the site keeps off its pages:\n{}", found.len(), found.join("\n"));
}

// ---------------------------------------------------------------------------------------------
// A frame is the Studio's workbench
// ---------------------------------------------------------------------------------------------

/// The test's own site, so a frame renders before any page of the app draws one: one page that draws a
/// frame over the front page's shelf, opened on the file its query names.
#[derive(Routable, Clone, PartialEq)]
enum Harness {
    #[route("/?:opens")]
    FramePage { opens: String },
}

#[component]
fn FramePage(opens: String) -> Element {
    let over = use_hook(|| -> &'static Shelf { Box::leak(Box::new(Shelf { opens: Box::leak(opens.into_boxed_str()), files: shelf().files })) });
    let state = use_shelf(over);
    rsx! {
        StudioFrame { state }
    }
}

fn harness() -> Element {
    rsx! {
        Router::<Harness> {}
    }
}

/// The server render of a frame over the shelf, opened on `opens`.
fn frame_on(opens: &str) -> String {
    let mut vdom = VirtualDom::new(harness);
    vdom.provide_root_context(Rc::new(MemoryHistory::with_initial_path(format!("/?opens={opens}"))) as Rc<dyn History>);
    vdom.rebuild_in_place();
    dioxus_ssr::render(&vdom)
}

/// The Studio page opened on `path`.
fn studio_on(path: &str) -> String {
    render_route(&studio_file_url(path))
}

fn front_page() -> &'static str {
    static HTML: OnceLock<String> = OnceLock::new();
    HTML.get_or_init(|| render_route("/"))
}

/// The element of `html` named `name` whose own class list holds `class`.
fn element_with_class<'a>(html: &'a str, name: &str, class: &str) -> &'a str {
    elements(html, name).into_iter().find(|element| has_class(&own_attributes(element), class)).unwrap_or_else(|| panic!("the page draws no {name}.{class}"))
}

/// What the skeleton reader does with an element.
#[derive(PartialEq)]
enum Read {
    /// The element and everything in it.
    Whole,
    /// The element, but not what it holds.
    Shallow,
    /// Neither.
    Skip,
}

const VOID: &[&str] = &["area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source", "track", "wbr"];
const RAW_TEXT: &[&str] = &["script", "style", "textarea"];

/// The element skeleton of `html`: each element's depth, its name and its classes, in document order;
/// text, attributes other than `class` and the content of raw-text elements left out. `read` decides
/// what of each element is read, from its attributes and the markup from its start tag on. The panel
/// row is a `main` on the Studio page and a `div` elsewhere, and reads as one element either way.
fn skeleton(html: &str, read: &dyn Fn(&BTreeMap<String, String>, &str) -> Read) -> Vec<String> {
    let mut out = Vec::new();
    let (mut depth, mut skipping): (usize, Option<usize>) = (0, None);
    let mut at = 0;
    while let Some(lt) = html[at..].find('<').map(|i| at + i) {
        let rest = &html[lt..];
        if rest.starts_with("<!--") {
            at = lt + rest.find("-->").map_or(rest.len(), |e| e + 3);
            continue;
        }
        let end = lt + rest.find('>').expect("a closed tag");
        let tag = &html[lt..=end];
        at = end + 1;
        if tag.starts_with("</") {
            depth = depth.saturating_sub(1);
            if skipping == Some(depth) {
                skipping = None;
            }
            continue;
        }
        let name: String = tag[1..].chars().take_while(|c| c.is_ascii_alphanumeric() || *c == '-').collect();
        let attrs = attributes(tag);
        let opens = !(VOID.contains(&name.as_str()) || tag.ends_with("/>"));
        if RAW_TEXT.contains(&name.as_str()) && opens {
            let close = format!("</{name}>");
            at = html[at..].find(&close).map_or(html.len(), |i| at + i);
        }
        let here = depth;
        if opens {
            depth += 1;
        }
        if skipping.is_some() {
            continue;
        }
        let verdict = if name == "style" { Read::Skip } else { read(&attrs, &html[lt..]) };
        if verdict == Read::Skip {
            if opens {
                skipping = Some(here);
            }
            continue;
        }
        let mut classes: Vec<&str> = attrs.get("class").map(|c| c.split_whitespace().collect()).unwrap_or_default();
        classes.sort();
        let name = if name == "main" && classes.contains(&"studio-main") { "div".to_string() } else { name };
        out.push(format!("{here} {name}{}", classes.iter().map(|c| format!(".{c}")).collect::<String>()));
        if verdict == Read::Shallow && opens {
            skipping = Some(here);
        }
    }
    out
}

/// What the Studio page and a frame draw differently by design, each with why.
const DRAWN_BY_ONE_HOST: &[(&str, &str)] = &[
    ("main-nav", "the Studio page's nav; the front page has its own"),
    ("mobile-drawer-overlay", "the nav's phone menu"),
    ("mobile-drawer", "the nav's phone menu"),
    ("file-browser-btn", "New file writes the reader's store, which only the Studio page opens"),
    ("file-browser-note", "the reader's store's note, which only the Studio page opens"),
    ("studio-open", "the frame's link that opens its file in the Studio"),
];

/// The Debug button, which opens the Studio page's debugger: a toolbar button like the others, known by
/// its label.
fn is_debug_button(element: &str) -> bool {
    element.starts_with("<button") && element[..element.find("</button>").unwrap_or(0)].contains(">Debug</span>")
}

/// The skeleton reader for comparing the two hosts: what one host draws by design left out, the Files
/// tree read as one element, since its rows are compared on their own
/// (`a_frames_tree_is_the_studios_tree_cut_to_its_shelf`), and the touch line of withheld actions
/// read as one, since it names the Debug button only the Studio page has.
fn read_for_comparison(attrs: &BTreeMap<String, String>, element: &str) -> Read {
    if DRAWN_BY_ONE_HOST.iter().any(|(class, _)| has_class(attrs, class)) || is_debug_button(element) {
        return Read::Skip;
    }
    if has_class(attrs, "file-tree") || has_class(attrs, "studio-reasons") {
        return Read::Shallow;
    }
    Read::Whole
}

#[test]
fn the_skeleton_reads_names_classes_and_depth_and_nothing_else() {
    let html = r#"<div class="studio-content b a" style="--x: 1"><!--#--><main class="studio-main"><textarea class="t">x < y</textarea><span>text</span></main><br><div class="file-tree"><div class="row"></div></div><div class="file-browser-note"><i></i></div><style>.q { }</style><button class="execute-btn"><span><svg></svg></span><span class="desktop-label">Debug</span></button><button class="execute-btn"><span>Run</span></button></div>"#;
    assert_eq!(
        skeleton(html, &read_for_comparison),
        ["0 div.a.b.studio-content", "1 div.studio-main", "2 textarea.t", "2 span", "1 br", "1 div.file-tree", "1 button.execute-btn", "2 span"],
    );
}

/// The skeleton of the workbench a page draws: a frame's, or the one inside the Studio page's container.
fn workbench_skeleton(page: &str) -> Vec<String> {
    let workbench = if page.contains(" id=\"studio-frame\"") { element_by_id(page, "studio-frame") } else { element_with_class(page, "div", "studio-container") };
    skeleton(inner(workbench), &read_for_comparison)
}

/// Where a frame's workbench parts from the Studio page's, if anywhere.
fn workbench_difference(frame: &str, studio: &str) -> Option<String> {
    let (a, b) = (workbench_skeleton(frame), workbench_skeleton(studio));
    if b.len() < 40 {
        return Some(format!("the Studio's skeleton is only {} elements, so the comparison proves little", b.len()));
    }
    let at = a.iter().zip(&b).position(|(x, y)| x != y).or((a.len() != b.len()).then(|| a.len().min(b.len())))?;
    let around = |s: &[String]| s[at.saturating_sub(3)..(at + 4).min(s.len())].join("\n    ");
    Some(format!("they part at element {at} of {} and {}:\n  frame\n    {}\n  /studio\n    {}", a.len(), b.len(), around(&a), around(&b)))
}

/// A frame draws the Studio's own workbench: element for element and class for class, the skeleton of
/// a frame opened on each of the shelf's files is the skeleton of the Studio page opened on it, less
/// what one host draws by design (`DRAWN_BY_ONE_HOST`).
#[test]
fn a_frame_draws_the_studios_workbench_on_every_shelf_file() {
    let parted: Vec<String> = shelf()
        .files
        .iter()
        .filter_map(|file| workbench_difference(&frame_on(file), &studio_on(file)).map(|why| format!("{file}: {why}")))
        .collect();
    assert!(parted.is_empty(), "a frame is not the Studio's workbench:\n{}", parted.join("\n"));
}

/// The front page's frame is the Studio's workbench, opened on the shelf's first file.
#[test]
fn the_front_page_draws_the_studios_workbench() {
    if let Some(why) = workbench_difference(front_page(), &studio_on(shelf().opens)) {
        panic!("the front page's frame is not the Studio's workbench; {why}");
    }
}

/// The Files tree's rows, each as its path (the row's tooltip), its depth and whether it is the open
/// file.
fn tree_rows(html: &str) -> Vec<(String, String, bool)> {
    html.split("class=\"tree-item file-tree-item")
        .skip(1)
        .map(|tail| {
            let tag = &tail[..tail.find('>').expect("an unclosed row")];
            let classes = &tag[..tag.find('"').expect("an unclosed class")];
            let attrs = attributes(&format!("<div {}>", &tag[tag.find('"').unwrap() + 1..]));
            let depth = attrs.get("style").and_then(|style| style.split(';').find(|d| d.trim_start().starts_with("padding-left"))).unwrap_or_default().trim().to_string();
            (attrs.get("title").cloned().unwrap_or_default(), depth, classes.split_whitespace().any(|c| c == "selected"))
        })
        .collect()
}

/// Where a frame's tree, opened on `opens`, parts from the Studio's tree cut to the shelf: the shelf's
/// files and the folders they sit in, in the Studio's order, at the Studio's depths, `opens` selected
/// in both.
fn tree_difference(frame: &str, studio: &str, opens: &str) -> Option<String> {
    let files = shelf().files;
    let keep = |path: &str| files.iter().any(|file| *file == path || file.starts_with(&format!("{path}/")));
    let frame_rows = tree_rows(frame);
    let studio_rows: Vec<_> = tree_rows(studio).into_iter().filter(|(path, _, _)| keep(path)).collect();
    let selected: Vec<&str> = frame_rows.iter().filter(|(_, _, selected)| *selected).map(|(path, _, _)| path.as_str()).collect();
    if studio_rows.len() < files.len() {
        Some(format!("the Studio's tree lacks some of the shelf's files: {studio_rows:?}"))
    } else if frame_rows != studio_rows {
        Some(format!("the frame's rows\n    {frame_rows:?}\n  are not the Studio's cut to the shelf\n    {studio_rows:?}"))
    } else if selected != [opens] {
        Some(format!("the frame selects {selected:?}, not {opens}"))
    } else {
        None
    }
}

#[test]
fn a_frames_tree_is_the_studios_tree_cut_to_its_shelf() {
    let parted: Vec<String> = shelf()
        .files
        .iter()
        .filter_map(|file| tree_difference(&frame_on(file), &studio_on(file), file).map(|why| format!("{file}: {why}")))
        .collect();
    assert!(parted.is_empty(), "a frame's tree is not the Studio's:\n{}", parted.join("\n"));
}

#[test]
fn the_front_page_tree_is_the_studios_tree_cut_to_its_shelf() {
    let opens = shelf().opens;
    if let Some(why) = tree_difference(element_by_id(front_page(), "studio-frame"), &studio_on(opens), opens) {
        panic!("the front page's tree is not the Studio's: {why}");
    }
}

/// The panel row of a page: a `main` on the Studio page, a `div` in a frame.
fn panel_row(page: &str) -> &str {
    let name = if page.contains(" id=\"studio-frame\"") { "div" } else { "main" };
    element_with_class(page, name, "studio-main")
}

/// Every shelf file opens in a frame as it opens in the Studio: its panel row reads the same, every
/// title, editor line and output the Studio's first frame shows, and the file's text is in it.
#[test]
fn every_shelf_file_opens_in_the_frame_as_it_opens_in_the_studio() {
    assert_eq!(shelf().files.first(), Some(&shelf().opens), "the shelf opens on its first file");
    let mut parted = Vec::new();
    for file in shelf().files {
        let source = seeded_examples().find(|spec| spec.vfs_path == *file).unwrap_or_else(|| panic!("{file} is not a shipped example")).source;
        let (frame, studio) = (frame_on(file), studio_on(file));
        let (frame_text, studio_text) = (visible_text(panel_row(&frame)), visible_text(panel_row(&studio)));
        let first_line = source.lines().map(|line| line.split_whitespace().collect::<Vec<_>>().join(" ")).find(|line| !line.is_empty()).expect("an example with text");
        if frame_text != studio_text {
            let at = frame_text.chars().zip(studio_text.chars()).position(|(a, b)| a != b).unwrap_or(frame_text.len().min(studio_text.len()));
            let around = |t: &str| t.chars().skip(at.saturating_sub(60)).take(120).collect::<String>();
            parted.push(format!("{file} reads differently from character {at}:\n  frame   {:?}\n  /studio {:?}", around(&frame_text), around(&studio_text)));
        } else if !frame_text.contains(&first_line) {
            parted.push(format!("{file}: the frame's editor does not show its first line {first_line:?}"));
        }
    }
    assert!(parted.is_empty(), "a shelf file opens differently in the frame:\n{}", parted.join("\n"));
}

/// A frame offers no New file and no toolbar Debug button, and leads into the Studio on the file it
/// shows. Its column bars drag as the Studio page's do, and its dock bar offers what the page's does
/// (the user, 2026-11-01: parity, so the Studio is portable).
#[test]
fn a_frame_leads_into_the_studio_and_keeps_the_pages_tools_to_the_page() {
    for file in shelf().files {
        let frame = frame_on(file);
        let workbench = element_by_id(&frame, "studio-frame");
        let into: Vec<_> = links(workbench).into_iter().filter(|(attrs, _)| has_class(attrs, "studio-open")).collect();
        assert_eq!(into.len(), 1, "{file}: the frame draws {} links into the Studio", into.len());
        assert_eq!(into[0].0.get("href"), Some(&studio_file_url(file)), "{file}: the link into the Studio does not open the file");
        assert_eq!(into[0].1, "Open in Studio", "{file}: the link into the Studio");
        for class in ["file-browser-btn", "file-browser-note", "dbg-drawer"] {
            assert!(!classes_in(workbench).contains(class), "{file}: the frame draws the Studio page's {class}");
        }
        let toolbar = elements(workbench, "div").into_iter().find(|d| has_class(&own_attributes(d), "studio-toolbar")).expect("the frame has a toolbar");
        assert!(!visible_text(toolbar).contains("Debug"), "{file}: the frame's toolbar offers Debug");
        assert!(classes_in(workbench).contains("panel-resizer"), "{file}: the frame has no column bars to drag");
    }
}

/// The selectors of every rule in `css`, at every depth of its `@media` blocks.
fn selectors(css: &str) -> Vec<String> {
    let mut css = css.to_string();
    while let Some(at) = css.find("/*") {
        let end = css[at..].find("*/").map_or(css.len(), |e| at + e + 2);
        css.replace_range(at..end, "");
    }
    let mut found = Vec::new();
    let mut rest = css.as_str();
    while let Some(open) = rest.find('{') {
        let head = rest[..open].rsplit(['}', ';']).next().unwrap_or("").trim();
        if !head.starts_with('@') && !head.is_empty() {
            found.extend(head.split(',').map(|s| s.split_whitespace().collect::<Vec<_>>().join(" ")));
        }
        rest = &rest[open + 1..];
    }
    found
}

#[test]
fn the_selector_reader_reads_every_rule_and_no_at_rule() {
    let css = "/* a { } */ .x .y, .z { color: red; }\n@media (max-width: 9px) { .x > .w { } .v{} }\n";
    assert_eq!(selectors(css), [".x .y", ".z", ".x > .w", ".v"]);
}

/// The frame's stylesheet reaches only inside the frame: every selector starts at `.studio-frame`, and
/// it defines no token of the site's palette and not the Studio's title type.
#[test]
fn the_frame_css_stays_inside_the_frame() {
    let found = selectors(FRAME_STYLE);
    assert!(found.len() >= 5, "FRAME_STYLE holds only {} selectors", found.len());
    for selector in &found {
        assert!(selector == ".studio-frame" || selector.starts_with(".studio-frame "), "FRAME_STYLE reaches outside the frame: {selector:?}");
    }
    for token in ["--bg-gradient-start", "--accent-primary", "--accent-secondary", "--accent-border", "--accent-glow", "--studio-bg", "--studio-text"] {
        assert!(!FRAME_STYLE.contains(&format!("{token}:")), "FRAME_STYLE defines the palette token {token}");
    }
    assert!(!FRAME_STYLE.contains("letter-spacing: 0.3px"), "FRAME_STYLE declares the Studio's title type a second time");
}

/// The marker the Studio's stylesheet is counted by.
const STUDIO_STYLE_MARKER: &str = "STUDIO PAGE - Design Tokens";

#[test]
fn a_frame_carries_the_studios_stylesheet_once() {
    assert!(STUDIO_STYLE.contains(STUDIO_STYLE_MARKER), "the Studio's stylesheet lost the marker this lock counts it by");
    let frame = frame_on(shelf().opens);
    assert_eq!(frame.matches(STUDIO_STYLE_MARKER).count(), 1, "a frame carries the Studio's stylesheet {} times", frame.matches(STUDIO_STYLE_MARKER).count());
    assert_eq!(frame.matches("\n.studio-frame {").count(), 1, "a frame carries its own stylesheet {} times", frame.matches("\n.studio-frame {").count());
}

#[test]
fn the_front_page_carries_the_studios_stylesheet_once() {
    let count = front_page().matches(STUDIO_STYLE_MARKER).count();
    assert_eq!(count, 1, "the front page carries the Studio's stylesheet {count} times");
}

/// Every class an element in `html` carries, the content of scripts and stylesheets left out.
fn classes_in(html: &str) -> BTreeSet<String> {
    let html = without_raw_text(html);
    let mut found = BTreeSet::new();
    let mut at = 0;
    while let Some(lt) = html[at..].find('<').map(|i| at + i) {
        let Some(end) = html[lt..].find('>').map(|e| lt + e) else { break };
        let tag = &html[lt..=end];
        at = end + 1;
        if tag.starts_with("</") || tag.starts_with("<!") {
            continue;
        }
        if let Some(classes) = attributes(tag).get("class") {
            found.extend(classes.split_whitespace().map(str::to_string));
        }
    }
    found
}

/// The classes `css` styles: every class its selectors name, less the state classes it styles only
/// beside a class of its own (`.output-mode-btn.active`), which cannot reach an element alone.
fn styled_classes(css: &str) -> BTreeSet<String> {
    const STATES: &[&str] = &["active", "open", "selected", "sized", "proved", "refuted", "wide", "last", "dragging", "mobile-expanded", "mobile-collapsed"];
    selectors(css)
        .iter()
        .flat_map(|selector| {
            selector
                .split(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.'))
                .filter_map(|part| part.strip_prefix('.'))
                .flat_map(|classes| classes.split('.').map(str::to_string).collect::<Vec<_>>())
                .collect::<Vec<_>>()
        })
        .filter(|class| !STATES.contains(&class.as_str()))
        .collect()
}

#[test]
fn the_class_readers_read_markup_and_selectors() {
    assert_eq!(classes_in(r#"<div class="a b"><script>if (x<y) { q = "<i class='no'>" }</script><span class="c"></span></div>"#), BTreeSet::from(["a".into(), "b".into(), "c".into()]));
    assert_eq!(styled_classes(".a.active > .b:hover, .c .d.selected { }"), BTreeSet::from(["a".into(), "b".into(), "c".into(), "d".into()]));
}

/// The Studio's stylesheet is global, so no class the front page uses outside its frame may be one it
/// styles.
#[test]
fn no_front_page_class_is_one_the_studio_css_styles() {
    let page = front_page();
    let outside = page.replacen(element_by_id(page, "studio-frame"), "", 1);
    let collide: Vec<String> = classes_in(&outside).intersection(&styled_classes(STUDIO_STYLE)).cloned().collect();
    assert!(collide.is_empty(), "the front page uses outside its frame classes the Studio's stylesheet styles: {collide:?}");
}

/// The front page draws no Studio of its own: its sources name no Studio, panel or mini Studio class,
/// and draw none of the workbench's.
#[test]
fn the_front_page_draws_no_studio_of_its_own() {
    let mut found = Vec::new();
    for path in sources(&web_ui().join("pages/landing")) {
        let source = std::fs::read_to_string(&path).unwrap();
        let drawn = drawn_classes(&source);
        if !drawn.is_empty() {
            found.push(format!("{} draws {}", relative(&path), drawn.join(", ")));
        }
        for literal in literals(&code_of(&source)) {
            if literal.split_whitespace().any(|class| ["studio-", "panel-", "mini-"].iter().any(|prefix| class.starts_with(prefix))) {
                found.push(format!("{} names {literal:?}", relative(&path)));
            }
        }
    }
    assert!(found.is_empty(), "the front page draws a Studio of its own:\n{}", found.join("\n"));
}

// ---------------------------------------------------------------------------------------------
// The Files tree takes the keyboard
// ---------------------------------------------------------------------------------------------

/// Where a page's trees part from the WAI-ARIA tree pattern: each `[data-tree]` a named `tree`, each
/// row a `treeitem` at its level, selected or not, an open or closed node saying which, each open
/// node's parts a `group`, and one tab stop, on the selected row when one is drawn and else on the
/// first.
fn aria_tree_faults(html: &str) -> Vec<String> {
    let mut faults = Vec::new();
    let trees: Vec<&str> = elements(html, "div").into_iter().filter(|div| own_attributes(div).contains_key("data-tree")).collect();
    if trees.is_empty() {
        faults.push("the page draws no tree".to_string());
    }
    for tree in trees {
        let attrs = own_attributes(tree);
        let kind = attrs.get("data-tree").cloned().unwrap_or_default();
        if attrs.get("role").map(String::as_str) != Some("tree") || attrs.get("aria-label").is_none_or(|l| l.is_empty()) {
            faults.push(format!("the {kind} is not a named tree: {attrs:?}"));
        }
        let mut depth = None;
        let mut rows = Vec::new();
        let mut at = 0;
        while let Some(lt) = tree[at..].find('<').map(|i| at + i) {
            let end = lt + tree[lt..].find('>').unwrap();
            let tag = &tree[lt..=end];
            at = end + 1;
            let a = attributes(tag);
            if has_class(&a, "tree-node") {
                depth = a.get("style").and_then(|s| s.split("--depth:").nth(1)).and_then(|d| d.trim().trim_end_matches(';').trim().parse::<usize>().ok());
            } else if has_class(&a, "tree-children") && a.get("role").map(String::as_str) != Some("group") {
                faults.push(format!("the {kind} has a node whose parts are not a group: {tag}"));
            } else if has_class(&a, "tree-item") {
                let body = &tree[at..tree[at..].find("class=\"tree-item").map_or(tree.len(), |i| at + i)];
                rows.push((a, depth, body.contains("class=\"twistie\""), body.contains("class=\"chevron open\"")));
            }
        }
        for (a, depth, expandable, open) in &rows {
            let name = a.get("title").cloned().unwrap_or_else(|| format!("{a:?}"));
            let want = |key: &str, value: &str| a.get(key).map(String::as_str) == Some(value);
            if !want("role", "treeitem") {
                faults.push(format!("{kind} row {name} is not a treeitem"));
            }
            if !depth.is_some_and(|d| want("aria-level", &(d + 1).to_string())) {
                faults.push(format!("{kind} row {name} is at depth {depth:?} but says aria-level {:?}", a.get("aria-level")));
            }
            if !want("aria-selected", if has_class(a, "selected") { "true" } else { "false" }) {
                faults.push(format!("{kind} row {name} says aria-selected {:?}", a.get("aria-selected")));
            }
            match (expandable, a.get("aria-expanded").map(String::as_str)) {
                (false, None) => {}
                (true, Some(said)) if said == if *open { "true" } else { "false" } => {}
                (_, said) => faults.push(format!("{kind} row {name} (expandable {expandable}, open {open}) says aria-expanded {said:?}")),
            }
        }
        let stops: Vec<usize> = rows.iter().enumerate().filter(|(_, (a, ..))| a.get("tabindex").map(String::as_str) == Some("0")).map(|(i, _)| i).collect();
        let selected = rows.iter().position(|(a, ..)| has_class(a, "selected"));
        if rows.iter().any(|(a, ..)| !matches!(a.get("tabindex").map(String::as_str), Some("0" | "-1"))) {
            faults.push(format!("a {kind} row is not focusable"));
        }
        if !rows.is_empty() && stops != vec![selected.unwrap_or(0)] {
            faults.push(format!("the {kind}'s tab stops are rows {stops:?}, not row {} (selected {selected:?})", selected.unwrap_or(0)));
        }
    }
    faults
}

/// The Files tree is an ARIA tree with one tab stop on the Studio page, on a deep link in every mode
/// and on a reader's file the server cannot see (so no row is selected), and in a frame on every shelf
/// file.
#[test]
fn every_files_tree_is_an_aria_tree_with_one_tab_stop() {
    let mut pages: Vec<(String, String)> = ["/studio", "/studio?file=examples/logic/simple-sentences.logic", "/studio?file=examples/hardware/arbiter.v", "/studio?file=workspace/untitled.logos"]
        .iter()
        .map(|route| (route.to_string(), render_route(route)))
        .collect();
    pages.extend(shelf().files.iter().map(|file| (format!("a frame on {file}"), frame_on(file))));
    let faults: Vec<String> = pages.iter().flat_map(|(page, html)| aria_tree_faults(html).into_iter().map(move |f| format!("{page}: {f}"))).collect();
    assert!(faults.is_empty(), "a tree parts from the ARIA tree pattern:\n{}", faults.join("\n"));
}

// ---------------------------------------------------------------------------------------------
// The workbench's media queries are named
// ---------------------------------------------------------------------------------------------

/// Every media query a stylesheet in `source` opens: the condition of each `@media` rule, split at its
/// top-level commas, with its whitespace collapsed.
fn media_queries(source: &str) -> Vec<String> {
    source
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix("@media"))
        .flat_map(|rest| {
            let condition = rest.split('{').next().unwrap_or("").trim();
            let mut parts = Vec::new();
            let (mut depth, mut start) = (0i32, 0usize);
            for (i, c) in condition.char_indices() {
                match c {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    ',' if depth == 0 => {
                        parts.push(condition[start..i].to_string());
                        start = i + 1;
                    }
                    _ => {}
                }
            }
            parts.push(condition[start..].to_string());
            parts.into_iter().map(|p| p.split_whitespace().collect::<Vec<_>>().join(" ")).collect::<Vec<_>>()
        })
        .collect()
}

#[test]
fn the_media_reader_splits_a_query_list_at_its_commas() {
    let css = "  @media (max-width: 768px), (min-width: 769px) and (pointer: coarse) {\n.a{}\n}\n// @media in a comment\n@media  (max-height:  500px) and (orientation: landscape) {";
    assert_eq!(media_queries(css), ["(max-width: 768px)", "(min-width: 769px) and (pointer: coarse)", "(max-height: 500px) and (orientation: landscape)"]);
}

/// Every `@media` in every stylesheet the Studio draws from is one of the named queries
/// (`responsive::media::ALL`), so a band is one query wherever it is written and the app's own
/// `matchMedia` checks read the same words.
#[test]
fn the_workbench_uses_only_named_media_queries() {
    use logicaffeine_web::ui::responsive::media;
    let mut unnamed = Vec::new();
    let mut seen = 0;
    for path in the_studios_sources().iter().chain([web_ui().join("responsive.rs")].iter()) {
        for query in media_queries(&std::fs::read_to_string(path).unwrap()) {
            seen += 1;
            if !media::ALL.contains(&query.as_str()) {
                unnamed.push(format!("{}: @media {query}", relative(path)));
            }
        }
    }
    assert!(seen >= 20, "the Studio's stylesheets open only {seen} media queries, so the lock reads too little");
    unnamed.sort();
    unnamed.dedup();
    assert!(unnamed.is_empty(), "media queries that are not named in responsive::media:\n{}", unnamed.join("\n"));
}

// ---------------------------------------------------------------------------------------------
// A phone shows one pane, chosen by the switch
// ---------------------------------------------------------------------------------------------

/// Where a page's phone switch parts from the pane it should open on: the switch is a tab list over
/// the panes, one tab selected, and one pane marked as the phone's, the one that tab names.
fn phone_pane_faults(html: &str, want: &str) -> Vec<String> {
    let mut faults = Vec::new();
    let switches: Vec<_> = tags(html, "nav").into_iter().filter(|nav| has_class(nav, "mobile-tabs")).collect();
    if switches.len() != 1 {
        faults.push(format!("{} phone switches", switches.len()));
        return faults;
    }
    if switches[0].get("role").map(String::as_str) != Some("tablist") || switches[0].get("aria-label").is_none_or(|l| l.is_empty()) {
        faults.push(format!("the switch is not a named tab list: {:?}", switches[0]));
    }
    let tabs: Vec<_> = tags(html, "button").into_iter().filter(|b| has_class(b, "mobile-tab")).collect();
    if tabs.iter().any(|tab| tab.get("role").map(String::as_str) != Some("tab")) {
        faults.push("a tab of the switch is not a tab".to_string());
    }
    let selected: Vec<_> = tabs.iter().filter(|tab| tab.get("aria-selected").map(String::as_str) == Some("true")).collect();
    if selected.len() != 1 || !has_class(selected[0], "active") {
        faults.push(format!("{} tabs selected", selected.len()));
    }
    let panes: Vec<String> = ["section", "aside"]
        .iter()
        .flat_map(|name| tags(html, name))
        .filter(|pane| has_class(pane, "studio-panel") && has_class(pane, "phone-shown"))
        .map(|pane| ["col-left", "col-center", "col-right"].into_iter().find(|c| has_class(&pane, c)).unwrap_or("?").to_string())
        .collect();
    if panes != [want] {
        faults.push(format!("the phone shows {panes:?}, not {want}"));
    }
    faults
}

/// A phone opens on the pane that shows the file, from the server's render on (`first_pane`): the
/// output for a file that runs or previews as it opens, the editor for Math. On the Studio page in
/// every mode, and in a frame on every shelf file.
#[test]
fn a_phone_opens_on_the_pane_that_shows_the_file_from_the_first_frame() {
    let pane = |path: &str| if path.starts_with("/examples/math/") { "col-left" } else { "col-center" };
    let mut faults = Vec::new();
    for path in ["/examples/logic/simple-sentences.logic", "/examples/code/hello-world.logos", "/examples/math/auto-tactic.logos", "/examples/hardware/handshake.hw", "/examples/code/web/counter.logos"] {
        faults.extend(phone_pane_faults(&studio_on(path), pane(path)).into_iter().map(|f| format!("/studio on {path}: {f}")));
    }
    for file in shelf().files {
        faults.extend(phone_pane_faults(&frame_on(file), pane(file)).into_iter().map(|f| format!("a frame on {file}: {f}")));
    }
    assert!(faults.is_empty(), "a phone opens on the wrong pane:\n{}", faults.join("\n"));
}

/// Android's keyboard resizes the page's layout rather than sliding over it (D18 of the homepage plan),
/// so a phone's editor keeps its caret line above the keyboard; iOS ignores the setting, and the
/// workbench reads its visual viewport instead (`use_keyboard_inset`).
#[test]
fn the_site_lays_itself_out_above_an_on_screen_keyboard() {
    let shell = std::fs::read_to_string(web_ui().join("../../index.html")).unwrap();
    let viewport = tags(&shell, "meta").into_iter().filter(|meta| meta.get("name").map(String::as_str) == Some("viewport")).collect::<Vec<_>>();
    assert_eq!(viewport.len(), 1, "the shell carries {} viewport metas", viewport.len());
    let content = viewport[0].get("content").cloned().unwrap_or_default();
    let parts: Vec<&str> = content.split(',').map(str::trim).collect();
    for want in ["width=device-width", "initial-scale=1.0", "interactive-widget=resizes-content"] {
        assert!(parts.contains(&want), "the viewport meta {content:?} lacks {want}");
    }
}

/// The bodies of the `@media` blocks in `css` whose query list is exactly `query`.
fn media_blocks<'a>(css: &'a str, query: &str) -> Vec<&'a str> {
    let opening = format!("@media {query} {{");
    css.match_indices(&opening)
        .map(|(at, _)| {
            let body = &css[at + opening.len()..];
            let mut depth = 1;
            let end = body
                .char_indices()
                .find(|(_, c)| {
                    match c {
                        '{' => depth += 1,
                        '}' => depth -= 1,
                        _ => {}
                    }
                    depth == 0
                })
                .map_or(body.len(), |(i, _)| i);
            &body[..end]
        })
        .collect()
}

/// On a phone the Files tree is folded from the server's render on (D5 of the homepage plan): the
/// render marks no drawer open, and the Studio's stylesheet folds an unopened drawer in every band
/// where the files are a drawer, so no tree covers the editor before the app takes over.
#[test]
fn a_phone_folds_the_files_from_the_server_render() {
    use logicaffeine_web::ui::responsive::media;
    let blocks = media_blocks(STUDIO_STYLE, &media::FILES_DRAWER.join(", "));
    let folds = blocks.iter().any(|block| {
        block.split('}').any(|rule| {
            let (selectors, body) = rule.split_once('{').unwrap_or(("", ""));
            selectors.split(',').any(|s| s.trim() == ".studio-sidebar:not(.drawer-open)") && body.contains("display: none")
        })
    });
    assert!(folds, "no rule under @media {} folds an unopened drawer", media::FILES_DRAWER.join(", "));
    let mut pages: Vec<(String, String)> = ["/examples/code/hello-world.logos", "/examples/logic/simple-sentences.logic"].iter().map(|p| (p.to_string(), studio_on(p))).collect();
    pages.push(("a frame".to_string(), frame_on(shelf().opens)));
    for (page, html) in pages {
        let drawers: Vec<_> = tags(&html, "div").into_iter().filter(|d| has_class(d, "studio-sidebar") || has_class(d, "sidebar-overlay")).collect();
        assert!(!drawers.is_empty(), "{page} renders no Files tree");
        assert!(drawers.iter().all(|d| !has_class(d, "drawer-open")), "{page} renders the drawer open");
    }
}

/// The most the server's render of the front page may weigh, now that it carries the Studio's
/// stylesheet and the workbench's markup for its shelf. It measured 719,062 bytes on 2026-10-16,
/// before the comparison and the library joined the page; most of it is the website's painted
/// preview, which carries the page it builds. This keeps it from quietly growing.
const FRONT_PAGE_BUDGET_BYTES: usize = 800 * 1024;

#[test]
fn the_prerendered_front_page_stays_within_its_budget() {
    let html = front_page();
    eprintln!("PRERENDER_BYTES / = {}", html.len());
    assert!(html.len() <= FRONT_PAGE_BUDGET_BYTES, "the prerendered front page weighs {} bytes, over its {FRONT_PAGE_BUDGET_BYTES}", html.len());
}
