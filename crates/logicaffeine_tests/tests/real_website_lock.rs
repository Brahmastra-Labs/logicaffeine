//! ★★★ CROWN LOCK — a real website, built and shipped both ways. ★★★
//!
//! Not a counter and not a TODO app: a site somebody would actually put on the
//! internet. A front page, an about page, a writing index that renders a list, a
//! contact form that binds a field and posts — each with its own title, description
//! and social card — plus the backend the scaffold ships.
//!
//! ★ WHY A WHOLE SITE AND NOT MORE CAPABILITY LOCKS. There are ~1,400 web tests in
//! this repo and every one of them proves a capability. This campaign's recurring
//! finding is that capabilities pass alone and fail where they MEET: a `get` form
//! navigated to a URL the router refused, a page URL the dev server answered and the
//! build never published, a module reference nothing rewrote. A whole site is the only
//! test that crosses those seams.
//!
//! ★ AND IT SHIPS BOTH WAYS, because those are two different failure surfaces. The
//! static export is a dumb file server mapping URLs to files; the binary is a process
//! that has to be reachable, serve its own assets and answer its API. A site that works
//! one way and not the other is a site with a deployment nobody can choose.
//!
//! The binary half is `#[ignore]`d beside the other deploy heavies: it compiles a real
//! release binary.

#[path = "cli_common/mod.rs"]
mod common;
use common::{largo_in, stderr};

use std::collections::HashMap;
use std::io::{Read, Write};
use std::net::{IpAddr, TcpStream, UdpSocket};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

// ── the site ───────────────────────────────────────────────────────────────────

const FRONT: &str = r#"# Front page

The site's front page. It says what Acme is and links the rest of the site.

## Page /

Head:
    <title>Acme — tools that stay out of the way</title>
    <meta name="description" content="Acme builds small, sharp tools for people who ship.">
    <meta property="og:title" content="Acme">

State:
    Let tagline be "Small, sharp tools for people who ship.".

View:
    <main>
      <h1>Acme</h1>
      <p class="lede">{tagline}</p>
      <nav>
        <a href="/about">About</a>
        <a href="/blog">Writing</a>
        <a href="/contact">Contact</a>
      </nav>
    </main>
"#;

const ABOUT: &str = r#"# About

Who Acme is.

## Page /about

Head:
    <title>About — Acme</title>
    <meta name="description" content="Three people, one workshop, since 2019.">

View:
    <main>
      <h1>About</h1>
      <p>Three people and one workshop, since 2019.</p>
      <a href="/">Home</a>
    </main>
"#;

const BLOG: &str = r#"# Writing

The list of posts. A real list, rendered from state.

## Page /blog

Head:
    <title>Writing — Acme</title>
    <meta name="description" content="Notes on tools, shipping and doing less.">

State:
    Let posts be ["On doing less", "Shipping is a habit", "Tools that vanish"].

View:
    <main>
      <h1>Writing</h1>
      <ul class="posts">
        {for each post in posts:}<li>{post}</li>{end}
      </ul>
      <a href="/">Home</a>
    </main>
"#;

const CONTACT: &str = r#"# Contact

A real form: it binds its field, and it posts.

## Page /contact

Head:
    <title>Contact — Acme</title>
    <meta name="description" content="Say hello.">

State:
    Let note be "".
    Let sent be false.

Actions:
    To send:
        Set sent to true.

View:
    <main>
      <h1>Contact</h1>
      {if sent:}
        <p id="thanks">Thank you — we will write back.</p>
      {else:}
        <form id="say" on-submit.prevent={send}>
          <label for="note">Your message</label>
          <input id="note" type="text" bind={note}>
          <button type="submit">Send</button>
        </form>
      {end}
      <a href="/">Home</a>
    </main>
"#;

/// Scaffold the project, then write the real site over it.
fn acme(dir: &Path) -> PathBuf {
    let out = largo_in(dir, &["new", "acme", "--web"]);
    assert!(out.status.success(), "scaffold: {}", stderr(&out));
    let root = dir.join("acme");
    std::fs::write(root.join("pages/app.lg"), FRONT).unwrap();
    std::fs::write(root.join("pages/about.lg"), ABOUT).unwrap();
    std::fs::write(root.join("pages/blog.lg"), BLOG).unwrap();
    std::fs::write(root.join("pages/contact.lg"), CONTACT).unwrap();
    let manifest = root.join("Largo.toml");
    let text = std::fs::read_to_string(&manifest).unwrap();
    std::fs::write(
        &manifest,
        text.replace("[package]", "[package]\nsite = \"https://acme.example\""),
    )
    .unwrap();
    root
}

// ── half one: the static export, served by a dumb file server ──────────────────

/// The `dist/` a static host would receive, as a URL → bytes map, resolved by the same
/// rule a static host applies: `/about` → `about/index.html`.
fn served(dist: &Path) -> HashMap<String, Vec<u8>> {
    fn walk(d: &Path, base: &Path, out: &mut HashMap<String, Vec<u8>>) {
        for e in std::fs::read_dir(d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                walk(&p, base, out);
                continue;
            }
            let rel = p
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .replace('\\', "/");
            let bytes = std::fs::read(&p).unwrap();
            // The host's directory rule: `about/index.html` also answers `/about`.
            if let Some(stem) = rel.strip_suffix("index.html") {
                out.insert(format!("/{}", stem.trim_end_matches('/')), bytes.clone());
            }
            out.insert(format!("/{rel}"), bytes);
        }
    }
    let mut out = HashMap::new();
    walk(dist, dist, &mut out);
    out
}

fn page(site: &HashMap<String, Vec<u8>>, url: &str) -> String {
    let bytes = site.get(url).unwrap_or_else(|| {
        let mut urls: Vec<&String> = site.keys().collect();
        urls.sort();
        panic!("a static host would 404 on {url}. It holds:\n{urls:#?}")
    });
    String::from_utf8_lossy(bytes).to_string()
}

/// ★ THE LOAD-BEARING TEST — every page of a real site answers, with its own content
/// and its own head, from files alone.
#[test]
fn the_whole_site_is_readable_from_a_static_host() {
    let dir = tempfile::tempdir().unwrap();
    let root = acme(dir.path());
    let out = largo_in(&root, &["build", "--static"]);
    assert!(
        out.status.success(),
        "`largo build --static`: {}",
        stderr(&out)
    );
    let site = served(&root.join("dist"));

    for (url, title, content) in [
        (
            "/",
            "Acme — tools that stay out of the way",
            "Small, sharp tools for people who ship.",
        ),
        ("/about", "About — Acme", "Three people and one workshop"),
        ("/blog", "Writing — Acme", "Shipping is a habit"),
        ("/contact", "Contact — Acme", "Your message"),
    ] {
        let html = page(&site, url);
        assert!(
            html.contains(&format!("<title>{title}</title>")),
            "{url} must carry its own title — this is what a tab, a bookmark and a search result \
             show:\n{html}"
        );
        assert!(
            html.contains(content),
            "{url} must carry its content in the document, before any script runs — a crawler \
             and a reader on a slow connection get only this:\n{html}"
        );
    }
}

/// A list rendered from state is in the shipped HTML, in order — not assembled later by
/// a module a crawler never runs.
#[test]
fn a_list_rendered_from_state_ships_in_the_document() {
    let dir = tempfile::tempdir().unwrap();
    let root = acme(dir.path());
    assert!(largo_in(&root, &["build", "--static"]).status.success());
    let html = page(&served(&root.join("dist")), "/blog");

    let (a, b, c) = (
        html.find("On doing less").expect("first post"),
        html.find("Shipping is a habit").expect("second post"),
        html.find("Tools that vanish").expect("third post"),
    );
    assert!(
        a < b && b < c,
        "the posts ship in the order the page declares them:\n{html}"
    );
}

/// A form ships as a form. Progressive enhancement is the whole point of pre-rendering:
/// the page works before the module loads, and a `bind` must not cost the page its
/// content (it once refused the whole render).
#[test]
fn a_form_ships_as_a_form() {
    let dir = tempfile::tempdir().unwrap();
    let root = acme(dir.path());
    assert!(largo_in(&root, &["build", "--static"]).status.success());
    let html = page(&served(&root.join("dist")), "/contact");
    assert!(html.contains("<form"), "the form itself:\n{html}");
    assert!(html.contains("id=\"note\""), "and its bound field:\n{html}");
    assert!(html.contains("Send"), "and its button:\n{html}");
}

/// ★ EVERY LINK ON THE SITE RESOLVES. A nav that 404s is the failure a person meets
/// first, and it is exactly what a page-URL divergence produces — one that this
/// campaign has already shipped twice.
#[test]
fn every_internal_link_resolves() {
    let dir = tempfile::tempdir().unwrap();
    let root = acme(dir.path());
    assert!(largo_in(&root, &["build", "--static"]).status.success());
    let site = served(&root.join("dist"));

    for url in ["/", "/about", "/blog", "/contact"] {
        let html = page(&site, url);
        for href in html
            .split("href=\"")
            .skip(1)
            .filter_map(|r| r.split('"').next())
        {
            if !href.starts_with('/') || href.starts_with("//") || href.starts_with("data:") {
                continue;
            }
            assert!(
                site.contains_key(href),
                "{url} links {href}, and a static host has no such file. A link that only works \
                 in development is the divergence a visitor finds first."
            );
        }
    }
}

/// The module a page references must be in the export under the name the page uses —
/// a content-addressed name nobody shipped is a page that loads nothing.
#[test]
fn every_page_ships_the_module_it_references() {
    let dir = tempfile::tempdir().unwrap();
    let root = acme(dir.path());
    assert!(largo_in(&root, &["build", "--static"]).status.success());
    let site = served(&root.join("dist"));

    for url in ["/", "/about", "/blog", "/contact"] {
        let html = page(&site, url);
        let shim = html
            .split("src=\"")
            .nth(1)
            .and_then(|r| r.split('"').next())
            .unwrap_or_else(|| panic!("{url} loads a module script:\n{html}"))
            .trim_start_matches('.')
            .to_string();
        assert!(
            site.contains_key(&shim),
            "{url} references {shim}, which is not in the export"
        );
        let js = String::from_utf8_lossy(&site[&shim]).to_string();
        let module = js
            .split(".wasm")
            .next()
            .and_then(|r| r.rsplit(['"', '\'', '/']).next())
            .map(|n| format!("/{n}.wasm"))
            .expect("the shim names its module");
        assert!(
            site.contains_key(&module),
            "{shim} references {module}, which is not in the export"
        );
    }
}

/// The site tells crawlers what it is: a sitemap of exactly its pages, and a robots
/// file pointing at it.
#[test]
fn the_site_tells_a_crawler_what_it_holds() {
    let dir = tempfile::tempdir().unwrap();
    let root = acme(dir.path());
    assert!(largo_in(&root, &["build", "--static"]).status.success());
    let site = served(&root.join("dist"));

    let map = page(&site, "/sitemap.xml");
    for path in ["/", "/about", "/blog", "/contact"] {
        let loc = if path == "/" {
            "<loc>https://acme.example/</loc>".to_string()
        } else {
            format!("<loc>https://acme.example{path}</loc>")
        };
        assert!(map.contains(&loc), "the sitemap must list {path}:\n{map}");
    }
    assert!(page(&site, "/robots.txt").contains("Sitemap: https://acme.example/sitemap.xml"));
}

// ── half two: the same site as one binary, reachable from the network ──────────

/// A non-loopback address of this machine — what a container's neighbour dials.
fn outward_address() -> Option<IpAddr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("203.0.113.1:9").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

fn request(addr: IpAddr, port: u16, target: &str, timeout: Duration) -> Option<String> {
    request_with(addr, port, target, "", timeout)
}

/// As [`request`], with extra request headers (each already `\r\n`-terminated).
fn request_with(
    addr: IpAddr,
    port: u16,
    target: &str,
    extra: &str,
    timeout: Duration,
) -> Option<String> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(mut s) =
            TcpStream::connect_timeout(&(addr, port).into(), Duration::from_millis(500))
        {
            let _ = s.write_all(
                format!("GET {target} HTTP/1.1\r\nHost: x\r\n{extra}Connection: close\r\n\r\n")
                    .as_bytes(),
            );
            let mut out = Vec::new();
            let _ = s.read_to_end(&mut out);
            if !out.is_empty() {
                return Some(String::from_utf8_lossy(&out).to_string());
            }
        }
        if Instant::now() >= deadline {
            return None;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// ★★★ THE DEPLOYMENT TEST — the same site as one binary, reached over the network the
/// way a container's neighbour reaches it, serving its pages, its assets and its API.
#[test]
#[ignore = "compiles a real release binary — run with --ignored beside the other deploy heavies"]
fn the_same_site_deploys_as_one_binary_and_answers_over_the_network() {
    let Some(outward) = outward_address() else {
        eprintln!("LOGOS_TOOL_SKIP: no non-loopback interface on this host");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let root = acme(dir.path());

    let out = largo_in(&root, &["build", "--release"]);
    assert!(
        out.status.success(),
        "`largo build --release`: {}",
        stderr(&out)
    );
    let bin = root.join("target/release/acme");
    assert!(
        bin.is_file(),
        "the build must leave a binary at {}",
        bin.display()
    );

    let mut binary = Command::new(&bin);
    binary
        .env("LOGOS_HOST", "0.0.0.0")
        .env("LOGOS_PORT", "0")
        .current_dir(&root);
    let served = common::http::Server::spawn(binary);
    let port = served.port();

    // Every page, over a real socket, on an address that is not loopback.
    for (target, needle) in [
        ("/", "Small, sharp tools"),
        ("/about", "one workshop"),
        ("/blog", "Shipping is a habit"),
        ("/contact", "Your message"),
    ] {
        let got = request(outward, port, target, Duration::from_secs(60));
        assert!(
            got.as_deref().is_some_and(|r| r.contains(needle)),
            "the deployed site must answer {target} on {outward}:{port} with its content.\ngot: {got:?}"
        );
    }

    // ★ A QUERIED ASSET. `/style.css?v=2` served in development and 404'd in the built
    // binary, because the asset table was gated on an empty query string — a cache
    // buster on a stylesheet is one of the most ordinary things anybody writes.
    let css = request(outward, port, "/style.css?v=2", Duration::from_secs(30));
    assert!(
        css.as_deref()
            .is_some_and(|r| r.starts_with("HTTP/1.1 200")),
        "a cache-busting query must not turn a stylesheet into a 404:\n{css:?}"
    );

    // ★ COMPRESSION ON THE WIRE. The module is the largest thing the page downloads, and
    // "the table holds a gzip variant" is not the same fact as "a client that asked for one
    // receives it, declared, with a Vary". Only a real socket answers that.
    //
    // Asked of a PAGE DOCUMENT, not the scaffold's stylesheet: that stylesheet is a dozen
    // lines, and gzip's own container is larger than the saving, so the build correctly
    // ships no variant for it. Choosing an asset too small to compress would have made
    // this assertion measure the threshold rather than the wire — my first version did
    // exactly that and failed for the wrong reason.
    let raw =
        request(outward, port, "/about", Duration::from_secs(30)).expect("the page serves at all");
    assert!(
        !raw.to_ascii_lowercase().contains("content-encoding"),
        "a client that did not ask for gzip must not be sent one:\n{raw}"
    );
    let gz = request_with(
        outward,
        port,
        "/about",
        "Accept-Encoding: gzip, deflate\r\n",
        Duration::from_secs(30),
    )
    .expect("the page serves to a gzip-capable client");
    let lower = gz.to_ascii_lowercase();
    assert!(
        lower.contains("content-encoding: gzip"),
        "a client that asked for gzip must be told what it got:\n{gz}"
    );
    assert!(
        lower.contains("vary: accept-encoding"),
        "and the response must say it depends on that header, or a shared cache hands the \
         compressed body to a client that cannot read one:\n{gz}"
    );

    // And the backend the scaffold ships still answers beside the pages.
    let api = request(outward, port, "/api/hello", Duration::from_secs(30));
    assert!(
        api.as_deref()
            .is_some_and(|r| r.starts_with("HTTP/1.1 200")),
        "the API must answer beside the site — one binary serves both:\n{api:?}"
    );
}
