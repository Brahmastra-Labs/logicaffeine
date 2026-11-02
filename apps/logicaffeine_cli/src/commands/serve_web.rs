//! `largo serve` for a site — the ONE-command dev loop for a pure-LOGOS site, with or without a
//! backend.
//!
//! largo owns the public port. Three zones, by convention (no manifest keys):
//! - **pages/**: each `pages/<stem>.lg` is a DOM view compiled to WASM in
//!   memory and served at `/<stem>` (with `/<stem>.js`, `/<stem>.wasm`,
//!   `pages/app.lg` is the front page at `/`.
//!   Top-level `static/*.css` files are linked into every built page.
//! - **static/**: served at root-relative paths, from memory; an exact match
//!   wins over everything below it.
//! - **the backend** (a full-stack site only): the entry's `Serve` program runs
//!   as a CHILD PROCESS (`current_exe()` with the hidden `--as-backend` flag) on
//!   a largo-chosen port delivered via `LOGOS_PORT`; every unmatched request
//!   proxies to it over the wire core's client half
//!   (`serialize_request`/`parse_response`, one `Connection: close` exchange per
//!   proxied request), while the PUBLIC side inherits keep-alive from `serve_on`.
//!   A site with no backend answers an unmatched request `404` — with its own
//!   `## Error page 404` when it declares one, exactly as a static host serves
//!   the exported `404.html`.
//!
//! Watch + hot reload is always on (this IS the dev loop): the recursive
//! fingerprint spans the backend's assembled closure ∪ `pages/` ∪ `static/`,
//! and a change is CLASSIFIED — a page/static edit rebuilds that zone in
//! memory and bumps `/__gen`; a backend edit kills the child, respawns it,
//! waits for its port to accept, THEN bumps `/__gen`. The reload client is
//! injected into built pages AND into proxied `text/html` responses. A bad
//! page edit serves the themed overlay over a live last-good bundle; a
//! backend that dies on boot serves its stderr in a themed 502 until the
//! next good save. Every dev artifact stays in memory — the watcher can
//! never self-trigger.

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use logicaffeine_system::http::server::{serve_on_shared_with, Listener};
use logicaffeine_system::http::wire::{
    parse_response, serialize_request, HttpRequest, HttpResponse, ParseError,
};
use logicaffeine_system::relay::RelayHub;

use crate::commands::emit::browser_host_js;
use crate::commands::serve_dom::{open_command, RELOAD_CLIENT};
use crate::diagnostics::{self, Placed};
use crate::project::shape::Project;
use crate::ui;
use logicaffeine_compile::web_host::error_overlay_html;

/// How long a proxied request may wait for the backend before the dev server
/// answers 502 — a hung handler must never wedge the whole dev loop.
const PROXY_READ_TIMEOUT: Duration = Duration::from_secs(60);

/// How long a respawned backend gets to accept its port before the dev server SAYS it is still
/// waiting — not before it gives up. Boot is bounded by [`BACKEND_BOOT_CEILING`].
const BACKEND_BOOT_PATIENCE: Duration = Duration::from_secs(30);

/// The point past which a backend that is alive and has still not bound its port is presumed
/// wedged rather than slow — an infinite loop before `Serve on port`, which no amount of waiting
/// resolves.
///
/// The two are separate because they answer different questions. A backend that EXITS is caught
/// instantly by `try_wait` with its stderr, which is what a broken program actually does; the only
/// thing a flat deadline can catch beyond that is a backend still working. Declaring failure there
/// measures the machine — a loaded box, a cold cache, a big project — and hands the developer "the
/// backend never accepted" for an app that was seconds from starting. Waiting longer costs a
/// developer nothing on a healthy machine, because the wait ends when the port opens.
const BACKEND_BOOT_CEILING: Duration = Duration::from_secs(240);

/// One built page: the artifacts served at `/<stem>*`, plus the overlay for a
/// broken edit (the last-good `wasm`/`js`/`html` stay live under it).
struct PageBundle {
    wasm: Vec<u8>,
    js: String,
    html: String,
    error_overlay: Option<String>,
    /// The same error as the terminal prints it, `file:line:col` and all.
    error_text: Option<String>,
    /// Where this page serves — its `## Page /path` header, else its file name. The router keys on
    /// THIS rather than the file stem, so a declared path is honoured in development too.
    path: String,
}

/// Everything the router serves from memory, swapped atomically by the watcher.
struct SiteState {
    generation: u64,
    /// `stem → bundle` for every `pages/<stem>.lg`.
    pages: BTreeMap<String, PageBundle>,
    /// Root-relative path (`/style.css`) → bytes for every file under `static/`.
    statics: BTreeMap<String, Vec<u8>>,
    /// A themed overlay when the backend died on boot (carrying its stderr);
    /// proxied paths serve it as a 502 until the next good save.
    backend_error: Option<String>,
    /// Every `## Error page`, `status → document`, served at `/<status>.html` — the file the
    /// export publishes and a static host answers an unmatched URL with.
    error_pages: BTreeMap<u16, String>,
}

fn response(status: u16, content_type: &str, body: Vec<u8>) -> HttpResponse {
    let mut headers = BTreeMap::new();
    headers.insert("Content-Type".to_string(), content_type.to_string());
    HttpResponse {
        status,
        headers,
        body,
    }
}

/// The MIME type for a served path, by extension — shared by the static table
/// and any future artifact lane.
pub(crate) fn content_type_for(path: &str) -> &'static str {
    // ★ ONE TABLE, and it is `static.lg`'s. This tool and a DEPLOYED LOGOS program must answer
    // this question identically, or a file is typed one way in development and another in
    // production — and when these were two tables they had already drifted (`text/css` against
    // `text/css; charset=utf-8`, plus six types the LOGOS side did not know at all, so a deployed
    // site served its own fonts as `application/octet-stream`).
    //
    // The table lives in LOGOS because that is where every tier runs it; adding a type there is
    // the only edit an author makes, and there is no second place to remember.
    logicaffeine_compile::loader::content_type_for(path)
}

/// The dev panel: what the SERVER knows about the site it is serving.
///
/// A page's action JOURNAL lives in the running page, not here — a separate `/__dev` document is
/// a different page with a journal of its own, which would be empty. Showing one would be a panel
/// that looks informative and tells you nothing, so this shows what a dev SERVER actually knows:
/// which pages exist and at what URLs, which of them currently fail to compile, what static
/// assets are being served, and the generation the reload client polls.
///
/// The in-page journal overlay (time travel by replaying it) belongs INSIDE the app page, where
/// the journal is — `the journal` and `Replay these actions:` are the pieces it will be built
/// from, and both are locked already.
fn dev_panel(state: &SiteState) -> String {
    fn esc(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }
    let mut out = String::from(
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
         <title>largo serve</title><style>\
         body{font:14px system-ui;margin:2rem auto;max-width:44rem;padding:0 1rem}\
         h1{font-size:1.2rem}h2{font-size:1rem;margin-top:2rem}\
         code{background:#f4f4f5;padding:.1rem .3rem;border-radius:3px}\
         .bad{color:#b00}li{margin:.2rem 0}</style></head><body>\n",
    );
    out.push_str(&format!(
        "<h1>largo serve — generation {}</h1>\n",
        state.generation
    ));

    out.push_str("<h2>Pages</h2>\n<ul>\n");
    if state.pages.is_empty() {
        out.push_str("<li>none — this site is backend and static assets only</li>\n");
    }
    for (stem, page) in &state.pages {
        let url = if stem == "app" {
            "/".to_string()
        } else {
            format!("/{stem}")
        };
        match &page.error_overlay {
            // A page that does not compile is the FIRST thing a developer needs to see, so it is
            // marked rather than listed as if it were fine.
            Some(_) => out.push_str(&format!(
                "<li class=\"bad\"><a href=\"{u}\"><code>{u}</code></a> — does not compile</li>\n",
                u = esc(&url)
            )),
            None => out.push_str(&format!(
                "<li><a href=\"{u}\"><code>{u}</code></a> — {n} bytes of wasm</li>\n",
                u = esc(&url),
                n = page.wasm.len()
            )),
        }
    }
    out.push_str("</ul>\n");

    out.push_str("<h2>Static assets</h2>\n<ul>\n");
    if state.statics.is_empty() {
        out.push_str("<li>none</li>\n");
    }
    for (path, bytes) in &state.statics {
        out.push_str(&format!(
            "<li><a href=\"{p}\"><code>{p}</code></a> — {n} bytes</li>\n",
            p = esc(path),
            n = bytes.len()
        ));
    }
    out.push_str("</ul>\n");

    if let Some(err) = &state.backend_error {
        out.push_str(&format!(
            "<h2>Backend</h2>\n<pre class=\"bad\">{}</pre>\n",
            esc(err)
        ));
    }
    out.push_str("</body></html>\n");
    out
}

/// Route one request against the in-memory site. `None` means "not ours —
/// proxy it to the backend". Precedence (locked by unit tests): `/__gen`,
/// then static assets (an asset SHADOWS any same-named route), then built
/// pages, then the proxy.
fn route_local(path: &str, state: &SiteState) -> Option<HttpResponse> {
    let path = path.split('?').next().unwrap_or(path);
    if path == "/__gen" {
        return Some(response(
            200,
            "text/plain",
            state.generation.to_string().into_bytes(),
        ));
    }
    if path == "/__dev" {
        return Some(response(
            200,
            "text/html; charset=utf-8",
            dev_panel(state).into_bytes(),
        ));
    }
    if let Some(bytes) = state.statics.get(path) {
        return Some(response(200, content_type_for(path), bytes.clone()));
    }
    if let Some(doc) = path
        .strip_prefix('/')
        .and_then(|p| p.strip_suffix(".html"))
        .and_then(|status| status.parse::<u16>().ok())
        .and_then(|status| state.error_pages.get(&status))
    {
        return Some(response(
            200,
            "text/html; charset=utf-8",
            doc.clone().into_bytes(),
        ));
    }
    // ★ A PAGE SERVES WHERE IT SAYS IT DOES, in development as in production. Its URLs come from
    // the ONE rule `largo build` publishes, applied to the page's OWN path — its `## Page /path`
    // header when it declares one, else its file name. Keying the router on the file stem is what
    // made a declared path do nothing.
    for (stem, page) in &state.pages {
        if crate::commands::web_bundle::page_html_paths_at(&page.path)
            .iter()
            .any(|p| p == path)
        {
            if let Some(resp) = page_html_response(state, stem) {
                return Some(resp);
            }
            // The page is there but has nothing to serve (a build error with no overlay); the
            // backend gets its chance rather than the request dying here.
            return None;
        }
        // The module and the shim hang off the page's own path, exactly as the build publishes
        // them — so a page at `/dashboard/settings` loads `/dashboard/settings.js`.
        // ★ AND A PARAMETERIZED PAGE ANSWERS EVERY PATH ITS PATTERN MATCHES. `/posts/:id` is a
        // pattern, not a URL: the visitor types `/posts/42`, the same document is served, and the
        // page reads `42` back out of the location with `routeParam` — the identical rule, which
        // is why the matcher is shared rather than written twice.
        if crate::commands::web_bundle::page_path_matches(&page.path, path) {
            if let Some(resp) = page_html_response(state, stem) {
                return Some(resp);
            }
            return None;
        }
        let base = if page.path == "/" {
            format!("/{stem}")
        } else {
            crate::commands::web_bundle::page_asset_base(&page.path)
        };
        if path == format!("{base}.js") {
            return Some(response(
                200,
                "text/javascript",
                page.js.clone().into_bytes(),
            ));
        }
        if path == format!("{base}.wasm") {
            return Some(response(200, "application/wasm", page.wasm.clone()));
        }
    }
    None
}

/// What a site with no backend answers for a request nothing in it matches: `404`, with the
/// site's own `## Error page 404` when it has one — what a static host does with the exported
/// `404.html`.
fn not_found(state: &SiteState) -> HttpResponse {
    let doc = state.error_pages.get(&404).cloned().unwrap_or_else(|| {
        "<!doctype html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><title>Not found</title></head>\
         <body><h1>Not found</h1><p>Nothing in this site answers that address.</p></body></html>\n"
            .to_string()
    });
    response(404, "text/html; charset=utf-8", doc.into_bytes())
}

/// The HTML answer for a page: its themed error overlay when the last edit
/// broke it (the caret page), else its host HTML.
fn page_html_response(state: &SiteState, stem: &str) -> Option<HttpResponse> {
    let page = state.pages.get(stem)?;
    let html = page.error_overlay.as_ref().unwrap_or(&page.html);
    Some(response(
        200,
        "text/html; charset=utf-8",
        html.clone().into_bytes(),
    ))
}

/// `<link rel="stylesheet">` tags for every top-level css asset — the
/// convention that makes `static/style.css` style every built page.
fn stylesheet_links(statics: &BTreeMap<String, Vec<u8>>) -> String {
    stylesheet_links_for(statics.keys().map(String::as_str))
}

/// ★ ONE RULE FOR WHICH STYLESHEETS A PAGE LINKS, shared by `largo serve` and `largo build`.
///
/// It was written twice — identically, differing only in the container each caller happened to
/// hold — which is the same shape the MIME table was in before it became one table, and the same
/// failure waiting to happen: a page styled in development and unstyled in production, or the
/// reverse, with nothing saying why. A convention two places implement is a convention only until
/// someone edits one of them.
///
/// TOP-LEVEL ONLY, and that is the convention rather than an accident: a `.css` at the site root
/// is a SITE stylesheet, while one nested under a directory belongs to whatever put it there and
/// is opted into by the page that wants it. Linking everything would let a component's private
/// stylesheet leak onto every page on the site.
/// ★ AND `theme.css` COMES FIRST, whatever order the caller hands them over in. Design tokens are
/// a BASE layer: a project customising its site writes rules in `style.css` and expects them to
/// win, and alphabetical order alone puts `style.css` first — so the theme it is customising
/// would override it. One rule, in the one place both callers go through, rather than each of
/// them remembering to sort a particular way.
pub(crate) fn stylesheet_links_for<'a>(paths: impl Iterator<Item = &'a str>) -> String {
    linked_stylesheets(paths)
        .into_iter()
        .map(|p| format!("<link rel=\"stylesheet\" href=\"{p}\">\n"))
        .collect()
}

/// The static files a page document links as stylesheets, in the order it links them: every
/// top-level `.css`, `/theme.css` first so the project's own sheets cascade over its tokens.
pub(crate) fn linked_stylesheets<'a>(paths: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    let sheets: Vec<&str> = paths
        .filter(|p| p.ends_with(".css") && p.matches('/').count() == 1)
        .collect();
    let (theme, rest): (Vec<&str>, Vec<&str>) =
        sheets.into_iter().partition(|p| *p == "/theme.css");
    theme.into_iter().chain(rest).collect()
}

/// Build one page from its assembled closure: wasm + host js (carrying the reload client) + host
/// html (carrying the site stylesheets). A compile error becomes the themed overlay and the
/// terminal text instead, both drawn against the file the mistake is in.
fn build_page(
    page: &crate::commands::web_bundle::PageClosure,
    css_links: &str,
    favicon: Option<&str>,
    site: Option<&str>,
    theme: logicaffeine_compile::web_host::ShellTheme,
) -> PageBundle {
    let (source, stem) = (page.closure.source.as_str(), page.stem.as_str());
    // Where this page serves — its declaration, or its file name. A page whose header is
    // malformed keeps the file-name default rather than vanishing from the router: the dev loop's
    // job is to keep showing you something while you fix it, and `largo build` is where a bad
    // path is a hard error.
    let path = crate::commands::web_bundle::page_path(source, stem)
        .unwrap_or_else(|_| format!("/{}", if stem == "app" { "" } else { stem }));
    match logicaffeine_compile::compile::compile_to_wasm(source) {
        Ok(wasm) => {
            let mut js = browser_host_js(stem);
            js.push_str(RELOAD_CLIENT);
            // ★ THE DEV SERVER SHOWS YOU THE DOCUMENT THAT SHIPS. `largo build` pre-renders a
            // page's markup into its document so a crawler, a slow connection and a reader with
            // scripts off all see something; the dev server serves that same document, so a page
            // the server renderer cannot draw is visible here at the edit that caused it.
            //
            // Pre-rendering is an improvement, never a new way to fail: a page the server walk
            // cannot draw ships its shell and works in the browser exactly as before.
            let canonical =
                site.and_then(|site| crate::commands::web_bundle::canonical_url(site, &path));
            let html = crate::commands::web_bundle::prerendered_html_in_site(
                source,
                stem,
                favicon,
                canonical.as_deref(),
                theme,
            )
            .replace("</head>", &format!("{css_links}</head>"));
            PageBundle {
                wasm,
                js,
                html,
                error_overlay: None,
                error_text: None,
                path,
            }
        }
        Err(pe) => {
            let at = Placed::locate(pe.span, source, &page.closure.source_map, &page.path);
            PageBundle {
                path,
                wasm: Vec::new(),
                js: String::new(),
                html: String::new(),
                error_overlay: Some(diagnostics::overlay(&pe, &at, RELOAD_CLIENT)),
                error_text: Some(diagnostics::render_parse_error(&pe, &at)),
            }
        }
    }
}

/// A themed overlay for a failure that has a message but no source line — an
/// unresolvable page import, a backend that died on boot.
fn message_overlay(message: &str) -> String {
    let pe = logicaffeine_language::error::ParseError {
        kind: logicaffeine_language::error::ParseErrorKind::Custom(message.to_string()),
        span: Default::default(),
    };
    error_overlay_html(&pe, "", RELOAD_CLIENT)
}

/// Every `pages/<stem>.lg`, sorted by stem — the enumeration `largo build` and `largo check`
/// read. A `pages/` that cannot be read serves as no pages; `largo check` is where that is an
/// error.
fn page_entries(pages_dir: &Path) -> Vec<(String, PathBuf)> {
    crate::commands::web_bundle::page_files(pages_dir).unwrap_or_default()
}

/// (Re)build the whole site: every page, and every `## Error page` as the document it publishes.
/// A page that fails keeps `previous`'s last-good artifacts live under its fresh overlay.
fn build_pages(
    pages_dir: &Path,
    statics: &BTreeMap<String, Vec<u8>>,
    previous: Option<&BTreeMap<String, PageBundle>>,
    theme: logicaffeine_compile::web_host::ShellTheme,
) -> (BTreeMap<String, PageBundle>, BTreeMap<u16, String>) {
    let css_links = stylesheet_links(statics);
    // ★ THE SAME FAVICON RULE THE BUILD APPLIES, or the icon in the tab changes when the
    // site ships.
    let favicon = ["/favicon.svg", "/favicon.png", "/favicon.ico"]
        .into_iter()
        .find(|p| statics.contains_key(*p));
    // The site's address from `Largo.toml`, for each page's canonical link — the same one the build
    // writes, so the document in development is the document that ships.
    let site = crate::commands::web_bundle::site_url(pages_dir);
    let mut pages = BTreeMap::new();
    let mut error_pages = BTreeMap::new();
    for (stem, path) in page_entries(pages_dir) {
        let mut fresh = match crate::closure::assemble_closure(&path) {
            Ok(closure) => {
                // An error page is not a page: it has no URL of its own and is published as
                // `/<status>.html`, which is what the dev server serves it as too.
                if let Some(status) =
                    crate::commands::web_bundle::error_page_status(&closure.source)
                {
                    let doc = crate::commands::web_bundle::error_page_document(
                        &closure.source,
                        status,
                        &css_links,
                        favicon,
                        theme,
                    )
                    .map(|doc| doc.replace("</body>", &format!("<script>{RELOAD_CLIENT}</script></body>")))
                    .unwrap_or_else(|why| {
                        message_overlay(&format!(
                            "pages/{stem}.lg declares `## Error page {status}` and it cannot be rendered: {why}"
                        ))
                    });
                    error_pages.insert(status, doc);
                    continue;
                }
                let page = crate::commands::web_bundle::PageClosure {
                    stem: stem.clone(),
                    path,
                    closure,
                };
                build_page(&page, &css_links, favicon, site.as_deref(), theme)
            }
            // The file would not even assemble, so there is no source to read a path out of —
            // the file-name default keeps the page in the router while the author fixes it.
            Err(message) => PageBundle {
                wasm: Vec::new(),
                js: String::new(),
                html: String::new(),
                error_overlay: Some(message_overlay(&message)),
                error_text: Some(format!("pages/{stem}.lg: {message}")),
                path: format!("/{}", if stem == "app" { "" } else { stem.as_str() }),
            },
        };
        if fresh.error_overlay.is_some() {
            if let Some(last_good) = previous.and_then(|p| p.get(&stem)) {
                fresh.wasm = last_good.wasm.clone();
                fresh.js = last_good.js.clone();
                fresh.html = last_good.html.clone();
            }
        }
        pages.insert(stem, fresh);
    }
    (pages, error_pages)
}

/// Tell the terminal about every page that does not build: which file, and the error itself —
/// the same text the browser overlay shows.
fn report_broken_pages(pages: &BTreeMap<String, PageBundle>) {
    for (stem, page) in pages {
        if let Some(error) = &page.error_text {
            ui::warning(format!(
                "pages/{stem}.lg does not build — serving its error overlay:\n{error}"
            ));
        }
    }
}

/// Load every file under `static/` into the in-memory table, keyed by its
/// root-relative URL path.
fn load_statics(static_dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<String, Vec<u8>>, depth: usize) {
        if depth > 32 {
            return;
        }
        let Ok(read) = std::fs::read_dir(dir) else {
            return;
        };
        for f in read.flatten() {
            let path = f.path();
            if path.is_dir() {
                walk(&path, root, out, depth + 1);
            } else if let (Ok(rel), Ok(bytes)) = (path.strip_prefix(root), std::fs::read(&path)) {
                let key = format!("/{}", rel.to_string_lossy().replace('\\', "/"));
                out.insert(key, bytes);
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(static_dir, static_dir, &mut out, 0);
    out
}

// ---------------------------------------------------------------------------
// The backend child
// ---------------------------------------------------------------------------

/// The running backend child. Its stdin pipe is the LIFELINE (`--as-backend`
/// exits on stdin EOF, so a killed dev server never orphans a port-squatting
/// child); its stderr is drained into `stderr_buf` for the boot-failure
/// overlay.
struct Backend {
    child: Child,
    _lifeline: ChildStdin,
    stderr_buf: Arc<Mutex<String>>,
}

impl Drop for Backend {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Spawn the backend: `current_exe() serve --as-backend` in the project root,
/// with the largo-chosen port delivered via `LOGOS_PORT` (the same override
/// seam production PaaS ports use).
fn spawn_backend(root: &Path, backend_port: u16) -> Result<Backend, String> {
    let exe = std::env::current_exe().map_err(|e| format!("cannot find largo itself: {e}"))?;
    let mut child = Command::new(exe)
        .args(["serve", "--as-backend"])
        .current_dir(root)
        .env("LOGOS_PORT", backend_port.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot spawn the backend child: {e}"))?;
    let lifeline = child.stdin.take().expect("stdin was piped");
    let stderr = child.stderr.take().expect("stderr was piped");
    let stderr_buf = Arc::new(Mutex::new(String::new()));
    let sink = Arc::clone(&stderr_buf);
    std::thread::spawn(move || {
        let mut stderr = stderr;
        let mut chunk = [0u8; 4096];
        loop {
            match stderr.read(&mut chunk) {
                Ok(0) | Err(_) => break,
                Ok(n) => sink
                    .lock()
                    .unwrap()
                    .push_str(&String::from_utf8_lossy(&chunk[..n])),
            }
        }
    });
    Ok(Backend {
        child,
        _lifeline: lifeline,
        stderr_buf,
    })
}

/// Wait until the backend accepts its port. On failure (the child exited, or the ceiling passed)
/// returns the story: its captured stderr, or an honest timeout message.
fn wait_for_backend(backend: &mut Backend, port: u16) -> Result<(), String> {
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
    let started = Instant::now();
    let patience = started + BACKEND_BOOT_PATIENCE;
    let ceiling = started + BACKEND_BOOT_CEILING;
    let mut said_waiting = false;
    loop {
        if std::net::TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok() {
            return Ok(());
        }
        if let Ok(Some(status)) = backend.child.try_wait() {
            // Give the drain thread a beat to flush the last stderr bytes.
            std::thread::sleep(Duration::from_millis(120));
            let stderr = backend.stderr_buf.lock().unwrap().clone();
            let detail = if stderr.trim().is_empty() {
                format!("(no stderr; exit status {status})")
            } else {
                stderr
            };
            return Err(format!(
                "The backend exited on boot ({status}). Its stderr:\n{detail}"
            ));
        }
        let now = Instant::now();
        if now > patience && !said_waiting {
            // The child is alive and has not bound yet. Say so ONCE, so a long wait reads as a
            // slow build rather than a hung dev server, and keep waiting.
            said_waiting = true;
            eprintln!(
                "largo serve: still waiting for the backend on port {port} ({}s so far)…",
                BACKEND_BOOT_PATIENCE.as_secs()
            );
        }
        if now > ceiling {
            return Err(format!(
                "The backend never accepted 127.0.0.1:{port} within {}s, and its process is still \
                 running — it is most likely looping before it reaches `Serve on port {port}.`",
                BACKEND_BOOT_CEILING.as_secs()
            ));
        }
        std::thread::sleep(Duration::from_millis(60));
    }
}

// ---------------------------------------------------------------------------
// The proxy
// ---------------------------------------------------------------------------

/// Forward one request to the backend over a fresh `Connection: close`
/// exchange (the wire core's client half; `serialize_request` owns the
/// framing headers, so the browser's hop-by-hop headers can never leak
/// through). The public side's keep-alive is untouched — `serve_on` owns it.
fn proxy_to_backend(req: &HttpRequest, backend_port: u16) -> Result<HttpResponse, String> {
    use std::io::Write;
    let addr = std::net::SocketAddr::from(([127, 0, 0, 1], backend_port));
    let mut stream = std::net::TcpStream::connect_timeout(&addr, Duration::from_secs(2))
        .map_err(|e| format!("connect: {e}"))?;
    stream
        .set_read_timeout(Some(PROXY_READ_TIMEOUT))
        .map_err(|e| format!("read timeout: {e}"))?;
    stream
        .write_all(&serialize_request(req))
        .map_err(|e| format!("write: {e}"))?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match parse_response(&buf) {
            Ok(resp) => return Ok(resp),
            Err(ParseError::Incomplete) => {}
            Err(e) => return Err(format!("unparseable backend response: {e:?}")),
        }
        let n = stream.read(&mut chunk).map_err(|e| format!("read: {e}"))?;
        if n == 0 {
            return parse_response(&buf)
                .map_err(|e| format!("the backend closed mid-response: {e:?}"));
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

/// Whether `path` is one of the site's page documents — what [`route_local`] answers from a page's
/// own URL (its declared path, or a path its pattern matches).
fn serves_a_page(path: &str, state: &SiteState) -> bool {
    let path = path.split('?').next().unwrap_or(path);
    state.pages.values().any(|page| {
        crate::commands::web_bundle::page_html_paths_at(&page.path)
            .iter()
            .any(|p| p == path)
            || crate::commands::web_bundle::page_path_matches(&page.path, path)
    })
}

/// A page document as this visitor receives it from a full-stack site: the backend answers the
/// visitor's CSRF token at its reserved path (a backend with a `## Session` does; one without
/// answers 404), and the document carries it in its head with the backend's cookie set, as a
/// compiled server composes it. Any failure to ask serves the page as it is.
fn page_for_visitor(page: HttpResponse, req: &HttpRequest, backend_port: u16) -> HttpResponse {
    let mut headers = BTreeMap::new();
    if let Some((_, cookie)) = req
        .headers
        .iter()
        .find(|(name, _)| name.eq_ignore_ascii_case("cookie"))
    {
        headers.insert("cookie".to_string(), cookie.clone());
    }
    let ask = HttpRequest {
        method: "GET".to_string(),
        path: logicaffeine_language::parser::lgx_lower::CSRF_TOKEN_PATH.to_string(),
        query: BTreeMap::new(),
        headers,
        body: Vec::new(),
    };
    match proxy_to_backend(&ask, backend_port) {
        Ok(token) => logicaffeine_system::http::server::page_with_visitor_token(&page, &token)
            .unwrap_or(page),
        Err(_) => page,
    }
}

/// Inject the reload client into a proxied `text/html` response (before
/// `</body>` when present, else appended). `Content-Length` needs no manual
/// recompute — the wire serializer owns it from the body bytes.
fn inject_reload_into_html(resp: &mut HttpResponse) {
    let is_html = resp.headers.iter().any(|(name, value)| {
        name.eq_ignore_ascii_case("content-type")
            && value
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("text/html")
    });
    if !is_html {
        return;
    }
    let script = format!("<script>{RELOAD_CLIENT}</script>");
    let lower: Vec<u8> = resp.body.to_ascii_lowercase();
    match lower.windows(7).position(|w| w == b"</body>") {
        Some(i) => {
            let mut body = Vec::with_capacity(resp.body.len() + script.len());
            body.extend_from_slice(&resp.body[..i]);
            body.extend_from_slice(script.as_bytes());
            body.extend_from_slice(&resp.body[i..]);
            resp.body = body;
        }
        None => resp.body.extend_from_slice(script.as_bytes()),
    }
}

/// The 502 a proxied path answers when the backend is down: the themed
/// overlay (boot stderr when we have it, else the connect failure), polling
/// `/__gen` so the page snaps back when the backend returns.
fn bad_gateway(state: &SiteState, detail: &str) -> HttpResponse {
    let overlay = state
        .backend_error
        .clone()
        .unwrap_or_else(|| message_overlay(&format!("The backend is not responding: {detail}")));
    response(502, "text/html; charset=utf-8", overlay.into_bytes())
}

// ---------------------------------------------------------------------------
// The watcher
// ---------------------------------------------------------------------------

/// The newest mtime across an explicit file list plus a recursive directory
/// sweep (`lg_only` restricts the sweep to `.lg`/`.md` sources; `target/` and
/// dot-directories are never scanned).
fn zone_fingerprint(files: &[PathBuf], sweep: Option<(&Path, bool)>) -> SystemTime {
    let mut newest = SystemTime::UNIX_EPOCH;
    for f in files {
        if let Ok(m) = std::fs::metadata(f).and_then(|m| m.modified()) {
            newest = newest.max(m);
        }
    }
    if let Some((dir, lg_only)) = sweep {
        scan_tree(dir, lg_only, &mut newest, 0);
    }
    newest
}

fn scan_tree(dir: &Path, lg_only: bool, newest: &mut SystemTime, depth: usize) {
    if depth > 32 {
        return;
    }
    let Ok(read) = std::fs::read_dir(dir) else {
        return;
    };
    for f in read.flatten() {
        let Ok(kind) = f.file_type() else { continue };
        if kind.is_dir() {
            let name = f.file_name();
            let name = name.to_string_lossy();
            if name == "target" || name.starts_with('.') {
                continue;
            }
            scan_tree(&f.path(), lg_only, newest, depth + 1);
        } else {
            let path = f.path();
            let ext = path.extension().and_then(|e| e.to_str());
            if !lg_only || matches!(ext, Some("lg") | Some("md")) {
                if let Ok(m) = f.metadata().and_then(|m| m.modified()) {
                    *newest = (*newest).max(m);
                }
            }
        }
    }
}

/// The three watched zones' fingerprints, computed together each tick.
struct Fingerprints {
    backend: SystemTime,
    pages: SystemTime,
    statics: SystemTime,
}

/// Backend zone = the assembled closure's files ∪ the `src/` source sweep, computed by the caller
/// (a site with no backend has none). Pages zone = every page's closure files ∪ the `pages/` sweep
/// (any file — a new page counts). Static zone = the `static/` sweep, any extension.
fn fingerprints(
    backend: SystemTime,
    page_files: &[PathBuf],
    pages_dir: &Path,
    static_dir: &Path,
) -> Fingerprints {
    Fingerprints {
        backend,
        pages: zone_fingerprint(page_files, Some((pages_dir, false))),
        statics: zone_fingerprint(&[], Some((static_dir, false))),
    }
}

/// Every file that feeds any page: each `pages/<stem>.lg`'s assembled closure
/// (so an edit to a module a page imports rebuilds it).
fn all_page_closure_files(pages_dir: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for (_stem, path) in page_entries(pages_dir) {
        match crate::closure::assemble_closure(&path) {
            Ok(closure) => files.extend(closure.files),
            Err(_) => files.push(path), // unassemblable: watch the entry itself
        }
    }
    files
}

/// The address another machine on this network would use to reach us.
///
/// Opening a UDP socket and "connecting" it sends nothing — it only asks the routing table which
/// local interface would carry traffic to that destination, which is exactly the interface a
/// phone on the same wifi reaches. Enumerating interfaces directly would need a crate and would
/// still have to guess which of them is the right one.
fn lan_address() -> Option<std::net::IpAddr> {
    let sock = std::net::UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    sock.local_addr().ok().map(|a| a.ip())
}

/// The console line for one completed reload, naming what changed.
fn reloaded_line(zone: &str, generation: u64, elapsed: Duration) -> String {
    format!(
        "{zone} in {}ms — generation {generation}",
        elapsed.as_millis()
    )
}

/// The line a rebuild that FAILED prints, naming the pages that did not compile — each one's
/// error is printed above it by [`report_broken_pages`], the same text its overlay shows.
fn rebuild_failed_line(broken: &[String], generation: u64, elapsed: Duration) -> String {
    format!(
        "{} did not compile in {}ms — generation {generation}, serving the last good build",
        broken.join(", "),
        elapsed.as_millis()
    )
}

// ---------------------------------------------------------------------------
// The command
// ---------------------------------------------------------------------------

/// Serve a site: pages + static from memory, hot reload across every zone. With `proxy`, the
/// entry's `Serve` program runs as a child behind the proxy and answers what the site does not;
/// without it, those requests are answered `404`, exactly as a static host answers them.
pub(crate) fn cmd_serve_web(
    project: &Project,
    port: u16,
    open: bool,
    host: Option<&str>,
    proxy: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let host = host.unwrap_or("127.0.0.1").to_string();
    let root = project.root.clone();
    let pages_dir = project.pages_dir();
    let static_dir = project.static_dir();

    // The backend's entry and compilation closure — what the watcher tracks for the kill/respawn
    // lane. It must assemble; fail loudly (and beautifully) if it cannot.
    let backend_entry = if proxy {
        Some(project.require_entry()?.to_path_buf())
    } else {
        None
    };
    let backend_files = match &backend_entry {
        Some(entry) => {
            crate::closure::assemble_closure(entry)
                .map_err(|e| format!("cannot serve — could not assemble the project: {e}"))?
                .files
        }
        None => Vec::new(),
    };
    let src_dir = backend_entry
        .as_deref()
        .and_then(Path::parent)
        .unwrap_or(&root)
        .to_path_buf();

    // Build the in-memory site.
    let theme = project.manifest.shell_theme();
    let statics = load_statics(&static_dir);
    let (pages, error_pages) = build_pages(&pages_dir, &statics, None, theme);
    report_broken_pages(&pages);
    let state = Arc::new(Mutex::new(SiteState {
        generation: 1,
        pages,
        statics,
        backend_error: None,
        error_pages,
    }));

    // Boot the backend child on a port of ours.
    let backend_port = match &backend_entry {
        Some(_) => Some(free_loopback_port()?),
        None => None,
    };
    let backend = match backend_port {
        Some(backend_port) => {
            let mut backend = spawn_backend(&root, backend_port)?;
            if let Err(story) = wait_for_backend(&mut backend, backend_port) {
                ui::warning(&story);
                state.lock().unwrap().backend_error = Some(message_overlay(&story));
            }
            Some(backend)
        }
        None => None,
    };

    // The watcher thread: fingerprint the zones, classify each change. It OWNS the backend child
    // (the handler only needs its port).
    {
        let state = Arc::clone(&state);
        let root = root.clone();
        let (pages_dir, static_dir, src_dir) =
            (pages_dir.clone(), static_dir.clone(), src_dir.clone());
        let mut backend_files = backend_files;
        std::thread::spawn(move || {
            let mut backend = backend;
            let backend_zone = |files: &[PathBuf]| match &backend_entry {
                Some(_) => zone_fingerprint(files, Some((&src_dir, true))),
                None => SystemTime::UNIX_EPOCH,
            };
            let mut page_files = all_page_closure_files(&pages_dir);
            let mut last = fingerprints(
                backend_zone(&backend_files),
                &page_files,
                &pages_dir,
                &static_dir,
            );
            loop {
                std::thread::sleep(Duration::from_millis(300));
                let now = fingerprints(
                    backend_zone(&backend_files),
                    &page_files,
                    &pages_dir,
                    &static_dir,
                );
                let backend_changed = now.backend != last.backend;
                let pages_changed = now.pages != last.pages;
                let statics_changed = now.statics != last.statics;
                if !(backend_changed || pages_changed || statics_changed) {
                    continue;
                }
                std::thread::sleep(Duration::from_millis(60)); // settle a burst of saves
                let started = Instant::now();
                let mut broken: Vec<String> = Vec::new();

                // STATIC zone: reload the asset table.
                if statics_changed {
                    let fresh = load_statics(&static_dir);
                    state.lock().unwrap().statics = fresh;
                }

                // PAGES zone: rebuild the page table (last-good kept per page). A static change
                // rebuilds too — the stylesheet links live in the built HTML.
                if pages_changed || statics_changed {
                    page_files = all_page_closure_files(&pages_dir);
                    let mut st = state.lock().unwrap();
                    let statics_now = st.statics.clone();
                    let (fresh, fresh_error_pages) =
                        build_pages(&pages_dir, &statics_now, Some(&st.pages), theme);
                    report_broken_pages(&fresh);
                    broken = fresh
                        .iter()
                        .filter(|(_, p)| p.error_overlay.is_some())
                        .map(|(stem, _)| format!("pages/{stem}.lg"))
                        .collect();
                    st.pages = fresh;
                    st.error_pages = fresh_error_pages;
                }

                // BACKEND zone: kill the old child FIRST (its port must free before the rebind),
                // respawn, wait for the port to accept — only then does the generation bump reload
                // the browser, so it never reloads into a dead backend.
                if let (true, Some(entry), Some(backend_port)) =
                    (backend_changed, &backend_entry, backend_port)
                {
                    if let Some(old) = backend.as_mut() {
                        let _ = old.child.kill();
                        let _ = old.child.wait();
                    }
                    if let Ok(closure) = crate::closure::assemble_closure(entry) {
                        backend_files = closure.files;
                    }
                    match spawn_backend(&root, backend_port) {
                        Ok(mut fresh) => {
                            let verdict = wait_for_backend(&mut fresh, backend_port);
                            backend = Some(fresh);
                            match verdict {
                                Ok(()) => state.lock().unwrap().backend_error = None,
                                Err(story) => {
                                    ui::warning(&story);
                                    state.lock().unwrap().backend_error =
                                        Some(message_overlay(&story));
                                }
                            }
                        }
                        Err(e) => {
                            // Spawning largo itself failed; keep the dead handle and tell the
                            // story.
                            ui::warning(&e);
                            state.lock().unwrap().backend_error = Some(message_overlay(&e));
                        }
                    }
                }

                let generation = {
                    let mut st = state.lock().unwrap();
                    st.generation += 1;
                    st.generation
                };
                last = fingerprints(
                    backend_zone(&backend_files),
                    &page_files,
                    &pages_dir,
                    &static_dir,
                );
                let zone = match (backend_changed, pages_changed, statics_changed) {
                    (true, _, _) => "backend restarted",
                    (false, true, _) => "pages rebuilt",
                    _ => "static reloaded",
                };
                if broken.is_empty() {
                    ui::phase(
                        "Reloaded",
                        reloaded_line(zone, generation, started.elapsed()),
                    );
                } else {
                    ui::warning(rebuild_failed_line(&broken, generation, started.elapsed()));
                }
            }
        });
    }

    // The public server.
    let rt = logicaffeine_system::tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("cannot start async runtime: {e}"))?;
    rt.block_on(async move {
        let mut listener = None;
        let mut last_err = String::new();
        for p in port..port.saturating_add(16) {
            match Listener::bind(&format!("{host}:{p}")).await {
                Ok(l) => {
                    listener = Some(l);
                    break;
                }
                Err(e) => last_err = e.to_string(),
            }
        }
        let listener = listener.ok_or_else(|| {
            format!(
                "cannot bind any port in {port}..{} (last: {last_err})",
                port.saturating_add(16)
            )
        })?;
        let bound = listener.local_addr().map_err(|e| format!("{e}"))?;
        let zones = match backend_port {
            Some(backend_port) => format!("pages + static + backend:{backend_port}"),
            None => "pages + static".to_string(),
        };
        ui::phase(
            "Serving",
            format!(
                "{name} on http://{bound}  ({zones} — live-reloading)",
                name = project.manifest.package.name
            ),
        );
        // `/__dev` lists every page and the URL it serves at, marks the ones that do not compile,
        // sizes each wasm module and shows a dead backend's stderr — announced here, because a
        // panel nobody is told about is never opened.
        ui::phase(
            "Panel",
            format!("http://{bound}/__dev  (pages, sizes, build errors)"),
        );
        // Binding this machine only is the default; `--host 0.0.0.0` opts in to the network, and
        // then the address another device would type is printed, since `0.0.0.0` is not one.
        if bound.ip().is_unspecified() {
            match lan_address() {
                Some(ip) => ui::phase("Network", format!("http://{ip}:{}", bound.port())),
                None => {
                    ui::info("(bound to every interface, but no LAN address was found)".to_string())
                }
            }
        }
        if open {
            let url = format!("http://{bound}");
            let (prog, args) = open_command(&url);
            match Command::new(prog).args(&args).spawn() {
                Ok(_) => ui::phase("Opening", url),
                Err(_) => ui::info(format!("(could not open a browser — visit {url})")),
            }
        }
        let handler_state = Arc::clone(&state);
        // `/_relay`: the same pub/sub hub the interpreter's `Raise`/`Publish` reach in-process,
        // answering a real WebSocket upgrade so a browser page can round-trip through it — the one
        // table every server registers (`RelayHub::upgrades`), a deployed backend's included.
        let relay = RelayHub::new().upgrades();
        // SHARED, NOT SERIALIZED: this handler answers the assets, the pages and the `/__gen`
        // reload poll, and proxies whatever else to the backend with a multi-second timeout. The
        // state is behind its own lock, released before the proxy runs, and `proxy_to_backend`
        // opens its own socket per call — so one slow backend route never queues the CSS, the
        // wasm or the reload poll behind it.
        serve_on_shared_with(
            listener,
            move |req: HttpRequest| {
                let (local, is_page) = {
                    let st = handler_state.lock().unwrap();
                    let is_page = serves_a_page(&req.path, &st);
                    let local = match (route_local(&req.path, &st), backend_port) {
                        (Some(resp), _) => Some(resp),
                        (None, None) => Some(not_found(&st)),
                        (None, Some(_)) if st.backend_error.is_some() => {
                            Some(bad_gateway(&st, "it exited on boot"))
                        }
                        (None, Some(_)) => None,
                    };
                    (local, is_page)
                };
                match (local, backend_port) {
                    (Some(resp), Some(backend_port)) if is_page => {
                        page_for_visitor(resp, &req, backend_port)
                    }
                    (Some(resp), _) => resp,
                    (None, Some(backend_port)) => match proxy_to_backend(&req, backend_port) {
                        Ok(mut resp) => {
                            inject_reload_into_html(&mut resp);
                            resp
                        }
                        Err(detail) => {
                            let st = handler_state.lock().unwrap();
                            bad_gateway(&st, &detail)
                        }
                    },
                    (None, None) => {
                        unreachable!("a site with no backend answers everything locally")
                    }
                }
            },
            relay,
        )
        .await
        .map_err(|e| format!("serve loop ended: {e}"))?;
        Ok::<(), String>(())
    })?;
    Ok(())
}

/// An OS-assigned free loopback port, released for the backend child to bind.
fn free_loopback_port() -> Result<u16, String> {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(|e| format!("cannot pick a backend port: {e}"))
}

#[cfg(test)]
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_cli/commands/serve_web/tests.rs"]
mod tests;
