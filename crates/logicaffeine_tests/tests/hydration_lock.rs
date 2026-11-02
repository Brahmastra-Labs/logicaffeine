//! ★ LOCK — hydration adopts the pre-rendered page, node for node (Wave 7.6). ★
//!
//! `largo build` renders a page's first paint into the document it ships, and marks the body
//! (`data-logos-prerendered`) as holding it. A visitor sees the page, focuses a field and starts
//! typing before the module has even downloaded. When the module's mount runs, its first commit does
//! not draw the page again: it ADOPTS the markup the browser already built from that document, each
//! element and text node bound to the id the retained tree gives it, so the element the visitor
//! focused is the element the page drives, its caret where they left it, and what they typed is
//! heard by the page as the input it was.
//!
//! What the rows hold, in a real browser:
//! - every element the document arrived with is still in the document after the first paint, and the
//!   page is shown once, exactly as the reference tier renders it;
//! - focus and the caret inside a field survive the mount;
//! - a value typed before the module ran reaches the page's state;
//! - two holes side by side, which the HTML parser merges into one text node, and an empty hole,
//!   which it cannot represent at all, are split and made again along the page's own claims, and
//!   patch in place afterwards;
//! - markup the parser rewrites (`<tr>` straight in a `<table>`) cannot be adopted: the mount draws
//!   the page afresh, and the template check names the markup that caused it;
//! - a document whose body holds markup without the mark — a refused pre-render, a placeholder —
//!   is cleared and drawn, never adopted.
//!
//! `LOGOS_REQUIRE_TOOLS=chrome` makes a missing browser a failure instead of a skip.

#[path = "cli_common/mod.rs"]
mod common;

use logicaffeine_cli::browser::{self, Browser, OpenOptions, Page, PageBundle};
use logicaffeine_compile::semantics::dom;
use logicaffeine_language::lgx::check::{check, FindingKind, ParserRewrite, Severity};
use logicaffeine_language::parser::lgx_lower::PRERENDERED_ATTR;

fn browser() -> Option<&'static Browser> {
    let Some(chrome) = browser::locate_chrome() else {
        let spec = std::env::var("LOGOS_REQUIRE_TOOLS").unwrap_or_default();
        common::require_tool_strict("chrome", &spec);
        return None;
    };
    Some(browser::shared(&chrome).unwrap_or_else(|e| {
        panic!(
            "Chrome is installed at {} and must start: {e}",
            chrome.display()
        )
    }))
}

/// The reference tier's document after `steps` (top-level statements) run against `source`.
fn reference(source: &str, steps: &[&str]) -> String {
    dom::reset();
    let program = if steps.is_empty() {
        source.to_string()
    } else {
        format!("{source}\n## Main\n{}\n", steps.join("\n"))
    };
    let out = logicaffeine_compile::compile::tw_outcome(&program);
    assert_eq!(
        out.error, None,
        "the reference tier must run the page:\n{program}"
    );
    dom::render()
}

fn body(page: &Page) -> String {
    page.snapshot()
        .unwrap_or_else(|e| panic!("snapshot: {e}"))
        .body_markup()
}

fn no_failures(page: &Page) {
    let errors: Vec<String> = page
        .console()
        .into_iter()
        .filter(|c| c.level == "error")
        .map(|c| c.text)
        .collect();
    assert!(
        page.exceptions().is_empty() && errors.is_empty(),
        "the page must not fail:\nexceptions: {:#?}\nconsole errors: {errors:#?}",
        page.exceptions()
    );
}

fn prerendered(source: &str) -> PageBundle {
    let bundle = PageBundle::prerendered(source).unwrap_or_else(|e| panic!("{e}\n{source}"));
    let shipped = String::from_utf8_lossy(bundle.file("/").expect("the document")).to_string();
    assert!(
        shipped.contains(&format!("<body {PRERENDERED_ATTR}=\"1\">")),
        "the shipped body carries the mark the mount adopts by:\n{shipped}"
    );
    bundle
}

/// The page opened with `early` run when parsing is done (`interactive`) and before any script of
/// the page — what a visitor does while the module is still downloading.
fn open_early(b: &Browser, bundle: &PageBundle, early: &str) -> Page {
    let script = format!(
        "document.addEventListener('readystatechange', () => {{\n\
            if (document.readyState === 'interactive') {{ {early} }}\n\
        }});"
    );
    b.open_with(
        bundle,
        &OpenOptions {
            init_scripts: vec![script],
            ..OpenOptions::default()
        },
    )
    .unwrap_or_else(|e| panic!("the page must open: {e}"))
}

/// Every element the document arrived with is still in it.
fn adopted_whole(page: &Page, at_least: usize) {
    let (total, replaced) = page.prerendered_fate().unwrap();
    assert!(
        total >= at_least,
        "the document arrived with its elements (saw {total})"
    );
    assert!(
        replaced.is_empty(),
        "the first paint adopts every pre-rendered element; it replaced {replaced:?}"
    );
}

const LISTED: &str = r##"## Component Row (label: Text)
State:
    Let seen be "no".
Actions:
    When this component appears:
        Set seen to "yes".
View:
    <li id="r-{label}" class="row">{label}:{seen}</li>

## Page /
State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
View:
    <main>
      <h1 title="list">Rows {n}</h1>
      <ul>{for each r in ["a", "b"]:}<Row label="{r}"/>{end}</ul>
      {if n > 0:}<p id="more">more</p>{end}
      <button id="go" on-click={bump}>go</button>
    </main>
"##;

#[test]
fn prerendered_nodes_are_adopted_not_replaced() {
    let Some(b) = browser() else { return };
    let p = b
        .open(&prerendered(LISTED))
        .unwrap_or_else(|e| panic!("the page must open: {e}"));
    no_failures(&p);
    assert_eq!(
        body(&p),
        reference(LISTED, &[]),
        "the page appears once, the mark gone with the hydration"
    );
    adopted_whole(&p, 6);
    p.click("#go").unwrap();
    assert_eq!(
        body(&p),
        reference(LISTED, &["Click \"#go\"."]),
        "the adopted page answers and patches in place"
    );
    adopted_whole(&p, 6);
}

const FIELD: &str = r#"## Page /

State:
    Let name be "Hello".
View:
    <main>
      <label for="name">Name</label>
      <input id="name" bind={name}>
      <p id="echo">{name}</p>
    </main>
"#;

#[test]
fn focus_and_caret_survive_hydration() {
    let Some(b) = browser() else { return };
    let p = open_early(
        b,
        &prerendered(FIELD),
        "const f = document.querySelector('#name'); f.focus(); f.setSelectionRange(1, 3);",
    );
    no_failures(&p);
    adopted_whole(&p, 4);
    let seen = p
        .eval("(() => { const f = document.querySelector('#name'); return [document.activeElement === f, f.selectionStart, f.selectionEnd, f.value]; })()")
        .unwrap();
    assert_eq!(
        seen,
        serde_json::json!([true, 1, 3, "Hello"]),
        "the field keeps focus, caret and value through the mount"
    );
    assert_eq!(body(&p), reference(FIELD, &[]));
}

#[test]
fn text_typed_before_hydration_reaches_state() {
    let Some(b) = browser() else { return };
    let p = open_early(
        b,
        &prerendered(FIELD),
        "const f = document.querySelector('#name'); f.focus(); f.value = 'Ada';",
    );
    p.settle().unwrap();
    no_failures(&p);
    adopted_whole(&p, 4);
    let snap = p.snapshot().unwrap();
    assert_eq!(
        snap.by_id("echo")[0].text_content(),
        "Ada",
        "the page heard what was typed before it ran"
    );
    assert_eq!(
        p.eval("[document.activeElement.id, document.querySelector('#name').value]")
            .unwrap(),
        serde_json::json!(["name", "Ada"]),
        "and the field keeps both the text and the focus"
    );
    assert_eq!(
        body(&p),
        reference(FIELD, &["Type \"Ada\" into \"#name\"."])
    );
}

const ADJACENT: &str = r#"## Page /

State:
    Let first be "x".
    Let second be "y".
Actions:
    To change:
        Set second to "z".
View:
    <main>
      <p id="pair">{first}{second}</p>
      <button id="go" on-click={change}>go</button>
    </main>
"#;

/// How many text nodes element `sel` holds.
fn text_nodes(page: &Page, sel: &str) -> serde_json::Value {
    page.eval(&format!(
        "Array.from(document.querySelector('{sel}').childNodes).filter((n) => n.nodeType === 3).length"
    ))
    .unwrap()
}

#[test]
fn adjacent_text_holes_hydrate() {
    let Some(b) = browser() else { return };
    let bundle = prerendered(ADJACENT);
    let shipped = String::from_utf8_lossy(bundle.file("/").unwrap()).to_string();
    assert!(
        shipped.contains("<p id=\"pair\">xy</p>"),
        "the two holes ship as the one text a parser joins:\n{shipped}"
    );
    let p = open_early(
        b,
        &bundle,
        "window.__joined = document.querySelector('#pair').firstChild;",
    );
    no_failures(&p);
    adopted_whole(&p, 3);
    assert_eq!(
        text_nodes(&p, "#pair"),
        serde_json::json!(2),
        "the joined text is split along the page's two holes"
    );
    assert_eq!(
        p.eval("document.querySelector('#pair').firstChild === window.__joined")
            .unwrap(),
        serde_json::json!(true),
        "the markup's own text node is the first hole's, split rather than replaced"
    );
    assert_eq!(body(&p), reference(ADJACENT, &[]));
    p.click("#go").unwrap();
    assert_eq!(
        body(&p),
        reference(ADJACENT, &["Click \"#go\"."]),
        "the second hole patches its own node"
    );
    assert_eq!(text_nodes(&p, "#pair"), serde_json::json!(2));
}

const EMPTY: &str = r#"## Page /

State:
    Let inner be "".
Actions:
    To fill:
        Set inner to "q".
View:
    <main>
      <p id="wrapped">[{inner}]</p>
      <button id="go" on-click={fill}>go</button>
    </main>
"#;

#[test]
fn empty_text_hole_hydrates() {
    let Some(b) = browser() else { return };
    let p = b
        .open(&prerendered(EMPTY))
        .unwrap_or_else(|e| panic!("the page must open: {e}"));
    no_failures(&p);
    adopted_whole(&p, 3);
    assert_eq!(
        text_nodes(&p, "#wrapped"),
        serde_json::json!(3),
        "the empty hole has a node of its own between the brackets"
    );
    assert_eq!(body(&p), reference(EMPTY, &[]));
    p.click("#go").unwrap();
    assert_eq!(
        body(&p),
        reference(EMPTY, &["Click \"#go\"."]),
        "and fills in place"
    );
}

const TABLE: &str = r#"## Page /

State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
View:
    <main>
      <table><tr><td id="cell">{n}</td></tr></table>
      <button id="go" on-click={bump}>go</button>
    </main>
"#;

#[test]
fn a_table_without_tbody_falls_back_and_is_linted() {
    let found: Vec<(FindingKind, Severity)> =
        logicaffeine_compile::ui_bridge::lgx_findings_of(TABLE)
            .into_iter()
            .map(|f| (f.kind, f.severity))
            .collect();
    assert!(
        found.contains(&(
            FindingKind::RewrittenByParser(ParserRewrite::WrapsRowInTbody, "`<tr>`".into()),
            Severity::Warning
        )),
        "the template check names the row the parser wraps: {found:?}"
    );
    let Some(b) = browser() else { return };
    let p = b
        .open(&prerendered(TABLE))
        .unwrap_or_else(|e| panic!("the page must open: {e}"));
    no_failures(&p);
    let (_, replaced) = p.prerendered_fate().unwrap();
    assert!(
        replaced.iter().any(|path| path.contains("table")),
        "the browser wrapped the row in a <tbody> the page never draws, so the mount drew the page afresh: {replaced:?}"
    );
    assert_eq!(
        body(&p),
        reference(TABLE, &[]),
        "drawn once, as the page draws it"
    );
    p.click("#go").unwrap();
    assert_eq!(body(&p), reference(TABLE, &["Click \"#go\"."]));
}

const COUNTER: &str = r#"## Page /

State:
    Let n be 0.
Actions:
    To bump:
        Set n to n + 1.
View:
    <main>
      <p id="out">Count: {n}</p>
      <button id="go" on-click={bump}>go</button>
    </main>
"#;

#[test]
fn a_refused_prerender_mounts_fresh() {
    let refused = "## Main\nShow 1.\n";
    assert!(
        PageBundle::prerendered(refused).is_err(),
        "a program that mounts nothing has no markup to ship"
    );
    let shipped = logicaffeine_cli::prerender_page_html(refused, "app");
    assert!(
        !shipped.contains(PRERENDERED_ATTR),
        "a page shipped without its markup carries no mark:\n{shipped}"
    );

    let Some(b) = browser() else { return };
    let page = PageBundle::page(COUNTER).unwrap_or_else(|e| panic!("{e}"));
    let shell = String::from_utf8_lossy(page.file("/").unwrap()).to_string();
    let placeholder = shell.replace("<body>\n", "<body>\n<p id=\"out\">Loading…</p>\n");
    assert_ne!(
        shell, placeholder,
        "the shell has a body to put the placeholder in"
    );
    let stem = logicaffeine_compile::page_host::PAGE_STEM;
    let bundle = PageBundle::new("/", placeholder)
        .with_file(
            &format!("/{stem}.wasm"),
            page.file(&format!("/{stem}.wasm")).unwrap().to_vec(),
        )
        .with_file(
            &format!("/{stem}.js"),
            page.file(&format!("/{stem}.js")).unwrap().to_vec(),
        );
    let p = b
        .open(&bundle)
        .unwrap_or_else(|e| panic!("the page must open: {e}"));
    no_failures(&p);
    assert_eq!(
        body(&p),
        reference(COUNTER, &[]),
        "markup the body holds without the mark is not the page's: it is cleared and the page drawn"
    );
}

// ---- The template check: markup a browser's HTML parser rewrites. ----

fn rewrites(src: &str) -> Vec<(ParserRewrite, String)> {
    let nodes = logicaffeine_language::lgx::template::parse(src)
        .unwrap_or_else(|e| panic!("{src:?} must parse: {e:?}"));
    check(&nodes)
        .into_iter()
        .filter_map(|f| match f.kind {
            FindingKind::RewrittenByParser(r, what) => {
                assert_eq!(
                    f.severity,
                    Severity::Warning,
                    "a rewrite is a warning: the page still works, drawn again"
                );
                assert!(
                    f.message.contains("pre-rendered"),
                    "the message says what it costs: {}",
                    f.message
                );
                Some((r, what))
            }
            _ => None,
        })
        .collect()
}

fn rewritten(src: &str, rewrite: ParserRewrite, what: &str) {
    let got = rewrites(src);
    assert!(
        got.contains(&(rewrite, what.to_string())),
        "{src:?} should report {rewrite:?} on {what}; got {got:?}"
    );
}

fn kept(src: &str) {
    let got = rewrites(src);
    assert!(got.is_empty(), "{src:?} is kept as written; got {got:?}");
}

#[test]
fn markup_the_parser_rewrites_is_linted_and_markup_it_keeps_is_not() {
    use ParserRewrite::*;
    rewritten(
        "<table><tr><td>a</td></tr></table>",
        WrapsRowInTbody,
        "`<tr>`",
    );
    rewritten(
        "<table><tbody><td>a</td></tbody></table>",
        WrapsCellInRow,
        "`<td>`",
    );
    rewritten("<table><th>a</th></table>", WrapsCellInRow, "`<th>`");
    rewritten("<table><col></table>", WrapsColInColgroup, "`<col>`");
    rewritten("<table><div>a</div></table>", MovesOutOfTable, "`<div>`");
    rewritten(
        "<table><tbody><tr>loose</tr></tbody></table>",
        MovesOutOfTable,
        "text",
    );
    rewritten(
        "<table><tbody><tr>{n}</tr></tbody></table>",
        MovesOutOfTable,
        "text",
    );
    rewritten("<p>a<div>b</div></p>", ClosesParagraph, "`<div>`");
    rewritten(
        "<p><span><ul><li>x</li></ul></span></p>",
        ClosesParagraph,
        "`<ul>`",
    );
    rewritten(
        "<a href=\"/a\">one <span><a href=\"/b\">two</a></span></a>",
        ClosesLink,
        "`<a>`",
    );
    rewritten(
        "<form><div><form></form></div></form>",
        DropsNestedForm,
        "`<form>`",
    );
    rewritten(
        "<button>a<span><button>b</button></span></button>",
        ClosesButton,
        "`<button>`",
    );
    rewritten("<ul><li>a<li>b</li></li></ul>", ClosesItem, "`<li>`");
    rewritten(
        "<ul><li><div><li>b</li></div></li></ul>",
        ClosesItem,
        "`<li>`",
    );
    rewritten("<dl><dt>a<dd>b</dd></dt></dl>", ClosesItem, "`<dd>`");
    rewritten(
        "<select><option>a<option>b</option></option></select>",
        ClosesOption,
        "`<option>`",
    );
    rewritten("<h1>a<h2>b</h2></h1>", ClosesHeading, "`<h2>`");
    rewritten(
        "<table>{if on:}<tr><td>a</td></tr>{end}</table>",
        WrapsRowInTbody,
        "`<tr>`",
    );
    rewritten(
        "<p>{for each x in xs:}<div>{x}</div>{end}</p>",
        ClosesParagraph,
        "`<div>`",
    );

    kept("<table><thead><tr><th>a</th></tr></thead><tbody><tr><td>{n}</td></tr></tbody></table>");
    kept("<table><caption>c</caption><colgroup><col></colgroup><tbody><tr><td>a</td></tr></tbody></table>");
    kept("<p>text <b>bold</b> <a href=\"/x\">link</a> {n}</p>");
    kept("<p><button><div>inside the button's own scope</div></button></p>");
    kept("<ul><li>a<ul><li>nested in a list</li></ul></li></ul>");
    kept("<a href=\"/x\"><td>a cell is a marker</td></a>");
    kept("<div><p>a</p><div>b</div></div>");
    kept("<p><Card/></p>");
    kept("<table>  <tbody><tr><td>a</td></tr></tbody>  </table>");
}

// ---- The mark: one name, written by the build and taken off by the host; the mount reads it (the rows above). ----

#[test]
fn the_mark_the_build_writes_is_the_one_the_host_takes_off() {
    let runtime = logicaffeine_compile::web_host::browser_host_js("app");
    assert!(
        runtime.contains(&format!("const PRERENDERED = '{PRERENDERED_ATTR}';")),
        "the browser host takes off the mark the lowering names"
    );
    let shipped = logicaffeine_cli::prerender_page_html(COUNTER, "app");
    assert!(
        shipped.contains(&format!("<body {PRERENDERED_ATTR}=\"1\">")),
        "the build writes it:\n{shipped}"
    );
}

const FORM: &str = r#"## Page /

State:
    Let size be "m".
    Let notes be "hi".
    Let agreed be true.
View:
    <main>
      <label>Size <select id="size" bind={size}><option value="s">S</option><option value="m">M</option></select></label>
      <label>Notes <textarea id="notes" bind={notes}></textarea></label>
      <label>Agree <input id="agree" type="checkbox" bind={agreed}></label>
    </main>
"#;

/// What the form's three controls hold, read by `from` — an expression naming a document.
fn controls(page: &Page, from: &str) -> serde_json::Value {
    page.eval(&format!(
        "(() => {{ const d = {from}; return [d.querySelector('#size').value, d.querySelector('#notes').value, d.querySelector('#agree').checked]; }})()"
    ))
    .unwrap()
}

#[test]
fn a_prerendered_form_shows_its_state_before_and_after_hydration() {
    let Some(b) = browser() else { return };
    let bundle = prerendered(FORM);
    let shipped = String::from_utf8_lossy(bundle.file("/").unwrap()).to_string();
    let p = b
        .open(&bundle)
        .unwrap_or_else(|e| panic!("the page must open: {e}"));
    let parsed = format!(
        "new DOMParser().parseFromString({}, 'text/html')",
        serde_json::to_string(&shipped).unwrap()
    );
    assert_eq!(
        controls(&p, &parsed),
        serde_json::json!(["m", "hi", true]),
        "a reader the module never reaches sees each control as the page set it:\n{shipped}"
    );
    no_failures(&p);
    adopted_whole(&p, 8);
    assert_eq!(
        controls(&p, "document"),
        serde_json::json!(["m", "hi", true]),
        "and the page keeps them through the mount"
    );
    assert_eq!(body(&p), reference(FORM, &[]));
}
