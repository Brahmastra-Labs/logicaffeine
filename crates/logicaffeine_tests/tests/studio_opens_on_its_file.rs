//! The Studio's first frame is the state it ends up in.
//!
//! The Studio has no mode switch: the open file is the mode. Its first frame used to be Logic
//! with an empty editor, whatever the route asked for, because only the browser's file system
//! could open a file; a moment later the page flipped to the file's mode. Every shipped example
//! is compiled into the site, so the route alone now decides what the Studio opens on, the same
//! on the server and in the browser. This pins the server's side, which is the page the site
//! prerenders: a bare `/studio` is Code with the first Code example in the editor, and a deep
//! link to an example is that example's mode with its text.

#![cfg(feature = "web-tests")]

mod common;

use dioxus::prelude::*;
use dioxus_history::{History, MemoryHistory};
use logicaffeine_web::ui::examples::{ExampleSpec, ALL_CODE_EXAMPLES, ALL_HARDWARE_SPECS, ALL_LOGIC_EXAMPLES, ALL_MATH_EXAMPLES};
use logicaffeine_web::ui::router::studio_file_url;
use logicaffeine_web::ui::state::StudioMode;
use logicaffeine_web::App;
use std::rc::Rc;

/// The first Code example the file tree shows (the user, 2026-10-01: "lets start it off on code
/// mode since its the first example").
const FIRST_CODE_EXAMPLE: &str = "/examples/code/basics/control-flow.logos";

fn render_route(path: &str) -> String {
    let mut vdom = VirtualDom::new(App);
    vdom.provide_root_context(Rc::new(MemoryHistory::with_initial_path(path)) as Rc<dyn History>);
    vdom.rebuild_in_place();
    dioxus_ssr::render(&vdom)
}

fn every_example() -> impl Iterator<Item = &'static ExampleSpec> {
    ALL_LOGIC_EXAMPLES.iter().chain(ALL_CODE_EXAMPLES).chain(ALL_MATH_EXAMPLES).chain(ALL_HARDWARE_SPECS)
}

fn example(path: &str) -> &'static ExampleSpec {
    every_example().find(|spec| spec.vfs_path == path).unwrap_or_else(|| panic!("no shipped example at {path}"))
}

/// The panel titles each mode draws, in order.
fn titles_of(mode: StudioMode) -> &'static [&'static str] {
    match mode {
        StudioMode::Logic => &["English Input", "Logic Output"],
        StudioMode::Code => &["Code Editor", "Output"],
        StudioMode::Math => &["Theorem Editor", "Output", "Context"],
        StudioMode::Hardware => &["Hardware Spec", "SVA"],
        StudioMode::Web => &["Code Editor", "Preview"],
    }
}

/// The mode the file browser opens a file in once its text is read.
fn opened_by_the_file_browser(spec: &ExampleSpec) -> StudioMode {
    match StudioMode::for_path(spec.vfs_path) {
        Some(StudioMode::Code) => StudioMode::for_logos_source(spec.source),
        other => other.expect("every example names its mode"),
    }
}

fn without_comments(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(at) = rest.find("<!--") {
        out.push_str(&rest[..at]);
        rest = match rest[at..].find("-->") {
            Some(end) => &rest[at + end + 3..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

fn without_tags(html: &str) -> String {
    let mut out = String::new();
    let mut inside = false;
    for c in html.chars() {
        match c {
            '<' => inside = true,
            '>' if inside => inside = false,
            c if !inside => out.push(c),
            _ => {}
        }
    }
    out
}

/// HTML text back to the characters it stands for.
fn decoded(text: &str) -> String {
    let mut out = String::new();
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        rest = &rest[at..];
        let Some(end) = rest.find(';').filter(|&end| end <= 10) else {
            out.push('&');
            rest = &rest[1..];
            continue;
        };
        let entity = &rest[1..end];
        let ch = match entity {
            "amp" => Some('&'),
            "lt" => Some('<'),
            "gt" => Some('>'),
            "quot" => Some('"'),
            "apos" => Some('\''),
            _ => entity
                .strip_prefix("#x")
                .or_else(|| entity.strip_prefix("#X"))
                .and_then(|hex| u32::from_str_radix(hex, 16).ok())
                .or_else(|| entity.strip_prefix('#').and_then(|dec| dec.parse().ok()))
                .and_then(char::from_u32),
        };
        match ch {
            Some(ch) => {
                out.push(ch);
                rest = &rest[end + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The page without its style sheets, whose comments may name any panel.
fn without_styles(html: &str) -> String {
    let mut out = String::new();
    let mut rest = html;
    while let Some(at) = rest.find("<style") {
        out.push_str(&rest[..at]);
        rest = match rest[at..].find("</style>") {
            Some(end) => &rest[at + end + "</style>".len()..],
            None => "",
        };
    }
    out.push_str(rest);
    out
}

/// The Studio's panel row: everything from its `main` on.
fn panel_row(html: &str) -> &str {
    let at = html.find("class=\"studio-main\"").expect("the Studio drew no panel row, so the checks below prove nothing");
    &html[at..]
}

/// The title of each panel the Studio draws, in order: the first `span` of every panel header.
fn panel_titles(html: &str) -> Vec<String> {
    panel_row(html)
        .split("class=\"panel-header\"")
        .skip(1)
        .map(|header| {
            let span = &header[header.find("<span").expect("a panel header with no title")..];
            let open = span.find('>').expect("an unclosed span") + 1;
            let close = span.find("</span>").expect("an unclosed span");
            decoded(&without_comments(&span[open..close])).trim().to_string()
        })
        .collect()
}

/// The text the first panel's editor shows: its highlighted copy of the source.
fn editor_text(html: &str) -> String {
    let row = panel_row(html);
    let at = row.find("class=\"code-editor-highlight\"").expect("the first panel draws no editor");
    let highlight = &row[at..];
    let open = highlight.find('>').expect("an unclosed editor") + 1;
    let close = highlight.find("</div>").expect("an unclosed editor");
    decoded(&without_tags(&without_comments(&highlight[open..close])))
}

#[test]
fn the_helpers_read_titles_and_editor_text_through_markup() {
    let html = r#"<main class="studio-main"><section><div class="panel-header"><span>Code <!--#-->Editor</span></div>
        <div class="code-editor-highlight"><span><span class="tok-keyword">Let</span> x &lt; <!--node-id3-->1 &amp;&#39;&#x41;</span></div></section>
        <section><div class="panel-header" data-x="1"><span><!--node-id9-->SVA<!--#--></span><div class="output-mode-toggle"><button>Rust</button></div></div></section></main>"#;
    assert_eq!(panel_titles(html), ["Code Editor", "SVA"]);
    assert_eq!(editor_text(html), "Let x < 1 &'A");
    let styled = r#"<style>/* "English Input" */ .a { color: red; }</style><main class="studio-main"><style>.b{}</style>Code Editor</main>"#;
    assert_eq!(without_styles(styled), r#"<main class="studio-main">Code Editor</main>"#);
}

#[test]
fn a_bare_studio_prerenders_code_mode() {
    let html = render_route("/studio");
    let titles = panel_titles(&html);
    assert_eq!(titles, titles_of(StudioMode::Code), "a bare /studio prerenders {titles:?}");
    assert!(!without_styles(&html).contains("English Input"), "a bare /studio prerenders Logic's English Input");
}

#[test]
fn a_bare_studio_prerenders_the_first_code_examples_text() {
    let html = render_route("/studio");
    let source = example(FIRST_CODE_EXAMPLE).source;
    let shown = editor_text(&html);
    let first_line = source.lines().next().expect("the first example has a first line");
    assert!(shown.starts_with(first_line), "a bare /studio's editor starts {:?}, not {first_line:?}", shown.lines().next());
    assert_eq!(shown, source, "a bare /studio's editor does not hold {FIRST_CODE_EXAMPLE}");
}

#[test]
fn a_logic_deep_link_prerenders_logic_with_its_text() {
    let path = "/examples/logic/quantifiers.logic";
    let html = render_route(&studio_file_url(path));
    assert_eq!(panel_titles(&html), titles_of(StudioMode::Logic), "{path}");
    assert_eq!(editor_text(&html), example(path).source, "{path} prerenders without its text");
    assert!(!without_styles(&html).contains("Code Editor"), "{path} prerenders Code's editor");
}

#[test]
fn a_page_deep_link_prerenders_web() {
    let path = "/examples/code/web/counter.logos";
    let html = render_route(&studio_file_url(path));
    assert_eq!(panel_titles(&html), titles_of(StudioMode::Web), "{path}");
    assert_eq!(editor_text(&html), example(path).source, "{path} prerenders without its text");
}

#[test]
fn a_deep_link_prerenders_its_files_mode_for_every_mode() {
    let mut wrong = Vec::new();
    for mode in StudioMode::ALL {
        let spec = every_example()
            .find(|spec| opened_by_the_file_browser(spec) == mode)
            .unwrap_or_else(|| panic!("no shipped example opens in {mode:?}"));
        let html = render_route(&studio_file_url(spec.vfs_path));
        let titles = panel_titles(&html);
        if titles != titles_of(mode) {
            wrong.push(format!("{} ({mode:?}) prerenders {titles:?}, not {:?}", spec.vfs_path, titles_of(mode)));
        }
        if editor_text(&html) != spec.source {
            wrong.push(format!("{} ({mode:?}) prerenders without its text", spec.vfs_path));
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
}

#[test]
fn a_readers_file_prerenders_its_paths_mode_with_an_empty_editor() {
    for (path, mode) in [
        ("/workspace/untitled.logos", StudioMode::Code),
        ("/workspace/untitled.logic", StudioMode::Logic),
        ("/workspace/spec.hw", StudioMode::Hardware),
    ] {
        let html = render_route(&studio_file_url(path));
        assert_eq!(panel_titles(&html), titles_of(mode), "{path}");
        assert_eq!(editor_text(&html), "", "{path} is not shipped; its text comes from the reader's file system");
    }
}

// ---------------------------------------------------------------------------------------------
// The file tree and New file on the first frame
// ---------------------------------------------------------------------------------------------

/// Each file-tree row the page draws: its path and whether it is the open file.
fn tree_rows(html: &str) -> Vec<(String, bool)> {
    html.split("class=\"tree-item file-tree-item")
        .skip(1)
        .map(|tail| {
            let tag = &tail[..tail.find('>').expect("an unclosed row")];
            let classes = &tag[..tag.find('"').expect("an unclosed class")];
            let title = tag.split("title=\"").nth(1).and_then(|t| t.split('"').next()).unwrap_or_default();
            (decoded(title), classes.split_whitespace().any(|c| c == "selected"))
        })
        .collect()
}

fn selected_rows(html: &str) -> Vec<String> {
    tree_rows(html).into_iter().filter(|(_, selected)| *selected).map(|(path, _)| path).collect()
}

#[test]
fn a_bare_studio_prerenders_the_examples_tree_with_its_file_selected() {
    let html = render_route("/studio");
    let rows: Vec<String> = tree_rows(&html).into_iter().map(|(path, _)| path).collect();
    let missing: Vec<&str> = every_example().map(|spec| spec.vfs_path).filter(|path| !rows.iter().any(|row| row == path)).collect();
    assert!(missing.is_empty(), "the prerendered tree is missing {} example(s), e.g. {:?}", missing.len(), &missing[..missing.len().min(3)]);
    assert_eq!(selected_rows(&html), [FIRST_CODE_EXAMPLE], "the prerendered tree does not show the open file");
}

#[test]
fn a_deep_link_prerenders_its_file_selected_in_the_tree() {
    let path = "/examples/logic/quantifiers.logic";
    let html = render_route(&studio_file_url(path));
    assert_eq!(selected_rows(&html), [path]);
}

#[test]
fn a_readers_file_prerenders_the_examples_and_selects_nothing_yet() {
    let html = render_route(&studio_file_url("/workspace/mine.logos"));
    assert!(tree_rows(&html).iter().any(|(path, _)| path == FIRST_CODE_EXAMPLE), "the examples are missing from the tree");
    assert!(selected_rows(&html).is_empty(), "a file the page has not read yet shows as open");
}

#[test]
fn new_file_waits_for_the_readers_files() {
    let html = render_route("/studio");
    let at = html.find("class=\"file-browser-btn\"").expect("the file browser draws no New file button");
    let tag = &html[html[..at].rfind('<').expect("a tag")..at + html[at..].find('>').expect("an unclosed tag")];
    assert!(tag.contains(" disabled"), "New file is offered before the reader's files have loaded: {tag}");
    assert!(tag.contains("title=\"Your files are still loading\""), "New file does not say why it waits: {tag}");
}

/// The most the server's render of a bare `/studio` may weigh. The prerendered page carries the
/// examples' part of the file tree (181 rows), so the reader sees it before the app has loaded. It
/// measured 200,263 bytes on 2026-10-02, and this keeps it from quietly growing.
const PRERENDER_BUDGET_BYTES: usize = 256 * 1024;

#[test]
fn the_prerendered_studio_stays_within_its_budget() {
    let html = render_route("/studio");
    let rows = tree_rows(&html).len();
    eprintln!("PRERENDER_BYTES /studio = {} ({rows} tree rows)", html.len());
    assert!(html.len() <= PRERENDER_BUDGET_BYTES, "the prerendered /studio weighs {} bytes, over its {PRERENDER_BUDGET_BYTES}", html.len());
}

// ---------------------------------------------------------------------------------------------
// The prerendered copy and deep links to other files
// ---------------------------------------------------------------------------------------------
//
// The site prerenders one `/studio` page and serves it for every deep link as well, so a deep link
// to another file is first painted from the bare page's copy. The copy reads the link's `file` the
// way the router does before it paints its panels, and keeps them unpainted when the link names
// another file. The browser layer (`a_logic_deep_link_shows_logic_from_the_first_frame`, run
// against a deploy build) proves the whole effect; these pin its parts in the prerendered page.

/// The class the guard sets on `<html>` when the link names a file the copy does not show.
const ELSEWHERE: &str = "studio-opens-elsewhere";

/// The body of the inline script that mentions `needle`.
fn script_with<'a>(html: &'a str, needle: &str) -> Option<&'a str> {
    html.split("<script").skip(1).find_map(|tail| {
        let body = &tail[tail.find('>')? + 1..tail.find("</script>")?];
        body.contains(needle).then_some(body)
    })
}

#[test]
fn the_prerendered_studio_checks_the_link_before_it_paints_its_panels() {
    let html = render_route("/studio");
    let guard = script_with(&html, ELSEWHERE).expect("the prerendered /studio carries no guard against deep links to other files");
    let opened = format!("\"{FIRST_CODE_EXAMPLE}\"");
    assert!(guard.contains(&opened), "the guard does not compare the link with the file the copy shows, {opened}: {guard}");
    for entity in ["&amp;", "&lt;", "&gt;", "&quot;", "&#"] {
        assert!(!guard.contains(entity), "the guard reaches the page HTML-escaped ({entity}), which breaks its JavaScript: {guard}");
    }
    let guard_at = html.find(guard).expect("the guard is in the page");
    let panels_at = html.find("class=\"studio-main\"").expect("the Studio drew no panels");
    assert!(guard_at < panels_at, "the guard runs after the panels have painted");
}

#[test]
fn a_deep_links_copy_guards_against_other_files_than_its_own() {
    let path = "/examples/logic/quantifiers.logic";
    let html = render_route(&studio_file_url(path));
    let guard = script_with(&html, ELSEWHERE).expect("a deep link's prerendered copy carries no guard");
    assert!(guard.contains(&format!("\"{path}\"")), "the guard names another file than the copy shows: {guard}");
}

#[test]
fn the_guards_rule_hides_the_prerendered_panels_and_nothing_else() {
    let html = render_route("/studio");
    let styles: Vec<&str> = html
        .split("<style")
        .skip(1)
        .filter_map(|tail| Some(&tail[tail.find('>')? + 1..tail.find("</style>")?]))
        .collect();
    let rules: Vec<&str> = styles
        .iter()
        .flat_map(|style| style.split('}'))
        .filter(|rule| rule.contains(ELSEWHERE))
        .collect();
    assert_eq!(rules.len(), 1, "the guard's class has {} rules: {rules:?}", rules.len());
    let (selector, body) = rules[0].split_once('{').expect("a rule with no body");
    let selector = selector.rsplit("*/").next().unwrap_or(selector).trim();
    assert_eq!(selector, format!(".{ELSEWHERE} #main .studio-main"), "the guard hides more, or less, than the prerendered panels");
    assert_eq!(body.trim(), "visibility: hidden;", "the guard's rule does more than keep the panels unpainted");
}

/// Run the guard under node once per `search` (the address's query, `?` included), and say for
/// each whether it hid the prerendered panels.
fn guard_hides(guard: &str, searches: &[&str]) -> Vec<bool> {
    use std::io::Write;
    let harness = format!(
        "const guard = {guard};\nconst searches = {searches};\nlet out = '';\nfor (const search of searches) {{\n  const added = [];\n  globalThis.location = {{ search }};\n  globalThis.document = {{ documentElement: {{ classList: {{ add: (c) => added.push(c) }} }} }};\n  new Function(guard)();\n  out += added.includes({class}) ? '1' : '0';\n}}\nconsole.log(out);\n",
        guard = json(guard),
        searches = format!("[{}]", searches.iter().map(|s| json(s)).collect::<Vec<_>>().join(", ")),
        class = json(ELSEWHERE),
    );
    let mut child = std::process::Command::new("node")
        .args(["--input-type=module"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("node was found but did not start");
    child.stdin.take().unwrap().write_all(harness.as_bytes()).unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(out.status.success(), "the guard threw under node:\n{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8_lossy(&out.stdout).trim().chars().map(|c| c == '1').collect()
}

/// `text` as a JavaScript string literal.
fn json(text: &str) -> String {
    let mut out = String::from("\"");
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 || c == '\u{2028}' || c == '\u{2029}' => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[test]
fn the_guard_reads_the_link_as_the_router_does() {
    use logicaffeine_web::ui::pages::studio_opening::StudioOpening;
    use logicaffeine_web::ui::router::Route;
    use std::str::FromStr;

    if !common::require_tool("node") {
        return;
    }
    let html = render_route("/studio");
    let guard = script_with(&html, ELSEWHERE).expect("the prerendered /studio carries no guard");
    let shown = StudioOpening::first_example().path;
    let searches = [
        "",
        "?file=examples/code/basics/control-flow.logos",
        "?file=/examples/code/basics/control-flow.logos",
        "?file=examples%2Fcode%2Fbasics%2Fcontrol-flow.logos",
        "?file=%2Fexamples%2Fcode%2Fbasics%2Fcontrol-flow.logos",
        "?file=",
        "?utm_source=newsletter",
        "?x=1&file=examples/code/basics/control-flow.logos",
        "?file=examples/code/basics/control-flow.logos&x=1",
        "?file=examples/logic/quantifiers.logic",
        "?file=workspace/mine.logos",
        "?file=//examples/code/basics/control-flow.logos",
        "?file=examples/code/basics/control-flow.logos&file=examples/logic/quantifiers.logic",
        "?file=examples/logic/quantifiers.logic&file=examples/code/basics/control-flow.logos",
        "?file=examples%26file%3Dexamples/logic/quantifiers.logic",
        "?file=examples/code/basics/control+flow.logos",
        "?file==examples/code/basics/control-flow.logos",
        "?file=%zz",
        "?file=examples/code/basics/control-flow.logos%",
        "?file=%E0%A4%A",
        "?file=%FF",
        "?file=%",
    ];
    let hidden = guard_hides(guard, &searches);
    assert_eq!(hidden.len(), searches.len(), "node answered for {} of {} links", hidden.len(), searches.len());
    assert!(hidden.iter().any(|hid| *hid) && hidden.iter().any(|hid| !*hid), "the table never exercises both a shown and a hidden copy");
    let mut wrong = Vec::new();
    for (search, hid) in searches.iter().zip(hidden) {
        let route = Route::from_str(&format!("/studio{search}")).unwrap_or_else(|e| panic!("/studio{search} does not parse: {e}"));
        let Route::Studio { file } = route else { panic!("/studio{search} is not the Studio") };
        let opens_elsewhere = StudioOpening::for_route(file.as_deref()).path != shown;
        if hid != opens_elsewhere {
            wrong.push(format!(
                "/studio{search}: the router reads file {file:?}, so the app opens {}, yet the guard {} the copy of {shown}",
                StudioOpening::for_route(file.as_deref()).path,
                if hid { "hides" } else { "shows" },
            ));
        }
    }
    assert!(wrong.is_empty(), "the guard and the router disagree:\n{}", wrong.join("\n"));
}
