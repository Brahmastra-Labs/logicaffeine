use super::*;

#[test]
fn an_asset_derives_its_content_type_from_the_path() {
    assert_eq!(
        Asset::new("/style.css", vec![]).content_type,
        "text/css; charset=utf-8"
    );
    assert_eq!(
        Asset::new("/app.wasm", vec![]).content_type,
        "application/wasm"
    );
    assert_eq!(
        Asset::new("/app.js", vec![]).content_type,
        "text/javascript"
    );
    // The front page and its index alias are HTML even though `/` has no extension.
    assert_eq!(
        Asset::new("/", vec![]).content_type,
        "text/html; charset=utf-8"
    );
    assert_eq!(
        Asset::new("/index.html", vec![]).content_type,
        "text/html; charset=utf-8"
    );
}

#[test]
fn the_generated_module_embeds_via_include_bytes_and_looks_up_before_returning() {
    let assets = vec![
        Asset::new("/", b"<h1>home</h1>".to_vec()),
        Asset::new("/style.css", b".card{}".to_vec()),
    ];
    let src = generate_assets_module(&assets, &[]);
    // The table embeds the bytes into the binary (no runtime disk read).
    assert!(
        src.contains("include_bytes!(\"asset_0.bin\")"),
        "front page rides via include_bytes:\n{src}"
    );
    assert!(
        src.contains("include_bytes!(\"asset_1.bin\")"),
        "css rides via include_bytes:\n{src}"
    );
    // The baked content types come from the shared map.
    assert!(
        src.contains("\"text/html; charset=utf-8\""),
        "front page ct baked:\n{src}"
    );
    assert!(
        src.contains("\"text/css; charset=utf-8\""),
        "css ct baked:\n{src}"
    );
    // The paths are the dev router's URL namespace.
    assert!(src.contains("\"/\""), "front page path:\n{src}");
    assert!(src.contains("\"/style.css\""), "css path:\n{src}");
    // A lookup fn exists and strips the query string (dev-router parity).
    assert!(
        src.contains("pub fn lookup(path: &str)"),
        "lookup is public:\n{src}"
    );
    assert!(
        src.contains("split('?')"),
        "lookup ignores the query string:\n{src}"
    );
}

#[test]
fn the_embedded_table_answers_a_parameterized_pages_whole_url_family() {
    // ★ THE THIRD DELIVERY PATH. The dev server matches `## Page /posts/:id` in
    // `route_local`, and a static host learns it from `_redirects` — but a
    // `largo build --release` binary serves from this generated table, whose `lookup`
    // was an EXACT match. So the one artifact meant for production was the one that
    // 404'd on `/posts/42`, while both other paths served it.
    let assets = vec![Asset::html(
        "/posts/id".to_string(),
        b"<p>post</p>".to_vec(),
    )];
    let patterns = vec![("/posts/:id".to_string(), "/posts/id".to_string())];
    let src = generate_assets_module(&assets, &patterns);
    assert!(
        src.contains("PAGE_PATTERNS"),
        "the patterns ride as their own table, not a wider asset row:\n{src}"
    );
    assert!(
        src.contains("\"/posts/:id\""),
        "the pattern the visitor's URL is matched against:\n{src}"
    );
    assert!(
        src.contains("fn matches_pattern"),
        "and the matcher that reads it — one segment per `:name`:\n{src}"
    );
}

#[test]
fn a_site_with_no_patterns_still_generates_a_valid_table() {
    let src = generate_assets_module(&[], &[]);
    assert!(
        src.contains("PAGE_PATTERNS"),
        "the table exists but is empty:\n{src}"
    );
}

#[test]
fn an_empty_site_still_generates_a_valid_empty_table() {
    let src = generate_assets_module(&[], &[]);
    // The table gained its cache headers: `(path, content-type, Cache-Control, ETag, bytes)`.
    // An asset served with only a content-type is re-downloaded on every page load.
    assert!(
        src.contains("pub static ASSETS: &[(&str, &str, &str, &str, &[u8])] = &[\n];"),
        "empty table:\n{src}"
    );
    assert!(
        src.contains("pub fn lookup(path: &str)"),
        "lookup still present:\n{src}"
    );
    assert!(
        !src.contains("include_bytes!(\""),
        "no asset is embedded:\n{src}"
    );
}

#[test]
fn staged_files_name_the_include_bytes_arguments_in_table_order() {
    let assets = vec![
        Asset::new("/", b"A".to_vec()),
        Asset::new("/b.css", b"BB".to_vec()),
    ];
    let staged = staged_asset_files(&assets);
    assert_eq!(staged.len(), 2);
    assert_eq!(staged[0].0, "asset_0.bin");
    assert_eq!(staged[0].1, b"A");
    assert_eq!(staged[1].0, "asset_1.bin");
    assert_eq!(staged[1].1, b"BB");
    let src = generate_assets_module(&assets, &[]);
    for (name, _) in &staged {
        assert!(
            src.contains(&format!("include_bytes!({name:?})")),
            "{name} is an include arg:\n{src}"
        );
    }
}

#[test]
fn static_assets_are_collected_by_root_relative_url_path_sorted() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("style.css"), b".x{}").unwrap();
    std::fs::create_dir_all(dir.path().join("img")).unwrap();
    std::fs::write(dir.path().join("img/logo.png"), vec![1, 2, 3]).unwrap();

    let assets = collect_static_assets(dir.path());
    let paths: Vec<&str> = assets.iter().map(|a| a.path.as_str()).collect();
    assert_eq!(
        paths,
        vec!["/img/logo.png", "/style.css"],
        "keyed by URL path, sorted"
    );
    let css = assets.iter().find(|a| a.path == "/style.css").unwrap();
    assert_eq!(css.bytes, b".x{}");
    assert_eq!(css.content_type, "text/css; charset=utf-8");
    let png = assets.iter().find(|a| a.path == "/img/logo.png").unwrap();
    assert_eq!(png.bytes, vec![1, 2, 3]);
    assert_eq!(png.content_type, "image/png");
}

#[test]
fn a_missing_static_dir_is_simply_no_assets() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("does-not-exist");
    assert!(collect_static_assets(&missing).is_empty());
}

/// Scaffold a `pages/`+`static/` site under `dir`; return `(pages, static)`.
fn scaffold_site(dir: &Path, app_page: &str) -> (PathBuf, PathBuf) {
    let pages = dir.join("pages");
    let statics = dir.join("static");
    std::fs::create_dir_all(&pages).unwrap();
    std::fs::create_dir_all(&statics).unwrap();
    std::fs::write(pages.join("app.lg"), app_page).unwrap();
    std::fs::write(statics.join("style.css"), b".card{}").unwrap();
    (pages, statics)
}

const GOOD_PAGE: &str = "## Component App\nView:\n    <h1>hi</h1>\n\n## Main\nMount App.\n";

#[test]
fn plan_site_lays_pages_and_statics_in_the_dev_router_namespace() {
    let dir = tempfile::tempdir().unwrap();
    let (pages, statics) = scaffold_site(dir.path(), GOOD_PAGE);

    let table = plan_site(&pages, &statics).expect("a good site plans");
    let paths: std::collections::HashSet<&str> = table.iter().map(|a| a.path.as_str()).collect();

    // The app page owns the front routes — `/` and `/index.html`, and no longer `/app`: a
    // page's URL is its `## Page` path (defaulting to the file name), and `pages/app.lg` is
    // where the source lives rather than an address anybody chose (`pages_routing_lock`). Its
    // module and shim are CONTENT-ADDRESSED so they can be cached forever, which is why their
    // names are asserted by shape rather than spelled out (`asset_cache_lock`).
    for p in ["/", "/index.html"] {
        assert!(paths.contains(p), "the app page serves {p}: {paths:?}");
    }
    for ext in [".wasm", ".js"] {
        assert!(
            paths
                .iter()
                .any(|p| p.starts_with("/app.") && p.ends_with(ext)),
            "the app page serves a content-addressed {ext}: {paths:?}"
        );
    }
    // The static asset is at the site root.
    assert!(
        paths.contains("/style.css"),
        "static css at root: {paths:?}"
    );

    // The front page and the wasm module carry the right bytes/type.
    let front = table.iter().find(|a| a.path == "/").unwrap();
    assert_eq!(front.content_type, "text/html; charset=utf-8");
    // The page links the shim by its HASHED name — a hash nobody links to is a 404.
    let js = table
        .iter()
        .find(|a| a.path.starts_with("/app.") && a.path.ends_with(".js"))
        .unwrap();
    assert!(
        String::from_utf8_lossy(&front.bytes).contains(js.path.trim_start_matches('/')),
        "front page loads its module by the name that ships"
    );
    assert!(
        String::from_utf8_lossy(&front.bytes).contains("href=\"/style.css\""),
        "site css is linked"
    );
    let wasm = table
        .iter()
        .find(|a| a.path.starts_with("/app.") && a.path.ends_with(".wasm"))
        .unwrap();
    assert_eq!(wasm.content_type, "application/wasm");
    assert_eq!(&wasm.bytes[..4], b"\0asm", "a real wasm module");

    // The shipped page carries NO dev reload client. (`js` is the content-addressed shim found
    // above — the built site has no `/app.js`, which is the whole point of hashing it.)
    assert!(
        !String::from_utf8_lossy(&js.bytes).contains("/__gen"),
        "prod js has no reload poller"
    );
}

#[test]
fn a_static_asset_shadows_a_same_named_page_asset() {
    let dir = tempfile::tempdir().unwrap();
    let (pages, statics) = scaffold_site(dir.path(), GOOD_PAGE);
    // A static file with the SAME path as the built page's module.
    std::fs::write(statics.join("app.js"), b"SHADOW").unwrap();

    let table = plan_site(&pages, &statics).expect("plans");
    // First-match-wins lookup: exactly one `/app.js`, and it is the static.
    let matches: Vec<&Asset> = table.iter().filter(|a| a.path == "/app.js").collect();
    assert_eq!(matches.len(), 1, "the page's /app.js is not also present");
    // The lookup fn (as generated) would return the static — assert the first is it.
    let first = table.iter().find(|a| a.path == "/app.js").unwrap();
    assert_eq!(
        first.bytes, b"SHADOW",
        "the static asset wins over the page module"
    );
}

#[test]
fn a_broken_page_fails_the_bundle_loudly() {
    let dir = tempfile::tempdir().unwrap();
    // `h1 "oops"` (no colon) is the classic broken view the dev overlay catches.
    let (pages, statics) = scaffold_site(dir.path(), "## Main\nRender:\n    h1 \"oops\"\n");
    let err = plan_site(&pages, &statics).expect_err("a broken page must fail the build");
    assert!(
        err.0.contains("app.lg"),
        "the error names the broken page: {}",
        err.0
    );
}

#[test]
fn a_site_with_only_static_assets_and_no_pages_plans_the_statics() {
    let dir = tempfile::tempdir().unwrap();
    let pages = dir.path().join("pages");
    let statics = dir.path().join("static");
    std::fs::create_dir_all(&statics).unwrap();
    std::fs::write(statics.join("robots.txt"), b"User-agent: *").unwrap();
    // No pages/ dir at all (a backend-rendered site with only static assets).
    let table = plan_site(&pages, &statics).expect("a static-only site plans");
    assert_eq!(table.len(), 1);
    assert_eq!(table[0].path, "/robots.txt");
    assert_eq!(table[0].content_type, "text/plain; charset=utf-8");
}

#[test]
fn asset_string_literals_are_escaped() {
    // A pathological path/ct with a quote can never break the generated module.
    let asset = Asset {
        path: "/we\"ird".to_string(),
        content_type: "text/pl\"ain".to_string(),
        bytes: b"x".to_vec(),
    };
    let src = generate_assets_module(&[asset], &[]);
    assert!(src.contains("\\\""), "the quote is escaped:\n{src}");
    // The generated module still parses as one table line.
    assert!(src.contains("include_bytes!(\"asset_0.bin\")"), "{src}");
}

#[test]
fn the_page_documents_are_every_pages_published_paths_and_no_error_pages() {
    let dir = tempfile::tempdir().unwrap();
    let pages = dir.path().join("pages");
    std::fs::create_dir_all(&pages).unwrap();
    std::fs::write(
        pages.join("app.lg"),
        "## Page /\nView:\n    <h1>home</h1>\n",
    )
    .unwrap();
    std::fs::write(
        pages.join("about.lg"),
        "## Page /about\nView:\n    <h1>about</h1>\n",
    )
    .unwrap();
    std::fs::write(
        pages.join("post.lg"),
        "## Page /posts/:id\nView:\n    <h1>post</h1>\n",
    )
    .unwrap();
    std::fs::write(
        pages.join("lost.lg"),
        "## Error page 404\nView:\n    <h1>Lost</h1>\n",
    )
    .unwrap();
    let mut documents = page_document_paths(&pages);
    documents.sort();
    assert_eq!(
        documents,
        [
            "/",
            "/about",
            "/about.html",
            "/index.html",
            "/posts/id",
            "/posts/id.html"
        ],
        "each page at its URL and its `.html` twin, a parameterized page at its one document"
    );
    let src = site_assets_module(&pages, &[], &page_patterns(&pages));
    assert!(src.contains("pub fn is_page(path: &str)"), "{src}");
    assert!(src.contains("\"/posts/id.html\""), "{src}");
    assert!(
        !src.contains("\"/404.html\""),
        "an error page is not a page document:\n{src}"
    );
}
