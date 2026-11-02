//! The shipped Web pages open at once: their preview documents (the page compiled to wasm, with
//! its first paint, as `preview_document` builds them) are served prebuilt from
//! `public/previews/`, so the browser fetches one instead of running the LOGOS compiler on every
//! click (the user, 2026-11-02: the web files were slow to load). The files are held to the
//! compiler in this tree; when the compiler or a page changes, this goes red and the regenerator
//! rewrites them: `LOGOS_REGEN=1 cargo test -p logicaffeine-tests --features web-tests --test
//! web_previews_lock -- --ignored regenerate_web_previews`.
#![cfg(feature = "web-tests")]

use logicaffeine_web::ui::examples::seeded_examples;
use logicaffeine_web::ui::pages::studio_workbench::{prebuilt_preview_path, preview_document};
use std::path::PathBuf;

fn public_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../apps/logicaffeine_web/public")
}

/// Every shipped page: its path, its source and where its preview is served.
fn pages() -> Vec<(&'static str, &'static str, String)> {
    seeded_examples().filter_map(|spec| prebuilt_preview_path(spec.vfs_path).map(|url| (spec.vfs_path, spec.source, url))).collect()
}

fn served(url: &str) -> PathBuf {
    public_dir().join(url.trim_start_matches('/'))
}

#[test]
fn only_the_shipped_pages_have_a_prebuilt_preview() {
    let pages = pages();
    assert!(pages.len() >= 4, "only {} shipped pages: {pages:?}", pages.len());
    for (path, _, url) in &pages {
        assert!(path.starts_with("/examples/code/web/"), "{path} is not under the web examples");
        assert_eq!(url, &format!("/previews{}.html", path.trim_start_matches("/examples")), "{path}: the preview's address");
    }
    for spec in seeded_examples() {
        if !spec.vfs_path.starts_with("/examples/code/web/") {
            assert_eq!(prebuilt_preview_path(spec.vfs_path), None, "{} is not a page and has a prebuilt preview", spec.vfs_path);
        }
    }
    assert_eq!(prebuilt_preview_path("/my/page.logos"), None, "a reader's own file has no prebuilt preview");
}

#[test]
fn every_shipped_page_is_served_prebuilt_and_current() {
    for (path, source, url) in pages() {
        let file = served(&url);
        let shipped = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{path}: {} is not served ({e}); regenerate (see the file's doc)", file.display()));
        assert!(shipped == preview_document(source), "{path}: {} is stale; regenerate (see the file's doc)", file.display());
    }
}

#[test]
fn no_stale_preview_is_served() {
    let dir = public_dir().join("previews");
    let mut files = Vec::new();
    fn walk(dir: &std::path::Path, out: &mut Vec<PathBuf>) {
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.is_dir() { walk(&p, out) } else { out.push(p) }
            }
        }
    }
    walk(&dir, &mut files);
    let wanted: Vec<PathBuf> = pages().into_iter().map(|(_, _, url)| served(&url)).collect();
    for file in files {
        assert!(wanted.iter().any(|w| w == &file), "{} is served but no shipped page is behind it", file.display());
    }
}

/// Not a check: the regenerator. `LOGOS_REGEN=1` opts in, since the full suite runs ignored tests.
#[test]
#[ignore = "writes public/previews; run explicitly with LOGOS_REGEN=1 to regenerate"]
fn regenerate_web_previews() {
    if std::env::var_os("LOGOS_REGEN").is_none() {
        eprintln!("skipping regeneration: set LOGOS_REGEN=1");
        return;
    }
    for (_, source, url) in pages() {
        let file = served(&url);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let tmp = file.with_extension("html.regen.tmp");
        std::fs::write(&tmp, preview_document(source)).unwrap();
        std::fs::rename(&tmp, &file).unwrap();
    }
}
