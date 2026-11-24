//! `scripts/web/pages-server.py` answers a built site the way Cloudflare Pages does (DAT-171).
//!
//! The smoke, render and screenshot checks serve the SSG build through it, so each local run sees
//! the status and the document production would: `_redirects` redirects before files, Pages' HTML
//! route matching with its 308s, rewrites only where no file matches, and the nearest `404.html`
//! with a 404 status for every other path. These tests hold the server to each rule on a synthetic
//! build laid out the way `merge-ssg.sh` lays one out.
//!
//! Run: `cargo nextest run -p logicaffeine-tests --test pages_server_lock`

#![cfg(not(target_arch = "wasm32"))]

use std::io::{BufRead, BufReader, Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

const LANDING: &str = "<!DOCTYPE html><title>landing</title>";
const GUIDE: &str = "<!DOCTYPE html><title>guide</title>";
const TERMS: &str = "<!DOCTYPE html><title>terms</title>";
const NOT_FOUND: &str = "<!DOCTYPE html><title>Page not found</title>";
const NEWS_NOT_FOUND: &str = "<!DOCTYPE html><title>No such article</title>";
const STYLE: &str = ":root { --bg: #09090b; }";
const NEWS: &str = "<!DOCTYPE html><title>news</title>";
const ARTICLE: &str = "<!DOCTYPE html><title>a first article</title>";

const REDIRECTS: &str = "\
/pricing  /contact  301
/workspace/*  /index.html  200
/registry/package/*  /index.html  200
";

/// A build: the landing page, a directory page, a flat page, a stylesheet, `_redirects`, and a
/// root `404.html` when `with_not_found` holds.
fn stage(tag: &str, with_not_found: bool) -> PathBuf {
    let public = std::env::temp_dir().join(format!("pages_server_{tag}_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&public);
    std::fs::create_dir_all(public.join("guide")).unwrap();
    std::fs::create_dir_all(public.join("assets")).unwrap();
    std::fs::write(public.join("index.html"), LANDING).unwrap();
    std::fs::write(public.join("guide/index.html"), GUIDE).unwrap();
    std::fs::write(public.join("terms.html"), TERMS).unwrap();
    std::fs::write(public.join("assets/style.css"), STYLE).unwrap();
    std::fs::write(public.join("_redirects"), REDIRECTS).unwrap();
    if with_not_found {
        std::fs::write(public.join("404.html"), NOT_FOUND).unwrap();
    }
    public
}

/// The server on a free port, killed when dropped.
struct Server {
    child: Child,
    port: u16,
    public: PathBuf,
}

impl Server {
    fn start(public: PathBuf) -> Server {
        let mut child = Command::new("python3")
            .arg(repo_root().join("scripts/web/pages-server.py"))
            .arg(&public)
            .arg("0")
            .stdout(Stdio::piped())
            .spawn()
            .expect("python3 runs scripts/web/pages-server.py");
        let mut line = String::new();
        BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
        let port = line
            .trim()
            .strip_prefix("listening on ")
            .unwrap_or_else(|| panic!("the server's first line names its port: {line:?}"))
            .parse()
            .unwrap();
        Server { child, port, public }
    }

    /// The status, the `Location` header and the body the server answers `path` with.
    fn get(&self, path: &str) -> (u16, Option<String>, String) {
        let mut stream = TcpStream::connect(("127.0.0.1", self.port)).unwrap();
        write!(stream, "GET {path} HTTP/1.0\r\nHost: localhost\r\n\r\n").unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        let (head, body) = response.split_once("\r\n\r\n").unwrap_or((&response, ""));
        let status = head.split_whitespace().nth(1).unwrap().parse().unwrap();
        let location = head
            .lines()
            .find_map(|line| line.strip_prefix("Location: "))
            .map(str::to_string);
        (status, location, body.to_string())
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.public);
    }
}

fn served(server: &Server, path: &str, body: &str) {
    let (status, location, got) = server.get(path);
    assert_eq!((status, location.as_deref()), (200, None), "{path} is not served directly");
    assert_eq!(got, body, "{path} serves the wrong document");
}

fn redirected(server: &Server, path: &str, status: u16, to: &str) {
    let (got, location, _) = server.get(path);
    assert_eq!((got, location.as_deref()), (status, Some(to)), "{path} does not answer {status} to {to}");
}

fn not_found(server: &Server, path: &str, body: &str) {
    let (status, _, got) = server.get(path);
    assert_eq!(status, 404, "{path} does not answer 404");
    assert_eq!(got, body, "{path} does not serve the nearest 404.html");
}

#[test]
fn a_page_in_its_own_directory_is_served_with_its_slash() {
    let server = Server::start(stage("directory", true));
    served(&server, "/", LANDING);
    served(&server, "/guide/", GUIDE);
    redirected(&server, "/guide", 308, "/guide/");
    redirected(&server, "/guide/index.html", 308, "/guide/");
}

#[test]
fn a_flat_page_is_served_without_its_slash_or_extension() {
    let server = Server::start(stage("flat", true));
    served(&server, "/terms", TERMS);
    redirected(&server, "/terms/", 308, "/terms");
    redirected(&server, "/terms.html", 308, "/terms");
}

#[test]
fn a_file_is_served_as_itself() {
    let server = Server::start(stage("file", true));
    served(&server, "/assets/style.css", STYLE);
}

#[test]
fn a_redirect_rule_answers_before_any_file() {
    let server = Server::start(stage("redirect", true));
    redirected(&server, "/pricing", 301, "/contact");
}

#[test]
fn a_rewrite_serves_the_app_shell_below_its_source() {
    let server = Server::start(stage("rewrite", true));
    served(&server, "/workspace/sets", LANDING);
    served(&server, "/registry/package/std-text", LANDING);
}

#[test]
fn every_other_path_is_the_404_page_with_a_404_status() {
    let server = Server::start(stage("not_found", true));
    not_found(&server, "/this-route-does-not-exist-xyz", NOT_FOUND);
    not_found(&server, "/news/no-such-article", NOT_FOUND);
    not_found(&server, "/registry/no-such-page", NOT_FOUND);
}

#[test]
fn the_nearest_404_page_answers() {
    let public = stage("nearest", true);
    std::fs::create_dir_all(public.join("news")).unwrap();
    std::fs::write(public.join("news/404.html"), NEWS_NOT_FOUND).unwrap();
    let server = Server::start(public);
    not_found(&server, "/news/no-such-article", NEWS_NOT_FOUND);
    not_found(&server, "/no-such-page", NOT_FOUND);
}

/// Without a `404.html` Pages takes the site for a single-page app: every unknown path is the
/// landing page with a 200, the soft 404 DAT-171 removed.
#[test]
fn without_a_404_page_every_unknown_path_is_the_landing_page() {
    let server = Server::start(stage("spa", false));
    served(&server, "/this-route-does-not-exist-xyz", LANDING);
}

/// The server is the one each local SSG check serves the build with.
#[test]
fn every_local_ssg_check_serves_the_build_through_it() {
    for script in ["test-ssg-smoke.sh", "test-ssg-render.sh", "test-ssg-screenshots.sh"] {
        let path = repo_root().join("scripts/web").join(script);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        assert!(text.contains("scripts/web/pages-server.py"), "{script} does not serve the build through pages-server.py");
        assert!(!text.contains("SimpleHTTPRequestHandler"), "{script} keeps a server of its own");
    }
    assert!(Path::new(&repo_root().join("scripts/web/pages-server.py")).is_file());
}

/// A section page and the pages below it, written the way `merge-ssg.sh` writes them
/// (`news.html` beside `news/`), are each served at their address without a slash (DAT-170).
#[test]
fn a_section_and_its_pages_are_served_without_a_slash() {
    let public = stage("section", true);
    std::fs::create_dir_all(public.join("news")).unwrap();
    std::fs::write(public.join("news.html"), NEWS).unwrap();
    std::fs::write(public.join("news/a-first-article.html"), ARTICLE).unwrap();
    let server = Server::start(public);
    served(&server, "/news", NEWS);
    redirected(&server, "/news/", 308, "/news");
    served(&server, "/news/a-first-article", ARTICLE);
    redirected(&server, "/news/a-first-article/", 308, "/news/a-first-article");
    redirected(&server, "/news/a-first-article.html", 308, "/news/a-first-article");
    not_found(&server, "/news/no-such-article", NOT_FOUND);
}

/// The browser suites in Node serve the build with a server of their own, and each one finds a page
/// at its address with `.html` as Pages does, or every page but the landing page would be the
/// landing page there.
#[test]
fn every_node_suite_serves_a_page_at_its_address() {
    for script in ["test-site-e2e.mjs", "test-studio-e2e.mjs", "test-guide-e2e.mjs", "test-learn-e2e.mjs"] {
        let path = repo_root().join("scripts/web").join(script);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        assert!(
            text.contains("if (!fs.existsSync(file) && fs.existsSync(`${file}.html`)) file = `${file}.html`;"),
            "{script} does not serve a page at its address with .html"
        );
    }
}
