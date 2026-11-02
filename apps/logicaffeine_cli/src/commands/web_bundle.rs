//! B4 — bundle a full-stack LOGOS site into ONE self-contained native binary.
//!
//! `largo build` on a web project (a `Serve` entry alongside `pages/` or
//! `static/`) does three extra things beyond the plain LOGOS→Rust→cargo path,
//! all owned here:
//!
//! 1. **Bundle** — every `pages/<stem>.lg` is compiled to its `wasm`/`js`/`html`
//!    trio (the SAME artifacts `largo serve` builds in memory, via
//!    [`crate::commands::emit`]'s `compile_to_wasm` + `web_host`), and every
//!    file under `static/**` is collected verbatim, into ONE flat asset table
//!    keyed by root-relative URL path — precisely the dev router's namespace.
//! 2. **Embed** — that table is emitted as a `logos_assets.rs` module in the
//!    generated cargo project: a `&[(path, content_type, &'static [u8])]` static
//!    built from `include_bytes!` plus a `lookup` fn, so the assets ride INSIDE
//!    the binary (no runtime disk dependency).
//! 3. **Serve hook** — the emitted `Serve` closure consults
//!    `logos_assets::lookup(&path)` BEFORE the handler (assets-first, exactly the
//!    dev router's precedence, so dev ≡ prod by construction). The hook is driven
//!    by `LOGOS_EMBED_ASSETS=1`, set around `compile_project` only for a web
//!    build with assets, so a non-web `Serve` program lowers byte-identically.
//!
//! The content-type map is the ONE shared [`crate::commands::serve_web::content_type_for`]
//! the dev server uses, so a page served in dev and the same page baked into the
//! binary carry the same `Content-Type`.

use std::path::{Path, PathBuf};

use crate::commands::emit::browser_host_js;
use crate::commands::serve_web::content_type_for;

/// The build-time error a broken page raises: the page path plus the compiler's
/// Socratic display, so `largo build` fails LOUDLY (never bundles a broken site).
#[derive(Debug)]
pub(crate) struct BundleError(pub String);

impl std::fmt::Display for BundleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for BundleError {}

/// One bundled asset: the root-relative URL path it is served at, the
/// content-type baked at build time, and its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Asset {
    /// The root-relative URL path (`/`, `/app.wasm`, `/style.css`).
    pub path: String,
    /// The content-type, computed from the path via [`content_type_for`].
    pub content_type: String,
    /// The asset bytes.
    pub bytes: Vec<u8>,
}

impl Asset {
    /// A page/static asset served at `path`, its content-type derived from the
    /// path so dev and prod agree by construction. `/` (the front page) is
    /// special-cased to `text/html`.
    fn new(path: impl Into<String>, bytes: Vec<u8>) -> Self {
        let path = path.into();
        let content_type = if path == "/" || path == "/index.html" {
            "text/html; charset=utf-8".to_string()
        } else {
            content_type_for(&path).to_string()
        };
        Asset {
            path,
            content_type,
            bytes,
        }
    }

    /// A DOCUMENT this build produced, served at `path`.
    ///
    /// ★ A PAGE IS HTML BECAUSE IT IS ONE, not because its URL ends in `.html`. A page's
    /// canonical URL has no extension — `/about`, `/docs/guide` — so deriving the type
    /// from the path typed it `application/octet-stream`, and a browser handed that
    /// **offers the file as a download** rather than rendering it. The front page
    /// escaped only because `/` was special-cased; every link off it did not.
    ///
    /// Guessing from the path stays right for `static/**`, where the file name is
    /// genuinely all there is to go on. It is never right for something the build itself
    /// produced and therefore knows the type of.
    fn html(path: impl Into<String>, bytes: Vec<u8>) -> Self {
        Asset {
            path: path.into(),
            content_type: "text/html; charset=utf-8".to_string(),
            bytes,
        }
    }
}

/// A Rust string-literal name for the generated table's `include_bytes!`
/// arguments: an ordinary file path (relative to the generated `src/`) that
/// cargo can read at compile time. Not user-facing — the on-disk staging name.
fn staged_file_name(index: usize) -> String {
    format!("asset_{index}.bin")
}

/// Escape a URL path / content-type for a Rust `"..."` string literal. Asset
/// paths and content types are ASCII and controlled (URL paths, MIME strings),
/// but escaping keeps a stray `"`/`\` from breaking the generated module.
#[allow(dead_code)]
fn rust_str_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out
}

/// [`generate_assets_module`] for the site built from `pages_dir`, plus `USES_RELAY`: whether any of
/// its pages talks to the relay (`analysis::relay`). A page reaches the relay only through the
/// server that served it, so the binary embedding such a site answers `/_relay` itself.
pub(crate) fn site_assets_module(
    pages_dir: &Path,
    assets: &[Asset],
    patterns: &[(String, String)],
) -> String {
    let uses_relay = page_sources(pages_dir).is_ok_and(|pages| {
        pages
            .iter()
            .any(|(_, source)| logicaffeine_compile::analysis::relay::source_uses_the_relay(source))
    });
    let mut out = generate_assets_module(assets, patterns);
    out.push_str(&format!(
        "\n/// Whether a page of this site talks to the relay, so the binary answers `/_relay`.\npub const USES_RELAY: bool = {uses_relay};\n"
    ));
    out.push_str("\n/// The published path of every page document — the documents a server with a `## Session`\n");
    out.push_str("/// composes for the visitor who asked.\n");
    out.push_str("pub static PAGE_DOCUMENTS: &[&str] = &[\n");
    for path in page_document_paths(pages_dir) {
        out.push_str(&format!("    {path:?},\n"));
    }
    out.push_str("];\n\n");
    out.push_str("/// Whether `path` asks for a page document: one a page published, or a URL a parameterized\n");
    out.push_str(
        "/// page answers. The `Serve` lowering reads it only for a program with a `## Session`.\n",
    );
    out.push_str("#[allow(dead_code)]\n");
    out.push_str("pub fn is_page(path: &str) -> bool {\n");
    out.push_str("    let path = path.split('?').next().unwrap_or(path);\n");
    out.push_str("    PAGE_DOCUMENTS.contains(&path) || PAGE_PATTERNS.iter().any(|(pat, _)| matches_pattern(pat, path))\n");
    out.push_str("}\n");
    out
}

/// The path of every page document built from `pages_dir`: each page at its canonical URL and its
/// `.html` twin, a parameterized page at its one published document. An error page publishes no
/// URL anyone asks for, so it has none. `serve_web::serves_a_page` applies the same two rules
/// (`page_html_paths_at`, `page_path_matches`) to the pages it serves live.
pub(crate) fn page_document_paths(pages_dir: &Path) -> Vec<String> {
    let Ok(sources) = page_sources(pages_dir) else {
        return Vec::new();
    };
    sources
        .iter()
        .filter(|(_, source)| error_page_status(source).is_none())
        .filter_map(|(stem, source)| page_path(source, stem).ok())
        .flat_map(|path| page_html_paths_at(&path))
        .collect()
}

/// Generate the `logos_assets.rs` module source for `assets`: a static
/// `&[(path, content_type, &'static [u8])]` table whose bytes ride via
/// `include_bytes!("asset_<i>.bin")` (the staged files [`stage_assets`] writes
/// beside it in `src/`), plus a `lookup(path) -> Option<(&str, &[u8])>` fn.
///
/// An EMPTY asset list still generates a valid, empty-table module so the
/// `include!`-free import in `main.rs` always resolves; but the build only
/// emits this module (and threads the serve hook) when there ARE assets, so a
/// non-web program never sees it.
pub(crate) fn generate_assets_module(assets: &[Asset], patterns: &[(String, String)]) -> String {
    let mut out = String::new();
    out.push_str("// Generated by `largo build` — the embedded full-stack site.\n");
    out.push_str("// Each entry rides inside the binary via include_bytes!; lookup is\n");
    out.push_str("// consulted BEFORE the handler so an asset shadows a backend route.\n");
    // ★ THE CACHE HEADERS RIDE WITH THE BYTES. An asset served with only a `Content-Type` is
    // re-downloaded on every page load, and a browser has no way to ask "has this changed?" short
    // of fetching the whole thing again — which is the WebAssembly module's entire weight, every
    // navigation. The `Cache-Control` and the `ETag` are computed HERE, at build time, from the
    // bytes that are actually shipped, so the served headers cannot describe a different body.
    out.push_str("pub static ASSETS: &[(&str, &str, &str, &str, &[u8])] = &[\n");
    for (i, asset) in assets.iter().enumerate() {
        out.push_str(&format!(
            "    ({:?}, {:?}, {:?}, {:?}, include_bytes!({:?})),\n",
            // ★ `{:?}` ALREADY WRITES A RUST STRING LITERAL, escapes and all. Escaping first and
            // then formatting escapes TWICE: `"abc"` becomes the six characters `\"abc\"`, and
            // the running program gets a value with literal backslashes in it. It survived
            // unnoticed because a path and a content-type contain no quotes — until an ETag,
            // which is quoted BY THE HEADER SYNTAX, went out as `ETag: \"857c…\"`.
            asset.path,
            asset.content_type,
            cache_control_for(&asset.path),
            etag_for(&asset.bytes),
            staged_file_name(i),
        ));
    }
    out.push_str("];\n\n");

    // ★ A SEPARATE TABLE, NOT A WIDER ROW. A compressed body is a different
    // REPRESENTATION of the same resource, not another column of it: most rows have none,
    // it carries its own entity tag, and every response built from `ASSETS` is unchanged
    // by its existence. Widening the row would have made every caller of the five-tuple
    // learn about an encoding it does not use.
    out.push_str("/// The gzip representation of an asset, with its OWN ETag — two bodies\n");
    out.push_str("/// under one entity tag is how a shared cache hands a compressed body to a\n");
    out.push_str("/// client that cannot read one.\n");
    out.push_str("pub static ENCODED: &[(&str, &str, &[u8])] = &[\n");
    for (i, asset) in assets.iter().enumerate() {
        if compressed_variant(asset).is_some() {
            out.push_str(&format!(
                "    ({:?}, {:?}, include_bytes!({:?})),\n",
                asset.path,
                format!("{}-gz", etag_for(&asset.bytes).trim_matches('"')),
                staged_gz_name(i),
            ));
        }
    }
    out.push_str("];\n\n");
    out.push_str("/// The compressed form of `path`, if one is worth shipping, and its ETag.\n");
    out.push_str("pub fn encoded(path: &str) -> Option<(&'static str, &'static [u8])> {\n");
    out.push_str("    let path = path.split('?').next().unwrap_or(path);\n");
    out.push_str("    ENCODED.iter().find(|(p, _, _)| *p == path).map(|(_, et, b)| (*et, *b))\n");
    out.push_str("}\n\n");

    // ★ A SEPARATE TABLE AGAIN, for the same reason `ENCODED` is one: a pattern is a fact about
    // a PAGE, and most rows are files. `(pattern, published document)` — the document is an
    // ordinary row of `ASSETS`, so a pattern hit resolves through the same bytes, content type,
    // cache headers and ETag an exact hit would.
    out.push_str(
        "/// Every `## Page` whose path carries `:name` segments, as `(pattern, document)`.\n",
    );
    out.push_str("pub static PAGE_PATTERNS: &[(&str, &str)] = &[\n");
    for (pattern, target) in patterns {
        out.push_str(&format!("    ({pattern:?}, {target:?}),\n"));
    }
    out.push_str("];\n\n");
    out.push_str(
        "/// Does a concrete request path match a page pattern? A `:name` segment matches\n",
    );
    out.push_str(
        "/// any ONE segment; everything else must be equal and the counts must agree —\n",
    );
    out.push_str(
        "/// the same rule `web.lg`'s `routeParam` reads the parameter back out with, so a\n",
    );
    out.push_str("/// URL that reaches the page is a URL the page can bind.\n");
    out.push_str("fn matches_pattern(pattern: &str, request: &str) -> bool {\n");
    out.push_str("    let p: Vec<&str> = pattern.split('/').collect();\n");
    out.push_str("    let r: Vec<&str> = request.split('/').collect();\n");
    out.push_str("    p.len() == r.len() && p.iter().zip(r.iter()).all(|(a, b)| a.starts_with(':') || a == b)\n");
    out.push_str("}\n\n");

    out.push_str(
        "/// The embedded asset for `path`, if the site bundles one — its content-type,\n",
    );
    out.push_str("/// cache headers and bytes. Consulted before the handler (assets-first).\n");
    out.push_str("///\n");
    out.push_str(
        "/// An exact name wins; a parameterized page answers the rest of its URL family.\n",
    );
    out.push_str(
        "/// ★ EXACT FIRST, ALWAYS: a real file at `/posts/id` must never be shadowed by the\n",
    );
    out.push_str("/// pattern that happens to publish there.\n");
    out.push_str("pub fn lookup(path: &str) -> Option<(&'static str, &'static str, &'static str, &'static [u8])> {\n");
    out.push_str("    let path = path.split('?').next().unwrap_or(path);\n");
    out.push_str("    let row = |want: &str| ASSETS.iter().find(|(p, _, _, _, _)| *p == want);\n");
    out.push_str("    if let Some((_, ct, cc, et, b)) = row(path) {\n");
    out.push_str("        return Some((*ct, *cc, *et, *b));\n");
    out.push_str("    }\n");
    out.push_str(
        "    let (_, doc) = PAGE_PATTERNS.iter().find(|(pat, _)| matches_pattern(pat, path))?;\n",
    );
    out.push_str("    row(doc).map(|(_, ct, cc, et, b)| (*ct, *cc, *et, *b))\n");
    out.push_str("}\n\n");
    out.push_str(
        "/// Whether a client's `If-None-Match` already holds this ETag. The header is a\n",
    );
    out.push_str("/// LIST — a client with several variants sends them all — and `*` means any\n");
    out.push_str(
        "/// version at all; comparing the whole header against one tag would re-send a\n",
    );
    out.push_str("/// body the client has, which is the saving the ETag exists for.\n");
    out.push_str("pub fn fresh(etag: &str, if_none_match: Option<&str>) -> bool {\n");
    out.push_str("    match if_none_match {\n");
    out.push_str("        None => false,\n");
    out.push_str("        Some(h) => {\n");
    out.push_str("            let h = h.trim();\n");
    out.push_str("            h == \"*\" || h.split(',').any(|t| t.trim().trim_start_matches(\"W/\") == etag)\n");
    out.push_str("        }\n");
    out.push_str("    }\n");
    out.push_str("}\n");
    out
}

/// The staged `(file_name, bytes)` pairs [`generate_assets_module`]'s
/// `include_bytes!` arguments name, in table order — written beside the module
/// in the generated `src/`.
pub(crate) fn staged_asset_files(assets: &[Asset]) -> Vec<(String, Vec<u8>)> {
    let mut out: Vec<(String, Vec<u8>)> = assets
        .iter()
        .enumerate()
        .map(|(i, a)| (staged_file_name(i), a.bytes.clone()))
        .collect();
    // The compressed variants ride beside the originals under the same index, so the two
    // tables cannot get out of step: `asset_3.bin` and `asset_3.gz` are the same resource.
    for (i, a) in assets.iter().enumerate() {
        if let Some(gz) = compressed_variant(a) {
            out.push((staged_gz_name(i), gz));
        }
    }
    out
}

/// The staged file name of asset `index`'s compressed form.
fn staged_gz_name(index: usize) -> String {
    format!("asset_{index}.gz")
}

/// The compressed form of an asset, when one is worth shipping.
fn compressed_variant(asset: &Asset) -> Option<Vec<u8>> {
    worth_compressing(&asset.content_type)
        .then(|| gzip_asset(&asset.bytes))
        .flatten()
}

/// Collect every file under `static_dir` (recursively) into [`Asset`]s keyed by
/// the same root-relative URL path the dev router serves (`/style.css`,
/// `/img/logo.png`), sorted for a deterministic table. A missing `static/` is
/// simply no assets.
pub(crate) fn collect_static_assets(static_dir: &Path) -> Vec<Asset> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<Asset>, depth: usize) {
        if depth > 32 {
            return;
        }
        let Ok(read) = std::fs::read_dir(dir) else {
            return;
        };
        let mut entries: Vec<PathBuf> = read.flatten().map(|e| e.path()).collect();
        entries.sort();
        for path in entries {
            if path.is_dir() {
                walk(&path, root, out, depth + 1);
            } else if let (Ok(rel), Ok(bytes)) = (path.strip_prefix(root), std::fs::read(&path)) {
                let key = format!("/{}", rel.to_string_lossy().replace('\\', "/"));
                out.push(Asset::new(key, bytes));
            }
        }
    }
    let mut out = Vec::new();
    walk(static_dir, static_dir, &mut out, 0);
    out
}

/// `<link rel="stylesheet">` tags for every top-level css asset — literally the SAME rule
/// `largo serve` applies, not a copy of it, so a page built into the binary carries exactly the
/// stylesheets it carried in development. Written twice, they were free to drift into a site
/// styled in one and unstyled in the other.
fn stylesheet_links(statics: &[Asset]) -> String {
    crate::commands::serve_web::stylesheet_links_for(statics.iter().map(|a| a.path.as_str()))
}

/// The page's document with its MARKUP ALREADY IN IT.
///
/// The shell alone held one `<script>` tag, so everything a reader sees arrived only after a
/// WebAssembly module downloaded, instantiated and painted: a slow connection saw a blank page, a
/// crawler saw nothing, and a reader with scripts off saw nothing at all. For a framework whose
/// premise is that LOGOS runs in the browser, shipping a blank document is the one outcome that
/// makes the premise irrelevant.
///
/// The markup is produced by MOUNTING the page on the reference tier — the same path that defines
/// what the page means — so state, derived fields, `{if}` and `{for each}` all render. The script
/// still loads and takes over; what changed is what is there before it does.
///
/// Pre-rendering is an IMPROVEMENT, never a new way to fail: a page the renderer cannot draw
/// ships its shell and works in the browser exactly as it did before.
/// Why a page could NOT be pre-rendered, or `None` when it was — for the caller that wants to
/// SAY so at build time.
///
/// ★ LOSING THE PRE-RENDER USED TO BE SILENT. "Pre-rendering is an improvement, never a new way
/// to fail" is the right rule — a page the server walk cannot draw still ships and still works in
/// a browser — but nothing ever said it had happened, so a real page could ship an empty `<body>`
/// holding one `<script>` and nobody would know until they viewed source, or a crawler did.
///
/// ★ AND IT REPORTS THE REAL REASON, never a guess at one. Its first version named `bind` and
/// `class:` as the cause, because that is what a bisect appeared to show; the actual cause was
/// that a `## Page` never mounted on any tier. A build warning that guesses sends whoever reads
/// it to rewrite working code. This one carries the error the render actually produced.
pub(crate) fn prerender_refusal(source: &str) -> Option<String> {
    logicaffeine_compile::web_host::first_paint(source).err()
}

/// The canonical URL of the page at `path` on the site at `site` (no trailing slash): the address
/// its document names as the one to index. `None` for a parameterized path — one document answers
/// the whole family, and its canonical address depends on the id it is showing.
pub(crate) fn canonical_url(site: &str, path: &str) -> Option<String> {
    if path.contains("/:") {
        return None;
    }
    Some(if path == "/" {
        format!("{site}/")
    } else {
        format!("{site}{path}")
    })
}

/// `doc` with a `<link rel="canonical">` naming `url` in its head — unless the page's own `Head:`
/// already declares one, which is the author's decision and wins.
fn with_canonical(doc: String, url: Option<&str>) -> String {
    let Some(url) = url else { return doc };
    let head_end = doc.find("</head>").unwrap_or(doc.len());
    if doc[..head_end].contains("rel=\"canonical\"") {
        return doc;
    }
    let tag = format!("<link rel=\"canonical\" href=\"{}\">\n", xml_escape(url));
    let mut out = doc;
    out.insert_str(head_end, &tag);
    out
}

fn prerendered_html(
    source: &str,
    stem: &str,
    favicon: Option<&str>,
    canonical: Option<&str>,
    theme: logicaffeine_compile::web_host::ShellTheme,
) -> String {
    with_canonical(
        prerendered_document(source, stem, favicon, theme),
        canonical,
    )
}

/// The page's document before the site's own additions: its head and its pre-rendered body.
fn prerendered_document(
    source: &str,
    stem: &str,
    favicon: Option<&str>,
    theme: logicaffeine_compile::web_host::ShellTheme,
) -> String {
    // The head is rendered BY the page's own mount (`Head:` reads the page's state),
    // so it is only known once the markup render has run — and a page whose markup
    // cannot be drawn still gets the default shell, because pre-rendering is an
    // improvement and never a new way to fail.
    let Ok(markup) = logicaffeine_compile::web_host::first_paint(source) else {
        // ★ THE HEAD IS NOT THE BODY'S TO LOSE. This used to pass `None`, so a page whose body
        // would not draw — a mistyped `<Nav/>` is enough — shipped `<title>{stem}</title>`, the
        // FILE NAME, with no description and no social card. That is the wrong half to drop and
        // the half that does not depend on the failure: a `Head:` is static markup, and the body
        // is what used the construct that failed.
        //
        // It is also the half a READER never sees and everything else does. A browser takes over
        // and paints the body a moment later; a crawler, a link preview and a search result read
        // the head and go. "Pre-rendering is an improvement, never a new way to fail" — losing a
        // title the author wrote IS a new way to fail, just a quiet one.
        //
        // `first_paint` resets the recorder and then runs, so whatever the head render
        // managed before the body gave out is still recorded here.
        let declared = logicaffeine_compile::semantics::dom::head_html();
        return logicaffeine_compile::web_host::browser_host_html_themed(
            stem,
            declared.as_deref(),
            favicon,
            theme,
        );
    };
    let declared = logicaffeine_compile::semantics::dom::head_html();
    let shell = logicaffeine_compile::web_host::browser_host_html_themed(
        stem,
        declared.as_deref(),
        favicon,
        theme,
    );
    // The mark tells the page's mount that this markup is its own first paint, to adopt rather
    // than clear and draw again.
    let mark = logicaffeine_language::parser::lgx_lower::PRERENDERED_ATTR;
    shell.replace("<body>\n", &format!("<body {mark}=\"1\">\n{markup}\n"))
}

/// Whether `document` is a page the build shipped with its markup — the mark its mount adopts by.
pub(crate) fn ships_prerendered(document: &str) -> bool {
    document.contains(&format!(
        "<body {}=\"1\">",
        logicaffeine_language::parser::lgx_lower::PRERENDERED_ATTR
    ))
}

/// [`prerendered_html`], re-exported at the crate root as `prerender_page_html` so the SSG lock
/// drives the REAL function rather than a copy of it that could drift.
pub(crate) fn prerendered_html_pub(source: &str, stem: &str) -> String {
    prerendered_html(
        source,
        stem,
        None,
        None,
        logicaffeine_compile::web_host::ShellTheme::Default,
    )
}

/// [`prerendered_html`] for a page served out of a SITE, so it links the site's own favicon and
/// names its canonical URL. `largo serve` uses this; the two-argument form above is the
/// single-page view (`largo emit`, the SSG lock), where there is no site to read either from.
pub(crate) fn prerendered_html_in_site(
    source: &str,
    stem: &str,
    favicon: Option<&str>,
    canonical: Option<&str>,
    theme: logicaffeine_compile::web_host::ShellTheme,
) -> String {
    prerendered_html(source, stem, favicon, canonical, theme)
}

/// ★ THE URL NAMESPACE A PAGE OWNS — ONE rule, for `largo serve` and `largo build` alike.
///
/// `pages/app.lg` is the front page: it answers `/` AND `/index.html`, because `/index.html` is
/// what a static host serves for a directory and what a person types when a link goes stale.
/// Every page also answers `/<stem>` and `/<stem>.html`, with its module at `/<stem>.wasm` and
/// its host shim at `/<stem>.js`.
///
/// IT WAS WRITTEN TWICE AND THEY HAD ALREADY DIVERGED: the dev server answered `/index.html`
/// (`path == "/" || path == "/index.html"`) and the BUILT site did not publish it — its own
/// comment claimed it did — so a link to `/index.html` worked all through development and 404'd
/// the moment the site shipped. That is the worst shape a divergence takes: invisible until
/// production, and visible there to everyone.
pub(crate) fn page_html_paths(stem: &str) -> Vec<String> {
    page_html_paths_at(&format!("/{}", if stem == "app" { "" } else { stem }))
}

/// ★ THE URLs A PAGE AT `path` ANSWERS.
///
/// `/` answers `/` and `/index.html` — the latter is what a static host serves for a directory and
/// what a person types when a link goes stale. Every other page answers its path and its `.html`
/// twin, so `/about` and `/about.html` are the same page.
/// The URL-and-filesystem-safe form of a page's declared path: a `:name` segment keeps the
/// parameter's NAME and loses the colon, so `/posts/:id` publishes under `/posts/id`.
///
/// ★ A COLON IS NOT SAFE IN A FILE NAME OR A URL. A parameterized page used to publish its
/// document, module and shim at the pattern verbatim — `/posts/:id.<hash>.wasm` — which is an
/// illegal filename on Windows, is percent-encoded inconsistently by CDNs, and is a URL no
/// visitor could ever type. The page compiled, bound its parameter correctly, and shipped
/// assets nothing could load.
/// Every parameterized page in `pages_dir`, as `(declared pattern, published document path)`.
///
/// ★ ONE LIST, THREE DELIVERY PATHS. The dev server matches these in `route_local`, the built
/// binary bakes them into `PAGE_PATTERNS`, and a static host reads them out of `_redirects`.
/// Computing them three times is how a page comes to work in development and 404 in production.
pub(crate) fn page_patterns(pages_dir: &Path) -> Vec<(String, String)> {
    let Ok(sources) = page_sources(pages_dir) else {
        return Vec::new();
    };
    sources
        .iter()
        .filter_map(|(stem, source)| {
            let path = page_path(source, stem).ok()?;
            path.contains("/:")
                .then(|| (path.clone(), page_asset_base(&path)))
        })
        .collect()
}

pub(crate) fn page_asset_base(path: &str) -> String {
    if !path.contains("/:") {
        return path.to_string();
    }
    let safe: Vec<String> = path
        .split('/')
        .map(|seg| seg.strip_prefix(':').unwrap_or(seg).to_string())
        .collect();
    safe.join("/")
}

/// Does a concrete request path match a page's declared path?
///
/// A `:name` segment matches any ONE segment; every other segment must be equal and the counts
/// must agree. ★ THIS IS `web.lg`'s `matchRoute` RULE, and it has to stay that rule: the page
/// binds its parameters with `routeParam(pattern, name, routePath())`, so if the build tool
/// routed by a different rule than the page reads by, a visitor could reach a page that then
/// binds nothing. `page_pattern_routing_lock` pins the two against each other.
pub(crate) fn page_path_matches(pattern: &str, request: &str) -> bool {
    if !pattern.contains("/:") {
        return pattern == request;
    }
    let p: Vec<&str> = pattern.split('/').collect();
    let r: Vec<&str> = request.split('/').collect();
    p.len() == r.len()
        && p.iter()
            .zip(r.iter())
            .all(|(a, b)| a.starts_with(':') || a == b)
}

pub(crate) fn page_html_paths_at(path: &str) -> Vec<String> {
    // A parameterized page publishes ONE document — the same bytes answer every id, because the
    // id is read from the location at run time — at the safe form of its own path.
    let safe = page_asset_base(path);
    let path = safe.trim_end_matches('/');
    if path.is_empty() {
        return vec!["/".to_string(), "/index.html".to_string()];
    }
    vec![path.to_string(), format!("{path}.html")]
}

/// ★ WHERE A PAGE SERVES: its `## Page /path` header if it declares one, else its file name.
///
/// The header was parsed, carried through the AST as `ComponentKind::Page { path, .. }`, and NEVER
/// READ — the URL came entirely from the file name, so a page declaring `/dashboard/settings`
/// served at `/app` and every link the author wrote to the address they chose 404'd. A declaration
/// the runtime does not honour is the worst outcome for whoever wrote it.
///
/// The file name stays the DEFAULT, because a scaffold should not need a header to work and every
/// existing project relies on `app.lg` being the front page. The header OVERRIDES it: convention
/// where you want none, a declaration where you do.
pub(crate) fn page_path(source: &str, stem: &str) -> Result<String, BundleError> {
    match declared_page_path(source) {
        Some(declared) => {
            if !declared.starts_with('/') {
                // A path is absolute or it is not a path. Resolving `about` to something plausible
                // would serve the page at a URL nobody can predict.
                return Err(BundleError(format!(
                    "pages/{stem}.lg declares `## Page {declared}`, but a page's path must start \
                     with `/` — it is the URL people type, so it is absolute. Did you mean \
                     `/{declared}`?"
                )));
            }
            Ok(declared)
        }
        None => Ok(format!("/{}", if stem == "app" { "" } else { stem })),
    }
}

/// The path a source's `## Page` header declares, if it has one.
fn declared_page_path(source: &str) -> Option<String> {
    use logicaffeine_language::ast::component::ComponentKind;
    use logicaffeine_language::ast::stmt::Stmt;
    logicaffeine_compile::ui_bridge::with_parsed_program(source, |parsed, interner| {
        let (stmts, _types, _policies) = parsed.ok()?;
        for stmt in stmts {
            if let Stmt::ComponentDef(def) = stmt {
                if let ComponentKind::Page { path, .. } = &def.kind {
                    return Some(interner.resolve(*path).to_string());
                }
            }
        }
        None
    })
}

/// Compile one `pages/<stem>.lg` (its assembled closure) into the prod asset trio
/// — `wasm` + host `js` + host `html` (site stylesheets linked) —
/// and file them under the dev router's URL namespace: `pages/app.lg` serves the
/// front page at `/` (and `/app`, `/app.html`), every page at `/<stem>*`. Unlike
/// `largo serve`, NO reload client is injected — this is the shipped site.
///
/// A page that fails to compile is a LOUD build error — the rendered diagnostic at the page's own
/// `file:line:col` — never a silently-dropped or overlay'd asset.
fn build_page_assets(
    page: &PageClosure,
    css_links: &str,
    favicon: Option<&str>,
    site: Option<&str>,
    out: &mut Vec<Asset>,
) -> Result<(), BundleError> {
    let (source, stem) = (page.closure.source.as_str(), page.stem.as_str());
    let path = page_path(source, stem)?;
    let canonical = site.and_then(|site| canonical_url(site, &path));
    // TODO(S16): the static/embedded-release build has no `Manifest` in scope here yet — wired to
    // `ShellTheme::Default` until `plan_site`'s callers (`commands/build.rs`, `project/build.rs`)
    // thread the project's own `[web]` opt-out through.
    let document = prerendered_html(
        source,
        stem,
        favicon,
        canonical.as_deref(),
        logicaffeine_compile::web_host::ShellTheme::Default,
    );
    // A page shipped with its markup is built to adopt it; one the build could not pre-render draws
    // fresh, and its module carries no hydration.
    let entries = match ships_prerendered(&document) {
        true => logicaffeine_compile::vm::reach::HostEntries::default(),
        false => logicaffeine_compile::vm::reach::HostEntries::default().drawing_fresh(),
    };
    let wasm =
        logicaffeine_compile::compile::compile_to_wasm_with(source, &entries).map_err(|pe| {
            let at = crate::diagnostics::Placed::locate(
                pe.span,
                source,
                &page.closure.source_map,
                &page.path,
            );
            BundleError(format!(
                "pages/{stem}.lg does not build:\n{}",
                crate::diagnostics::render_parse_error(&pe, &at)
            ))
        })?;
    // ★ CONTENT-ADDRESSED NAMES, AND EVERY REFERENCE REWRITTEN WITH THEM.
    //
    // The module is the largest thing a page ships and the least likely to change between visits,
    // so it is the one that must be cacheable forever — which means its NAME has to change when
    // its bytes do. The rename is only half of it: a hash nobody links to is a 404, so the shim's
    // reference to the module and the page's reference to the shim are rewritten in the same
    // breath. The two are done together on purpose; a rename that forgets the reference ships a
    // site whose module never loads, which is worse than no caching at all.
    // Where this page LIVES — its declared `## Page /path`, else its file name. Its module and
    // shim hang off the same path, so a page at `/dashboard/settings` does not put its module at
    // the site root where another page's could collide with it.
    let base = if path == "/" {
        format!("/{stem}")
    } else {
        page_asset_base(&path)
    };
    let wasm_path = hashed_path(&format!("{base}.wasm"), &wasm);
    let wasm_name = wasm_path.trim_start_matches('/').to_string();
    let js = browser_host_js(stem).replace(&format!("{stem}.wasm"), &wasm_name);
    let js_bytes = js.into_bytes();
    let js_path = hashed_path(&format!("{base}.js"), &js_bytes);
    let js_name = js_path.trim_start_matches('/').to_string();
    let html = document
        .replace("</head>", &format!("{css_links}</head>"))
        .replace(&format!("{stem}.js"), &js_name);

    // The HTML routes come from the ONE rule the dev server also follows, so a URL that works in
    // development is a URL the shipped site serves. The PAGE keeps its own name — it is what
    // people link to, so its bytes change under it and it is revalidated rather than cached.
    for p in page_html_paths_at(&path) {
        out.push(Asset::html(p, html.clone().into_bytes()));
    }
    out.push(Asset::new(wasm_path, wasm));
    out.push(Asset::new(js_path, js_bytes));
    Ok(())
}

/// Every `pages/<stem>.lg` as `(stem, file)`, sorted by stem — the ONE enumeration `largo serve`,
/// `largo build`, `largo check` and `largo test` share, so they read the same page set.
pub(crate) fn page_files(pages_dir: &Path) -> Result<Vec<(String, PathBuf)>, BundleError> {
    let mut entries: Vec<(String, PathBuf)> = Vec::new();
    // ★ A MISSING DIRECTORY AND AN UNREADABLE ONE ARE DIFFERENT ANSWERS. A site with `static/` and
    // no `pages/` has no pages, which is ordinary and not a fault; a `pages/` that EXISTS and
    // cannot be read is permissions or a broken mount, and answering it with zero pages would ship
    // a site with every page missing and call it checked. `NotFound` is answered here so no caller
    // owes an `is_dir()` test it could silently forget.
    let read = match std::fs::read_dir(pages_dir) {
        Ok(read) => read,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => {
            return Err(BundleError(format!(
                "could not read {}: {e}",
                pages_dir.display()
            )))
        }
    };
    for f in read {
        // A per-ENTRY error is the same fault one level down; dropping it would skip one page
        // silently.
        let f = f.map_err(|e| {
            BundleError(format!(
                "could not read an entry of {}: {e}",
                pages_dir.display()
            ))
        })?;
        let path = f.path();
        if path.extension().and_then(|e| e.to_str()) == Some("lg") {
            if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                entries.push((stem.to_string(), path));
            }
        }
    }
    entries.sort();
    Ok(entries)
}

/// One page and the closure it compiles as: the page plus every module it imports.
pub(crate) struct PageClosure {
    /// The file stem — `app` for `pages/app.lg`.
    pub stem: String,
    /// The page's file.
    pub path: PathBuf,
    /// Its assembled closure, whose `SourceMap` places a diagnostic back in the file it is about.
    pub closure: crate::closure::Closure,
}

/// Every page with its assembled closure, sorted by stem. A page that will not assemble is an
/// error naming it.
pub(crate) fn page_closures(pages_dir: &Path) -> Result<Vec<PageClosure>, BundleError> {
    page_files(pages_dir)?
        .into_iter()
        .map(|(stem, path)| {
            let closure = crate::closure::assemble_closure(&path)
                .map_err(|e| BundleError(format!("pages/{stem}.lg: {e}")))?;
            Ok(PageClosure {
                stem,
                path,
                closure,
            })
        })
        .collect()
}

/// Every page's assembled source, sorted by stem.
pub(crate) fn page_sources(pages_dir: &Path) -> Result<Vec<(String, String)>, BundleError> {
    Ok(page_closures(pages_dir)?
        .into_iter()
        .map(|p| (p.stem, p.closure.source))
        .collect())
}

/// Plan the WHOLE embedded site: compile every page, collect every static asset,
/// and merge them into ONE flat table in the dev router's URL namespace.
///
/// Precedence matches `serve_web::route_local`: **a static asset SHADOWS a page**
/// (an exact static match wins). We realize that with first-match-wins `lookup`
/// by placing statics FIRST — a `static/app.js` overrides the built page's
/// module, exactly as in dev.
/// ★ A CONTENT HASH — the short digest an asset's file name carries.
///
/// The name IS the cache rule: a different body is a different URL, which is what makes
/// `immutable` a statement about what the URL means rather than a promise about the future. So the
/// digest must be OF THE BYTES and nothing else — same bytes, same name, on every machine and
/// every run, or an unchanged rebuild pointlessly busts every visitor's cache.
///
/// FNV-1a, 64 bits, rendered as 16 hex characters. Not a cryptographic hash and not asked to be:
/// nothing here defends against a chosen collision, it only has to tell one build's bytes from
/// another's, and a URL is not a signature.
pub(crate) fn content_hash(bytes: &[u8]) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in bytes {
        h ^= *b as u64;
        h = h.wrapping_mul(0x1000_0000_01b3);
    }
    format!("{h:016x}")
}

/// An asset's `ETag` — its content hash, quoted as the header syntax requires.
///
/// A browser holding this can ask "has it changed?" with `If-None-Match` and be told **304** with
/// no body. Without one, the only way to ask is to fetch the whole thing again, which is what
/// every LOGOS site did on every page load.
pub(crate) fn etag_for(bytes: &[u8]) -> String {
    format!("\"{}\"", content_hash(bytes))
}

/// Whether a URL's name carries a content hash — `<stem>.<16 hex>.<ext>`.
///
/// This is the whole cache decision, and it is made on the NAME because the name is the only thing
/// a cache and a server both see. A page keeps the name people link to, so its bytes can change
/// under it; a hashed asset cannot, by construction.
fn is_content_addressed(path: &str) -> bool {
    let file = path.rsplit('/').next().unwrap_or(path);
    let mut parts: Vec<&str> = file.split('.').collect();
    if parts.len() < 3 {
        return false;
    }
    parts.pop();
    let digest = parts.pop().unwrap_or("");
    digest.len() == 16 && digest.chars().all(|c| c.is_ascii_hexdigit())
}

/// The `Cache-Control` an asset is served with.
///
/// ★ TWO RULES, AND THEY MUST BE OPPOSITE. A content-addressed asset is cached for a year and
/// never revalidated — safe precisely because a different body would be a different URL. Anything
/// else is a URL people LINK to, whose bytes change under it, so it is revalidated every visit:
/// `immutable` there would make a deploy invisible until the cache expired, which on a year-long
/// max-age is forever.
pub(crate) fn cache_control_for(path: &str) -> String {
    if is_content_addressed(path) {
        "public, max-age=31536000, immutable".to_string()
    } else {
        "no-cache".to_string()
    }
}

/// Answer a conditional request: `(status, body length)` given the asset's ETag and the client's
/// `If-None-Match`.
///
/// `If-None-Match` is a LIST — a client holding several variants sends them all — and `*` means
/// "any version at all". Comparing the whole header against one tag would re-send a body the
/// client already has, which is the saving this exists for.
pub(crate) fn conditional_response(
    etag: &str,
    if_none_match: Option<&str>,
    len: usize,
) -> (u16, usize) {
    let Some(header) = if_none_match else {
        return (200, len);
    };
    let header = header.trim();
    if header == "*"
        || header
            .split(',')
            .any(|t| t.trim().trim_start_matches("W/") == etag)
    {
        return (304, 0);
    }
    (200, len)
}

/// Rewrite `<stem>.<ext>` to `<stem>.<hash>.<ext>` — the content-addressed name.
fn hashed_path(path: &str, bytes: &[u8]) -> String {
    match path.rsplit_once('.') {
        Some((stem, ext)) => format!("{stem}.{}.{ext}", content_hash(bytes)),
        None => path.to_string(),
    }
}

/// gzip `bytes`, or `None` when a compressed form is not worth shipping.
///
/// ★ SMALLER OR NOTHING. gzip's container is 18 bytes before any content, so a tiny or
/// already-compressed file (a PNG, a woff2) comes out LARGER — and a "compressed" variant
/// that costs bytes and a decompression step is pure loss on both ends. The margin is
/// deliberate rather than `<`: a saving of a few bytes does not pay for a second
/// representation, a second ETag and a `Vary` on every cache in the path.
pub(crate) fn gzip_asset(bytes: &[u8]) -> Option<Vec<u8>> {
    use std::io::Write;
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::best());
    enc.write_all(bytes).ok()?;
    let out = enc.finish().ok()?;
    (out.len() + 128 < bytes.len()).then_some(out)
}

/// Which content types are worth compressing.
///
/// An allowlist, not a size rule: a PNG, a JPEG, a woff2 and a `.gz` are already
/// compressed, and running deflate over them spends CPU to produce something bigger. A
/// list of what IS worth it can only fail by being too small, which costs bandwidth; a
/// rule that guessed could ship every image twice.
fn worth_compressing(content_type: &str) -> bool {
    const COMPRESSIBLE: &[&str] = &[
        "text/",
        "application/wasm",
        "application/json",
        "application/javascript",
        "image/svg+xml",
        "application/xml",
        "text/xml",
    ];
    COMPRESSIBLE.iter().any(|c| content_type.starts_with(c))
}

/// Drop the module `<script>` from a document.
///
/// An error page is content and nothing else — there is no page module to load, and a
/// `<script src>` pointing at a file the build never produced is a console error on the
/// one page a visitor reaches when something has already gone wrong.
fn strip_module_script(doc: &str) -> String {
    doc.lines()
        .filter(|l| !l.trim_start().starts_with("<script type=\"module\""))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The HTTP status an `## Error page` file declares, if the file is one.
///
/// A `pages/` file holding an `## Error page` is not a page: it has no URL of its own, so
/// it must be told apart from its neighbours BEFORE anything asks it for a path.
pub(crate) fn error_page_status(source: &str) -> Option<u16> {
    use logicaffeine_language::ast::component::ComponentKind;
    use logicaffeine_language::ast::stmt::Stmt;
    logicaffeine_compile::ui_bridge::with_parsed_program(source, |parsed, _interner| {
        let (stmts, _types, _policies) = parsed.ok()?;
        for stmt in stmts {
            if let Stmt::ComponentDef(def) = stmt {
                if let ComponentKind::ErrorPage { status } = &def.kind {
                    return Some(*status);
                }
            }
        }
        None
    })
}

/// Render an `## Error page` through the REAL router — the same walk a deployed server
/// uses to answer an unmatched URL — and return its markup.
///
/// ★ THROUGH THE ROUTER, NOT AROUND IT. The error page's markup is what `route()` answers
/// for a URL nothing matches; rendering it by some other path here would be a second
/// implementation, free to disagree with the one that ships. The probe route exists
/// because a program with no routes has no dispatcher to ask.
fn render_error_page(source: &str) -> Result<String, String> {
    let probe = format!(
        "{source}\n## Route GET /__lgx_probe -> Text:\n    Return \"x\".\n\n\
         ## Main\n\nShow route(\"GET\", \"/__lgx_no_such_url\").\n"
    );
    let out = logicaffeine_compile::compile::tw_outcome(&probe);
    if let Some(e) = out.error {
        return Err(e);
    }
    let markup = out.output.trim_end().to_string();
    if markup.is_empty() {
        return Err("it rendered nothing".to_string());
    }
    Ok(markup)
}

/// The document an `## Error page` publishes as `/<status>.html` — its markup rendered through
/// the router, in the site's shell with the site's stylesheets, and no module script: an error
/// page is content, and the one thing it must do is render on a host that runs nothing. `largo
/// build` ships it and `largo serve` answers with it, from this one function.
pub(crate) fn error_page_document(
    source: &str,
    status: u16,
    css_links: &str,
    favicon: Option<&str>,
    theme: logicaffeine_compile::web_host::ShellTheme,
) -> Result<String, String> {
    let markup = render_error_page(source)?;
    let doc = logicaffeine_compile::web_host::browser_host_html_themed(
        &format!("{status}"),
        None,
        favicon,
        theme,
    )
    .replace("</head>", &format!("{css_links}</head>"))
    .replace("<body>\n", &format!("<body>\n{markup}\n"));
    Ok(strip_module_script(&doc))
}

/// The icon a site's pages link, when the project ships one.
///
/// ★ SERVING A FILE IS NOT LINKING IT. A `static/favicon.ico` was already served at
/// `/favicon.ico` and every page still pointed at the built-in data URI, so the file
/// the author added did nothing and nothing said why. Browsers do probe `/favicon.ico`
/// on their own, which is exactly what made this invisible for `.ico` and broken for
/// every other format.
///
/// Order is preference, not discovery: an SVG scales to every size a browser asks for,
/// so a project that ships both means the SVG.
fn favicon_href(statics: &[Asset]) -> Option<String> {
    ["/favicon.svg", "/favicon.png", "/favicon.ico"]
        .into_iter()
        .find(|p| statics.iter().any(|a| a.path == *p))
        .map(str::to_string)
}

/// The site's own address from `Largo.toml`, with any trailing slash removed.
///
/// ★ THE TRAILING SLASH MATTERS. `https://a.com/` joined to `/about` is
/// `https://a.com//about`, which every crawler and cache treats as a different URL to
/// the one the site actually serves — so the sitemap would advertise pages that 404.
/// Normalised once, here, rather than at each join.
///
/// The manifest is found by WALKING UP from `pages/`, the same way a bare `Import`
/// resolves its project, so `largo build`, `largo serve` and a plain `plan_site` cannot
/// disagree about which project this is.
pub(crate) fn site_url(pages_dir: &Path) -> Option<String> {
    let mut dir = pages_dir.parent();
    while let Some(d) = dir {
        if d.join("Largo.toml").is_file() {
            return crate::project::manifest::Manifest::load(d)
                .ok()
                .and_then(|m| m.package.site)
                .map(|u| u.trim_end_matches('/').to_string())
                .filter(|u| !u.is_empty());
        }
        dir = d.parent();
    }
    None
}

/// `robots.txt`, generated when the project ships none.
///
/// Permissive: a site somebody built and deployed is a site they want read. The one
/// thing it adds beyond that is the pointer to the sitemap, which is how a crawler
/// finds one without being told where to look.
fn generated_robots(site: Option<&str>) -> String {
    let mut out = String::from("User-agent: *\nAllow: /\n");
    if let Some(url) = site {
        out.push_str(&format!("\nSitemap: {url}/sitemap.xml\n"));
    }
    out
}

/// `sitemap.xml` over a site's canonical page URLs.
///
/// ★ ONE PAGE, ONE `<loc>`. Every page also answers a `.html` twin (and the front page
/// answers `/index.html`), and listing those beside the canonical URL asks a crawler to
/// index one page under two addresses and split its ranking between them. The canonical
/// URL is the page's own declared path, which is the one thing a person links.
fn generated_sitemap(url: &str, paths: &[String]) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for p in paths {
        // The root is spelled `https://site/`, not `https://site` — a bare origin with
        // no path is not a URL a `<loc>` may carry.
        let loc = if p == "/" {
            format!("{url}/")
        } else {
            format!("{url}{p}")
        };
        out.push_str(&format!("  <url><loc>{}</loc></url>\n", xml_escape(&loc)));
    }
    out.push_str("</urlset>\n");
    out
}

/// The five characters XML reserves. A page path can carry an `&` (a query never
/// reaches here, but a path segment may), and an unescaped one makes the document
/// malformed — which a crawler discards WHOLE rather than partially.
fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

pub(crate) fn plan_site(pages_dir: &Path, static_dir: &Path) -> Result<Vec<Asset>, BundleError> {
    let statics = collect_static_assets(static_dir);
    let css_links = stylesheet_links(&statics);

    // ★ TWO PAGES AT ONE URL IS A LOUD ERROR, checked BEFORE anything is built. Which page you got
    // would otherwise depend on the order the directory happened to be read in — a decision nobody
    // made, that changes between machines — and the build would ship a site serving the wrong page.
    //
    // A site may legitimately have no `pages/` at all — a backend-rendered site whose only bundled
    // files are `static/` — and `page_files` answers that with no pages, while a `pages/` that
    // exists and cannot be read fails the build.
    let all_pages = page_closures(pages_dir)?;
    // ★ AN ERROR PAGE IS NOT A PAGE. It has no URL people link — it is what somebody gets
    // when they asked for something else — so it must not claim `/<stem>`, must not reach
    // the duplicate-URL check, and must not appear in the sitemap.
    let (error_pages, pages): (Vec<_>, Vec<_>) = all_pages
        .into_iter()
        .partition(|p| error_page_status(&p.closure.source).is_some());
    let mut claimed: std::collections::BTreeMap<String, String> = std::collections::BTreeMap::new();
    for PageClosure { stem, closure, .. } in &pages {
        let source = &closure.source;
        // ★ SAY IT WHEN A PAGE SHIPS EMPTY. The fallback is deliberate and stays — a page the
        // server walk cannot draw still works in a browser — but a SILENT fallback means a real
        // site goes out with a `<body>` holding one `<script>`, and the author finds out from a
        // crawler. A `bind` or a `class:` toggle is enough to trigger it, which is most real
        // pages. Naming it costs one line and removes the surprise.
        if let Some(why) = prerender_refusal(source) {
            eprintln!(
                "warning: pages/{stem}.lg is not pre-rendered — its `Head:` still ships, so links \
                 and search results are unaffected, but the BODY fills in only once its module \
                 loads, so a reader with no script sees an empty page. The render said: {why}"
            );
        }
        let path = page_path(source, stem)?;
        // ★ CLAIM THE PUBLISHED PATH, NOT THE PATTERN. `/posts/:id` publishes at `/posts/id`, so
        // a literal `## Page /posts/id` in another file would land on the same bytes; keying the
        // claim on the raw pattern would let the two through and ship whichever was read last.
        if let Some(other) = claimed.insert(page_asset_base(&path), stem.clone()) {
            return Err(BundleError(format!(
                "pages/{other}.lg and pages/{stem}.lg both serve `{path}` — one URL is one page, \
                 and which you got would depend on the order the directory was read in. Give one \
                 of them a different `## Page` path."
            )));
        }
    }

    let favicon = favicon_href(&statics);
    let site = site_url(pages_dir);
    let mut page_assets = Vec::new();
    for page in &pages {
        build_page_assets(
            page,
            &css_links,
            favicon.as_deref(),
            site.as_deref(),
            &mut page_assets,
        )?;
    }

    // ★ THE FILE A STATIC HOST SERVES FOR AN UNMATCHED URL. There is no handler on a CDN,
    // so a declared error page has to exist as `/<status>.html` — the name GitHub Pages,
    // Netlify, Cloudflare Pages and S3 all look for. The binary answers it through the
    // router; this is the same page, delivered the other way.
    for PageClosure { stem, closure, .. } in error_pages {
        let status = error_page_status(&closure.source).expect("partitioned on exactly this");
        // TODO(S16): unwired to the manifest here — see `build_page_assets`'s own TODO above.
        let doc = error_page_document(
            &closure.source,
            status,
            &css_links,
            favicon.as_deref(),
            logicaffeine_compile::web_host::ShellTheme::Default,
        )
        .map_err(|why| {
            // Loud, like every other build refusal — a declared error page that silently
            // did not ship is the defect this whole feature exists to end.
            BundleError(format!(
                "pages/{stem}.lg declares `## Error page {status}` and it cannot be \
                 rendered: {why}"
            ))
        })?;
        page_assets.push(Asset::html(format!("/{status}.html"), doc.into_bytes()));
    }

    // ★ GENERATED FILLS A GAP; IT NEVER OVERRULES A DECISION. These are pushed onto the
    // PAGE side of the table, which the static merge below skips for any path the
    // project already claimed — so a hand-written `robots.txt` wins by the same rule
    // that lets a static file shadow a page.
    page_assets.push(Asset::new(
        "/robots.txt".to_string(),
        generated_robots(site.as_deref()).into_bytes(),
    ));
    if let Some(url) = &site {
        // `claimed` is the canonical path of every page — one `<loc>` each, never the
        // `.html` twins, which are the same page at a second address.
        let paths: Vec<String> = claimed.keys().cloned().collect();
        page_assets.push(Asset::new(
            "/sitemap.xml".to_string(),
            generated_sitemap(url, &paths).into_bytes(),
        ));
    }

    // Statics first (they shadow pages under first-match-wins lookup); then each
    // page asset whose path a static did not already claim.
    let mut table = statics;
    let claimed: std::collections::HashSet<String> = table.iter().map(|a| a.path.clone()).collect();
    for asset in page_assets {
        if !claimed.contains(&asset.path) {
            table.push(asset);
        }
    }
    Ok(table)
}

#[cfg(test)]
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_cli/commands/web_bundle/tests.rs"]
mod tests;

#[cfg(test)]
#[path = "../../../../crates/logicaffeine_tests/unit/logicaffeine_cli/commands/web_bundle/pages_dir_tests.rs"]
mod pages_dir_tests;
