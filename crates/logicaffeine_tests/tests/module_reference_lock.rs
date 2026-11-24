//! ★ LOCK — a built page names its module by the module's own address, and nothing else changes. ★
//!
//! A page's module and its WebAssembly ship under content-addressed names, and the build rewrites
//! the page's reference to them. It did that two wrong ways.
//!
//! ★ IT REWROTE TEXT, NOT A REFERENCE. The page's shim was renamed with a replace of `<stem>.js`
//! over the WHOLE document, so a page named `web` that said "the lessons are listed in
//! lessons/web.json" shipped "lessons/web.<hash>.json": a sentence naming a file that does not
//! exist. Only the script's own `src` is a reference to the module.
//!
//! ★ AND THE REFERENCE IT WROTE WAS RELATIVE. The hashed names kept the shell's `./`, so a page at
//! `/guide/state` loaded `./guide/state.<hash>.js`, which the browser asks for at
//! `/guide/state/guide/state.<hash>.js`, and the shim fetched its WebAssembly relative to the
//! document too. A page at `/` worked, which is why nothing caught it. Both addresses are absolute.

#[path = "cli_common/mod.rs"]
mod common;
use common::{largo_in, stderr};

use std::path::{Path, PathBuf};

fn site(dir: &Path, pages: &[(&str, &str)]) -> PathBuf {
    let root = dir.join("site");
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(
        root.join("Largo.toml"),
        "[package]\nname = \"site\"\nversion = \"0.1.0\"\n",
    )
    .unwrap();
    for (name, src) in pages {
        std::fs::write(root.join("pages").join(name), src).unwrap();
    }
    root
}

fn export(root: &Path) -> PathBuf {
    let out = largo_in(root, &["build", "--static"]);
    assert!(out.status.success(), "`largo build --static` failed:\n{}", stderr(&out));
    root.join("dist")
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{} must exist: {e}", path.display()))
}

/// The `src` of a document's module script.
fn module_src(doc: &str) -> String {
    doc.split("<script type=\"module\" src=\"")
        .nth(1)
        .and_then(|r| r.split('"').next())
        .map(str::to_string)
        .unwrap_or_else(|| panic!("the page loads a module:\n{doc}"))
}

/// A page that has something to run, so it ships a module whatever else changes.
const COUNTER: &str = "State:\n    Let n be 0.\n\nActions:\n    To bump:\n        Set n to n + 1.\n\nView:\n    <button on-click={bump}>{n}</button>\n";

/// ★ F20 — the text a page wrote is the text it ships.
#[test]
fn prose_naming_a_file_like_the_page_is_left_as_written() {
    let page = format!(
        "## Page /web\n\n{}",
        COUNTER.replace(
            "<button on-click={bump}>{n}</button>",
            "<main><p>The lessons are listed in lessons/web.json and web.js.map.</p><button on-click={bump}>{n}</button></main>"
        )
    );
    let dir = tempfile::tempdir().unwrap();
    let dist = export(&site(dir.path(), &[("web.lg", &page)]));
    let doc = read(&dist.join("web/index.html"));
    assert!(
        doc.contains("The lessons are listed in lessons/web.json and web.js.map."),
        "the page's own words are not a module reference:\n{doc}"
    );
    let src = module_src(&doc);
    assert!(
        src.contains("web.") && src.ends_with(".js") && src != "./web.js",
        "and the script still loads the hashed shim: {src}"
    );
}

/// ★ F20 — the shim's own text is rewritten only where it names the WebAssembly.
#[test]
fn the_shim_names_its_hashed_webassembly_once() {
    let dir = tempfile::tempdir().unwrap();
    let dist = export(&site(dir.path(), &[("app.lg", &format!("## Page /\n\n{COUNTER}"))]));
    let doc = read(&dist.join("index.html"));
    let src = module_src(&doc);
    let shim = read(&dist.join(src.trim_start_matches("./").trim_start_matches('/')));
    let first = shim.lines().next().unwrap_or_default();
    assert!(
        first.starts_with("const WASM_URL = ") && first.contains(".wasm"),
        "the shim opens by naming its module: {first}"
    );
    let named = first.trim_start_matches("const WASM_URL = \"").trim_end_matches("\";");
    assert!(
        dist.join(named.trim_start_matches("./").trim_start_matches('/')).is_file(),
        "and that module is published: {named}"
    );
}

/// ★ F21 — a page below the root loads its module and WebAssembly from the site root.
#[test]
#[ignore = "DAT-426: needs Collin — absolute module URLs change what static_export_lock::the_module_and_shim_are_exported_under_the_names_the_pages_reference, build_web_release and todo_deploy_e2e read (they parse the src as ./relative), and rule 4 forbids editing those tests without asking"]
fn a_nested_page_loads_its_module_from_the_site_root() {
    let dir = tempfile::tempdir().unwrap();
    let dist = export(&site(dir.path(), &[("state.lg", &format!("## Page /guide/state\n\n{COUNTER}"))]));
    let doc = read(&dist.join("guide/state/index.html"));
    let src = module_src(&doc);
    assert!(
        src.starts_with("/guide/state.") && src.ends_with(".js"),
        "the script's address is absolute, so the browser finds it from /guide/state: {src}"
    );
    assert!(dist.join(src.trim_start_matches('/')).is_file(), "and it is published: {src}");
    let shim = read(&dist.join(src.trim_start_matches('/')));
    let first = shim.lines().next().unwrap_or_default();
    assert!(
        first.starts_with("const WASM_URL = \"/guide/state.") && first.ends_with(".wasm\";"),
        "the shim fetches its WebAssembly from the site root too: {first}"
    );
    let wasm = first.trim_start_matches("const WASM_URL = \"").trim_end_matches("\";");
    assert!(dist.join(wasm.trim_start_matches('/')).is_file(), "and it is published: {wasm}");
}
