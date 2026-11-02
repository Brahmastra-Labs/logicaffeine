//! docs.logicaffeine.com must answer every address honestly.
//!
//! Cloudflare Pages treats a deployment with no top-level `404.html` as a single-page app: an
//! address it has no file for is answered with `/index.html` and a 200. The docs site shipped
//! that way, so `/search.index/` — the directory rustdoc's search reads its shards from, never a
//! page — answered with a copy of the landing page, and crawlers indexed it as one. Every other
//! dead address did the same, which hid a second defect: `build.sh` hand-picked pieces of
//! rustdoc's output and left out `help.html`, `settings.html`, `trait.impl/` and `type.impl/`, so
//! every trait page's implementors script came back as the landing page, and nothing complained.
//!
//! Building the site honestly surfaced a third: `cargo doc --no-deps` does not order one crate's
//! doc job after its dependencies', and rustdoc links another crate's items only when that crate's
//! docs already exist as it starts — so `logicaffeine_proof` shipped with no link into
//! `logicaffeine_kernel` at all, and `logicaffeine_language` none into `logicaffeine_base`.
//!
//! These tests document a real fixture workspace with the toolchain under test, through the
//! `site.py` the deploy runs, and read the result through a model of Pages' routing.

use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("repo root is two levels above the crate manifest")
        .to_path_buf()
}

const ORIGIN: &str = "https://docs.fixture.test";
const SHELL: [&str; 5] = ["index.html", "404.html", "robots.txt", "sitemap.xml", "llms.txt"];

/// The "start here" routes the fixture site is built with — `routes.toml`'s shape.
const FIXTURE_ROUTES: &str = r#"[[route]]
section = "Greeting"
task = "Greet something"
target = "alpha/trait.Greet.html"
note = "The trait every greeter implements."

[[route]]
section = "Greeting"
task = "Find a widget"
target = "alpha/inner/index.html"
note = "Where widgets live."

[[route]]
section = "Gadgets"
task = "Build a gadget"
target = "beta/struct.Gadget.html"
note = "Beta's one type."
"#;

const WORKSPACE_MANIFEST: &str = r#"[workspace]
resolver = "2"
members = ["crates/alpha", "crates/beta", "crates/gamma", "crates/delta"]

[workspace.package]
documentation = "https://docs.fixture.test/"
"#;

const ALPHA_MANIFEST: &str = r#"[package]
name = "alpha"
version = "0.1.0"
edition = "2021"
description = "Alpha <fixture> & friends"
documentation.workspace = true

[dependencies]
delta = { path = "../delta" }
"#;

const ALPHA_LIB: &str = r#"//! Alpha. See [`Greet`] and [`inner::Widget`].

pub mod bare {}

/// Widgets live here.
pub mod inner {
    /// A widget.
    pub struct Widget;
}

mod hidden {
    /// A secret, re-exported from a private module.
    pub struct Secret;
}
pub use hidden::Secret;

pub mod macros {
    /// A macro exported from a submodule.
    #[macro_export]
    macro_rules! shout {
        () => {};
    }
}

/// Greets.
pub trait Greet {
    /// Says hi.
    fn hi(&self) -> String;
}

impl Greet for inner::Widget {
    fn hi(&self) -> String {
        "widget".into()
    }
}

/// An alias, so rustdoc writes a `type.impl/` script.
pub type WidgetAlias = inner::Widget;

/// A trait nothing outside `alpha` implements, so rustdoc writes no implementors script for it.
pub trait Lonely {
    /// Keeps to itself.
    fn alone(&self) {}
}

fn _tags(widget: &inner::Widget) {
    delta::Tagged::tag(widget);
}
"#;

const BETA_MANIFEST: &str = r#"[package]
name = "beta"
version = "0.1.0"
edition = "2021"
description = "Beta fixture"

[dependencies]
alpha = { path = "../alpha" }

[[bin]]
name = "beta-tool"
path = "src/main.rs"
"#;

const BETA_LIB: &str = r#"//! Beta.

/// A gadget.
pub struct Gadget;

impl alpha::Greet for Gadget {
    fn hi(&self) -> String {
        "gadget".into()
    }
}
"#;

const GAMMA_MANIFEST: &str = r#"[package]
name = "gamma"
version = "0.1.0"
edition = "2021"
description = "Gamma fixture"
"#;

const BETA_MAIN: &str = "fn main() {}\n";

const GAMMA_LIB: &str = "//! Gamma.\n\n/// A gizmo.\npub struct Gizmo;\n";

const DELTA_MANIFEST: &str = r#"[package]
name = "delta"
version = "0.1.0"
edition = "2021"
description = "Delta fixture"
"#;

const DELTA_LIB: &str = r#"//! Delta: a dependency the site does not document.

/// Tags anything.
pub trait Tagged {
    /// Tags it — see [the tagging guide](guide/index.html#tags).
    fn tag(&self) {}
}

impl<T> Tagged for T {}
"#;

/// The one link rustdoc copies onto our pages from delta's blanket impl — relative to delta's docs.
const FOREIGN_LINK: &str = "guide/index.html#tags";

/// A workspace of four crates. `alpha` defines a trait, a type alias, an item re-exported from a
/// private module and a macro exported from a submodule — the last two are the shapes rustdoc
/// writes redirect stubs for — and a `Lonely` trait nothing else implements. `beta` depends on
/// alpha, implements alpha's trait from outside it (so alpha's trait page loads a cross-crate
/// implementors script) and has a binary. `gamma` is a member the site does not curate. `delta` is
/// a dependency of alpha's the site never documents, whose blanket impl rustdoc copies — docs,
/// relative links and all — onto alpha's pages, as `tracing` does onto ours.
struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new() -> Self {
        let fixture = Fixture { dir: tempfile::tempdir().expect("tempdir") };
        fixture.write("Cargo.toml", WORKSPACE_MANIFEST);
        fixture.write("crates/alpha/Cargo.toml", ALPHA_MANIFEST);
        fixture.write("crates/alpha/src/lib.rs", ALPHA_LIB);
        fixture.write("crates/beta/Cargo.toml", BETA_MANIFEST);
        fixture.write("crates/beta/src/lib.rs", BETA_LIB);
        fixture.write("crates/beta/src/main.rs", BETA_MAIN);
        fixture.write("crates/gamma/Cargo.toml", GAMMA_MANIFEST);
        fixture.write("crates/gamma/src/lib.rs", GAMMA_LIB);
        fixture.write("crates/delta/Cargo.toml", DELTA_MANIFEST);
        fixture.write("crates/delta/src/lib.rs", DELTA_LIB);
        fixture.write("routes.toml", FIXTURE_ROUTES);
        fixture
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn doc(&self) -> PathBuf {
        self.root().join("target/doc")
    }

    fn dist(&self) -> PathBuf {
        self.root().join("dist")
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.root().join(rel);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
    }

    /// `site.py <command> --source <fixture> <args…>`, under the lint gate `build.sh` sets.
    fn site_py(&self, command: &str, args: &[&str]) -> Output {
        Command::new("python3")
            .arg(repo_root().join("apps/logicaffeine_docs/site.py"))
            .arg(command)
            .arg("--source")
            .arg(self.root())
            .args(args)
            .current_dir(self.root())
            .env("CARGO_TARGET_DIR", self.root().join("target"))
            .env("CARGO_NET_OFFLINE", "true")
            .env("RUSTDOCFLAGS", "-D rustdoc::broken_intra_doc_links -D rustdoc::bare_urls")
            .output()
            .expect("python3 must be available to run apps/logicaffeine_docs/site.py")
    }

    /// Documents `crates` the way the deploy does — `site.py document`.
    fn document(&self, crates: &[&str]) {
        succeeded(&self.site_py("document", crates));
    }

    fn routes(&self) -> String {
        self.root().join("routes.toml").to_str().unwrap().to_string()
    }

    fn build(&self, crates: &[&str]) -> Output {
        let (out, routes) = (self.dist(), self.routes());
        let mut args = vec!["--out", out.to_str().unwrap(), "--routes", &routes];
        args.extend_from_slice(crates);
        self.site_py("build", &args)
    }

    fn assemble_with(&self, crates: &[&str], extra: &[&str]) -> Output {
        let (doc, out, routes) = (self.doc(), self.dist(), self.routes());
        let mut args = vec!["--doc", doc.to_str().unwrap(), "--out", out.to_str().unwrap(), "--routes", &routes];
        args.extend_from_slice(extra);
        args.extend_from_slice(crates);
        self.site_py("assemble", &args)
    }

    fn assemble(&self, crates: &[&str]) -> Output {
        self.assemble_with(crates, &[])
    }

    fn read(&self, rel: &str) -> String {
        fs::read_to_string(self.dist().join(rel))
            .unwrap_or_else(|e| panic!("read dist/{rel}: {e}"))
    }
}

fn succeeded(out: &Output) {
    assert!(
        out.status.success(),
        "site.py failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

fn refused(out: &Output) -> String {
    assert!(
        !out.status.success(),
        "site.py should have refused, but succeeded:\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
    String::from_utf8_lossy(&out.stderr).into_owned()
}

/// alpha and beta built into a site by `site.py build`, as the deploy builds the real crates.
fn site() -> Fixture {
    let fixture = Fixture::new();
    succeeded(&fixture.build(&["alpha", "beta"]));
    fixture
}

/// Every file under `root`, as `/`-separated paths relative to it.
fn files(root: &Path) -> BTreeSet<String> {
    fn walk(root: &Path, dir: &Path, out: &mut BTreeSet<String>) {
        for entry in fs::read_dir(dir).unwrap_or_else(|e| panic!("read_dir {}: {e}", dir.display())) {
            let path = entry.unwrap().path();
            if path.is_dir() {
                walk(root, &path, out);
            } else {
                let rel = path.strip_prefix(root).unwrap();
                let parts: Vec<_> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
                out.insert(parts.join("/"));
            }
        }
    }
    let mut out = BTreeSet::new();
    walk(root, root, &mut out);
    out
}

fn html_pages(root: &Path) -> Vec<String> {
    files(root).into_iter().filter(|f| f.ends_with(".html")).collect()
}

// ---------------------------------------------------------------------------------------------
// A model of how Cloudflare Pages answers a request for a static deployment.
// ---------------------------------------------------------------------------------------------

#[derive(Debug, PartialEq)]
enum Route {
    /// A file, served with 200.
    File(PathBuf),
    /// A 308 to another address on the site.
    Redirect(String),
    /// Nothing here: the nearest `404.html`, served with 404.
    NotFound(PathBuf),
    /// Nothing here, and no top-level `404.html`: Pages assumes a single-page app and serves
    /// `/index.html` with 200 — the defect this file exists to keep dead.
    SpaFallback,
}

/// How Pages answers a request for `path` (absolute, percent-decoded, no query or fragment).
/// `x.html` redirects to `x`, `dir/index.html` to `dir/`, `x` serves `x.html`, `dir` redirects
/// to `dir/`, and an address with no file gets the nearest `404.html` up the tree.
fn route(dist: &Path, path: &str) -> Route {
    let rel = path.strip_prefix('/').unwrap_or_else(|| panic!("route wants an absolute path, got {path:?}"));
    if let Some(stem) = rel.strip_suffix(".html") {
        if dist.join(rel).is_file() {
            return match stem.strip_suffix("index") {
                Some(dir) if dir.is_empty() || dir.ends_with('/') => Route::Redirect(format!("/{dir}")),
                _ => Route::Redirect(format!("/{stem}")),
            };
        }
        return not_found(dist, rel);
    }
    if rel.is_empty() || rel.ends_with('/') {
        let index = dist.join(rel).join("index.html");
        if index.is_file() {
            return Route::File(index);
        }
        let bare = rel.trim_end_matches('/');
        if !bare.is_empty() && dist.join(format!("{bare}.html")).is_file() {
            return Route::Redirect(format!("/{bare}"));
        }
        return not_found(dist, rel);
    }
    let exact = dist.join(rel);
    if exact.is_file() {
        return Route::File(exact);
    }
    let page = dist.join(format!("{rel}.html"));
    if page.is_file() {
        return Route::File(page);
    }
    if dist.join(rel).join("index.html").is_file() {
        return Route::Redirect(format!("/{rel}/"));
    }
    not_found(dist, rel)
}

fn not_found(dist: &Path, rel: &str) -> Route {
    let mut dir = match rel.rfind('/') {
        Some(i) => rel[..i].to_string(),
        None => String::new(),
    };
    loop {
        let candidate = dist.join(&dir).join("404.html");
        if candidate.is_file() {
            return Route::NotFound(candidate);
        }
        if dir.is_empty() {
            return Route::SpaFallback;
        }
        dir = match dir.rfind('/') {
            Some(i) => dir[..i].to_string(),
            None => String::new(),
        };
    }
}

/// Follows redirects from `path` to the file Pages finally serves.
fn serve(dist: &Path, path: &str) -> Result<PathBuf, String> {
    let mut current = path.to_string();
    for _ in 0..8 {
        match route(dist, &current) {
            Route::File(file) => return Ok(file),
            Route::Redirect(next) => current = next,
            Route::NotFound(page) => return Err(format!("{current} is a 404 ({})", page.display())),
            Route::SpaFallback => return Err(format!("{current} falls back to /index.html with a 200")),
        }
    }
    Err(format!("{path} redirects more than 8 times"))
}

/// The address Pages serves the file at `rel` (relative to the site root) under.
fn canonical(rel: &str) -> String {
    if rel == "index.html" {
        "/".to_string()
    } else if let Some(dir) = rel.strip_suffix("/index.html") {
        format!("/{dir}/")
    } else if let Some(stem) = rel.strip_suffix(".html") {
        format!("/{stem}")
    } else {
        format!("/{rel}")
    }
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok();
            if let Some(byte) = hex.and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(byte);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8(out).expect("percent-decoded path is UTF-8")
}

/// Resolves `reference`, as written in the page served at `page_url`, to an absolute site path
/// with no query or fragment. `None` for a reference that leaves the site (a scheme, `//host`) or
/// only moves within the page (`#frag`, `?query`, empty).
fn resolve(page_url: &str, reference: &str) -> Option<Result<String, String>> {
    let reference = reference.trim();
    if reference.starts_with("//") {
        return None;
    }
    if let Some(colon) = reference.find(':') {
        if !reference[..colon].contains(['/', '?', '#']) {
            return None;
        }
    }
    let end = reference.find(['?', '#']).unwrap_or(reference.len());
    let path = &reference[..end];
    if path.is_empty() {
        return None;
    }
    let joined = if path.starts_with('/') {
        path.to_string()
    } else {
        let base = &page_url[..=page_url.rfind('/').expect("page urls are absolute")];
        format!("{base}{path}")
    };
    let trailing_slash = joined.ends_with('/') || joined.ends_with("/.") || joined.ends_with("/..");
    let mut segments: Vec<&str> = Vec::new();
    for segment in joined.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if segments.pop().is_none() {
                    return Some(Err(format!("{reference} climbs above the site root from {page_url}")));
                }
            }
            s => segments.push(s),
        }
    }
    let mut resolved = format!("/{}", segments.join("/"));
    if trailing_slash && resolved != "/" {
        resolved.push('/');
    }
    Some(Ok(percent_decode(&resolved)))
}

// ---------------------------------------------------------------------------------------------
// A small HTML reader: start tags and their attributes, markup only.
// ---------------------------------------------------------------------------------------------

type Tag = (String, Vec<(String, String)>);

/// Every start tag in `html`, lowercased names, attribute values with `&amp;` decoded. Comments
/// and the bodies of `<script>` and `<style>` are skipped.
fn tags(html: &str) -> Vec<Tag> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(lt) = rest.find('<') {
        rest = &rest[lt..];
        if let Some(after) = rest.strip_prefix("<!--") {
            rest = after.find("-->").map_or("", |end| &after[end + 3..]);
            continue;
        }
        let Some((name, attrs, len)) = start_tag(rest) else {
            rest = &rest[1..];
            continue;
        };
        rest = &rest[len..];
        let raw = name == "script" || name == "style";
        out.push((name.clone(), attrs));
        if raw {
            let close = format!("</{name}");
            rest = rest.to_ascii_lowercase().find(&close).map_or("", |i| &rest[i..]);
        }
    }
    out
}

/// The start tag at the head of `s`: name, attributes, and its length in bytes.
fn start_tag(s: &str) -> Option<(String, Vec<(String, String)>, usize)> {
    let b = s.as_bytes();
    let mut i = 1;
    if i >= b.len() || !b[i].is_ascii_alphabetic() {
        return None;
    }
    while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'-') {
        i += 1;
    }
    let name = s[1..i].to_ascii_lowercase();
    let mut attrs = Vec::new();
    loop {
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        match *b.get(i)? {
            b'>' => return Some((name, attrs, i + 1)),
            b'/' | b'=' => {
                i += 1;
                continue;
            }
            _ => {}
        }
        let start = i;
        while i < b.len() && !b[i].is_ascii_whitespace() && !matches!(b[i], b'=' | b'>' | b'/') {
            i += 1;
        }
        let attr = s[start..i].to_ascii_lowercase();
        while i < b.len() && b[i].is_ascii_whitespace() {
            i += 1;
        }
        let mut value = String::new();
        if b.get(i) == Some(&b'=') {
            i += 1;
            while i < b.len() && b[i].is_ascii_whitespace() {
                i += 1;
            }
            let quote = *b.get(i)?;
            if quote == b'"' || quote == b'\'' {
                let v = i + 1;
                i = v + s[v..].find(quote as char)?;
                value = s[v..i].to_string();
                i += 1;
            } else {
                let v = i;
                while i < b.len() && !b[i].is_ascii_whitespace() && b[i] != b'>' {
                    i += 1;
                }
                value = s[v..i].to_string();
            }
        }
        attrs.push((attr, value.replace("&amp;", "&")));
    }
}

fn attr<'a>(tag: &'a Tag, name: &str) -> Option<&'a str> {
    tag.1.iter().find(|(n, _)| n == name).map(|(_, v)| v.as_str())
}

/// Every address `html` asks a browser to load or follow: `href` and `src` on any tag, and the
/// target of a `<meta http-equiv="refresh">`.
fn references(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    for tag in tags(html) {
        for name in ["href", "src"] {
            if let Some(value) = attr(&tag, name) {
                out.push(value.to_string());
            }
        }
        if let Some(target) = refresh_target(&tag) {
            out.push(target);
        }
    }
    out
}

fn refresh_target(tag: &Tag) -> Option<String> {
    if tag.0 != "meta" || !attr(tag, "http-equiv")?.eq_ignore_ascii_case("refresh") {
        return None;
    }
    let content = attr(tag, "content")?;
    let at = content.to_ascii_lowercase().find("url=")?;
    Some(content[at + 4..].trim().to_string())
}

fn is_redirect_stub(html: &str) -> bool {
    tags(html).iter().any(|tag| refresh_target(tag).is_some())
}

/// Sitemap `<loc>` values, in order.
fn locs(xml: &str) -> Vec<String> {
    xml.split("<loc>")
        .skip(1)
        .map(|chunk| chunk[..chunk.find("</loc>").expect("closed <loc>")].replace("&amp;", "&"))
        .collect()
}

// ---------------------------------------------------------------------------------------------
// The model itself.
// ---------------------------------------------------------------------------------------------

#[test]
fn the_route_model_follows_pages_rules() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for rel in ["index.html", "a/index.html", "a/b.html", "x.js", "c.html", "d/index.html", "d/404.html"] {
        fs::create_dir_all(root.join(rel).parent().unwrap()).unwrap();
        fs::write(root.join(rel), rel).unwrap();
    }

    assert_eq!(route(root, "/"), Route::File(root.join("index.html")));
    assert_eq!(route(root, "/index.html"), Route::Redirect("/".into()));
    assert_eq!(route(root, "/a/index.html"), Route::Redirect("/a/".into()));
    assert_eq!(route(root, "/a"), Route::Redirect("/a/".into()));
    assert_eq!(route(root, "/a/"), Route::File(root.join("a/index.html")));
    assert_eq!(route(root, "/a/b.html"), Route::Redirect("/a/b".into()));
    assert_eq!(route(root, "/a/b"), Route::File(root.join("a/b.html")));
    assert_eq!(route(root, "/c/"), Route::Redirect("/c".into()));
    assert_eq!(route(root, "/x.js"), Route::File(root.join("x.js")));
    assert_eq!(route(root, "/d/missing"), Route::NotFound(root.join("d/404.html")));
    assert_eq!(route(root, "/a/missing"), Route::SpaFallback, "no top-level 404.html: Pages goes SPA");
    assert_eq!(route(root, "/a/missing.html"), Route::SpaFallback);

    fs::write(root.join("404.html"), "404").unwrap();
    assert_eq!(route(root, "/a/missing"), Route::NotFound(root.join("404.html")));
    assert_eq!(route(root, "/missing/"), Route::NotFound(root.join("404.html")));
    assert_eq!(route(root, "/d/deeper/missing"), Route::NotFound(root.join("d/404.html")));

    assert_eq!(serve(root, "/a/b.html"), Ok(root.join("a/b.html")));
    assert_eq!(serve(root, "/a"), Ok(root.join("a/index.html")));
    assert!(serve(root, "/nope").is_err());
}

#[test]
fn reference_resolution_is_relative_to_the_served_address() {
    let r = |page: &str, reference: &str| resolve(page, reference);
    assert_eq!(r("/alpha/trait.Greet", "../static.files/a.css"), Some(Ok("/static.files/a.css".into())));
    assert_eq!(r("/alpha/", "trait.Greet.html"), Some(Ok("/alpha/trait.Greet.html".into())));
    assert_eq!(r("/alpha/inner/", "../index.html#x"), Some(Ok("/alpha/index.html".into())));
    assert_eq!(r("/alpha/", "./"), Some(Ok("/alpha/".into())));
    assert_eq!(r("/alpha/", ".."), Some(Ok("/".into())));
    assert_eq!(r("/alpha/", "/beta/"), Some(Ok("/beta/".into())));
    assert_eq!(r("/alpha/", "macro.shout%21.html"), Some(Ok("/alpha/macro.shout!.html".into())));
    assert_eq!(r("/alpha/", "x.html?search=y"), Some(Ok("/alpha/x.html".into())));
    assert_eq!(r("/alpha/", "#section"), None);
    assert_eq!(r("/alpha/", "?search=y"), None);
    assert_eq!(r("/alpha/", "https://doc.rust-lang.org/std/"), None);
    assert_eq!(r("/alpha/", "//cdn.example/x.js"), None);
    assert_eq!(r("/alpha/", "mailto:x@example.com"), None);
    assert!(matches!(r("/alpha/", "../../x.html"), Some(Err(_))));
}

#[test]
fn the_html_reader_sees_markup_only() {
    let html = r#"<!DOCTYPE html><html><head>
        <meta http-equiv="refresh" content="0;URL=../../alpha/struct.Secret.html">
        <link rel="stylesheet" href="a.css"><script src='b.js'></script>
        <script>var s = '<a href="in-script.html">';</script>
        <style>a[href="in-style.html"] {}</style>
        <!-- <a href="in-comment.html"> -->
        </head><body><a class=x href=c.html>c</a><img src="d.png"/>
        <pre>&lt;a href=&quot;escaped.html&quot;&gt;</pre>
        <a href="e.html?q=1&amp;r=2">e</a></body></html>"#;
    assert_eq!(
        references(html),
        vec!["../../alpha/struct.Secret.html", "a.css", "b.js", "c.html", "d.png", "e.html?q=1&r=2"]
    );
    assert!(is_redirect_stub(html));
    assert!(!is_redirect_stub("<html><head><meta charset=\"utf-8\"></head></html>"));
}

// ---------------------------------------------------------------------------------------------
// The site.
// ---------------------------------------------------------------------------------------------

#[test]
fn the_search_index_directory_is_a_404_not_the_landing_page() {
    let site = site();
    let dist = site.dist();
    let not_found = Route::NotFound(dist.join("404.html"));
    for path in [
        "/search.index/",
        "/search.index",
        "/static.files/",
        "/trait.impl/",
        "/src/",
        "/no/such/page",
        "/alpha/no-such-item",
        "/alpha/no-such-item.html",
    ] {
        assert_eq!(route(&dist, path), not_found, "{path}");
    }
}

#[test]
fn the_404_page_is_its_own_page_and_is_never_indexed() {
    let site = site();
    let page = site.read("404.html");
    assert_ne!(page, site.read("index.html"), "the 404 page must not be the landing page");
    assert!(page.contains(r#"<meta name="robots" content="noindex">"#), "{page}");
    assert!(page.to_lowercase().contains("<title>page not found"), "{page}");

    let refs = references(&page);
    assert!(refs.iter().any(|r| r == "/"), "the 404 page links home: {refs:?}");
    for crate_name in ["alpha", "beta"] {
        assert!(refs.contains(&format!("/{crate_name}/")), "the 404 page links {crate_name}: {refs:?}");
    }
    for reference in refs.iter().filter(|r| !r.contains("://")) {
        assert!(
            reference.starts_with('/'),
            "the 404 page is served at every depth, so its links must be root-relative: {reference}"
        );
        assert!(
            matches!(route(&site.dist(), reference), Route::File(_)),
            "404 link {reference} must be a canonical address, got {:?}",
            route(&site.dist(), reference)
        );
    }
}

/// Every reference in every page the site does not serve, as `(page, reference)`.
fn unserved(dist: &Path) -> (Vec<(String, String)>, usize) {
    let mut checked = 0;
    let mut unserved = Vec::new();
    for page in html_pages(dist) {
        let html = fs::read_to_string(dist.join(&page)).unwrap();
        let page_url = if page == "404.html" { "/no/such/page".to_string() } else { canonical(&page) };
        for reference in references(&html) {
            match resolve(&page_url, &reference) {
                None => {}
                Some(Err(e)) => unserved.push((page.clone(), format!("{reference} ({e})"))),
                Some(Ok(path)) => {
                    checked += 1;
                    if serve(dist, &path).is_err() {
                        unserved.push((page.clone(), reference));
                    }
                }
            }
        }
    }
    (unserved, checked)
}

/// The implementors script rustdoc names on `Lonely`'s page but never writes: nothing outside
/// alpha implements the trait.
const LONELY_SCRIPT: &str = "../trait.impl/alpha/trait.Lonely.js";

#[test]
fn every_reference_in_every_page_resolves_but_the_two_rustdoc_answers_for() {
    let site = site();
    let (unserved, checked) = unserved(&site.dist());
    let ours: Vec<String> = unserved
        .iter()
        .filter(|(page, reference)| {
            !(page == "alpha/trait.Lonely.html" && reference == LONELY_SCRIPT) && reference != FOREIGN_LINK
        })
        .map(|(page, reference)| format!("{page} → {reference}"))
        .collect();
    assert!(ours.is_empty(), "dangling references:\n{}", ours.join("\n"));
    assert!(
        unserved.iter().any(|(_, r)| r == LONELY_SCRIPT),
        "the fixture no longer exercises an implementors script rustdoc does not write"
    );
    assert!(
        unserved.iter().any(|(_, r)| r == FOREIGN_LINK),
        "the fixture no longer exercises a foreign link rustdoc copies onto our pages"
    );
    assert!(checked > 100, "only {checked} references checked — is the HTML reader seeing the pages?");
}

#[test]
fn a_trait_nobody_else_implements_loads_an_implementors_script_that_is_an_honest_404() {
    let site = site();
    let html = site.read("alpha/trait.Lonely.html");
    assert!(references(&html).iter().any(|r| r == LONELY_SCRIPT), "{html}");
    let path = resolve(&canonical("alpha/trait.Lonely.html"), LONELY_SCRIPT).unwrap().unwrap();
    assert_eq!(route(&site.dist(), &path), Route::NotFound(site.dist().join("404.html")));
    let tag = tags(&html).into_iter().find(|tag| attr(tag, "src") == Some(LONELY_SCRIPT)).unwrap();
    assert!(
        tag.1.iter().any(|(name, _)| name == "async"),
        "rustdoc loads implementor scripts async — the page does not wait on the 404"
    );
}

#[test]
fn documentation_copied_from_a_foreign_trait_keeps_its_foreign_links() {
    let site = site();
    let page = "alpha/inner/struct.Widget.html";
    let html = site.read(page);
    assert!(
        html.contains("impl&lt;T&gt; Tagged for T"),
        "rustdoc shows delta's blanket impl on alpha's types:\n{html}"
    );
    assert!(references(&html).iter().any(|r| r == FOREIGN_LINK), "{html}");
    assert!(!site.dist().join("delta").exists(), "delta is a dependency, never documented");
}

#[test]
fn our_own_trait_docs_may_not_carry_dead_links_even_inside_an_impl() {
    let fixture = Fixture::new();
    fixture.write(
        "crates/alpha/src/lib.rs",
        &format!(
            "{ALPHA_LIB}\n/// Ours.\npub trait Local {{\n    /// See [nowhere](nowhere/index.html).\n    fn local(&self) {{}}\n}}\n\nimpl<T> Local for T {{}}\n"
        ),
    );
    let stderr = refused(&fixture.build(&["alpha", "beta"]));
    assert!(stderr.contains("alpha/inner/struct.Widget.html → nowhere/index.html"), "{stderr}");
    assert!(!fixture.dist().exists());
}

#[test]
fn every_curated_crates_doc_directory_exists_before_rustdoc_starts() {
    use std::os::unix::fs::PermissionsExt;
    let fixture = Fixture::new();
    let seen = fixture.root().join("seen-by-cargo-doc.txt");
    let real = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    fixture.write(
        "cargo-shim.sh",
        &format!(
            "#!/bin/sh\nif [ \"$1\" = doc ]; then ls \"$CARGO_TARGET_DIR/doc\" > '{}'; fi\nexec '{real}' \"$@\"\n",
            seen.display()
        ),
    );
    let shim = fixture.root().join("cargo-shim.sh");
    fs::set_permissions(&shim, fs::Permissions::from_mode(0o755)).unwrap();
    fixture.write("target/doc/stale/index.html", "a crate from an earlier build");

    let out = Command::new("python3")
        .arg(repo_root().join("apps/logicaffeine_docs/site.py"))
        .args(["document", "--source"])
        .arg(fixture.root())
        .args(["beta", "alpha"])
        .current_dir(fixture.root())
        .env("CARGO", &shim)
        .env("CARGO_TARGET_DIR", fixture.root().join("target"))
        .env("CARGO_NET_OFFLINE", "true")
        .output()
        .expect("python3");
    succeeded(&out);
    assert_eq!(
        fs::read_to_string(&seen).expect("the shim saw `cargo doc` run"),
        "alpha\nbeta\n",
        "rustdoc links a crate's items only if its doc directory exists when rustdoc starts — every \
         curated crate's must, and nothing stale may"
    );
}

#[test]
fn the_real_curated_crates_are_checked_in_parallel_then_documented_one_at_a_time() {
    let root = repo_root();
    let crates = curated_crates();
    let out = Command::new("python3")
        .arg(root.join("apps/logicaffeine_docs/site.py"))
        .args(["document", "--dry-run", "--source"])
        .arg(&root)
        .args(&crates)
        .output()
        .expect("python3");
    succeeded(&out);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    assert_eq!(
        lines.len(),
        2,
        "one check, then one doc — single invocations, so features unify across the whole curated set:\n{stdout}"
    );
    let check: Vec<&str> = lines[0].split_whitespace().collect();
    let doc: Vec<&str> = lines[1].split_whitespace().collect();
    assert_eq!(&check[..2], ["check", "--lib"], "the parallel warm-up builds what rustdoc needs:\n{stdout}");
    assert_eq!(
        &doc[..5],
        ["doc", "--no-deps", "--lib", "--jobs", "1"],
        "one rustdoc at a time, so no crate's doc directory is missing while another rustdoc starts:\n{stdout}"
    );
    let expected: Vec<String> = crates.iter().map(|c| c.replace('_', "-")).collect();
    for args in [&check, &doc] {
        let packages: Vec<&str> = args.windows(2).filter(|w| w[0] == "-p").map(|w| w[1]).collect();
        assert_eq!(packages, expected, "every curated crate, and nothing else, in curated order");
    }
}

#[test]
fn every_cross_crate_type_links_to_its_documentation() {
    let site = site();
    let html = site.read("beta/struct.Gadget.html");
    assert!(
        references(&html).iter().any(|r| r == "../alpha/trait.Greet.html"),
        "beta's `impl Greet for Gadget` must link alpha's trait:\n{html}"
    );
    let landing = site.read("beta/index.html");
    assert!(!landing.contains("href=\"alpha"), "an unresolved cross-crate link fell back to a raw URL");
}

#[test]
fn only_libraries_are_documented() {
    let site = site();
    let crates_js = site.read("crates.js");
    assert!(crates_js.contains(r#"window.ALL_CRATES = ["alpha","beta"];"#), "{crates_js}");
    assert!(!site.dist().join("beta_tool").exists(), "beta's binary is not a documented crate");
}

/// The curated list, as `build.sh` declares it.
fn curated_crates() -> Vec<String> {
    let build = fs::read_to_string(repo_root().join("apps/logicaffeine_docs/build.sh")).unwrap();
    let start = build.find("CRATES=(").expect("build.sh declares CRATES=(") + "CRATES=(".len();
    let end = start + build[start..].find(')').unwrap();
    build[start..end]
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(str::to_string)
        .collect()
}

#[test]
fn rustdoc_runtime_assets_named_by_every_page_ship() {
    let site = site();
    let dist = site.dist();
    let mut checked = 0;
    for page in html_pages(&dist) {
        let html = fs::read_to_string(dist.join(&page)).unwrap();
        for tag in tags(&html) {
            if tag.0 != "meta" || attr(&tag, "name") != Some("rustdoc-vars") {
                continue;
            }
            let static_root = attr(&tag, "data-static-root-path").expect("rustdoc-vars names its static root");
            for (name, value) in &tag.1 {
                if !(name.ends_with("-js") || name.ends_with("-css")) || value.is_empty() {
                    continue;
                }
                let path = resolve(&canonical(&page), &format!("{static_root}{value}"))
                    .expect("a site-relative asset")
                    .unwrap();
                assert!(serve(&dist, &path).is_ok(), "{page}: {name}={value} → {path} does not ship");
                checked += 1;
            }
        }
    }
    assert!(checked > 0, "no rustdoc-vars assets seen");
}

#[test]
fn a_trait_page_loads_its_implementors_from_other_crates() {
    let site = site();
    let dist = site.dist();
    let page = "alpha/trait.Greet.html";
    let html = site.read(page);
    let scripts: Vec<String> = tags(&html)
        .iter()
        .filter(|tag| tag.0 == "script")
        .filter_map(|tag| attr(tag, "src"))
        .filter(|src| !src.contains("static.files"))
        .map(|src| resolve(&canonical(page), src).unwrap().unwrap())
        .collect();
    assert!(!scripts.is_empty(), "{page} loads no per-item scripts");
    let bodies: Vec<String> = scripts
        .iter()
        .map(|path| {
            let file = serve(&dist, path).unwrap_or_else(|e| panic!("{page} → {path}: {e}"));
            fs::read_to_string(file).unwrap()
        })
        .collect();
    for body in &bodies {
        assert!(!body.trim_start().starts_with('<'), "a script came back as HTML:\n{body}");
    }
    assert!(
        bodies.iter().any(|body| body.contains("Gadget")),
        "beta's `impl Greet for Gadget` must reach alpha's trait page via {scripts:?}"
    );
}

#[test]
fn ships_every_file_rustdoc_wrote_byte_for_byte() {
    let site = site();
    let (doc, dist) = (site.doc(), site.dist());
    let mut missing = Vec::new();
    for rel in files(&doc) {
        if rel.split('/').any(|part| part.starts_with('.')) {
            continue;
        }
        match fs::read(dist.join(&rel)) {
            Ok(bytes) if bytes == fs::read(doc.join(&rel)).unwrap() => {}
            Ok(_) => missing.push(format!("{rel} (altered)")),
            Err(_) => missing.push(rel),
        }
    }
    assert!(missing.is_empty(), "rustdoc output that did not ship intact: {missing:?}");
}

#[test]
fn ships_nothing_but_rustdoc_output_and_the_site_shell() {
    let site = site();
    let expected: BTreeSet<String> = files(&site.doc())
        .into_iter()
        .filter(|rel| !rel.split('/').any(|part| part.starts_with('.')))
        .chain(SHELL.iter().map(|s| s.to_string()))
        .chain(["llms/alpha.txt".to_string(), "llms/beta.txt".to_string()])
        .collect();
    let shipped = files(&site.dist());
    assert_eq!(
        shipped.difference(&expected).collect::<Vec<_>>(),
        Vec::<&String>::new(),
        "unexpected files in the site"
    );
    assert_eq!(
        expected.difference(&shipped).collect::<Vec<_>>(),
        Vec::<&String>::new(),
        "files missing from the site"
    );
}

#[test]
fn no_dotfile_ships() {
    let site = site();
    let dotfiles: Vec<String> = files(&site.dist())
        .into_iter()
        .filter(|rel| rel.split('/').any(|part| part.starts_with('.')))
        .collect();
    assert!(dotfiles.is_empty(), "dotfiles in the site (cargo's target/doc/.lock?): {dotfiles:?}");
}

#[test]
fn the_landing_page_lists_every_curated_crate_with_its_manifest_description() {
    let site = site();
    let landing = site.read("index.html");
    assert!(
        landing.contains(r#"<a href="alpha/">alpha</a> — Alpha &lt;fixture&gt; &amp; friends"#),
        "{landing}"
    );
    assert!(landing.contains(r#"<a href="beta/">beta</a> — Beta fixture"#), "{landing}");
    assert!(landing.find(">alpha<").unwrap() < landing.find(">beta<").unwrap());
    assert!(!landing.contains("gamma"), "gamma is not curated:\n{landing}");
    assert!(landing.contains(&format!(r#"<link rel="canonical" href="{ORIGIN}/">"#)), "{landing}");
    for reference in references(&landing).iter().filter(|r| !r.contains("://")) {
        let path = resolve("/", reference).unwrap().unwrap();
        assert!(
            matches!(route(&site.dist(), &path), Route::File(_)),
            "landing link {reference} must be a canonical address, got {:?}",
            route(&site.dist(), &path)
        );
    }
}

#[test]
fn the_landing_page_keeps_the_curated_order() {
    let fixture = Fixture::new();
    fixture.document(&["alpha", "beta"]);
    succeeded(&fixture.assemble(&["beta", "alpha"]));
    let landing = fixture.read("index.html");
    assert!(landing.find(">beta<").unwrap() < landing.find(">alpha<").unwrap(), "{landing}");
}

#[test]
fn robots_allows_everything_and_names_the_sitemap() {
    let site = site();
    assert_eq!(
        site.read("robots.txt"),
        format!("User-agent: *\nAllow: /\n\nSitemap: {ORIGIN}/sitemap.xml\n")
    );
}

/// The canonical address of every page the sitemap should list: the landing page and every
/// crate page that is not a redirect stub.
fn expected_sitemap(site: &Fixture, crates: &[&str]) -> BTreeSet<String> {
    let dist = site.dist();
    let mut expected = BTreeSet::from([format!("{ORIGIN}/")]);
    for page in html_pages(&dist) {
        let crate_name = page.split('/').next().unwrap();
        if !crates.contains(&crate_name) || !page.contains('/') {
            continue;
        }
        if !is_redirect_stub(&fs::read_to_string(dist.join(&page)).unwrap()) {
            expected.insert(format!("{ORIGIN}{}", canonical(&page)));
        }
    }
    expected
}

fn assert_sitemap_urls(site: &Fixture, urls: &[String]) {
    let unique: BTreeSet<String> = urls.iter().cloned().collect();
    assert_eq!(unique.len(), urls.len(), "duplicate sitemap entries");
    assert_eq!(unique, expected_sitemap(site, &["alpha", "beta"]));
    for url in urls {
        let path = url.strip_prefix(ORIGIN).unwrap_or_else(|| panic!("{url} is off-site"));
        assert!(
            matches!(route(&site.dist(), &percent_decode(path)), Route::File(_)),
            "{url} must be served directly, not via {:?}",
            route(&site.dist(), &percent_decode(path))
        );
    }
    for present in ["/", "/alpha/", "/alpha/trait.Greet", "/alpha/struct.Secret", "/alpha/macro.shout", "/beta/struct.Gadget"] {
        assert!(unique.contains(&format!("{ORIGIN}{present}")), "sitemap lacks {present}");
    }
    for absent in ["/alpha/hidden/struct.Secret", "/alpha/macro.shout!", "/404", "/help", "/settings"] {
        assert!(!unique.contains(&format!("{ORIGIN}{absent}")), "sitemap lists {absent}");
    }
    assert!(!unique.iter().any(|u| u.contains("/src/")), "sitemap lists source views");
    assert!(!unique.iter().any(|u| u.ends_with(".html")), "sitemap lists .html addresses Pages redirects");
}

#[test]
fn the_sitemap_lists_every_page_once_at_its_canonical_address() {
    let site = site();
    let xml = site.read("sitemap.xml");
    assert!(xml.starts_with(r#"<?xml version="1.0" encoding="UTF-8"?>"#), "{xml}");
    assert!(xml.contains(r#"<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">"#), "{xml}");
    assert!(xml.trim_end().ends_with("</urlset>"), "{xml}");
    assert_sitemap_urls(&site, &locs(&xml));
}

#[test]
fn a_sitemap_over_the_limit_splits_under_an_index() {
    let fixture = Fixture::new();
    fixture.document(&["alpha", "beta"]);
    succeeded(&fixture.assemble_with(&["alpha", "beta"], &["--sitemap-limit", "5"]));
    let index = fixture.read("sitemap.xml");
    assert!(index.contains(r#"<sitemapindex xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">"#), "{index}");
    let parts = locs(&index);
    assert!(parts.len() >= 2, "{index}");
    let mut urls = Vec::new();
    for (i, part) in parts.iter().enumerate() {
        assert_eq!(part, &format!("{ORIGIN}/sitemap-{}.xml", i + 1));
        let chunk = fixture.read(&format!("sitemap-{}.xml", i + 1));
        assert!(chunk.contains(r#"<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">"#), "{chunk}");
        let chunk_urls = locs(&chunk);
        assert!(!chunk_urls.is_empty() && chunk_urls.len() <= 5, "chunk {} has {} urls", i + 1, chunk_urls.len());
        urls.extend(chunk_urls);
    }
    assert_sitemap_urls(&fixture, &urls);
    assert_eq!(
        route(&fixture.dist(), &format!("/sitemap-{}.xml", parts.len() + 1)),
        Route::NotFound(fixture.dist().join("404.html"))
    );
}

#[test]
fn the_site_origin_comes_from_the_workspace_documentation_url() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        &WORKSPACE_MANIFEST.replace("https://docs.fixture.test/", "https://elsewhere.fixture.test"),
    );
    fixture.document(&["alpha", "beta"]);
    succeeded(&fixture.assemble(&["alpha", "beta"]));
    assert!(fixture.read("robots.txt").contains("Sitemap: https://elsewhere.fixture.test/sitemap.xml\n"));
    assert!(locs(&fixture.read("sitemap.xml")).iter().all(|u| u.starts_with("https://elsewhere.fixture.test/")));
}

// ---------------------------------------------------------------------------------------------
// What the assembler refuses to ship.
// ---------------------------------------------------------------------------------------------

#[test]
fn refuses_a_doc_tree_documenting_crates_off_the_list() {
    let fixture = Fixture::new();
    fixture.document(&["alpha", "beta", "gamma"]);
    let stderr = refused(&fixture.assemble(&["alpha", "beta"]));
    assert!(stderr.contains("gamma"), "{stderr}");
    assert!(!fixture.dist().exists(), "a refused build must leave no site behind");
}

#[test]
fn refuses_a_listed_crate_rustdoc_never_documented() {
    let fixture = Fixture::new();
    fixture.document(&["alpha", "beta"]);
    let stderr = refused(&fixture.assemble(&["alpha", "beta", "gamma"]));
    assert!(stderr.contains("gamma"), "{stderr}");
    assert!(!fixture.dist().exists());
}

#[test]
fn refuses_to_ship_a_dangling_reference() {
    let fixture = Fixture::new();
    fixture.document(&["alpha", "beta"]);
    let all = fs::read_to_string(fixture.doc().join("alpha/all.html")).unwrap();
    assert!(references(&all).iter().any(|r| r == "inner/struct.Widget.html"), "{all}");
    fs::remove_file(fixture.doc().join("alpha/inner/struct.Widget.html")).unwrap();

    let stderr = refused(&fixture.assemble(&["alpha", "beta"]));
    // The assembler names the missing page from whichever reference it meets first, the crate's
    // all.html or the llms map that links the same page.
    assert!(stderr.contains("inner/struct.Widget"), "{stderr}");
    assert!(!fixture.dist().exists());
}

#[test]
fn refuses_to_overwrite_rustdoc_output_with_the_site_shell() {
    let fixture = Fixture::new();
    fixture.document(&["alpha", "beta"]);
    for shell in SHELL.iter().chain(&["llms"]) {
        let path = fixture.doc().join(shell);
        fs::write(&path, "rustdoc wrote this").unwrap();
        let stderr = refused(&fixture.assemble(&["alpha", "beta"]));
        assert!(stderr.contains(shell), "{shell}: {stderr}");
        assert!(!fixture.dist().exists());
        fs::remove_file(path).unwrap();
    }
    succeeded(&fixture.assemble(&["alpha", "beta"]));
}

#[test]
fn refuses_a_curated_crate_without_a_description() {
    let fixture = Fixture::new();
    fixture.write("crates/beta/Cargo.toml", &BETA_MANIFEST.replace("description = \"Beta fixture\"\n", ""));
    fixture.document(&["alpha", "beta"]);
    let stderr = refused(&fixture.assemble(&["alpha", "beta"]));
    assert!(stderr.contains("beta") && stderr.contains("description"), "{stderr}");
    assert!(!fixture.dist().exists());
}

#[test]
fn refuses_a_workspace_without_a_documentation_url() {
    let fixture = Fixture::new();
    fixture.write(
        "Cargo.toml",
        &WORKSPACE_MANIFEST.replace("documentation = \"https://docs.fixture.test/\"\n", ""),
    );
    fixture.write("crates/alpha/Cargo.toml", &ALPHA_MANIFEST.replace("documentation.workspace = true\n", ""));
    fixture.document(&["alpha", "beta"]);
    let stderr = refused(&fixture.assemble(&["alpha", "beta"]));
    assert!(stderr.contains("documentation"), "{stderr}");
    assert!(!fixture.dist().exists());
}

#[test]
fn a_rebuild_replaces_the_previous_site() {
    let fixture = Fixture::new();
    fixture.document(&["alpha", "beta"]);
    fixture.write("dist/stale.html", "stale");
    fixture.write("dist/alpha/stale.html", "stale");
    succeeded(&fixture.assemble(&["alpha", "beta"]));
    assert!(!fixture.dist().join("stale.html").exists());
    assert!(!fixture.dist().join("alpha/stale.html").exists());
    assert!(fixture.dist().join("alpha/index.html").is_file());
}

#[test]
fn a_refused_rebuild_leaves_no_previous_site_behind() {
    let fixture = Fixture::new();
    fixture.document(&["alpha", "beta"]);
    succeeded(&fixture.assemble(&["alpha", "beta"]));
    refused(&fixture.assemble(&["alpha", "beta", "gamma"]));
    assert!(!fixture.dist().exists(), "a stale site must never survive a failed build to be deployed");
}

// ---------------------------------------------------------------------------------------------
// The map for agents: /llms.txt and one /llms/<crate>.txt per crate.
// ---------------------------------------------------------------------------------------------

/// Every `[text](url)` link in a markdown document, in order.
fn markdown_links(text: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        rest = &rest[open + 1..];
        let Some(close) = rest.find("](") else { break };
        let label = &rest[..close];
        let after = &rest[close + 2..];
        let Some(end) = after.find(')') else { break };
        if !label.contains('\n') {
            out.push((label.to_string(), after[..end].to_string()));
        }
        rest = &after[end..];
    }
    out
}

/// The site path a map URL names, asserting it is on the site.
fn site_path(url: &str) -> String {
    let path = url.strip_prefix(ORIGIN).unwrap_or_else(|| panic!("{url} is not on {ORIGIN}"));
    assert!(path.starts_with('/'), "{url}");
    percent_decode(path)
}

#[test]
fn llms_txt_is_the_front_door_for_agents() {
    let site = site();
    let text = site.read("llms.txt");
    assert!(text.starts_with("# "), "{text}");
    assert!(text.lines().any(|l| l.starts_with("> ")), "the llms.txt summary line:\n{text}");
    for heading in ["## How to navigate this site", "## Start here", "## Crates"] {
        assert!(text.contains(heading), "missing {heading}:\n{text}");
    }
    assert!(text.contains("https://logicaffeine.com/llms.txt"), "points at the language's own llms.txt");
    for expected in [
        format!("### Greeting\n- [Greet something]({ORIGIN}/alpha/trait.Greet): The trait every greeter implements."),
        format!("- [Find a widget]({ORIGIN}/alpha/inner/): Where widgets live."),
        format!("### Gadgets\n- [Build a gadget]({ORIGIN}/beta/struct.Gadget): Beta's one type."),
        format!("### [alpha]({ORIGIN}/alpha/)\nAlpha <fixture> & friends"),
        format!("- Map: [llms/alpha.txt]({ORIGIN}/llms/alpha.txt)"),
        format!("- [mod inner]({ORIGIN}/alpha/inner/): Widgets live here."),
        format!("### [beta]({ORIGIN}/beta/)\nBeta fixture"),
    ] {
        assert!(text.contains(&expected), "missing:\n{expected}\n\nin:\n{text}");
    }
    assert!(text.find("### [alpha]").unwrap() < text.find("### [beta]").unwrap(), "curated order");
    assert!(!text.contains("gamma") && !text.contains("delta"), "only curated crates:\n{text}");
}

#[test]
fn every_link_in_the_agent_maps_is_a_page_the_site_serves() {
    let site = site();
    let dist = site.dist();
    let mut checked = 0;
    for file in ["llms.txt", "llms/alpha.txt", "llms/beta.txt"] {
        for (label, url) in markdown_links(&site.read(file)) {
            let path = site_path(&url);
            assert!(
                matches!(route(&dist, &path), Route::File(_)),
                "{file}: [{label}]({url}) must be a canonical address, got {:?}",
                route(&dist, &path)
            );
            checked += 1;
        }
    }
    assert!(checked >= 30, "only {checked} links in the maps");
}

/// The canonical address of every page of `crate_name` an agent map must list: every page under
/// the crate's directory but its redirect stubs and the all-items page.
fn crate_pages(site: &Fixture, crate_name: &str) -> BTreeSet<String> {
    let dist = site.dist();
    html_pages(&dist)
        .into_iter()
        .filter(|page| page.starts_with(&format!("{crate_name}/")) && page != &format!("{crate_name}/all.html"))
        .filter(|page| !is_redirect_stub(&fs::read_to_string(dist.join(page)).unwrap()))
        .map(|page| canonical(&page))
        .collect()
}

#[test]
fn each_crate_map_lists_every_page_of_the_crate() {
    let site = site();
    for crate_name in ["alpha", "beta"] {
        let map = site.read(&format!("llms/{crate_name}.txt"));
        let listed: BTreeSet<String> = markdown_links(&map)
            .into_iter()
            .map(|(_, url)| site_path(&url))
            .filter(|path| path.starts_with(&format!("/{crate_name}/")) && path != &format!("/{crate_name}/all"))
            .collect();
        let expected = crate_pages(&site, crate_name);
        let missing: Vec<&String> = expected.difference(&listed).collect();
        let stray: Vec<&String> = listed.difference(&expected).collect();
        assert!(missing.is_empty(), "{crate_name}'s map misses {missing:?}");
        assert!(stray.is_empty(), "{crate_name}'s map lists pages that are not the crate's: {stray:?}");
    }
    let alpha = site.read("llms/alpha.txt");
    assert!(!alpha.contains("hidden/struct.Secret"), "redirect stubs are not pages to visit");
    assert!(!alpha.contains("macro.shout!"), "redirect stubs are not pages to visit");
}

#[test]
fn the_crate_map_pairs_each_item_with_its_own_summary() {
    let site = site();
    let map = site.read("llms/alpha.txt");
    assert!(map.starts_with("# alpha"), "{map}");
    assert!(map.contains("\n> Alpha <fixture> & friends\n"), "{map}");
    assert!(
        map.contains(&format!("\n- [mod bare]({ORIGIN}/alpha/bare/)\n")),
        "an undocumented item has no summary — and must not borrow the next one's:\n{map}"
    );
    for expected in [
        format!("- [mod inner]({ORIGIN}/alpha/inner/): Widgets live here."),
        format!("- [trait Greet]({ORIGIN}/alpha/trait.Greet): Greets."),
        format!("- [trait Lonely]({ORIGIN}/alpha/trait.Lonely): A trait nothing outside `alpha` implements"),
        format!("- [macro shout]({ORIGIN}/alpha/macro.shout): A macro exported from a submodule."),
        format!("## alpha::inner\nWidgets live here.\n\n- [struct Widget]({ORIGIN}/alpha/inner/struct.Widget): A widget."),
        format!("- All items on one page: [alpha/all]({ORIGIN}/alpha/all)"),
        format!("- Every crate: [llms.txt]({ORIGIN}/llms.txt)"),
    ] {
        assert!(map.contains(&expected), "missing:\n{expected}\n\nin:\n{map}");
    }
    assert!(map.find("## alpha\n").unwrap() < map.find("## alpha::inner\n").unwrap(), "the crate root comes first");
}

#[test]
fn refuses_a_route_to_a_page_the_site_does_not_ship() {
    let fixture = Fixture::new();
    fixture.write(
        "routes.toml",
        &FIXTURE_ROUTES.replace("alpha/trait.Greet.html", "alpha/trait.Nope.html"),
    );
    let stderr = refused(&fixture.build(&["alpha", "beta"]));
    assert!(stderr.contains("Greet something") && stderr.contains("alpha/trait.Nope.html"), "{stderr}");
    assert!(!fixture.dist().exists());
}

#[test]
fn refuses_an_agent_map_that_misses_a_page() {
    let fixture = Fixture::new();
    fixture.document(&["alpha", "beta"]);
    let page = fixture.doc().join("alpha/inner/index.html");
    let html = fs::read_to_string(&page).unwrap();
    let start = html.find(r#"<dt><a class="struct" href="struct.Widget.html""#).expect("inner lists Widget");
    let end = start + html[start..].find("</dd>").unwrap() + "</dd>".len();
    fs::write(&page, format!("{}{}", &html[..start], &html[end..])).unwrap();

    let stderr = refused(&fixture.assemble(&["alpha", "beta"]));
    assert!(stderr.contains("alpha/inner/struct.Widget.html"), "{stderr}");
    assert!(!fixture.dist().exists());
}

#[test]
fn the_404_page_routes_a_lost_visitor() {
    let site = site();
    let refs = references(&site.read("404.html"));
    for expected in ["/llms.txt", "/alpha/trait.Greet", "/alpha/inner/", "/beta/struct.Gadget", "/alpha/", "/beta/"] {
        assert!(refs.iter().any(|r| r == expected), "the 404 page links {expected}: {refs:?}");
    }
}

#[test]
fn the_landing_page_points_agents_at_their_map() {
    let site = site();
    let refs = references(&site.read("index.html"));
    for expected in ["llms.txt", "alpha/trait.Greet", "alpha/inner/", "llms/alpha.txt"] {
        assert!(refs.iter().any(|r| r == expected), "the landing page links {expected}: {refs:?}");
    }
}

#[test]
fn the_real_routes_point_into_curated_crates_and_real_modules() {
    let root = repo_root();
    let crates = curated_crates();
    let routes: toml::Value =
        toml::from_str(&fs::read_to_string(root.join("apps/logicaffeine_docs/routes.toml")).unwrap()).unwrap();
    let routes = routes["route"].as_array().expect("[[route]] entries");
    assert!(routes.len() >= 20, "the start-here map is the agents' highway — {} routes is too thin", routes.len());
    let mut tasks = BTreeSet::new();
    for route in routes {
        let field = |name: &str| {
            route
                .get(name)
                .and_then(|v| v.as_str())
                .filter(|v| !v.trim().is_empty())
                .unwrap_or_else(|| panic!("route {route:?} lacks {name}"))
                .to_string()
        };
        let (task, target) = (field("task"), field("target"));
        field("section");
        field("note");
        assert!(tasks.insert(task.clone()), "task {task:?} is listed twice");
        assert!(target.ends_with(".html"), "{task}: target {target} names a rustdoc file");
        let parts: Vec<&str> = target.split('/').collect();
        assert!(crates.iter().any(|c| c == parts[0]), "{task}: {} is not a curated crate", parts[0]);
        let crate_dir = ["crates", "apps"].iter().map(|d| root.join(d).join(parts[0])).find(|p| p.is_dir()).unwrap();
        let modules = &parts[1..parts.len() - 1];
        if !modules.is_empty() {
            let rel = modules.join("/");
            let src = crate_dir.join("src");
            assert!(
                src.join(format!("{rel}.rs")).is_file() || src.join(&rel).join("mod.rs").is_file() || src.join(&rel).is_dir(),
                "{task}: module {} has no source in {}",
                modules.join("::"),
                src.display()
            );
        }
    }
}

// ---------------------------------------------------------------------------------------------
// The deploy build wires rustdoc to the assembler.
// ---------------------------------------------------------------------------------------------

/// What `build.sh`'s `export RUSTDOCFLAGS=…` line leaves in the variable, given the caller's.
fn build_rustdocflags(caller: Option<&str>) -> String {
    let build = fs::read_to_string(repo_root().join("apps/logicaffeine_docs/build.sh")).unwrap();
    let export = build
        .lines()
        .find(|line| line.starts_with("export RUSTDOCFLAGS="))
        .expect("build.sh exports RUSTDOCFLAGS");
    let mut cmd = Command::new("bash");
    cmd.arg("-c").arg(format!("{export}\nprintf '%s' \"$RUSTDOCFLAGS\""));
    match caller {
        Some(value) => cmd.env("RUSTDOCFLAGS", value),
        None => cmd.env_remove("RUSTDOCFLAGS"),
    };
    let out = cmd.output().expect("bash");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn the_deploy_build_gates_on_broken_links_unless_the_caller_opts_out() {
    let gate = "-D rustdoc::broken_intra_doc_links -D rustdoc::private_intra_doc_links -D rustdoc::bare_urls";
    assert_eq!(build_rustdocflags(None), gate, "unset: the docs-quality gate applies");
    assert_eq!(build_rustdocflags(Some("")), "", "RUSTDOCFLAGS=\"\" is the documented permissive build");
    assert_eq!(build_rustdocflags(Some("--cfg docsrs")), "--cfg docsrs", "a caller's own flags win");
}

#[test]
fn the_deploy_build_is_site_py_build_over_the_curated_crates() {
    let build = fs::read_to_string(repo_root().join("apps/logicaffeine_docs/build.sh")).unwrap();
    let code: String = build
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n");
    assert!(
        code.contains("apps/logicaffeine_docs/site.py build") && code.contains(r#""${CRATES[@]}""#),
        "build.sh must hand the curated list to `site.py build`"
    );
    assert!(
        code.contains("--routes apps/logicaffeine_docs/routes.toml"),
        "build.sh must build the agents' start-here routes"
    );
    assert!(
        !code.contains("cargo doc"),
        "site.py documents the crates, in dependency order — a bare `cargo doc` races the cross-crate links"
    );
    assert!(
        !code.contains("--workspace"),
        "documenting the whole workspace puts crates the site does not ship into its search index"
    );
    assert!(!code.contains("cp "), "rustdoc's output ships whole through site.py, never hand-picked");
}

/// A link written as inline code — `` `[Alias](URI)` ``, as a module documenting a syntax shows one — is text, not a link:
/// the agent maps carry it as the docs wrote it, and the check that every map link is a page the site ships leaves it be.
#[test]
fn a_link_written_as_code_in_the_docs_is_not_a_link_the_site_must_ship() {
    let fixture = Fixture::new();
    fixture.write(
        "crates/beta/src/lib.rs",
        &format!("{BETA_LIB}\n/// Reads the `[Alias](URI)` lines of a file's first paragraph.\npub fn reads_links() {{}}\n"),
    );
    succeeded(&fixture.build(&["alpha", "beta"]));
}
