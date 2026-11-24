//! ★ LOCK — what the dev server serves, the shipped binary serves. ★
//!
//! This campaign has now found FIVE divergences of one shape: the development
//! server and the built site answered the same request differently, and the
//! difference only ever showed up in production. `/index.html`, the pre-rendered
//! body, the page URL namespace, the MIME table, the module reference. Each was
//! found by a person, late.
//!
//! ★ THE ONE THIS FILE CLOSES: **a query string.** `largo serve` splits the query
//! off before matching (`serve_web::route_local`), and the generated
//! `logos_assets::lookup` splits it off too — but the emitted `Serve` gated the
//! whole asset table on `__raw.query.is_empty()`, so the split inside `lookup`
//! was unreachable. `/style.css?v=2` served in development and **404'd in
//! production**, and a cache-busting query on a stylesheet is one of the most
//! ordinary things anybody writes.
//!
//! ★ ON THE MEASURE. Proving this on the wire needs a release binary, which is a
//! multi-minute cargo build; that proof lives in the crown site lock beside the
//! other deploy heavies, which asks for a queried asset over a real socket. What
//! is asserted HERE is the thing that regresses: the guard itself. It is a scan,
//! and a scan is honest about a guard's *absence* in a way it is not about
//! behaviour — the guard is one literal expression, and its return would be
//! visible right here.

/// A site whose `Serve` carries an embedded asset table — the shape
/// `largo build` emits for a `--web` project.
const SERVER_LG: &str = "# Site\n\n\
    ## To handle (request: Request) -> Response:\n\
    \x20   Return plain(\"hi\").\n\n\
    ## Main\n\n\
    Serve on port 8080 with handle.\n";

/// The generated Rust for a web build (asset embedding ON, as `largo build`
/// turns it on for a project with `pages/` or `static/`).
fn embedded_serve_rust() -> String {
    logicaffeine_compile::codegen::with_embed_assets(|| {
        logicaffeine_compile::compile_to_rust(SERVER_LG).expect("the server compiles")
    })
}

/// ★ THE LOAD-BEARING TEST.
#[test]
fn a_queried_asset_is_not_gated_out_of_the_shipped_asset_table() {
    let rust = embedded_serve_rust();
    assert!(
        rust.contains("logos_assets::lookup"),
        "the embedded-asset lowering must be present for this test to mean anything:\n{rust}"
    );
    assert!(
        !rust.contains("query.is_empty()"),
        "the shipped asset table must not be gated on an empty query string. With that guard, \
         `/style.css?v=2` served in `largo serve` and 404'd in the built binary — a cache-busting \
         query is ordinary, and the split inside `logos_assets::lookup` was written for exactly \
         this and made unreachable by the guard.\n{rust}"
    );
}

/// The method gate is a DIFFERENT question and must survive: a `POST` to a path
/// an asset happens to occupy belongs to the handler, not to a static file.
#[test]
fn only_a_get_is_answered_from_the_asset_table() {
    let rust = embedded_serve_rust();
    assert!(
        rust.contains("__raw.method == \"GET\""),
        "assets answer GET only — a POST to the same path is the handler's, and dropping this \
         guard would silently shadow every form that posts to a path a file also occupies:\n{rust}"
    );
}

/// And the asset lookup keeps its own `?`-split, which is what makes the guard's
/// removal safe: the path a wire request carries is already query-free, but a
/// caller that passes a raw target still lands on the right asset.
///
/// This reads the OTHER generated artifact — the `logos_assets` module itself,
/// not the `Serve` lowering — because that is where the split lives.
#[test]
fn the_asset_lookup_still_splits_a_query_off_itself() {
    let dir = tempfile::tempdir().unwrap();
    let pages = dir.path().join("pages");
    let statics = dir.path().join("static");
    std::fs::create_dir_all(&pages).unwrap();
    std::fs::create_dir_all(&statics).unwrap();
    std::fs::write(statics.join("style.css"), b".card{}").unwrap();

    let module = logicaffeine_cli::assets_module(&pages, &statics).expect("the table generates");
    assert!(
        module.contains("/style.css"),
        "the table must hold the asset for this test to mean anything:\n{module}"
    );
    assert!(
        module.contains("split('?')"),
        "`logos_assets::lookup` must keep splitting the query off, so the table answers by PATH \
         however the caller spells the target:\n{module}"
    );
}

// ── The URLs a static host answers for a directory ───────────────────────────────────────────────
//
// A static host serves `apps/index.html` for `/apps` AND `/apps/`, and `tour/index.html` for `/tour`
// and `/tour/`: that is the whole of how it maps a URL to a file. `largo serve` answered neither the
// trailing slash nor a `static/` directory's own `index.html`, so a link that worked on the built
// site 404'd in development. Development answers every URL the built site answers.

#[path = "cli_common/mod.rs"]
mod common;

/// A site with a front page, a page at `/apps`, a hand-written `static/tour/index.html`, and a
/// static folder with no index.
fn directory_site(dir: &std::path::Path) -> std::path::PathBuf {
    let root = dir.join("dirs");
    for (rel, text) in [
        ("Largo.toml", "[package]\nname = \"dirs\"\nversion = \"0.1.0\"\n"),
        ("pages/app.lg", "## Page /\n\nView:\n    <main><h1>front page</h1></main>\n"),
        ("pages/apps.lg", "## Page /apps\n\nView:\n    <main><h1>the apps page</h1></main>\n"),
        ("static/tour/index.html", "<!doctype html><title>tour</title><p>the tour, by hand</p>\n"),
        ("static/notes/readme.txt", "no index here\n"),
    ] {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    root
}

fn shown(raw: &[u8]) -> String {
    String::from_utf8_lossy(raw).into_owned()
}

#[test]
fn the_dev_server_answers_a_page_with_a_trailing_slash() {
    let dir = tempfile::tempdir().unwrap();
    let root = directory_site(dir.path());
    let mut server = common::http::Server::start(&root);
    let raw = server.get("/apps/");
    assert_eq!(common::http::status(&raw), 200, "`/apps/` is the page at `/apps`:\n{}", shown(&raw));
    assert!(common::http::body(&raw).contains("the apps page"), "{}", shown(&raw));
    let front = server.get("/");
    assert!(common::http::body(&front).contains("front page"), "and `/` is still the front page");
}

#[test]
fn the_dev_server_answers_a_static_directory_with_its_index() {
    let dir = tempfile::tempdir().unwrap();
    let root = directory_site(dir.path());
    let mut server = common::http::Server::start(&root);
    for url in ["/tour", "/tour/", "/tour/index.html"] {
        let raw = server.get(url);
        assert_eq!(common::http::status(&raw), 200, "`{url}` is static/tour/index.html:\n{}", shown(&raw));
        assert!(common::http::body(&raw).contains("the tour, by hand"), "`{url}`");
        assert!(
            common::http::header(&raw, "content-type").is_some_and(|t| t.starts_with("text/html")),
            "`{url}` is served as a document:\n{}",
            shown(&raw)
        );
    }
}

#[test]
fn a_directory_without_an_index_is_still_not_found() {
    let dir = tempfile::tempdir().unwrap();
    let root = directory_site(dir.path());
    let mut server = common::http::Server::start(&root);
    for url in ["/notes", "/notes/", "/nope/"] {
        assert_eq!(common::http::status(&server.get(url)), 404, "`{url}` names no file");
    }
    assert_eq!(
        common::http::body(&server.get("/notes/readme.txt")),
        "no index here\n",
        "while the file in it is served as ever"
    );
}

/// Every document URL the static export answers — each `.html` file written, and each folder
/// holding an `index.html`, with and without its slash — the dev server answers too. (The modules
/// ship under content-addressed names the dev server does not use, and `robots.txt` is generated
/// for the export; both are outside what a person links.)
#[test]
fn the_dev_server_answers_every_document_url_the_static_export_answers() {
    let dir = tempfile::tempdir().unwrap();
    let root = directory_site(dir.path());
    let out = common::largo_in(&root, &["build", "--static"]);
    assert!(out.status.success(), "{}", common::stderr(&out));
    let dist = root.join("dist");
    let mut urls = Vec::new();
    let mut stack = vec![dist.clone()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            let rel = p.strip_prefix(&dist).unwrap().to_string_lossy().replace('\\', "/");
            if !rel.ends_with(".html") {
                continue;
            }
            urls.push(format!("/{rel}"));
            if let Some(folder) = rel.strip_suffix("index.html") {
                let folder = folder.trim_end_matches('/');
                if !folder.is_empty() {
                    urls.push(format!("/{folder}"));
                    urls.push(format!("/{folder}/"));
                }
            }
        }
    }
    urls.sort();
    let mut server = common::http::Server::start(&root);
    let missing: Vec<String> = urls
        .iter()
        .filter(|u| common::http::status(&server.get(u)) != 200)
        .cloned()
        .collect();
    assert!(
        missing.is_empty(),
        "the built site answers these and `largo serve` does not: {missing:?}\nall: {urls:?}"
    );
}
