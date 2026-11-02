//! ★ LOCK — a built page carries its own content, not an empty shell. ★
//!
//! `largo build` produced an HTML document whose `<body>` held one `<script>` tag. Everything a
//! reader sees arrived only after a WebAssembly module downloaded, instantiated and painted — so
//! a slow connection saw a blank page, a crawler saw nothing, and a reader with scripts off saw
//! nothing at all. For a framework whose whole premise is that LOGOS runs in the browser rather
//! than JavaScript, shipping a blank document is the one outcome that makes the premise
//! irrelevant.
//!
//! The page's markup is now rendered AT BUILD TIME and put in the body. The script still loads
//! and takes over; what changed is what is there before it does.

#[path = "cli_common/mod.rs"]
mod common;

use common::{largo_in, stderr};

/// Scaffold a `--web` project and build it, returning the front page's HTML.
fn built_front_page(dir: &std::path::Path) -> String {
    let out = largo_in(dir, &["new", "site", "--web"]);
    assert!(
        out.status.success(),
        "scaffold must succeed: {}",
        stderr(&out)
    );
    let root = dir.join("site");
    let page = std::fs::read_to_string(root.join("pages/app.lg")).unwrap();
    logicaffeine_cli::prerender_page_html(&page, "app")
}

#[test]
fn a_built_page_has_its_content_in_the_body() {
    let dir = tempfile::tempdir().unwrap();
    let html = built_front_page(dir.path());
    assert!(
        html.contains("is alive"),
        "the scaffolded page's heading must be IN the document, not painted later:\n{html}"
    );
    assert!(
        html.contains("Clicks so far: 0"),
        "and its initial state, so a reader sees the real page:\n{html}"
    );
}

#[test]
fn the_script_still_loads_so_the_page_becomes_interactive() {
    // Pre-rendering must not replace the app — it precedes it. A page that dropped the script
    // would be a static picture of an interactive page, which is worse than either.
    let dir = tempfile::tempdir().unwrap();
    let html = built_front_page(dir.path());
    assert!(html.contains("app.js"), "the module still loads:\n{html}");
}

#[test]
fn the_document_is_still_a_whole_document() {
    let dir = tempfile::tempdir().unwrap();
    let html = built_front_page(dir.path());
    assert!(
        html.starts_with("<!doctype html>"),
        "still an HTML5 document"
    );
    assert!(html.contains("<title>"), "still titled");
    assert!(
        html.contains("</body>\n</html>"),
        "still closed properly:\n{html}"
    );
}

#[test]
fn the_prerendered_markup_is_escaped() {
    // The content reaches people who never load the app, so it is the tier that must not forget.
    let page = "## Component App\nState:\n    Let bad be \"<img src=x onerror=alert(1)>\".\n\
        View:\n    <p>{bad}</p>\n\n## Main\nMount App.\n";
    let html = logicaffeine_cli::prerender_page_html(page, "app");
    assert!(
        !html.contains("<img src=x"),
        "the payload is not markup:\n{html}"
    );
    assert!(html.contains("&lt;img"), "it is escaped text:\n{html}");
}

#[test]
fn a_page_that_cannot_be_prerendered_still_builds() {
    // Pre-rendering is an IMPROVEMENT, not a new way to fail. A page the server renderer cannot
    // draw must still ship its shell and work in the browser exactly as it did before — a build
    // that broke on a construct the browser handles fine would be a bad trade.
    let page = "## Component App\nState:\n    Let raw be \"<b>x</b>\".\n\
        View:\n    <p>{raw raw}</p>\n\n## Main\nMount App.\n";
    let html = logicaffeine_cli::prerender_page_html(page, "app");
    assert!(
        html.contains("app.js"),
        "the shell still loads the module:\n{html}"
    );
    assert!(html.starts_with("<!doctype html>"));
}

#[test]
fn pre_rendering_is_deterministic() {
    // Two builds of the same source must produce the same bytes, or every asset hash and every
    // cache header downstream is a coin flip.
    let page =
        "## Component App\nState:\n    Let n be 3.\nView:\n    <p>{n}</p>\n\n## Main\nMount App.\n";
    let a = logicaffeine_cli::prerender_page_html(page, "app");
    let b = logicaffeine_cli::prerender_page_html(page, "app");
    assert_eq!(a, b);
}
