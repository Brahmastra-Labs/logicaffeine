//! ★ LOCK — a page ships only the script it runs, and a site ships its runtime once. ★
//!
//! Every page got a module script, a `<page>.<hash>.js` holding the whole 45 KB browser runtime
//! with the page's WebAssembly address prepended, and a `<page>.<hash>.wasm`, even a page with no
//! state and no handlers that has nothing to hydrate. Because the address was prepended, the runtime's
//! bytes and hash differed per page, so a browser downloaded the same runtime again on every page of
//! a site.
//!
//! A page with nothing to run ships as HTML and nothing else. Pages that do run share one
//! content-addressed runtime, fetched once per site, with only their small module differing.

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

fn files(dist: &Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut stack = vec![dist.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if let Ok(rel) = p.strip_prefix(dist) {
                out.push(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    out.sort();
    out
}

const COUNTER: &str = "State:\n    Let n be 0.\n\nActions:\n    To bump:\n        Set n to n + 1.\n\nView:\n    <button on-click={bump}>{n}</button>\n";

#[test]
#[ignore = "DAT-426: needs Collin — a page with nothing to run shipping no script changes what static_export_lock::the_module_and_shim_are_exported_under_the_names_the_pages_reference expects of a state-less page (it reads the module script), and rule 4 forbids editing that test without asking"]
fn a_page_with_nothing_to_run_ships_no_script() {
    let dir = tempfile::tempdir().unwrap();
    let dist = export(&site(
        dir.path(),
        &[("app.lg", "## Page /\n\nView:\n    <main><h1>hello</h1></main>\n")],
    ));
    let doc = std::fs::read_to_string(dist.join("index.html")).unwrap();
    assert!(doc.contains("<h1>hello</h1>"), "the page is its markup:\n{doc}");
    assert!(!doc.contains("<script"), "and nothing to run:\n{doc}");
    let shipped = files(&dist);
    assert!(
        !shipped.iter().any(|f| f.ends_with(".js") || f.ends_with(".wasm")),
        "no module and no runtime ship for it: {shipped:?}"
    );
}

#[test]
#[ignore = "DAT-426: needs a design decision from Collin — one content-addressed runtime per site (`/_logos/runtime.<hash>.js`) imported by a per-page module changes the shim every delivery path serves (largo serve, the release binary, runtime_js_lock)"]
fn two_pages_that_run_share_one_runtime_file() {
    let dir = tempfile::tempdir().unwrap();
    let dist = export(&site(
        dir.path(),
        &[
            ("app.lg", &format!("## Page /\n\n{COUNTER}")),
            ("other.lg", &format!("## Page /other\n\n{COUNTER}")),
        ],
    ));
    let runtime_bytes = logicaffeine_compile::web_host::BROWSER_RUNTIME_JS.len();
    let big: Vec<(String, u64)> = files(&dist)
        .into_iter()
        .filter(|f| f.ends_with(".js"))
        .map(|f| {
            let len = std::fs::metadata(dist.join(&f)).unwrap().len();
            (f, len)
        })
        .filter(|(_, len)| *len as usize >= runtime_bytes)
        .collect();
    assert_eq!(
        big.len(),
        1,
        "the runtime ships once for the site, not once per page: {big:?}"
    );
    for page in ["index.html", "other/index.html"] {
        let doc = std::fs::read_to_string(dist.join(page)).unwrap();
        assert!(doc.contains("<script type=\"module\""), "{page} still runs:\n{doc}");
    }
}
