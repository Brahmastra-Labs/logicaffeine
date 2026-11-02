use super::*;

/// [`build_page`] over `src` written as `pages/<stem>.lg` of a throwaway project.
fn page_from(src: &str, stem: &str, css_links: &str) -> PageBundle {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("pages")).unwrap();
    let path = dir.path().join(format!("pages/{stem}.lg"));
    std::fs::write(&path, src).unwrap();
    let closure = crate::closure::assemble_closure(&path).expect("assembles");
    let page = crate::commands::web_bundle::PageClosure {
        stem: stem.to_string(),
        path,
        closure,
    };
    build_page(
        &page,
        css_links,
        None,
        None,
        logicaffeine_compile::web_host::ShellTheme::Default,
    )
}

fn state() -> SiteState {
    let mut pages = BTreeMap::new();
    pages.insert(
        "app".to_string(),
        PageBundle {
            wasm: vec![0, 97, 115, 109],
            js: "console.log('app')".into(),
            html: "<html>app page</html>".into(),
            error_overlay: None,
            error_text: None,
            path: "/".to_string(),
        },
    );
    pages.insert(
        "about".to_string(),
        PageBundle {
            wasm: vec![0, 97, 115, 109, 9],
            js: "console.log('about')".into(),
            html: "<html>about page</html>".into(),
            error_overlay: None,
            error_text: None,
            path: "/about".to_string(),
        },
    );
    let mut statics = BTreeMap::new();
    statics.insert("/style.css".to_string(), b".card{}".to_vec());
    statics.insert("/img/logo.png".to_string(), vec![1, 2, 3]);
    SiteState {
        generation: 7,
        pages,
        statics,
        backend_error: None,
        error_pages: BTreeMap::new(),
    }
}

/// ★ The dev panel shows what the SERVER knows. A page's action journal lives in the running
/// PAGE, so a separate `/__dev` document would have a journal of its own — an empty one.
/// Showing that would be a panel that looks informative and tells you nothing.
#[test]
fn the_dev_panel_lists_the_pages_and_assets_being_served() {
    let st = state();
    let body = String::from_utf8(route_local("/__dev", &st).unwrap().body).unwrap();
    assert!(
        body.starts_with("<!doctype html>"),
        "it is a document:\n{body}"
    );
    assert!(
        body.contains("generation 7"),
        "it shows what the reload client polls:\n{body}"
    );
    // Every page, at the URL it is actually served from — `app` owns `/`.
    assert!(
        body.contains(">/<"),
        "the front page is listed at `/`:\n{body}"
    );
    assert!(body.contains("/about"), "and every other page:\n{body}");
    // Every static asset, at its real path.
    assert!(
        body.contains("/style.css"),
        "static assets are listed:\n{body}"
    );
    assert!(
        body.contains("/img/logo.png"),
        "including nested ones:\n{body}"
    );
}

#[test]
fn the_dev_panel_marks_a_page_that_does_not_compile() {
    // The first thing a developer needs to see. A broken page listed as if it were fine is
    // worse than not listing it: they go looking somewhere else.
    let mut st = state();
    st.pages.get_mut("about").unwrap().error_overlay = Some("boom".to_string());
    let body = String::from_utf8(route_local("/__dev", &st).unwrap().body).unwrap();
    assert!(
        body.contains("does not compile"),
        "the broken page is marked:\n{body}"
    );
    assert!(
        body.contains("class=\"bad\""),
        "and marked visibly:\n{body}"
    );
}

#[test]
fn the_dev_panel_shows_a_backend_error() {
    let mut st = state();
    st.backend_error = Some("the backend did not build".to_string());
    let body = String::from_utf8(route_local("/__dev", &st).unwrap().body).unwrap();
    assert!(body.contains("the backend did not build"), "{body}");
}

#[test]
fn the_dev_panel_escapes_what_it_shows() {
    // Its content comes from compiler messages and file names, and it is rendered as HTML.
    // A compiler error carrying markup must not become markup.
    let mut st = state();
    st.backend_error = Some("<script>alert(1)</script>".to_string());
    let body = String::from_utf8(route_local("/__dev", &st).unwrap().body).unwrap();
    assert!(
        !body.contains("<script>alert"),
        "the error is not markup:\n{body}"
    );
    assert!(
        body.contains("&lt;script&gt;"),
        "it is escaped text:\n{body}"
    );
}

#[test]
fn the_dev_panel_lives_in_the_reserved_double_underscore_namespace() {
    // Precedence here is deliberate and already decided: `/__gen` is checked BEFORE statics,
    // so `__`-prefixed paths are the dev SERVER's namespace and a project's own asset does
    // not shadow them. `/__dev` joins that namespace rather than inventing a second rule —
    // two conflicting precedence rules in one router is how a site becomes unservable for
    // reasons nobody can find.
    //
    // It costs a project the `__` prefix, which is the point of reserving one, and it costs
    // it only in DEV: `largo build` ships no panel, so nothing here can shadow a built site.
    let mut st = state();
    st.statics.insert("/__dev".to_string(), b"mine".to_vec());
    let body = String::from_utf8(route_local("/__dev", &st).unwrap().body).unwrap();
    assert!(
        body.contains("largo serve"),
        "the dev server owns `__`-prefixed paths, as it already does for `/__gen`"
    );
    // And the reservation is exactly that prefix — nothing wider.
    st.statics.insert("/dev".to_string(), b"mine".to_vec());
    assert_eq!(
        route_local("/dev", &st).unwrap().body,
        b"mine",
        "a project's own `/dev` is its own"
    );
}

#[test]
fn an_empty_site_still_gets_a_panel() {
    let st = SiteState {
        generation: 0,
        pages: BTreeMap::new(),
        statics: BTreeMap::new(),
        backend_error: None,
        error_pages: BTreeMap::new(),
    };
    let body = String::from_utf8(route_local("/__dev", &st).unwrap().body).unwrap();
    assert!(
        body.contains("none"),
        "it says so rather than rendering an empty list:\n{body}"
    );
}

#[test]
fn router_serves_gen_statics_pages_and_defers_the_rest() {
    let st = state();
    assert_eq!(route_local("/__gen", &st).unwrap().body, b"7");
    assert_eq!(route_local("/style.css", &st).unwrap().body, b".card{}");
    assert_eq!(
        route_local("/img/logo.png", &st).unwrap().body,
        vec![1, 2, 3]
    );
    assert_eq!(
        route_local("/", &st).unwrap().body,
        b"<html>app page</html>"
    );
    assert_eq!(
        route_local("/index.html", &st).unwrap().body,
        b"<html>app page</html>"
    );
    assert_eq!(
        route_local("/about", &st).unwrap().body,
        b"<html>about page</html>"
    );
    assert_eq!(
        route_local("/about.html", &st).unwrap().body,
        b"<html>about page</html>"
    );
    assert_eq!(
        route_local("/app.wasm", &st).unwrap().body,
        vec![0, 97, 115, 109]
    );
    assert_eq!(
        route_local("/app.js", &st).unwrap().body,
        b"console.log('app')"
    );
    // A page no longer ships a `.mjs`. It was `wasm_host_shim` — the NODE command-line shim,
    // which reads `process.argv` — served on a web route where nothing ever loaded it: the
    // page's HTML has always pulled `./<stem>.js`. Dead weight on every page of every site.
    assert!(
        route_local("/app.mjs", &st).is_none(),
        "a page ships no `.mjs` shim"
    );
    // Everything unmatched defers to the proxy.
    assert!(route_local("/api/users", &st).is_none());
    assert!(route_local("/nope", &st).is_none());
    assert!(route_local("/nope.png", &st).is_none());
}

#[test]
fn a_static_asset_shadows_a_page_and_the_proxy() {
    let mut st = state();
    // The same path as a page asset — the static table wins.
    st.statics
        .insert("/app.js".to_string(), b"shadowed!".to_vec());
    assert_eq!(route_local("/app.js", &st).unwrap().body, b"shadowed!");
    // A static index.html shadows the built front page.
    st.statics
        .insert("/index.html".to_string(), b"static index".to_vec());
    assert_eq!(
        route_local("/index.html", &st).unwrap().body,
        b"static index"
    );
}

#[test]
fn a_query_string_never_affects_routing() {
    let st = state();
    assert_eq!(
        route_local("/app.wasm?v=3", &st).unwrap().body,
        vec![0, 97, 115, 109]
    );
    assert_eq!(route_local("/__gen?x=1", &st).unwrap().body, b"7");
}

#[test]
fn without_an_app_page_the_root_defers_to_the_backend() {
    let mut st = state();
    st.pages.remove("app");
    assert!(
        route_local("/", &st).is_none(),
        "the backend owns / when there is no front page"
    );
}

#[test]
fn a_broken_page_serves_its_overlay_over_the_html_route() {
    let mut st = state();
    st.pages.get_mut("app").unwrap().error_overlay = Some("<html>OVERLAY</html>".into());
    assert_eq!(route_local("/", &st).unwrap().body, b"<html>OVERLAY</html>");
    // The last-good module stays live beside it.
    assert_eq!(
        route_local("/app.wasm", &st).unwrap().body,
        vec![0, 97, 115, 109]
    );
}

#[test]
fn content_types_cover_the_web_surface() {
    assert_eq!(content_type_for("/style.css"), "text/css; charset=utf-8");
    assert_eq!(content_type_for("/a/b/app.wasm"), "application/wasm");
    assert_eq!(content_type_for("/app.js"), "text/javascript");
    assert_eq!(content_type_for("/app.mjs"), "text/javascript");
    assert_eq!(content_type_for("/index.html"), "text/html; charset=utf-8");
    assert_eq!(content_type_for("/logo.svg"), "image/svg+xml");
    assert_eq!(content_type_for("/logo.PNG"), "image/png");
    assert_eq!(content_type_for("/data.json"), "application/json");
    assert_eq!(
        content_type_for("/no-extension"),
        "application/octet-stream"
    );
    assert_eq!(content_type_for("/weird.xyz"), "application/octet-stream");
}

#[test]
fn reload_injection_targets_only_html_and_splices_before_body_close() {
    let mut html = response(
        200,
        "text/html; charset=utf-8",
        b"<html><body>hi</body></html>".to_vec(),
    );
    // Proxied headers arrive lowercased from parse_response.
    let mut headers = BTreeMap::new();
    headers.insert("content-type".to_string(), "text/html".to_string());
    html.headers = headers;
    inject_reload_into_html(&mut html);
    let text = String::from_utf8(html.body).unwrap();
    assert!(
        text.contains("/__gen"),
        "the reload client rides in: {text}"
    );
    assert!(
        text.find("/__gen").unwrap() < text.find("</body>").unwrap(),
        "spliced before </body>: {text}"
    );

    let mut plain = response(200, "text/plain", b"just text".to_vec());
    let mut headers = BTreeMap::new();
    headers.insert("content-type".to_string(), "text/plain".to_string());
    plain.headers = headers;
    inject_reload_into_html(&mut plain);
    assert_eq!(
        plain.body, b"just text",
        "non-HTML bodies are never touched"
    );

    let mut binary = response(200, "application/wasm", vec![0, 97, 115, 109]);
    let mut headers = BTreeMap::new();
    headers.insert("content-type".to_string(), "application/wasm".to_string());
    binary.headers = headers;
    inject_reload_into_html(&mut binary);
    assert_eq!(
        binary.body,
        vec![0, 97, 115, 109],
        "binary bodies are never touched"
    );
}

#[test]
fn stylesheet_links_take_top_level_css_only() {
    let mut statics = BTreeMap::new();
    statics.insert("/style.css".to_string(), Vec::new());
    statics.insert("/extra.css".to_string(), Vec::new());
    statics.insert("/deep/nested.css".to_string(), Vec::new());
    statics.insert("/logo.png".to_string(), Vec::new());
    let links = stylesheet_links(&statics);
    assert!(links.contains("href=\"/style.css\""), "{links}");
    assert!(links.contains("href=\"/extra.css\""), "{links}");
    assert!(
        !links.contains("nested.css"),
        "nested css is opt-in via the page: {links}"
    );
    assert!(!links.contains("logo.png"), "{links}");
}

#[test]
fn built_pages_carry_the_reload_client_and_the_site_stylesheets() {
    let mut statics = BTreeMap::new();
    statics.insert("/style.css".to_string(), b".x{}".to_vec());
    let links = stylesheet_links(&statics);
    let page = page_from(
        "## Component App\nView:\n    <h1>hi</h1>\n\n## Main\nMount App.\n",
        "app",
        &links,
    );
    assert!(page.error_overlay.is_none(), "a minimal view builds");
    assert!(page.js.contains("/__gen"), "the page js polls for reloads");
    assert!(
        page.html.contains("href=\"/style.css\""),
        "the site css is linked:\n{}",
        page.html
    );
    assert_eq!(&page.wasm[..4], b"\0asm");
}

#[test]
fn a_bad_page_builds_to_a_themed_overlay() {
    let page = page_from("## Main\nRender:\n    h1 \"oops\"\n", "app", "");
    let overlay = page.error_overlay.expect("a bad view is an overlay");
    assert!(overlay.contains("LOGOS build error"), "{overlay}");
    assert!(
        overlay.contains("/__gen"),
        "the overlay polls so a fix snaps back"
    );
}

/// ★ THE OVERLAY REPORTS THE PAGE'S OWN LINE, not a line in the combined source.
///
/// It draws its caret against whatever source it is handed, and that was the ASSEMBLED
/// closure — every `Import` spliced in AHEAD of the page's own text. A page importing a
/// twelve-line module reported its own line 6 as line 18 and drew the caret under a line
/// from the module: a confident pointer at innocent code, which is worse than none.
///
/// Driven through the REAL assembler rather than a hand-built map, because the whole bug is
/// which source the overlay is given — a test that constructed the offsets itself would be
/// asserting its own arithmetic.
#[test]
fn the_overlay_reports_the_pages_own_line_not_the_spliced_one() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(
        root.join("Largo.toml"),
        "[package]\nname = \"ovl\"\nversion = \"0.1.0\"\nentry = \"src/main.lg\"\n",
    )
    .unwrap();
    std::fs::write(root.join("src/main.lg"), "## Main\nShow \"ok\".\n").unwrap();
    // Twelve lines of module, spliced in first.
    std::fs::write(
        root.join("src/shared.lg"),
        "# Shared\n\n## To one () -> Int:\n    Return 1.\n\n## To two () -> Int:\n    Return 2.\n\n\
             ## To three () -> Int:\n    Return 3.\n\n## To four () -> Int:\n    Return 4.\n",
    )
    .unwrap();
    // The page's own mistake — an unclosed `<div>` — is on ITS line 7.
    let page_src = "# App\n\nImport Shared.\n\n## Component App\nView:\n    <div><p>hi</p>\n\n## Main\nMount App.\n";
    let page_path = root.join("pages/app.lg");
    std::fs::write(&page_path, page_src).unwrap();

    let closure = crate::closure::assemble_closure(&page_path).expect("assembles");
    let page = crate::commands::web_bundle::PageClosure {
        stem: "app".into(),
        path: page_path,
        closure,
    };
    let built = build_page(
        &page,
        "",
        None,
        None,
        logicaffeine_compile::web_host::ShellTheme::Default,
    );
    let overlay = built.error_overlay.expect("an unclosed tag is an overlay");

    assert!(
        overlay.contains("app.lg"),
        "the overlay must NAME the file it is drawing — a dev server serves many pages and \
             each imports several modules:\n{overlay}"
    );
    assert!(
        overlay.contains("line 7"),
        "the unclosed `<div>` is on the PAGE's line 7. Counted through the twelve-line module \
             spliced in ahead of it, it is not:\n{overlay}"
    );
    assert!(
        overlay.contains("&lt;div&gt;"),
        "and the excerpt under the caret must be the page's own line, not a line from the \
             module that happens to sit at that offset in the combined source:\n{overlay}"
    );
}

/// ★ THE LINE NUMBER IS STILL WRONG, and it is a DIFFERENT bug from the one above.
///
/// Every page build error reports **line 1**, and always has — not because the overlay reads
/// the wrong source (that is fixed above) but because it is handed a span of `(0, 0)`.
/// `ui_bridge::parse_seam` turns a real `ParseError` into a `String` via
/// `socratic_explanation` before any caller sees it, so `compile_to_wasm` has nothing but a
/// message and builds its `ParseError` with `Span::default()`.
///
/// I first wrote this as "the overlay mis-numbers a page that imports something". That was
/// wrong: an imported page and a standalone one both report line 1, so the number was never
/// off-by-a-splice — it was never computed at all.
///
/// ★ CLOSED. `ui_bridge::parse_seam_e` now hands the `ParseError` over WHOLE, and the
/// message-shaped `parse_seam` is a thin wrapper over it — so the 150-odd callers that only
/// ever wanted the sentence are untouched, and the few that point AT a location keep the span.
#[test]
fn the_overlay_reports_the_line_the_mistake_is_on() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    std::fs::create_dir_all(root.join("pages")).unwrap();
    std::fs::write(
        root.join("Largo.toml"),
        "[package]\nname = \"ovl2\"\nversion = \"0.1.0\"\nentry = \"src/main.lg\"\n",
    )
    .unwrap();
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/main.lg"), "## Main\nShow \"ok\".\n").unwrap();
    let page_path = root.join("pages/app.lg");
    std::fs::write(
        &page_path,
        "# App\n\n## Component App\nView:\n    <div><p>hi</p>\n\n## Main\nMount App.\n",
    )
    .unwrap();
    let closure = crate::closure::assemble_closure(&page_path).expect("assembles");
    let page = crate::commands::web_bundle::PageClosure {
        stem: "app".into(),
        path: page_path,
        closure,
    };
    let built = build_page(
        &page,
        "",
        None,
        None,
        logicaffeine_compile::web_host::ShellTheme::Default,
    );
    let overlay = built.error_overlay.expect("an unclosed tag is an overlay");
    assert!(
        overlay.contains("line 5"),
        "the unclosed `<div>` is on line 5, and the overlay says line 1:\n{overlay}"
    );
}
