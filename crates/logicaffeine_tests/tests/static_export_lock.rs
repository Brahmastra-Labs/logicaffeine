//! ★ LOCK — a LOGOS site can be published to a static host. ★
//!
//! `largo build` produced exactly one artifact: a native binary with the whole site
//! embedded in it. That is a good way to ship a site and it was the ONLY way — there was
//! no `dist/`, so GitHub Pages, Netlify, Cloudflare Pages, S3 and every CDN were closed
//! to a LOGOS site. The cheapest way to put a website on the internet was the one way
//! this framework could not.
//!
//! ★ AND A BROCHURE SITE COULD NOT BE BUILT AT ALL. The web bundle was planned only when
//! the source carried a `Serve` statement, so a project of pure `pages/` — no backend,
//! nothing to serve, exactly the site a static host wants — produced nothing.
//!
//! ★ THE FILE LAYOUT IS THE WHOLE DESIGN. A static host is dumb by construction: it maps
//! a URL to a file and nothing else. A page at `/about` must therefore be written as
//! `about/index.html`, because that is the one spelling every host serves for a
//! directory URL. Writing `dist/about` instead depends on a host's extensionless-file
//! rule, which some have and some do not — and the ones that do not serve a download
//! prompt rather than a page.

#[path = "cli_common/mod.rs"]
mod common;
use common::{largo_in, stderr};

use std::path::{Path, PathBuf};

/// A site with a front page, a nested page, a stylesheet — and NO backend, which is the
/// case that could not be built before.
fn brochure(dir: &Path, name: &str) -> PathBuf {
    let out = largo_in(dir, &["new", name, "--web"]);
    assert!(out.status.success(), "scaffold: {}", stderr(&out));
    let root = dir.join(name);
    std::fs::write(
        root.join("pages/app.lg"),
        "## Page /\n\nHead:\n    <title>Acme</title>\n\nView:\n    <main><h1>Acme</h1></main>\n",
    )
    .unwrap();
    std::fs::write(
        root.join("pages/about.lg"),
        "## Page /about\n\nView:\n    <main><h1>About</h1></main>\n",
    )
    .unwrap();
    root
}

fn export(root: &Path) -> PathBuf {
    let out = largo_in(root, &["build", "--static"]);
    assert!(
        out.status.success(),
        "`largo build --static` failed:\n{}",
        stderr(&out)
    );
    root.join("dist")
}

fn read(dist: &Path, rel: &str) -> String {
    std::fs::read_to_string(dist.join(rel))
        .unwrap_or_else(|e| panic!("dist/{rel} must exist ({e}); dist holds:\n{}", tree(dist)))
}

/// Every file under `dist`, relative and sorted — the failure message that makes a
/// missing file obvious instead of mysterious.
fn tree(dist: &Path) -> String {
    fn walk(d: &Path, base: &Path, out: &mut Vec<String>) {
        if let Ok(entries) = std::fs::read_dir(d) {
            for e in entries.flatten() {
                let p = e.path();
                if p.is_dir() {
                    walk(&p, base, out);
                } else if let Ok(r) = p.strip_prefix(base) {
                    out.push(r.display().to_string());
                }
            }
        }
    }
    let mut out = Vec::new();
    walk(dist, dist, &mut out);
    out.sort();
    out.join("\n")
}

/// ★ THE LOAD-BEARING TEST — a site with no backend exports, and the URLs a static host
/// derives from the file names are the URLs the pages declared.
#[test]
fn a_backend_less_site_exports_the_files_a_static_host_serves() {
    let dir = tempfile::tempdir().unwrap();
    let root = brochure(dir.path(), "brochure");
    let dist = export(&root);

    let front = read(&dist, "index.html");
    assert!(
        front.contains("Acme"),
        "the front page is at dist/index.html:\n{front}"
    );
    assert!(
        front.contains("<title>Acme</title>"),
        "with its own head:\n{front}"
    );

    let about = read(&dist, "about/index.html");
    assert!(
        about.contains("About"),
        "a page at `/about` must be written `about/index.html` — the one spelling every static \
         host serves for a directory URL:\n{about}"
    );
}

/// The module and its shim ride along, under the content-addressed names the pages
/// reference — a hash nobody links to is a 404, and here nothing rewrites it for you.
#[test]
fn the_module_and_shim_are_exported_under_the_names_the_pages_reference() {
    let dir = tempfile::tempdir().unwrap();
    let root = brochure(dir.path(), "refs");
    let dist = export(&root);
    let front = read(&dist, "index.html");

    let shim = front
        .split("src=\"")
        .nth(1)
        .and_then(|r| r.split('"').next())
        .expect("the page loads a module script")
        .trim_start_matches("./")
        .to_string();
    assert!(
        dist.join(&shim).is_file(),
        "the page references `{shim}` and the export must contain it, or the site loads nothing.\n\
         dist holds:\n{}",
        tree(&dist)
    );

    let js = read(&dist, &shim);
    let module = js
        .split(".wasm")
        .next()
        .and_then(|r| r.rsplit(['"', '\'', '/']).next())
        .map(|n| format!("{n}.wasm"))
        .expect("the shim names its module");
    assert!(
        dist.join(&module).is_file(),
        "the shim references `{module}` and the export must contain it.\ndist holds:\n{}",
        tree(&dist)
    );
}

/// Static assets are copied verbatim, at the URLs they serve at.
#[test]
fn static_assets_are_exported_where_they_serve() {
    let dir = tempfile::tempdir().unwrap();
    let root = brochure(dir.path(), "assets");
    std::fs::write(root.join("static/extra.css"), b".x{color:red}").unwrap();
    let dist = export(&root);
    assert_eq!(read(&dist, "extra.css"), ".x{color:red}");
    assert!(
        dist.join("theme.css").is_file(),
        "the scaffolded theme too:\n{}",
        tree(&dist)
    );
}

/// The site metadata files come along — they are part of being a website, not part of
/// being a server.
#[test]
fn the_export_carries_robots_and_the_sitemap() {
    let dir = tempfile::tempdir().unwrap();
    let root = brochure(dir.path(), "meta");
    let manifest = root.join("Largo.toml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        text.replace("[package]", "[package]\nsite = \"https://acme.example\""),
    )
    .unwrap();

    let dist = export(&root);
    assert!(read(&dist, "robots.txt").contains("User-agent: *"));
    let map = read(&dist, "sitemap.xml");
    assert!(
        map.contains("<loc>https://acme.example/about</loc>"),
        "{map}"
    );
}

/// ★ CACHING MUST NOT BE LOST AT THE DOOR. The binary sends `Cache-Control` and an
/// `ETag` with every asset; a static host sends whatever its own rules say unless the
/// export tells it otherwise. `_headers` is the file Netlify and Cloudflare Pages read,
/// so the content-addressed module is cached for a year on both delivery paths rather
/// than only one.
#[test]
fn the_export_carries_the_same_cache_rules_the_binary_sends() {
    let dir = tempfile::tempdir().unwrap();
    let root = brochure(dir.path(), "cache");
    let dist = export(&root);
    let headers = read(&dist, "_headers");
    assert!(
        headers.contains("immutable"),
        "the content-addressed module must still be cached forever:\n{headers}"
    );
    assert!(
        headers.contains("/*.wasm") || headers.contains(".wasm"),
        "and the rule must name the files it is about:\n{headers}"
    );
}

/// A rebuild must not leave yesterday's pages behind. A deleted page whose file survives
/// in `dist` is a URL that still answers after the page is gone — the worst kind of
/// stale, because nothing in the project mentions it any more.
#[test]
fn an_export_replaces_the_previous_one() {
    let dir = tempfile::tempdir().unwrap();
    let root = brochure(dir.path(), "stale");
    let dist = export(&root);
    assert!(dist.join("about/index.html").is_file());

    std::fs::remove_file(root.join("pages/about.lg")).unwrap();
    let dist = export(&root);
    assert!(
        !dist.join("about/index.html").exists(),
        "a page that no longer exists must not still be published:\n{}",
        tree(&dist)
    );
    assert!(
        dist.join("index.html").is_file(),
        "and the rest of the site survives"
    );
}
