//! ★ LOCK — the pre-render is the page's first paint (Wave 7.6). ★
//!
//! `largo build` ships each page with its markup already in the body, and the module's mount adopts
//! that markup rather than drawing it again. Adoption only works when the two are the same page:
//! the markup must be what the page's OWN program draws first — the lowered page the module runs,
//! run in process to its first commit — spelled so that a browser's HTML parser builds exactly that
//! document from it. Then hydration keeps every element, and a reader the module never reaches (a
//! crawler, a slow connection, scripts off) sees the page as it is.
//!
//! The rows: the served body is the served spelling of the lowered page's first paint; the served
//! spelling differs from the recorded one only where HTML needs it to (a select's chosen option, a
//! textarea's content, a newline the parser would drop); and in a real browser, every example page
//! hydrates without drawing anything again, its document after the mount the first paint itself.
//!
//! `LOGOS_REQUIRE_TOOLS=chrome` makes a missing browser a failure instead of a skip.

#[path = "cli_common/mod.rs"]
mod common;

use logicaffeine_cli::browser::{self, Browser, PageBundle};
use logicaffeine_compile::semantics::dom;
use logicaffeine_language::parser::lgx_lower::PRERENDERED_ATTR;
use std::path::{Path, PathBuf};

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

/// Every page of every example project, assembled with its project (layouts, shared files) — the
/// source `largo build` compiles. An `## Error page` is served as content without a module, so it
/// has nothing to hydrate and is not a page here.
fn example_pages() -> Vec<(String, String)> {
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    let mut out = Vec::new();
    let mut projects: Vec<PathBuf> = std::fs::read_dir(&examples)
        .expect("examples/ is readable")
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.join("Largo.toml").is_file())
        .collect();
    projects.sort();
    for project in projects {
        let Ok(dir) = std::fs::read_dir(project.join("pages")) else {
            continue;
        };
        let mut pages: Vec<PathBuf> = dir
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "lg"))
            .collect();
        pages.sort();
        for page in pages {
            let source = logicaffeine_compile::analysis::assemble_project(&page)
                .unwrap_or_else(|e| panic!("{} assembles: {e}", page.display()))
                .source;
            if source.contains("## Error page") {
                continue;
            }
            let name = format!(
                "{}/{}",
                project.file_name().unwrap().to_string_lossy(),
                page.file_name().unwrap().to_string_lossy()
            );
            out.push((name, source));
        }
    }
    assert!(
        out.len() >= 12,
        "the sweep must reach the example pages; it found {}",
        out.len()
    );
    out
}

/// The body a pre-rendered document ships, between its marked `<body>` and `</body>`.
fn served_body(document: &str) -> String {
    let open = format!("<body {PRERENDERED_ATTR}=\"1\">\n");
    let start = document
        .find(&open)
        .unwrap_or_else(|| panic!("the document carries the mark:\n{document}"))
        + open.len();
    let end = document
        .rfind("\n</body>")
        .expect("the document closes its body");
    document[start..end].to_string()
}

/// The lowered page's first paint, in process, at the page's own URL — where a browser that
/// navigated there paints it: `spell` reads the document it leaves.
fn first_paint(source: &str, spell: fn() -> String) -> String {
    dom::reset();
    logicaffeine_compile::ui_bridge::with_parsed_program(source, |parsed, interner| {
        if let Ok((stmts, ..)) = parsed {
            logicaffeine_compile::page_host::boot_at_declared_route(stmts, interner);
        }
    });
    let out = logicaffeine_compile::compile::vm_outcome(source);
    assert_eq!(
        out.error, None,
        "the lowered page runs in process:\n{source}"
    );
    let doc = spell();
    doc.strip_prefix("<body>")
        .and_then(|d| d.strip_suffix("</body>"))
        .unwrap_or(&doc)
        .to_string()
}

#[test]
fn the_served_body_is_the_lowered_pages_first_paint() {
    for (name, source) in example_pages() {
        let document = logicaffeine_cli::prerender_page_html(&source, "app");
        assert_eq!(
            served_body(&document),
            first_paint(&source, dom::render_served),
            "{name}: the shipped body is the page's own first paint, spelled for a browser"
        );
    }
}

const CONTROLS: &str = r#"## Page /

State:
    Let size be "m".
    Let notes be "\ntwo lines".
    Let agreed be true.
    Let code be "\nindented".
View:
    <main>
      <label>Size <select id="size" bind={size}><option value="s" selected>S</option><option value="m">M</option></select></label>
      <label>Notes <textarea id="notes" bind={notes}></textarea></label>
      <label>Agree <input id="agree" type="checkbox" bind={agreed}></label>
      <pre>{code}</pre>
    </main>
"#;

#[test]
fn the_served_spelling_differs_from_the_recorded_one_only_where_html_needs_it() {
    let recorded = first_paint(CONTROLS, dom::render);
    let served = first_paint(CONTROLS, dom::render_served);
    assert!(
        recorded.contains("<select id=\"size\" value=\"m\">"),
        "recorded, the select holds its value: {recorded}"
    );
    assert!(
        served.contains("<select id=\"size\"><option value=\"s\">S</option><option value=\"m\" selected>M</option></select>"),
        "served, its value is the option a parser selects, and the markup's own `selected` gives way: {served}"
    );
    assert!(
        served.contains("<textarea id=\"notes\">\n\ntwo lines</textarea>"),
        "served, a textarea's value is its content, a first newline doubled for the parser to drop: {served}"
    );
    assert!(
        served.contains("<pre>\n\nindented</pre>"),
        "and so is a <pre>'s: {served}"
    );
    assert!(
        served.contains("<input id=\"agree\" type=\"checkbox\" checked>"),
        "a checkbox is the same either way: {served}"
    );

    let plain = "## Page /\nState:\n    Let n be 3.\nView:\n    <main><p id=\"a\" class=\"x\">Count: {n}</p><input value=\"v\"><br></main>\n";
    assert_eq!(
        first_paint(plain, dom::render_served),
        first_paint(plain, dom::render),
        "nothing else differs"
    );
}

#[test]
fn every_example_page_hydrates_without_drawing_anything_again() {
    let Some(b) = browser() else { return };
    let mut hydrated = 0usize;
    for (name, source) in example_pages() {
        let bundle = PageBundle::prerendered(&source).unwrap_or_else(|e| panic!("{name}: {e}"));
        let page = b
            .open(&bundle)
            .unwrap_or_else(|e| panic!("{name}: the page must open: {e}"));
        let (total, replaced) = page.prerendered_fate().unwrap();
        assert!(total > 0, "{name}: the document arrived with its markup");
        assert!(
            replaced.is_empty(),
            "{name}: the mount adopted the pre-rendered page; it replaced {replaced:?}"
        );
        let body = page
            .snapshot()
            .unwrap_or_else(|e| panic!("{name}: snapshot: {e}"))
            .body_markup();
        assert_eq!(
            body,
            format!("<body>{}</body>", first_paint(&source, dom::render)),
            "{name}: after the mount the browser's document is the first paint the markup was"
        );
        hydrated += 1;
    }
    assert!(
        hydrated >= 12,
        "the sweep must reach the example pages; it hydrated {hydrated}"
    );
}
