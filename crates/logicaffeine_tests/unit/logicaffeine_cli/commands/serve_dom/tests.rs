use super::*;

fn art() -> Artifacts {
    Artifacts {
        generation: 7,
        wasm: vec![0, 97, 115, 109],
        js: "console.log(1)".into(),
        html: "<html>hi</html>".into(),
        mjs: "export {}".into(),
        error_overlay: None,
        error_text: None,
    }
}

/// [`build`] over `src` written as a one-file project's entry, the way the server builds it.
fn build_src(src: &str, generation: u64, watch: bool) -> Artifacts {
    let dir = tempfile::tempdir().unwrap();
    let entry = dir.path().join("app.lg");
    std::fs::write(&entry, src).unwrap();
    let closure = crate::closure::assemble_closure(&entry).expect("assembles");
    build(&closure, &entry, "app", generation, watch)
}

#[test]
fn route_serves_each_artifact_by_path() {
    let a = art();
    assert_eq!(route("/", "app", &a).body, b"<html>hi</html>");
    assert_eq!(route("/index.html", "app", &a).body, b"<html>hi</html>");
    assert_eq!(route("/app.wasm", "app", &a).body, vec![0, 97, 115, 109]);
    assert_eq!(route("/app.js", "app", &a).body, b"console.log(1)");
    assert_eq!(route("/app.mjs", "app", &a).body, b"export {}");
    assert_eq!(route("/__gen", "app", &a).body, b"7");
    assert_eq!(route("/nope", "app", &a).status, 404);
}

#[test]
fn wasm_and_js_carry_the_right_content_type() {
    let a = art();
    assert_eq!(
        route("/app.wasm", "app", &a)
            .headers
            .get("Content-Type")
            .unwrap(),
        "application/wasm"
    );
    assert_eq!(
        route("/app.js", "app", &a)
            .headers
            .get("Content-Type")
            .unwrap(),
        "text/javascript"
    );
}

#[test]
fn a_query_string_is_ignored_when_routing() {
    assert_eq!(route("/app.wasm?v=3", "app", &art()).status, 200);
}

#[test]
fn gen_reflects_the_generation_counter() {
    let mut a = art();
    a.generation = 42;
    assert_eq!(route("/__gen", "app", &a).body, b"42");
}

#[test]
fn watch_mode_appends_the_reload_client_to_the_js() {
    // A minimal DOM program (a bare component view) must build, and the served JS must
    // carry the `/__gen` poll client so the browser live-reloads.
    let src = "## Component App\nView:\n    <h1>hi</h1>\n\n## Main\nMount App.\n";
    let built = build_src(src, 1, true);
    assert!(built.error_overlay.is_none(), "minimal DOM app builds");
    assert!(
        built.js.contains("/__gen"),
        "watch JS must carry the reload client"
    );
    assert!(!built.wasm.is_empty(), "a wasm module was produced");
    let no_watch = build_src(src, 1, false);
    assert!(
        !no_watch.js.contains("/__gen"),
        "no reload client without --watch"
    );
}

#[test]
fn reload_client_preserves_scroll_and_flashes_a_toast() {
    let c = RELOAD_CLIENT;
    // Saves the scroll position before reloading, and restores it after.
    assert!(
        c.contains("window.scrollY") && c.contains("sessionStorage"),
        "saves scroll: {c}"
    );
    assert!(
        c.contains("window.scrollTo"),
        "restores scroll after reload"
    );
    // Flashes a subtle toast so the reload has feedback.
    assert!(
        c.contains("updated") && c.contains("createElement"),
        "shows an 'updated' toast"
    );
    assert!(
        c.contains("location.reload()") && c.contains("/__gen"),
        "still polls + reloads"
    );
    // Well-formed: balanced braces/parens so it can't break the served page.
    assert_eq!(
        c.matches('{').count(),
        c.matches('}').count(),
        "braces balance"
    );
    assert_eq!(
        c.matches('(').count(),
        c.matches(')').count(),
        "parens balance"
    );
}

#[test]
fn open_command_targets_the_current_platform() {
    let (prog, args) = open_command("http://127.0.0.1:8080");
    // The URL is always the last argument, whatever the platform's opener is.
    assert_eq!(
        args.last().map(String::as_str),
        Some("http://127.0.0.1:8080")
    );
    let expected = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(target_os = "windows") {
        "cmd"
    } else {
        "xdg-open"
    };
    assert_eq!(prog, expected, "picks this OS's default-browser opener");
}

#[test]
fn console_lines_carry_the_url_generation_and_timing() {
    let addr: std::net::SocketAddr = "127.0.0.1:8080".parse().unwrap();
    let banner = serving_line("myapp", &addr, true);
    assert!(
        banner.contains("myapp") && banner.contains("http://127.0.0.1:8080"),
        "banner: {banner}"
    );
    assert!(
        banner.contains("watching"),
        "the watch banner hints live-reload: {banner}"
    );
    assert!(
        !serving_line("myapp", &addr, false).contains("watching"),
        "no hint without --watch"
    );
    let ok = rebuilt_line(7, Duration::from_millis(12));
    assert!(
        ok.contains("12ms") && ok.contains("generation 7"),
        "rebuilt line: {ok}"
    );
    let bad = rebuild_failed_line(Duration::from_millis(5), "app.lg:3:5 unclosed <div>");
    assert!(
        bad.contains("5ms") && bad.contains("serving last good"),
        "failed line: {bad}"
    );
    assert!(
        bad.contains("app.lg:3:5 unclosed <div>"),
        "the terminal carries the error itself: {bad}"
    );
}

#[test]
fn fingerprint_sees_a_nested_module_and_skips_target_and_dot_dirs() {
    let dir = tempfile::tempdir().unwrap();
    let write = |rel: &str| {
        let p = dir.path().join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, "x").unwrap();
        p
    };
    write("src/main.lg");
    let before = source_fingerprint(dir.path(), &[]);

    // Ignored trees never move the fingerprint, however fresh their files.
    std::thread::sleep(Duration::from_millis(30));
    write("target/junk.lg");
    write(".git/blob.lg");
    assert_eq!(
        source_fingerprint(dir.path(), &[]),
        before,
        "target/ and dot-dirs are outside the watched program"
    );

    // A NESTED module edit ticks it — the recursive sweep, not read_dir(entry_dir).
    std::thread::sleep(Duration::from_millis(30));
    write("src/ui/deep/msg.lg");
    assert!(
        source_fingerprint(dir.path(), &[]) > before,
        "a nested .lg must move the fingerprint"
    );
}

#[test]
fn fingerprint_tracks_explicit_closure_files_outside_the_sweep() {
    // A closure file the sweep can't see (an imported `.md` module, or a file
    // outside the root) is still tracked through the explicit file list.
    let dir = tempfile::tempdir().unwrap();
    let module = dir.path().join("geometry.md");
    std::fs::write(&module, "x").unwrap();
    let root = dir.path().join("elsewhere");
    std::fs::create_dir_all(&root).unwrap();
    let before = source_fingerprint(&root, &[module.clone()]);
    std::thread::sleep(Duration::from_millis(30));
    std::fs::write(&module, "y").unwrap();
    assert!(
        source_fingerprint(&root, &[module]) > before,
        "an explicit closure file moves the fingerprint even outside the sweep"
    );
}

#[test]
fn an_assemble_failure_serves_a_themed_overlay() {
    let art = assemble_failure_artifacts("Failed to resolve import 'ghost.lg'", 5);
    assert_eq!(art.generation, 5);
    let resp = route("/", "app", &art);
    assert_eq!(resp.status, 200);
    let body = String::from_utf8_lossy(&resp.body);
    assert!(body.contains("LOGOS build error"), "titled overlay");
    assert!(body.contains("ghost.lg"), "carries the assembler's message");
    assert!(body.contains("/__gen"), "keeps polling so a fix snaps back");
}

#[test]
fn a_build_error_becomes_a_themed_overlay_not_a_crash() {
    // A view element missing its `:` — a bad edit serves a THEMED overlay with the
    // source line, a caret, and the Socratic message + reload client (not a crash).
    let built = build_src("## Main\nRender:\n    h1 \"hi\"\n", 3, true);
    assert!(
        built.error_overlay.is_some(),
        "a bad program yields a build error"
    );
    let resp = route("/", "app", &built);
    assert_eq!(resp.status, 200);
    let body = String::from_utf8_lossy(&resp.body);
    assert!(body.contains("LOGOS build error"), "the overlay is titled");
    assert!(
        body.contains("class=\"caret\"") && body.contains('^'),
        "shows a caret"
    );
    assert!(
        body.contains(":root {") && body.contains("--accent"),
        "is themed"
    );
    assert!(body.contains("/__gen"), "embeds the reload client");
}

// ---------------------------------------------------------------------------------------
// ★ THE RELOAD CLIENT AND `preserved across reloads` ARE ONE MECHANISM. ★
//
// T7.7 asks that a hot reload keep what you typed. That is not a feature of the dev server
// OR of the language on its own: the dev server does a real `location.reload()`, and a
// `## Store … preserved across reloads` writes its fields to `sessionStorage` — which is
// exactly the storage that survives a reload and dies with the tab. If either half changed
// independently, the promise would break with nothing anywhere saying so, so the two are
// pinned together here.
//
// The reload client had NO test of any kind before this. It is a JavaScript string in a Rust
// constant, injected into every page the dev loop serves — the shape this campaign keeps
// finding unexercised.

#[test]
fn the_reload_client_reloads_rather_than_navigating() {
    // `location.reload()` keeps the tab, and the tab is what `sessionStorage` is scoped to.
    // A navigation (`location.href = …`) would also refresh the page and would ALSO throw
    // away every `preserved across reloads` store, silently and only on the dev loop.
    assert!(
        RELOAD_CLIENT.contains("location.reload()"),
        "the dev loop must RELOAD the tab, or session-scoped state does not survive it"
    );
    assert!(
        !RELOAD_CLIENT.contains("location.href="),
        "a navigation would discard session storage — and with it every preserved store"
    );
}

#[test]
fn the_reload_clients_own_key_cannot_collide_with_a_stores() {
    // The client keeps the scroll position in session storage too. A preserved store's key is
    // `reload:<StoreName>`, and a store named `__logos_scroll` cannot be written (a `##`
    // header takes an identifier), so the two namespaces are disjoint by construction — but
    // only while the client's key keeps its prefix, which is what this pins.
    assert!(
        RELOAD_CLIENT.contains("'__logos_scroll'"),
        "the dev client's own storage key stays in its `__logos_` namespace, clear of the \
             `reload:<Store>` keys a preserved store writes"
    );
}

#[test]
fn the_reload_client_survives_storage_being_unavailable() {
    // Session storage THROWS in a private window, with site data blocked, or over a full
    // quota. A dev loop that died of that would stop reloading entirely — the one thing it
    // exists to do — so every access is wrapped.
    // MEASURED STRUCTURALLY, not by counting: one `try` block legitimately covers several
    // accesses (the client reads and then removes its key inside one), so "guards >= accesses"
    // reports a failure for correct code. What matters is that no access sits OUTSIDE a guard.
    let mut depth = 0usize;
    let mut unguarded = Vec::new();
    let bytes = RELOAD_CLIENT.as_bytes();
    for (i, _) in RELOAD_CLIENT.match_indices(|c| c == '{' || c == '}' || c == 's') {
        if bytes[i] == b'{' {
            if RELOAD_CLIENT[..i].ends_with("try") {
                depth += 1;
            }
        } else if bytes[i] == b'}' {
            if RELOAD_CLIENT[i..].starts_with("}catch") {
                depth = depth.saturating_sub(1);
            }
        } else if RELOAD_CLIENT[i..].starts_with("sessionStorage") && depth == 0 {
            unguarded.push(i);
        }
    }
    assert!(
        RELOAD_CLIENT.contains("sessionStorage"),
        "the client uses session storage"
    );
    assert!(
        unguarded.is_empty(),
        "session storage is touched outside a try/catch at byte offset(s) {unguarded:?} — a \
             throw there (private window, blocked site data, full quota) stops the dev loop \
             reloading at all"
    );
}

// ★ THE DISCRIMINATOR IS THE POLLER, NOT THE WORD `reload`. Both halves of this pair used to
// key on `location.reload()`, which stopped separating them the moment `runtime.js` began
// shipping a `dom_reload` host import in EVERY build — the unwatched half went red asserting
// the absence of a string that is now always present, and the watched half quietly became
// vacuous, asserting a string it could not fail to find. `/__gen` is the route only the dev
// loop's poller fetches, so it is the thing that is actually true of one build and not the
// other; the sibling `watch_js_polls_gen` already keyed on it.
#[test]
fn a_page_the_dev_loop_serves_carries_the_reload_client() {
    // The tie-back: the client is not merely well-formed, it is actually SHIPPED with a page.
    let built = build_src(
        "## Component App\nView:\n    <p>hi</p>\n\n## Main\nMount App.\n",
        1,
        true,
    );
    assert!(built.error_overlay.is_none(), "the page builds");
    assert!(
        built.js.contains("/__gen"),
        "a watched page ships the reload client's generation poller"
    );
}

#[test]
fn an_unwatched_build_ships_no_reload_client() {
    // The other half: a built site is not a dev loop. Shipping the poller to real visitors
    // would have every page fetching `/__gen` forever against a server that has no such route.
    let built = build_src(
        "## Component App\nView:\n    <p>hi</p>\n\n## Main\nMount App.\n",
        1,
        false,
    );
    assert!(
        !built.js.contains("/__gen"),
        "an unwatched build must not carry the dev loop's poller"
    );
}
